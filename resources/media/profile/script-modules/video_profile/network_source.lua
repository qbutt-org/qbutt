local utils = require "mp.utils"

local M = {}

function M.resolve(filename, stream, ytdl_result)
    if type(filename) ~= "string" then
        return nil, "адрес открытого потока недоступен"
    end
    if filename:match("^https?://") then
        if filename:match("^http://127%.0%.0%.1:%d+/media/%x+/[^?#]+$") then
            filename = filename .. "?role=probe"
        end
        return { path = filename, video_stream = tostring(stream.ff_index) }
    end

    -- ytdl may combine separate video/audio formats into an mpv-only EDL.
    -- Its track titles identify the actual selected format, including after
    -- a quality switch. Reuse that URL instead of resolving the page again.
    local document = type(ytdl_result) == "table" and ytdl_result.status == 0
        and utils.parse_json(ytdl_result.stdout or "") or nil
    if type(document) ~= "table" then
        return nil, "данные открытого сетевого видео недоступны"
    end

    local title = stream.title or ""
    local formats = title ~= "" and document.formats
        or document.requested_formats or document.requested_downloads or { document }
    local selected
    for index, format in ipairs(formats or {}) do
        local format_title = format.format or format.format_note or ""
        if format.vcodec and format.vcodec ~= "none"
                and type(format.url) == "string" and format.url:match("^https?://")
                and filename:find(format.url, 1, true)
                and (title == "" or title == format_title
                    or title == format_title .. " muxed-" .. index) then
            if selected then
                return nil, "адрес выбранной видеодорожки неоднозначен"
            end
            -- EDL track indices belong to the virtual mpv file. Each ytdl
            -- format URL has its own first video stream, even for muxed audio.
            selected = { path = format.url, video_stream = "v:0" }
        end
    end
    return selected, not selected and "не найден адрес выбранной сетевой видеодорожки" or nil
end

return M
