local M = {}

local function signature(prefix, track)
    if type(track) ~= "table" then
        return prefix
    end
    return table.concat({
        prefix,
        tostring(track.id or "?"),
        tostring(track["ff-index"] or "?"),
        tostring(track["external-filename"] or ""),
        tostring(track.title or ""),
    }, ":")
end

function M.resolve(tracks)
    if type(tracks) ~= "table" then
        return nil, "список видеодорожек недоступен", "unavailable"
    end

    local selected = nil
    for _, track in ipairs(tracks) do
        if type(track) == "table"
            and track.type == "video"
            and track.selected == true
            and tonumber(track["main-selection"]) == 0
        then
            if selected then
                return nil, "выбрано несколько основных видеодорожек", "ambiguous"
            end
            selected = track
        end
    end

    if not selected then
        return nil, "основная видеодорожка не выбрана или управляется lavfi-complex", "none"
    end
    if selected.external == true then
        return nil, "выбрана внешняя видеодорожка", signature("external", selected)
    end

    local ff_index = tonumber(selected["ff-index"])
    if not ff_index or ff_index < 0 or ff_index ~= math.floor(ff_index) then
        return nil, "FFmpeg index выбранной видеодорожки недоступен", signature("invalid", selected)
    end

    return {
        ff_index = ff_index,
        id = tonumber(selected.id),
        title = selected.title,
    }, nil, signature("embedded", selected)
end

return M
