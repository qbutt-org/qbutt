local M = {}

local minimum_content_confidence = 0.55
local default_output_width = 1920
local default_output_height = 1080

local function number(value)
    value = tonumber(value)
    return value and value > 0 and value or nil
end

local function boolean(value)
    if type(value) == "boolean" then
        return value
    end
    if type(value) == "string" then
        value = value:lower()
        return value == "yes" or value == "true" or value == "1"
    end
    return value == 1
end

local function gcd(a, b)
    while b ~= 0 do
        a, b = b, a % b
    end
    return math.abs(a)
end

local function rational(num, den)
    num = number(num)
    den = number(den)
    if not num or not den then
        return nil
    end
    num = math.floor(num + 0.5)
    den = math.floor(den + 0.5)
    local divisor = gcd(num, den)
    return { num = num / divisor, den = den / divisor }
end

local function rational_from_float(value)
    value = number(value)
    if not value then
        return nil
    end
    local common_rates = {
        { 24000, 1001 }, { 30000, 1001 }, { 48000, 1001 },
        { 60000, 1001 }, { 120000, 1001 }, { 144000, 1001 },
    }
    for _, rate in ipairs(common_rates) do
        if math.abs(value - rate[1] / rate[2]) < 0.01 then
            return { num = rate[1], den = rate[2] }
        end
    end
    return rational(math.floor(value * 1000 + 0.5), 1000)
end

local function rate_value(rate)
    return rate and rate.num / rate.den or nil
end

local function disabled(status, selection, reason)
    return {
        status = status,
        enabled = false,
        selection = selection,
        reason = reason,
    }
end

local function content_kind(info)
    local kind = tostring(info.content_kind or "unknown"):lower()
    if kind == "video" or kind == "live-action" or kind == "live_action" then
        kind = "movie"
    end
    if kind ~= "anime" and kind ~= "movie" then
        return "unknown"
    end
    local confidence = tonumber(info.content_confidence)
    return confidence and confidence < minimum_content_confidence and "unknown" or kind
end

local function fit_size(width, height, max_width, max_height)
    local scale = math.min(1, max_width / width, max_height / height)
    return math.max(2, math.floor(width * scale / 2 + 1e-9) * 2),
        math.max(2, math.floor(height * scale / 2 + 1e-9) * 2)
end

local function validate(info, profile, selection)
    if selection == "off" then
        return disabled("off", selection, "Frame generation выключен")
    end
    if not boolean(info.has_video) then
        return disabled("guarded", selection, "В источнике нет видеодорожки")
    end
    if not boolean(profile.runtime_available) then
        return disabled("unavailable", selection, "Компоненты frame generation не установлены")
    end
    if info.vfr ~= true and info.vfr ~= false then
        return disabled("guarded", selection, "Не удалось определить временную шкалу видео")
    end
    if boolean(info.interlaced) then
        return disabled("guarded", selection, "Чересстрочное видео нужно сначала деинтерлейсить")
    end
end

local function build_option(info, profile, pipeline, catalog)
    local capability = type(profile.pipelines) == "table" and profile.pipelines[pipeline.id]
    if type(capability) ~= "table" or capability.available ~= true then
        return { id = pipeline.id, available = false, reason = "Модель не установлена" }
    end
    if not catalog.supports_content(pipeline, content_kind(info)) then
        return { id = pipeline.id, available = false, reason = "Модель не подходит видео" }
    end

    local width = number(info.width)
    local height = number(info.height)
    local source = rational(info.source_num, info.source_den)
    local display = rational(info.display_num, info.display_den)
        or rational_from_float(info.display_fps)
    if not width or not height then
        return { id = pipeline.id, available = false, reason = "Не удалось определить разрешение" }
    end
    if not source then
        return { id = pipeline.id, available = false, reason = "Не удалось определить FPS источника" }
    end
    if not display then
        return { id = pipeline.id, available = false, reason = "Не удалось определить герцовку" }
    end
    if rate_value(display) <= rate_value(source) then
        return { id = pipeline.id, available = false, reason = "Герцовка не выше FPS источника" }
    end

    local requested_target = rational(info.framegen_target_num, info.framegen_target_den)
    if requested_target then
        local requested_fps = rate_value(requested_target)
        if requested_fps <= rate_value(source) or requested_fps > rate_value(display) then
            requested_target = nil
        end
    end
    local target = requested_target or display

    local output_width, output_height = fit_size(
        width, height, default_output_width, default_output_height
    )
    return {
        id = pipeline.id,
        available = true,
        quality_tier = tonumber(capability.quality_tier) or pipeline.quality_tier or 0,
        target_num = target.num,
        target_den = target.den,
        target_fps = rate_value(target),
        process_scale = tonumber(pipeline.process_scale) or 1,
        output_width = output_width,
        output_height = output_height,
        fps_kind = requested_target and "session-adaptive" or "display-target",
        reason = string.format(
            "цель %.1f FPS, обработка %dx%d",
            rate_value(target), output_width, output_height
        ),
    }
end

local function activate(selection, option, pipeline)
    return {
        status = "active",
        enabled = true,
        selection = selection,
        pipeline_id = pipeline.id,
        model = pipeline.model,
        target_num = option.target_num,
        target_den = option.target_den,
        target_fps = option.target_fps,
        process_scale = option.process_scale,
        output_width = option.output_width,
        output_height = option.output_height,
        scene_threshold = pipeline.scene_threshold,
        fps_kind = option.fps_kind,
        reason = string.format("%s: %s", pipeline.name, option.reason),
    }
end

function M.choose(info, profile, selection, catalog)
    info = type(info) == "table" and info or {}
    profile = type(profile) == "table" and profile or {}
    selection = tostring(selection or "auto"):lower()

    local invalid = validate(info, profile, selection)
    if invalid then
        return invalid
    end
    if selection ~= "auto" and not catalog.find(selection) then
        return disabled("guarded", "off", "Неизвестный frame generation пресет: " .. selection)
    end

    local wanted_pipeline = selection ~= "auto" and catalog.find(selection) or nil
    local wanted_option
    local unavailable_pipeline
    local unavailable_option
    for _, pipeline in ipairs(catalog.pipelines or {}) do
        local option = build_option(info, profile, pipeline, catalog)
        if not unavailable_option then
            unavailable_pipeline = pipeline
            unavailable_option = option
        end
        if pipeline == wanted_pipeline or selection == "auto"
                and option.available
                and (not wanted_option or option.quality_tier > wanted_option.quality_tier) then
            wanted_pipeline = pipeline
            wanted_option = option
        end
    end
    if not wanted_pipeline or not wanted_option or not wanted_option.available then
        local rejected_option = wanted_option or unavailable_option
        local result = disabled(
            "unavailable",
            selection,
            rejected_option and rejected_option.reason or "В каталоге нет frame generation модели"
        )
        local rejected_pipeline = wanted_pipeline or unavailable_pipeline
        result.pipeline_id = rejected_pipeline and rejected_pipeline.id or nil
        return result
    end
    return activate(selection, wanted_option, wanted_pipeline)
end

return M
