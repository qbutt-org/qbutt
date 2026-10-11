local M = {}

local sample_fractions = { 0.12, 0.27, 0.42, 0.57, 0.72, 0.87 }
local sample_frames = 12
local style_frames = { 1, 5, 9 }
local repeat_diff_threshold = 0.15
local min_repeats_per_burst = 2

local tag_names = {
    ydiff = "lavfi.signalstats.YDIF",
    saturation = "lavfi.signalstats.SATAVG",
    yhigh = "lavfi.signalstats.YHIGH",
    yavg = "lavfi.signalstats.YAVG",
    entropy = "lavfi.entropy.normalized_entropy.normal.Y",
}

local function clamp(value, minimum, maximum)
    return math.max(minimum, math.min(maximum, value))
end

local function unit(value)
    return clamp(value, 0, 1)
end

local function percent(value)
    return math.floor(unit(value) * 100 + 0.5)
end

local function average(values)
    if #values == 0 then
        return nil
    end

    local sum = 0
    for _, value in ipairs(values) do
        sum = sum + value
    end
    return sum / #values
end

local function escape_lavfi_path(path)
    if type(path) ~= "string" or path == "" or path:find("[%z\r\n]") then
        return nil, "путь к видео недоступен для lavfi"
    end

    -- movie uses FFmpeg's filtergraph parser rather than the shell. Its filename
    -- crosses both graph and option-token parsing, hence the three backslashes
    -- needed for a literal apostrophe between adjacent quoted fragments.
    path = path:gsub("\\", "/")
    path = path:gsub(":", "\\:")
    path = path:gsub("'", "'" .. string.rep("\\", 3) .. "''")
    return "'" .. path .. "'"
end

local function build_filtergraph(path, duration, ff_index)
    local escaped_path, path_error = escape_lavfi_path(path)
    if not escaped_path then
        return nil, path_error
    end

    local sources = {}
    local base_filters = {}
    local edge_filters = {}
    for index, fraction in ipairs(sample_fractions) do
        local base_label = "base" .. tostring(index - 1)
        local edge_label = "edge" .. tostring(index - 1)
        sources[#sources + 1] = string.format(
            "movie=filename=%s:stream_index=%d:seek_point=%.3f:dec_threads=1,"
                .. "trim=end_frame=%d,setpts=PTS-STARTPTS,"
                .. "scale=96:54:flags=area,split=2[%s][%s]",
            escaped_path,
            ff_index,
            duration * fraction,
            sample_frames,
            base_label,
            edge_label
        )
        base_filters[#base_filters + 1] = string.format(
            "[%s]signalstats,entropy[out%d]",
            base_label,
            index - 1
        )
        edge_filters[#edge_filters + 1] = string.format(
            "[%s]edgedetect=low=0.024:high=0.094:mode=wires,signalstats,entropy[out%d]",
            edge_label,
            index - 1 + #sample_fractions
        )
    end

    local filters = {}
    for _, group in ipairs({ sources, base_filters, edge_filters }) do
        for _, filter in ipairs(group) do
            filters[#filters + 1] = filter
        end
    end
    return table.concat(filters, ";")
end

local function build_ffprobe_args(ffprobe, path, duration, ff_index)
    local filtergraph, graph_error = build_filtergraph(path, duration, ff_index)
    if not filtergraph then
        return nil, graph_error
    end

    local tag_entries = table.concat({
        tag_names.ydiff,
        tag_names.saturation,
        tag_names.yhigh,
        tag_names.yavg,
        tag_names.entropy,
    }, ",")
    return {
        ffprobe,
        "-v", "error",
        "-f", "lavfi",
        "-i", filtergraph,
        "-show_entries", "frame=stream_index:frame_tags=" .. tag_entries,
        "-of", "json",
    }
end

local function classify_metrics(metrics)
    local repeat_fraction = unit(tonumber(metrics.repeat_fraction) or 0)
    local repeat_bursts = clamp(tonumber(metrics.repeat_bursts) or 0, 0, #sample_fractions)
    local edge_average = math.max(tonumber(metrics.edge_average) or 0, 0)
    local edge_high = math.max(tonumber(metrics.edge_high) or 0, 0)
    local edge_entropy = unit(tonumber(metrics.edge_entropy) or 0)
    local base_entropy = unit(tonumber(metrics.base_entropy) or 0)
    local saturation = math.max(tonumber(metrics.saturation) or 0, 0)

    local cadence_score = 0.60 * unit((repeat_fraction - 0.12) / 0.43)
        + 0.40 * unit((repeat_bursts - 1) / 4)
    local drawn_score = 0.30 * unit((edge_average - 10) / 10)
        + 0.20 * unit((edge_high - 30) / 60)
        + 0.20 * unit((edge_entropy - 0.14) / 0.07)
        + 0.15 * unit((base_entropy - 0.64) / 0.14)
        + 0.15 * unit((saturation - 5) / 15)
    local anime_score = 0.55 * drawn_score + 0.45 * cadence_score

    local strong_anime = drawn_score >= 0.48
        and cadence_score >= 0.30
        and anime_score >= 0.58
        and edge_average >= 12
    local low_detail_anime = edge_average >= 11.5
        and edge_high >= 40
        and edge_entropy >= 0.16
        and saturation >= 10
        and repeat_fraction >= 0.36
        and cadence_score >= 0.55
        and anime_score >= 0.38
    local cadence_dominant_anime = cadence_score >= 0.90
        and repeat_fraction >= 0.60
        and repeat_bursts >= 5
        and drawn_score >= 0.25
        and anime_score >= 0.58

    local kind
    local confidence
    if strong_anime or low_detail_anime or cadence_dominant_anime then
        kind = "anime"
        if strong_anime then
            confidence = 0.70 + 0.28 * unit((anime_score - 0.58) / 0.32)
        elseif cadence_dominant_anime then
            confidence = 0.62 + 0.18 * unit((anime_score - 0.58) / 0.20)
        else
            confidence = 0.56 + 0.18 * unit((anime_score - 0.38) / 0.22)
        end
    elseif drawn_score <= 0.20 then
        -- Cadence is intentionally not enough. A static or telecined live-action
        -- source may contain many duplicates while retaining photographic texture.
        kind = "movie"
        confidence = 0.76 + 0.20 * unit((0.20 - drawn_score) / 0.20)
    elseif cadence_score <= 0.12 then
        kind = "movie"
        confidence = 0.68 + 0.20 * unit((0.12 - cadence_score) / 0.12)
    else
        kind = "unknown"
        confidence = 0.45 + 0.10 * unit(math.abs(anime_score - 0.50) / 0.50)
    end

    local reason = string.format(
        "рисованная структура %d%%, покадровая анимация %d%% (%d%% повторов, %d/%d выборок)",
        percent(drawn_score),
        percent(cadence_score),
        percent(repeat_fraction),
        repeat_bursts,
        #sample_fractions
    )

    return {
        kind = kind,
        confidence = unit(confidence),
        reason = reason,
        metrics = {
            anime_score = anime_score,
            drawn_score = drawn_score,
            cadence_score = cadence_score,
            repeat_fraction = repeat_fraction,
            repeat_bursts = repeat_bursts,
            edge_average = edge_average,
            edge_high = edge_high,
            edge_entropy = edge_entropy,
            base_entropy = base_entropy,
            saturation = saturation,
        },
    }
end

local function tag_number(frame, name)
    local tags = type(frame) == "table" and frame.tags or nil
    local value = type(tags) == "table" and tonumber(tags[name]) or nil
    if value and value == value and value ~= math.huge and value ~= -math.huge then
        return value
    end
    return nil
end

local function analyze_document(document)
    if type(document) ~= "table" or type(document.frames) ~= "table" then
        return nil, "ffprobe вернул некорректные metadata"
    end

    local stream_count = #sample_fractions * 2
    local streams = {}
    for index = 0, stream_count - 1 do
        streams[index] = {}
    end
    for _, frame in ipairs(document.frames) do
        local stream_index = tonumber(frame.stream_index)
        if stream_index
            and stream_index == math.floor(stream_index)
            and streams[stream_index]
        then
            local frames = streams[stream_index]
            frames[#frames + 1] = frame
        end
    end

    for index = 0, stream_count - 1 do
        if #streams[index] < sample_frames then
            return nil, string.format(
                "ffprobe вернул недостаточно кадров для выборки %d",
                index + 1
            )
        end
    end

    local repeat_pairs = 0
    local repeat_bursts = 0
    local base_entropy_values = {}
    local saturation_values = {}
    local edge_average_values = {}
    local edge_high_values = {}
    local edge_entropy_values = {}

    for sample_index = 0, #sample_fractions - 1 do
        local base_frames = streams[sample_index]
        local edge_frames = streams[sample_index + #sample_fractions]
        local burst_repeats = 0

        for frame_index = 2, sample_frames do
            local difference = tag_number(base_frames[frame_index], tag_names.ydiff)
            if not difference then
                return nil, "ffprobe не вернул temporal metadata"
            end
            if difference <= repeat_diff_threshold then
                repeat_pairs = repeat_pairs + 1
                burst_repeats = burst_repeats + 1
            end
        end
        if burst_repeats >= min_repeats_per_burst then
            repeat_bursts = repeat_bursts + 1
        end

        for _, frame_index in ipairs(style_frames) do
            local base_frame = base_frames[frame_index]
            local edge_frame = edge_frames[frame_index]
            local base_entropy = tag_number(base_frame, tag_names.entropy)
            local saturation = tag_number(base_frame, tag_names.saturation)
            local edge_average = tag_number(edge_frame, tag_names.yavg)
            local edge_high = tag_number(edge_frame, tag_names.yhigh)
            local edge_entropy = tag_number(edge_frame, tag_names.entropy)
            if not base_entropy
                or not saturation
                or not edge_average
                or not edge_high
                or not edge_entropy
            then
                return nil, "ffprobe не вернул spatial metadata"
            end

            base_entropy_values[#base_entropy_values + 1] = base_entropy
            saturation_values[#saturation_values + 1] = saturation
            edge_average_values[#edge_average_values + 1] = edge_average
            edge_high_values[#edge_high_values + 1] = edge_high
            edge_entropy_values[#edge_entropy_values + 1] = edge_entropy
        end
    end

    return classify_metrics({
        repeat_fraction = repeat_pairs / (#sample_fractions * (sample_frames - 1)),
        repeat_bursts = repeat_bursts,
        edge_average = average(edge_average_values),
        edge_high = average(edge_high_values),
        edge_entropy = average(edge_entropy_values),
        base_entropy = average(base_entropy_values),
        saturation = average(saturation_values),
    })
end

function M.start(path, duration, opts, callback)
    opts = opts or {}
    duration = tonumber(duration)
    local ff_index = tonumber(opts.ff_index)
    local ffprobe = opts.ffprobe
    if type(path) ~= "string" or path == "" or path:find("://", 1, true) then
        return nil, "источник не является локальным файлом"
    end
    if not duration or duration <= 0 then
        return nil, "длительность видео неизвестна"
    end
    if not ff_index or ff_index < 0 or ff_index ~= math.floor(ff_index) then
        return nil, "FFmpeg index видеодорожки недоступен"
    end
    if type(ffprobe) ~= "string" or ffprobe == "" then
        return nil, "bundled ffprobe не настроен"
    end
    if type(callback) ~= "function" then
        return nil, "callback не задан"
    end

    local args, args_error = build_ffprobe_args(ffprobe, path, duration, ff_index)
    if not args then
        return nil, args_error
    end

    local timeout = clamp(tonumber(opts.timeout) or 8, 1, 30)
    local task = {}
    local command
    local timer
    local done = false

    local function finish(result, error_reason)
        if done then
            return
        end

        done = true
        if timer then
            timer:kill()
            timer = nil
        end
        callback(result, error_reason)
    end

    function task.cancel()
        if done then
            return
        end

        done = true
        if timer then
            timer:kill()
            timer = nil
        end
        if command then
            mp.abort_async_command(command)
            command = nil
        end
    end

    command = mp.command_native_async({
        name = "subprocess",
        args = args,
        playback_only = true,
        capture_stdout = true,
        capture_stderr = true,
    }, function(success, result, error)
        command = nil
        if done then
            return
        end

        result = result or {}
        if not success or result.status ~= 0 then
            local reason = result.error_string
            if reason == "init" then
                reason = "bundled ffprobe не удалось запустить"
            elseif not reason or reason == "" then
                reason = error or "ffprobe не смог прочитать кадровые выборки"
            end
            finish(nil, reason)
            return
        end

        local parse_ok, document = pcall(function()
            return require("mp.utils").parse_json(result.stdout or "")
        end)
        if not parse_ok or not document then
            finish(nil, "ffprobe вернул некорректный JSON")
            return
        end

        local profile, analyze_error = analyze_document(document)
        finish(profile, analyze_error)
    end)

    timer = mp.add_timeout(timeout, function()
        if command then
            mp.abort_async_command(command)
            command = nil
        end
        finish(nil, string.format("анализ кадров превысил %.0f секунд", timeout))
    end)

    return task
end

M.analyze_document = analyze_document
M.classify_metrics = classify_metrics
M.escape_lavfi_path = escape_lavfi_path

return M
