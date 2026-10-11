local M = {}

function M.utf8_len(text)
    local _, count = tostring(text):gsub("[^\128-\191]", "")
    return count
end

function M.wrap_text(text, max_chars)
    local lines = {}
    local line = ""

    for word in tostring(text):gmatch("%S+") do
        local separator = line == "" and "" or " "
        if line ~= "" and M.utf8_len(line .. separator .. word) > max_chars then
            table.insert(lines, line)
            line = word
        else
            line = line .. separator .. word
        end
    end

    if line ~= "" then
        table.insert(lines, line)
    end

    return lines
end

function M.append_panel(ass, opts)
    local lines = M.wrap_text(opts.text, opts.max_chars)
    local longest = 0
    for _, line in ipairs(lines) do
        longest = math.max(longest, M.utf8_len(line))
    end

    local font_size = opts.font_size
    local pad_x = opts.pad_x or 10
    local pad_y = opts.pad_y or 7
    local line_h = opts.line_h or (font_size + 4)
    local panel_w = (longest * font_size * 0.62) + (pad_x * 2)
    local panel_h = (#lines * line_h) + (pad_y * 2)
    local panel_x
    local panel_y
    local margin = opts.margin or 12
    local anchor = opts.anchor

    if anchor then
        local left_x = anchor.x1 - margin - panel_w
        local right_x = anchor.x2 + margin
        local centered_x = ((anchor.x1 + anchor.x2) / 2) - (panel_w / 2)

        if opts.side == "left" and left_x >= 12 then
            panel_x = left_x
        elseif opts.side == "right" and right_x + panel_w <= opts.playresx - 12 then
            panel_x = right_x
        elseif left_x >= 12 then
            panel_x = left_x
        elseif right_x + panel_w <= opts.playresx - 12 then
            panel_x = right_x
        else
            panel_x = centered_x
        end

        panel_y = ((anchor.y1 + anchor.y2) / 2) - (panel_h / 2)
    else
        panel_x = opts.x
        panel_y = opts.y
    end

    panel_x = math.max(12, math.min(panel_x, opts.playresx - 12 - panel_w))

    if panel_y + panel_h > opts.playresy - 8 then
        panel_y = math.max(8, opts.playresy - panel_h - 8)
    end

    ass:new_event()
    ass:pos(panel_x, panel_y)
    ass:an(7)
    ass:append("{\\blur2\\bord1\\1c&H050505&\\3c&H000000&\\1a&H22&\\3a&H88&}")
    ass:draw_start()
    ass:round_rect_cw(0, 0, panel_w, panel_h, 7)
    ass:draw_stop()

    for index, line in ipairs(lines) do
        ass:new_event()
        ass:pos(panel_x + pad_x, panel_y + pad_y + ((index - 1) * line_h))
        ass:an(7)
        ass:append(opts.style)
        ass:append(line)
    end
end

return M
