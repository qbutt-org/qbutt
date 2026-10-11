local msg = require "mp.msg"
local options = require "mp.options"

local user_opts = {
    auto_select = true,
    auto_delay = 0.35,
    show_auto_message = false,
    hardware_tier = "balanced",
    shader_quality = "auto",
    allow_gan_auto = false,
    max_gan_auto = "off",
}

options.read_options(user_opts, "shader_presets")

local function load_mod(name)
    local path = mp.command_native({ "expand-path", "~~/script-modules/shader_presets/" .. name .. ".lua" })
    local chunk, err = loadfile(path)
    if not chunk then
        error("Failed to load shader preset module " .. name .. ": " .. tostring(err))
    end

    return chunk()
end

local catalog = load_mod("catalog")
local auto_select = load_mod("auto_select")
local presets = catalog.presets

local current_index = 1
local menu_active = false
local menu_selected_index = 1
local auto_timer = nil
local auto_generation = 0
local auto_attempts = 0
local auto_applied = false
local manual_override = false
local last_content_kind = nil
local last_framegen_status = nil
local apply_preset
local schedule_auto_select

local menu_bindings = {
    { key = "UP", name = "shader-preset-menu-up" },
    { key = "DOWN", name = "shader-preset-menu-down" },
    { key = "WHEEL_UP", name = "shader-preset-menu-wheel-up" },
    { key = "WHEEL_DOWN", name = "shader-preset-menu-wheel-down" },
    { key = "ENTER", name = "shader-preset-menu-apply" },
    { key = "ESC", name = "shader-preset-menu-close" }
}

local function preset_label(preset)
    return preset and (preset.display or preset.name) or "?"
end

local function compact_reason(reason)
    reason = tostring(reason or "")

    local width, height = reason:match("(%d+)x(%d+)")
    local scale = reason:match("к дисплею (x[%d%.]+)") or reason:match("оценка (x[%d%.]+)")
    local parts = {}

    width = tonumber(width)
    height = tonumber(height)
    if width and height then
        parts[#parts + 1] = tostring(math.min(width, height)) .. "p"
    end
    if scale then
        parts[#parts + 1] = scale
    end

    if #parts == 0 then
        return ""
    end

    return table.concat(parts, ", ")
end

local function current_shader_string()
    return mp.get_property("glsl-shaders", "")
end

local function framegen_requested()
    return mp.get_property_native("user-data/framegen/enabled", false) == true
end

local function framegen_controls_fx()
    if not framegen_requested() then
        return false
    end
    local status = tostring(mp.get_property_native("user-data/framegen/status", ""))
    return status == "active" or status == "starting"
end

local function detect_current_index()
    local _, index = catalog.find_by_shader_string(current_shader_string())
    return index or current_index
end

function apply_preset(index, apply_opts)
    apply_opts = apply_opts or {}

    local preset = presets[index]
    if not preset then
        msg.warn("Неизвестный индекс FX пресета: " .. tostring(index))
        return false
    end
    if framegen_controls_fx() and preset.name ~= "FSR" and not apply_opts.replaces_framegen then
        if not apply_opts.auto then
            mp.osd_message("При включённом framegen используется FSR")
        end
        return false
    end

    local shaders = catalog.resolve_shaders(preset, user_opts.hardware_tier, user_opts.shader_quality)
    mp.set_property("glsl-shaders", catalog.join_shaders(shaders))

    current_index = index
    mp.set_property("user-data/shader-preset", preset.name)
    mp.set_property("user-data/shader-preset-display", preset_label(preset))
    mp.set_property("user-data/shader-preset-mode", apply_opts.auto and "auto" or "manual")
    mp.set_property("user-data/shader-preset-reason", apply_opts.reason or "")
    mp.set_property("user-data/shader-preset-summary", compact_reason(apply_opts.reason))
    mp.set_property("user-data/shader-preset-hardware-tier", tostring(user_opts.hardware_tier or ""))
    mp.set_property("user-data/shader-preset-quality", catalog.resolve_quality(user_opts.hardware_tier, user_opts.shader_quality))

    if apply_opts.auto then
        msg.info("Автовыбор FX: " .. preset.name .. " (" .. (apply_opts.reason or "без причины") .. ")")
        if user_opts.show_auto_message then
            local summary = compact_reason(apply_opts.reason)
            local suffix = summary ~= "" and (" [" .. summary .. "]") or ""
            mp.osd_message("FX авто: " .. preset_label(preset) .. suffix, 1.6)
        end
    else
        manual_override = true
        mp.osd_message("FX: " .. preset_label(preset))
    end

    return true
end

local function render_menu()
    local lines = { "FX пресеты" }

    for index, preset in ipairs(presets) do
        local marker = index == menu_selected_index and ">" or " "
        local active = index == detect_current_index() and " *" or ""
        lines[#lines + 1] = string.format("%s %s%s", marker, preset_label(preset), active)
    end

    lines[#lines + 1] = ""
    lines[#lines + 1] = "Вверх/вниз: выбор  Enter: применить  Esc: закрыть"
    mp.osd_message(table.concat(lines, "\n"), 9999)
end

local function close_menu()
    if not menu_active then
        return
    end

    menu_active = false
    for _, binding in ipairs(menu_bindings) do
        mp.remove_key_binding(binding.name)
    end
    mp.osd_message("", 0)
end

local function move_menu(delta)
    if not menu_active then
        return
    end

    menu_selected_index = ((menu_selected_index - 1 + delta) % #presets) + 1
    render_menu()
end

local function apply_menu_selection()
    if not menu_active then
        return
    end

    local selected = menu_selected_index
    close_menu()
    mp.commandv("script-message-to", "framegen", "framegen-set-enabled", "off")
    apply_preset(selected, { replaces_framegen = true })
end

local function open_menu()
    if menu_active then
        close_menu()
        return
    end

    menu_active = true
    menu_selected_index = detect_current_index()

    mp.add_forced_key_binding("UP", "shader-preset-menu-up", function() move_menu(-1) end)
    mp.add_forced_key_binding("DOWN", "shader-preset-menu-down", function() move_menu(1) end)
    mp.add_forced_key_binding("WHEEL_UP", "shader-preset-menu-wheel-up", function() move_menu(-1) end)
    mp.add_forced_key_binding("WHEEL_DOWN", "shader-preset-menu-wheel-down", function() move_menu(1) end)
    mp.add_forced_key_binding("ENTER", "shader-preset-menu-apply", apply_menu_selection)
    mp.add_forced_key_binding("ESC", "shader-preset-menu-close", close_menu)

    render_menu()
end

local function cycle_preset(direction)
    direction = tonumber(direction) or 1
    local detected = detect_current_index()
    local next_index = ((detected - 1 + direction) % #presets) + 1
    mp.commandv("script-message-to", "framegen", "framegen-set-enabled", "off")
    apply_preset(next_index, { replaces_framegen = true })
end

local function select_preset(name)
    if not name or name == "" then
        cycle_preset(1)
        return
    end

    local _, index = catalog.find_by_name(name)
    if index then
        mp.commandv("script-message-to", "framegen", "framegen-set-enabled", "off")
        apply_preset(index, { replaces_framegen = true })
        return
    end

    msg.warn("Неизвестный FX пресет: " .. name)
    mp.osd_message("Неизвестный FX пресет: " .. name)
end

local function selected_video_track()
    local vid = mp.get_property("vid")
    return vid and vid ~= "no"
end

local function first_number(...)
    for index = 1, select("#", ...) do
        local value = select(index, ...)
        local number = tonumber(value)
        if number and number > 0 then
            return number
        end
    end

    return nil
end

local function collect_video_info()
    local params = mp.get_property_native("video-params", {}) or {}
    local dec_params = mp.get_property_native("video-dec-params", {}) or {}
    local video_profile = mp.get_property_native("user-data/video-profile", {}) or {}

    local source_num = tonumber(video_profile["source-num"])
    local source_den = tonumber(video_profile["source-den"])
    local verified_fps = source_num and source_den and source_den > 0
        and source_num / source_den or nil
    local fps = first_number(verified_fps, mp.get_property_number("container-fps"))

    return {
        has_video = selected_video_track(),
        width = first_number(params.w, dec_params.w, mp.get_property_number("width")),
        height = first_number(params.h, dec_params.h, mp.get_property_number("height")),
        fps = fps,
        display_width = first_number(mp.get_property_number("display-width")),
        display_height = first_number(mp.get_property_number("display-height")),
        video_bitrate = first_number(mp.get_property_number("video-bitrate")),
        file_size = first_number(mp.get_property_number("file-size")),
        duration = first_number(mp.get_property_number("duration")),
        content_kind = video_profile.kind,
        content_reason = video_profile.reason,
        content_status = video_profile.status,
    }
end

local function run_auto_select()
    if not user_opts.auto_select or manual_override or auto_applied then
        return
    end

    local info = collect_video_info()
    if info.has_video and (not info.width or not info.height) and auto_attempts < 8 then
        auto_attempts = auto_attempts + 1
        if auto_timer then
            auto_timer:kill()
        end
        auto_timer = mp.add_timeout(0.25, run_auto_select)
        return
    end

    if auto_select.needs_content_profile(info, user_opts)
        and (info.content_status == "idle" or info.content_status == "analyzing")
    then
        return
    end

    local preset_name
    local reason
    if framegen_controls_fx() then
        preset_name = "FSR"
        reason = "Frame generation включён: FSR сохраняет устойчивую частоту кадров"
    else
        preset_name, reason = auto_select.choose(info, user_opts)
    end
    local _, index = catalog.find_by_name(preset_name)
    if not index then
        msg.warn("Автовыбор вернул неизвестный FX пресет: " .. tostring(preset_name))
        return
    end

    auto_applied = apply_preset(index, { auto = true, reason = reason })
end

function schedule_auto_select(delay)
    if not user_opts.auto_select or manual_override or auto_applied then
        return
    end

    if auto_timer then
        auto_timer:kill()
    end

    local generation = auto_generation
    auto_timer = mp.add_timeout(delay or user_opts.auto_delay, function()
        if generation == auto_generation then
            run_auto_select()
        end
    end)
end

local function reset_auto_state()
    auto_generation = auto_generation + 1
    auto_attempts = 0
    auto_applied = false
    manual_override = false

    if auto_timer then
        auto_timer:kill()
        auto_timer = nil
    end
end

mp.register_event("start-file", reset_auto_state)
mp.register_event("file-loaded", function() schedule_auto_select(user_opts.auto_delay) end)
mp.register_event("video-reconfig", function() schedule_auto_select(0.15) end)
mp.observe_property("user-data/qbutt-shader-profile", "native", function(_, profile)
    if type(profile) ~= "table" then
        return
    end

    user_opts.hardware_tier = profile.hardware_tier
    user_opts.shader_quality = profile.shader_quality
    user_opts.allow_gan_auto = profile.allow_gan_auto
    user_opts.max_gan_auto = profile.max_gan_auto
    if not manual_override then
        auto_applied = false
        schedule_auto_select(0)
    end
end)
mp.observe_property("user-data/video-profile/status", "native", function(_, status)
    if status == "ready" or status == "unavailable" then
        schedule_auto_select(0)
    end
end)
mp.observe_property("user-data/video-profile/kind", "native", function(_, kind)
    if kind == last_content_kind then
        return
    end

    last_content_kind = kind
    if not manual_override then
        auto_applied = false
        local status = mp.get_property_native("user-data/video-profile/status")
        if status == "ready" or status == "unavailable" then
            schedule_auto_select(0)
        end
    end
end)
mp.observe_property("user-data/framegen/status", "native", function(_, status)
    status = tostring(status or "")
    if status == last_framegen_status then
        return
    end
    last_framegen_status = status

    if framegen_controls_fx() then
        manual_override = false
        auto_applied = false
        schedule_auto_select(0)
    elseif manual_override then
        return
    elseif status == "off" or status == "guarded" or status == "unavailable"
        or status == "error"
    then
        -- Restore the regular content-aware FX policy after frame generation
        -- has been removed from the filter graph.
        auto_applied = false
        schedule_auto_select(0.35)
    end
end)

mp.register_script_message("select-fx-option", function(name)
    if tostring(name or ""):lower() == "framegen" then
        local _, index = catalog.find_by_name("FSR")
        if not index then
            mp.osd_message("FSR не найден в каталоге FX")
            return
        end
        apply_preset(index)
        mp.commandv("script-message-to", "framegen", "framegen-set-enabled", "on")
        return
    end
    select_preset(name)
end)

mp.register_script_message("cycle-shader-preset", cycle_preset)
mp.register_script_message("select-shader-preset", select_preset)
mp.register_script_message("open-shader-preset-menu", open_menu)
mp.add_key_binding(nil, "open-shader-preset-menu", open_menu)
