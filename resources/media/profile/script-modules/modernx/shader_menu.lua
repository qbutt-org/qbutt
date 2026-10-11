local M = {}

M.area_name = "shader-preset-menu"
M.dismiss_section = "shader-preset-menu-dismiss"

local function load_catalog()
    local path = mp.command_native({ "expand-path", "~~/script-modules/shader_presets/catalog.lua" })
    local chunk, err = loadfile(path)
    if not chunk then
        error("Failed to load shader preset catalog: " .. tostring(err))
    end

    return chunk()
end

local catalog = load_catalog()

M.items = catalog.presets

local function framegen_selected()
    if mp.get_property_native("user-data/framegen/enabled", false) ~= true then
        return false
    end
    local status = tostring(mp.get_property_native("user-data/framegen/status", "off"))
    return status == "active" or status == "starting" or status == "loading"
end

local function framegen_note()
    local status = tostring(mp.get_property_native("user-data/framegen/status", "off"))
    local reason = tostring(mp.get_property_native("user-data/framegen/reason", ""))
    local fps = mp.get_property_number("user-data/framegen/target-fps", 0)
    if status == "active" and fps > 0 then
        return string.format("RIFE + FSR, %.1f FPS", fps)
    end
    if reason ~= "" and (status == "guarded" or status == "error" or status == "unavailable") then
        return reason
    end
    if status == "starting" or status == "loading" then
        return reason ~= "" and reason or "RIFE + FSR запускается"
    end
    return "RIFE + FSR"
end

local function short_preset_name(name)
    return tostring(name or "")
        :gsub("^Anime4K ", "")
        :gsub("^GAN ", "GAN ")
end

local function auto_note()
    if framegen_selected() then
        return "Авто: Frame generation"
    end
    local mode = tostring(mp.get_property_native("user-data/shader-preset-mode", ""))
    local display = tostring(mp.get_property_native("user-data/shader-preset-display", ""))

    if display == "" then
        return "Авто: пока не выбрано"
    end

    local prefix = mode == "manual" and "Ручной: " or "Авто: "
    return prefix .. short_preset_name(display)
end

local function auto_reason()
    local reason = tostring(mp.get_property_native("user-data/shader-preset-reason", ""))
    if reason == "" then
        return "Автовыбор появится после открытия видео."
    end

    return reason
end

function M.toggle(state, request_init)
    state.shader_preset_menu_open = not state.shader_preset_menu_open
    request_init()
end

function M.close(state, request_init)
    if not state.shader_preset_menu_open then
        return
    end

    state.shader_preset_menu_open = false
    request_init()
end

function M.create_elements(ctx)
    local ne = ctx.new_element("shader_preset_menu_bg", "box")

    ne = ctx.new_element("shader_preset_menu_title", "button")
    ne.content = ctx.title
    ne.eventresponder = nil

    ne = ctx.new_element("shader_preset_menu_auto", "button")
    ne.content = auto_note
    ne.eventresponder = nil
    ne.tooltip_style = ctx.tooltip_style
    ne.tooltipF = auto_reason
    ne.tooltip_max_chars = 34
    ne.tooltip_side = "left"

    ne = ctx.new_element("shader_preset_menu_framegen", "button")
    ne.content = function()
        return (framegen_selected() and "* " or "  ") .. "Frame generation"
    end
    ne.tooltip_style = ctx.tooltip_style
    ne.tooltipF = framegen_note
    ne.tooltip_max_chars = 38
    ne.tooltip_side = "left"
    ne.eventresponder["mbtn_left_up"] = function()
        mp.commandv("script-message", "select-fx-option", "framegen")
        M.close(ctx.state, ctx.request_init)
    end

    for _, item in ipairs(M.items) do
        ne = ctx.new_element("shader_preset_menu_" .. item.id, "button")
        ne.content = function()
            local active = not framegen_selected()
                and mp.get_property_native("user-data/shader-preset", "Off") == item.name
            return (active and "* " or "  ") .. (item.display or item.name)
        end
        ne.tooltip_style = ctx.tooltip_style
        ne.tooltipF = item.hint
        ne.tooltip_max_chars = 32
        ne.eventresponder["mbtn_left_up"] = function()
            mp.commandv("script-message", "select-fx-option", item.name)
            M.close(ctx.state, ctx.request_init)
        end
    end
end

function M.add_layouts(ctx)
    if not ctx.state.shader_preset_menu_open then
        return
    end

    local menu_w = 270
    local title_h = 32
    local auto_h = 26
    local row_h = 28
    local menu_h = title_h + auto_h + ((#M.items + 1) * row_h) + 12
    local menu_x = math.max(menu_w / 2 + 12, math.min(ctx.anchor_x, ctx.osc_param.playresx - menu_w / 2 - 12))
    local menu_y = math.max(18, ctx.anchor_y - menu_h - 20)

    ctx.add_area(M.area_name, ctx.get_hitbox_coords(menu_x, menu_y, 8, menu_w, menu_h + 10))

    local lo = ctx.add_layout("shader_preset_menu_bg")
    lo.geometry = { x = menu_x, y = menu_y, an = 8, w = menu_w, h = menu_h }
    lo.style = ctx.styles.shader_menu_bg
    lo.layer = 75
    lo.box.radius = 8
    lo.alpha[1] = 18
    lo.alpha[3] = 170

    lo = ctx.add_layout("shader_preset_menu_title")
    lo.geometry = { x = menu_x, y = menu_y + 18, an = 8, w = menu_w - 20, h = 24 }
    lo.style = ctx.styles.shader_menu_title
    lo.layer = 76
    lo.button.hoverstyle = ""
    lo.button.glow = false

    lo = ctx.add_layout("shader_preset_menu_auto")
    lo.geometry = { x = menu_x, y = menu_y + title_h + 10, an = 8, w = menu_w - 22, h = 22 }
    lo.style = ctx.styles.shader_menu_note
    lo.layer = 76
    lo.button.hoverstyle = ""
    lo.button.glow = false

    lo = ctx.add_layout("shader_preset_menu_framegen")
    lo.geometry = {
        x = menu_x,
        y = menu_y + title_h + auto_h + 15,
        an = 8,
        w = menu_w - 18,
        h = row_h - 4
    }
    lo.style = ctx.styles.shader_menu_item
    lo.layer = 77
    lo.button.hoverstyle = ctx.styles.shader_menu_item_hover
    lo.button.glow = false

    for index, item in ipairs(M.items) do
        lo = ctx.add_layout("shader_preset_menu_" .. item.id)
        lo.geometry = {
            x = menu_x,
            y = menu_y + title_h + auto_h + (index * row_h) + 15,
            an = 8,
            w = menu_w - 18,
            h = row_h - 4
        }
        lo.style = ctx.styles.shader_menu_item
        lo.layer = 77
        lo.button.hoverstyle = ctx.styles.shader_menu_item_hover
        lo.button.glow = false
    end
end

function M.update_dismiss_layer(ctx)
    if ctx.state.shader_preset_menu_open then
        ctx.set_mouse_area(0, 0, ctx.osc_param.playresx, ctx.osc_param.playresy, M.dismiss_section)
        if not ctx.state.shader_preset_menu_dismiss_enabled then
            mp.enable_key_bindings(M.dismiss_section)
            ctx.state.shader_preset_menu_dismiss_enabled = true
        end
    else
        ctx.set_mouse_area(0, 0, 0, 0, M.dismiss_section)
        if ctx.state.shader_preset_menu_dismiss_enabled then
            mp.disable_key_bindings(M.dismiss_section)
            ctx.state.shader_preset_menu_dismiss_enabled = false
        end
    end
end

function M.handle_mouse_down(ctx)
    if not ctx.state.shader_preset_menu_open then
        return false
    end

    local over_fx = ctx.elements.shader_preset and ctx.mouse_hit(ctx.elements.shader_preset)
    local over_menu = ctx.mouse_hit_area(M.area_name)

    if over_fx then
        M.close(ctx.state, ctx.request_init)
        return true
    end

    if over_menu then
        return false
    end

    M.close(ctx.state, ctx.request_init)
    return not (ctx.mouse_hit_area("input") or ctx.mouse_hit_area("window-controls"))
end

return M
