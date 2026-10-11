local M = {}

function M.add_right_controls(ctx)
    local right_button_index = 0
    local positions = {}

    local function add_button(name, min_width, width)
        width = width or 24
        local x = ctx.osc_geo.w - 37 - (right_button_index * 45)
        right_button_index = right_button_index + 1

        local lo = ctx.add_layout(name)
        lo.geometry = { x = x, y = ctx.ref_y - 40, an = 5, w = width, h = 24 }
        lo.style = ctx.style
        lo.visible = (ctx.osc_param.playresx >= min_width - ctx.outeroffset)

        positions[name] = x
        return x
    end

    add_button("tog_fs", 250)

    if ctx.loop_button then
        add_button("tog_loop", 600)
    end

    if ctx.ontop_button then
        add_button("tog_ontop", 700)
    end

    if ctx.info_button then
        add_button("tog_info", 500)
    end

    if ctx.shader_preset_button then
        add_button("shader_preset", 300)
    end

    if ctx.screenshot_button then
        add_button("screenshot", 300)
    end

    if ctx.download_button then
        add_button("download", 400)
    end

    return positions
end

return M
