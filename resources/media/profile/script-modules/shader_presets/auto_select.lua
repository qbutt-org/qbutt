local M = {}

local defaults = {
    min_upscale_ratio = 1.08,
    secondary_ratio = 2.0,
    gan_ratio = 2.8,
    gan_extreme_ratio = 3.7,
    high_fps_threshold = 50,
    allow_gan_auto = false,
    max_gan_auto = "off"
}

local function opt(opts, key)
    if opts and opts[key] ~= nil then
        return opts[key]
    end

    return defaults[key]
end

local function opt_number(opts, key)
    local value = tonumber(opt(opts, key))
    if value then
        return value
    end

    return defaults[key]
end

local function opt_bool(opts, key)
    local value = opt(opts, key)
    if type(value) == "boolean" then
        return value
    end

    if type(value) == "string" then
        value = value:lower()
        return value == "yes" or value == "true" or value == "1"
    end

    return value ~= false
end

local function lower(value)
    return tostring(value or ""):lower()
end

local function can_auto_gan(opts, level)
    if not opt_bool(opts, "allow_gan_auto") then
        return false
    end

    local max_gan = lower(opt(opts, "max_gan_auto") or "off")
    local ranks = {
        off = 0,
        no = 0,
        x2 = 2,
        x3 = 3,
        x4 = 4,
        all = 4
    }

    return (ranks[max_gan] or 0) >= level
end

local function describe_size(info)
    if info.width and info.height then
        return string.format("%dx%d", info.width, info.height)
    end

    return "размер неизвестен"
end

local function describe_fps(info)
    if info.fps and info.fps > 0 then
        return string.format("%.0f fps", info.fps)
    end

    return "FPS неизвестен"
end

local function describe_display(info)
    if info.display_width and info.display_height then
        return string.format("дисплей %dx%d", info.display_width, info.display_height)
    end

    return "дисплей неизвестен"
end

local function describe_scale(analysis)
    if not analysis.scale then
        return "масштаб неизвестен"
    end

    local source = analysis.scale_known and "к дисплею" or "оценка"
    return string.format("%s x%.1f", source, analysis.scale)
end

local function display_scale(info)
    local video_w = tonumber(info.width) or 0
    local video_h = tonumber(info.height) or 0
    local display_w = tonumber(info.display_width) or 0
    local display_h = tonumber(info.display_height) or 0

    if video_w <= 0 or video_h <= 0 or display_w <= 0 or display_h <= 0 then
        return nil
    end

    local video_short = math.min(video_w, video_h)
    local video_long = math.max(video_w, video_h)
    local display_short = math.min(display_w, display_h)
    local display_long = math.max(display_w, display_h)

    return math.max(display_short / video_short, display_long / video_long)
end

local function fallback_scale(short_edge)
    if short_edge <= 0 then
        return nil
    elseif short_edge <= 540 then
        return 4.0
    elseif short_edge <= 720 then
        return 3.0
    elseif short_edge <= 1080 then
        return 2.0
    elseif short_edge <= 1440 then
        return 1.5
    end

    return 1.0
end

local function estimate_bpp(info)
    local width = tonumber(info.width) or 0
    local height = tonumber(info.height) or 0
    local fps = tonumber(info.fps) or 0
    local bitrate = tonumber(info.video_bitrate) or 0

    if bitrate <= 0 then
        local file_size = tonumber(info.file_size) or 0
        local duration = tonumber(info.duration) or 0
        if file_size > 0 and duration > 0 then
            bitrate = (file_size * 8 / duration) * 0.85
        end
    end

    if width <= 0 or height <= 0 or fps <= 0 or bitrate <= 0 then
        return nil
    end

    return bitrate / (width * height * fps)
end

local function analyze(info, opts)
    local width = tonumber(info.width) or 0
    local height = tonumber(info.height) or 0
    local short_edge = math.min(width, height)
    local long_edge = math.max(width, height)
    local fps = tonumber(info.fps) or 0

    local scale = display_scale(info)
    local scale_known = scale ~= nil
    if not scale then
        scale = fallback_scale(short_edge)
    end

    local degraded_score = 0
    local bpp = estimate_bpp(info)

    if fps >= opt_number(opts, "high_fps_threshold") then
        degraded_score = math.max(0, degraded_score - 1)
    end

    if short_edge > 0 and short_edge <= 540 then
        degraded_score = degraded_score + 2
    elseif short_edge > 0 and short_edge <= 720 then
        degraded_score = degraded_score + 1
    end

    if bpp then
        if bpp < 0.055 then
            degraded_score = degraded_score + 3
        elseif bpp < 0.085 then
            degraded_score = degraded_score + 2
        elseif bpp < 0.12 then
            degraded_score = degraded_score + 1
        end
    end

    return {
        width = width,
        height = height,
        short_edge = short_edge,
        long_edge = long_edge,
        fps = fps,
        scale = scale,
        scale_known = scale_known,
        needs_upscale = not scale or scale > opt_number(opts, "min_upscale_ratio"),
        secondary = scale and scale >= opt_number(opts, "secondary_ratio"),
        gan = scale and scale >= opt_number(opts, "gan_ratio"),
        gan_extreme = scale and scale >= opt_number(opts, "gan_extreme_ratio"),
        high_fps = fps >= opt_number(opts, "high_fps_threshold"),
        degraded_score = degraded_score,
        bpp = bpp
    }
end

local function context(info, analysis)
    return table.concat({
        describe_size(info),
        describe_fps(info),
        describe_display(info),
        describe_scale(analysis)
    }, ", ")
end

local function anime_profile(analysis, opts)
    if analysis.short_edge <= 540 then
        if analysis.gan and analysis.degraded_score >= 3 then
            if analysis.gan_extreme and can_auto_gan(opts, 4) then
                return "Anime4K GAN x4", "очень низкое разрешение и большой апскейл, пробуем тяжелый GAN x4"
            end

            if can_auto_gan(opts, 3) then
                return "Anime4K GAN x3", "очень низкое разрешение и большой апскейл, пробуем GAN x3"
            end

            if can_auto_gan(opts, 2) then
                return "Anime4K GAN x2", "очень низкое разрешение и большой апскейл, пробуем GAN x2"
            end
        end

        if analysis.secondary then
            if analysis.degraded_score >= 3 then
                return "Anime4K A+A", "SD/480p выглядит деградированным, нужен сильный Mode A+A"
            end

            return "Anime4K C+A", "SD/480p и x2+ апскейл, берем более аккуратный Mode C+A"
        end

        if analysis.degraded_score >= 3 then
            return "Anime4K A", "SD/480p с признаками блюра или компрессии, берем Mode A"
        end

        return "Anime4K C", "SD/480p без явного старого блюра, берем Mode C"
    end

    if analysis.short_edge <= 720 then
        if analysis.gan and analysis.degraded_score >= 4 and can_auto_gan(opts, 2) then
            return "Anime4K GAN x2", "720p сильно деградирован и хорошо апскейлится, пробуем GAN x2"
        end

        if analysis.secondary then
            if analysis.degraded_score >= 3 then
                return "Anime4K A+A", "720p выглядит сильно сжатым по bitrate-per-pixel, Mode A+A восстановит линии сильнее"
            end

            return "Anime4K B+B", "720p и x2+ апскейл, официальный сильный вариант Mode B+B"
        end

        return "Anime4K B", "720p-аниме, официальный Mode B"
    end

    if analysis.short_edge <= 1080 then
        if analysis.secondary and analysis.degraded_score >= 2 then
            return "Anime4K A+A", "1080p с заметной деградацией и x2+ апскейлом, включаем Mode A+A"
        end

        return "Anime4K A", "1080p-аниме, официальный Mode A"
    end

    if analysis.short_edge <= 1440 then
        return "Anime4K A", "1440p меньше текущего дисплея, аккуратно включаем Mode A"
    end

    return "Anime4K A", "аниме высокого разрешения, используем аккуратный Mode A"
end

function M.needs_content_profile(info, opts)
    return info and info.has_video == true
end

function M.choose(info, opts)
    info = info or {}
    opts = opts or {}

    if not info.has_video then
        return "Off", "в файле нет видеодорожки"
    end

    local analysis = analyze(info, opts)
    local base = context(info, analysis)

    if analysis.width <= 0 or analysis.height <= 0 then
        return "Off", "не удалось определить разрешение видео"
    end

    if info.content_kind == "anime" then
        local preset, reason = anime_profile(analysis, opts)
        local detection = info.content_reason and ("профиль аниме (" .. info.content_reason .. "); ") or ""
        return preset, base .. ": " .. detection .. reason
    end

    local detection = info.content_reason or "профиль аниме не подтвержден"
    return "FSR", base .. ": " .. detection .. "; используем универсальный FSR"
end

return M
