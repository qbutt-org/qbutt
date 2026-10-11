local msg = require "mp.msg"
local options = require "mp.options"
local utils = require "mp.utils"

local user_opts = {
    override = "auto",
    probe_timeout = 8,
    network_probe_timeout = 30,
}

options.read_options(user_opts, "video_profile")

local function load_module(name)
    local path = mp.command_native({
        "expand-path", "~~/script-modules/video_profile/" .. name .. ".lua",
    })
    local chunk, load_error = loadfile(path)
    if not chunk then
        error("Failed to load video profile module " .. name .. ": " .. tostring(load_error))
    end
    return chunk()
end

local classifier = load_module("classifier")
local stream_selection = load_module("stream_selection")
local timing_probe = load_module("timing")
local network_source = load_module("network_source")
local ffprobe_path = os.getenv("QBUTT_FFPROBE")

local generation = 0
local loaded = false
local classifier_task = nil
local timing_task = nil
local track_change_timer = nil
local duration_retry_timer = nil
local last_track_signature = nil

local override_labels = {
    auto = "автоматически",
    anime = "аниме",
    movie = "фильм",
}

local function normalize_override(value)
    value = tostring(value or ""):lower():gsub("^%s+", ""):gsub("%s+$", "")
    if value == "video" or value == "film" or value == "live-action" then
        value = "movie"
    end
    if override_labels[value] then
        return value
    end
    return nil
end

local current_override = normalize_override(
    mp.get_property_native("user-data/video-profile/override", user_opts.override)
) or "auto"

local function set_profile(status, kind, confidence, reason, source, metrics)
    metrics = metrics or {}
    if status ~= "ready" then
        mp.set_property_native("user-data/video-profile/status", status)
    end
    mp.set_property_native("user-data/video-profile/kind", kind)
    mp.set_property_number("user-data/video-profile/confidence", confidence or 0)
    mp.set_property_native("user-data/video-profile/reason", reason or "")
    mp.set_property_native("user-data/video-profile/source", source or "auto")
    mp.set_property_number("user-data/video-profile/anime-score", metrics.anime_score or 0)
    mp.set_property_number("user-data/video-profile/drawn-score", metrics.drawn_score or 0)
    mp.set_property_number("user-data/video-profile/cadence-score", metrics.cadence_score or 0)
    if status == "ready" then
        mp.set_property_native("user-data/video-profile/status", status)
    end
end

local function set_timing(result, error_reason)
    result = result or {}
    local status = result.status
    if status ~= "idle"
        and status ~= "analyzing"
        and status ~= "ready"
        and status ~= "unavailable"
    then
        status = "unavailable"
    end
    local vfr = result.vfr
    if vfr ~= true and vfr ~= false then
        vfr = "unknown"
    end

    if status ~= "ready" then
        mp.set_property_native("user-data/video-profile/timing-status", status)
    end
    mp.set_property_native("user-data/video-profile/vfr", vfr)
    mp.set_property_number("user-data/video-profile/source-num", result.source_num or 0)
    mp.set_property_number("user-data/video-profile/source-den", result.source_den or 0)
    mp.set_property_number("user-data/video-profile/avg-num", result.avg_num or 0)
    mp.set_property_number("user-data/video-profile/avg-den", result.avg_den or 0)
    mp.set_property_native(
        "user-data/video-profile/timing-reason",
        result.reason or error_reason or "timing не проверен"
    )
    if status == "ready" then
        mp.set_property_native("user-data/video-profile/timing-status", status)
    end
end

local function cancel_analysis()
    generation = generation + 1
    if classifier_task then
        classifier_task.cancel()
        classifier_task = nil
    end
    if timing_task then
        timing_task.cancel()
        timing_task = nil
    end
    if duration_retry_timer then
        duration_retry_timer:kill()
        duration_retry_timer = nil
    end
end

local function selected_video_stream()
    return stream_selection.resolve(mp.get_property_native("track-list", {}))
end

local function analyze_current_file()
    cancel_analysis()

    if not loaded then
        return
    end

    local stream, stream_error, track_signature = selected_video_stream()
    last_track_signature = track_signature
    if current_override ~= "auto" and track_signature ~= "none" then
        set_profile(
            "ready",
            current_override,
            1,
            "профиль выбран вручную: " .. override_labels[current_override],
            "override"
        )
    end
    if not stream then
        set_timing(nil, stream_error)
        if current_override == "auto" or track_signature == "none" then
            set_profile("unavailable", "unknown", 0, stream_error, "auto")
        end
        return
    end

    local network = mp.get_property_native("demuxer-via-network", false)
    local source = { path = mp.get_property("path"), video_stream = tostring(stream.ff_index) }
    local duration = mp.get_property_number("duration")
    if network then
        local source_error
        source, source_error = network_source.resolve(
            mp.get_property("stream-open-filename"),
            stream,
            mp.get_property_native("user-data/mpv/ytdl/json-subprocess-result")
        )
        if current_override == "auto" then
            set_profile("unavailable", "unknown", 0, "сетевой поток: анализ стиля пропущен", "auto")
        end
        if not source then
            set_timing(nil, source_error)
            return
        end
        if mp.get_property("cookies-file", "") ~= "" then
            -- FFmpeg's independent cookie handling cannot preserve mpv's
            -- exact-host session boundary across nested playlists.
            set_timing(nil, "защищённый сетевой источник: временная шкала не проверена")
            return
        end
    else
        local file_info = source.path and utils.file_info(source.path) or nil
        if not file_info or not file_info.is_file then
            if current_override == "auto" then
                set_profile("unavailable", "unknown", 0, "источник не является локальным файлом", "auto")
            end
            set_timing(nil, "источник не является локальным файлом")
            return
        end
    end

    local analysis_generation = generation
    set_timing({ status = "analyzing", reason = "проверяем timestamp-дельты через ffprobe" })
    local timing_error
    timing_task, timing_error = timing_probe.start(source.path, duration, {
        ffprobe = ffprobe_path,
        video_stream = source.video_stream,
        timeout = network and user_opts.network_probe_timeout or user_opts.probe_timeout,
        network = network,
        http_headers = network and mp.get_property_native("http-header-fields", {}),
        http_proxy = network and mp.get_property("http-proxy"),
        user_agent = network and mp.get_property("user-agent"),
        referrer = network and mp.get_property("referrer"),
        request_size = network and mp.get_property_native("stream-lavf-o", {}).request_size,
    }, function(result, error_reason)
        if analysis_generation ~= generation then
            return
        end
        timing_task = nil
        set_timing(result, error_reason)
    end)
    if not timing_task then
        set_timing(nil, timing_error)
    end

    if network or current_override ~= "auto" then
        return
    end

    set_profile("analyzing", "unknown", 0, "анализируем стиль и частоту смены кадров", "auto")

    local start_error
    classifier_task, start_error = classifier.start(source.path, duration, {
        ffprobe = ffprobe_path,
        ff_index = stream.ff_index,
        timeout = user_opts.probe_timeout,
    }, function(result, error_reason)
        if analysis_generation ~= generation then
            return
        end

        classifier_task = nil
        if not result then
            set_profile("unavailable", "unknown", 0, error_reason, "auto")
            msg.warn("Профиль видео недоступен: " .. tostring(error_reason))
            return
        end

        set_profile(
            "ready",
            result.kind,
            result.confidence,
            result.reason,
            "auto",
            result.metrics
        )
        msg.info(string.format(
            "Профиль видео: %s, уверенность %d%% (%s)",
            result.kind,
            math.floor(result.confidence * 100 + 0.5),
            result.reason
        ))
    end)

    if not classifier_task then
        set_profile("unavailable", "unknown", 0, start_error, "auto")
    end
end

local function set_override(value, show_message)
    local normalized = normalize_override(value)
    if not normalized then
        msg.warn("Неизвестный профиль видео: " .. tostring(value))
        if show_message then
            mp.osd_message("Профиль видео: auto, anime или movie")
        end
        return false
    end

    local changed = normalized ~= current_override
    current_override = normalized
    if mp.get_property_native("user-data/video-profile/override") ~= normalized then
        mp.set_property_native("user-data/video-profile/override", normalized)
    end

    if changed and loaded then
        analyze_current_file()
    end
    if show_message then
        mp.osd_message("Профиль видео: " .. override_labels[normalized])
    end
    return true
end

mp.set_property_native("user-data/video-profile/override", current_override)

mp.observe_property("user-data/video-profile/override", "native", function(_, value)
    if value and value ~= current_override then
        if not set_override(value, false) then
            mp.set_property_native("user-data/video-profile/override", current_override)
        end
    end
end)

mp.register_event("start-file", function()
    loaded = false
    last_track_signature = nil
    if track_change_timer then
        track_change_timer:kill()
        track_change_timer = nil
    end
    cancel_analysis()
    set_profile("idle", "unknown", 0, "ожидание загрузки видео", "auto")
    set_timing({ status = "idle", reason = "ожидание загрузки видео" })
end)

mp.register_event("file-loaded", function()
    loaded = true
    analyze_current_file()
end)

local function schedule_track_analysis()
    if not loaded then
        return
    end

    local _, _, signature = selected_video_stream()
    if signature == last_track_signature then
        return
    end
    if track_change_timer then
        track_change_timer:kill()
    end
    track_change_timer = mp.add_timeout(0.05, function()
        track_change_timer = nil
        local _, _, current_signature = selected_video_stream()
        if current_signature ~= last_track_signature then
            analyze_current_file()
        end
    end)
end

local function schedule_duration_retry(_, duration)
    duration = tonumber(duration)
    if not loaded or timing_task or not duration or duration <= 0
            or mp.get_property_native("demuxer-via-network", false) then
        return
    end
    local timing_status = mp.get_property_native(
        "user-data/video-profile/timing-status",
        "idle"
    )
    if timing_status == "ready" or timing_status == "analyzing" then
        return
    end
    if duration_retry_timer then
        duration_retry_timer:kill()
    end
    duration_retry_timer = mp.add_timeout(0.05, function()
        duration_retry_timer = nil
        if loaded and mp.get_property_number("duration", 0) > 0 then
            analyze_current_file()
        end
    end)
end

mp.observe_property("vid", "native", schedule_track_analysis)
mp.observe_property("track-list", "native", schedule_track_analysis)
mp.observe_property("duration", "number", schedule_duration_retry)

mp.register_script_message("set-video-profile", function(value)
    set_override(value, true)
end)

set_profile("idle", "unknown", 0, "видео не загружено", "auto")
set_timing({ status = "idle", reason = "видео не загружено" })
