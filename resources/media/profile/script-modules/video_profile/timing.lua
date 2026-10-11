local M = {}

local sample_fractions = { 0.12, 0.36, 0.62, 0.86 }
local frames_per_sample = 48
local minimum_deltas = 80

local function gcd(a, b)
    a = math.floor(math.abs(a))
    b = math.floor(math.abs(b))
    while b ~= 0 do
        a, b = b, a % b
    end
    return a
end

local function reduce(num, den)
    num = tonumber(num)
    den = tonumber(den)
    if not num or not den or num <= 0 or den <= 0 then
        return nil
    end

    local divisor = gcd(num, den)
    return math.floor(num / divisor), math.floor(den / divisor)
end

local function parse_rational(value)
    if type(value) ~= "string" then
        return nil
    end

    local num, den = value:match("^(%d+)/(%d+)$")
    return reduce(num, den)
end

local function rate(num, den)
    if not num or not den or den <= 0 then
        return nil
    end
    return num / den
end

local function allowed_deltas(num, den, time_base, timestamp_quantum)
    local fps = rate(num, den)
    if not fps or not time_base or time_base <= 0 then
        return nil
    end

    local nominal = 1 / (fps * time_base)
    local quantum = math.max(1, tonumber(timestamp_quantum) or 1)
    local quantized = nominal / quantum
    local rounded = math.floor(quantized + 0.5)
    if math.abs(quantized - rounded) < 1e-6 then
        return rounded * quantum, rounded * quantum
    end
    return math.floor(quantized) * quantum, math.ceil(quantized) * quantum
end

local function candidate_score(deltas, num, den, time_base, timestamp_quantum)
    local lower, upper = allowed_deltas(num, den, time_base, timestamp_quantum)
    if not lower or lower <= 0 then
        return nil
    end

    local matches = 0
    for _, delta in ipairs(deltas) do
        if delta == lower or delta == upper then
            matches = matches + 1
        end
    end
    return matches / #deltas, lower, upper
end

local function choose_nominal(stream, deltas, time_base, timestamp_quantum)
    local r_num, r_den = parse_rational(stream.r_frame_rate)
    local avg_num, avg_den = parse_rational(stream.avg_frame_rate)
    local r_score, r_lower, r_upper = candidate_score(
        deltas, r_num, r_den, time_base, timestamp_quantum
    )
    local avg_score, avg_lower, avg_upper = candidate_score(
        deltas, avg_num, avg_den, time_base, timestamp_quantum
    )

    if r_score and r_score >= 0.50 and (not avg_score or r_score >= avg_score) then
        return r_num, r_den, avg_num, avg_den, r_lower, r_upper
    end
    if avg_score and avg_score >= 0.50 then
        return avg_num, avg_den, avg_num, avg_den, avg_lower, avg_upper
    end
    -- A genuine VFR stream may have no dominant frame duration. Its average
    -- rate still preserves the complete stream duration when the frame-indexed
    -- interpolation graph normalizes it to a CFR output timeline.
    if avg_num then
        local lower, upper = allowed_deltas(avg_num, avg_den, time_base, timestamp_quantum)
        return avg_num, avg_den, avg_num, avg_den, lower, upper
    end
    if r_num then
        local lower, upper = allowed_deltas(r_num, r_den, time_base, timestamp_quantum)
        return r_num, r_den, avg_num, avg_den, lower, upper
    end
    return nil, nil, avg_num, avg_den
end

local function packet_timestamps(packets)
    local timestamps = {}
    local last_dts
    for _, packet in ipairs(packets) do
        local pts = tonumber(packet.pts)
        if not pts then
            return nil, "ffprobe не вернул PTS сетевой видеодорожки"
        end
        timestamps[#timestamps + 1] = pts
        local dts = tonumber(packet.dts)
        if dts then
            last_dts = math.max(last_dts or dts, dts)
        end
    end

    -- Packets arrive in decode order. Check their presentation order, and
    -- exclude frames beyond the last DTS: a short probe can stop before the
    -- intervening B-frames arrive, leaving an artificial gap at its end.
    table.sort(timestamps)
    while last_dts and #timestamps > 0 and timestamps[#timestamps] > last_dts do
        table.remove(timestamps)
    end
    return timestamps
end

function M.analyze(document)
    if type(document) ~= "table" then
        return nil, "ffprobe вернул некорректный JSON"
    end

    local stream = type(document.streams) == "table" and document.streams[1] or nil
    if type(stream) ~= "table" then
        return nil, "ffprobe не нашел видеодорожку"
    end

    local avg_num, avg_den = parse_rational(stream.avg_frame_rate)

    local tb_num, tb_den = parse_rational(stream.time_base)
    local time_base = tb_num and tb_den and tb_num / tb_den or nil
    if not time_base or time_base <= 0 then
        return {
            status = "unavailable",
            vfr = "unknown",
            source_num = 0,
            source_den = 0,
            avg_num = avg_num or 0,
            avg_den = avg_den or 0,
            reason = "ffprobe не вернул time_base видеодорожки",
        }
    end

    local timestamps = {}
    local sample_size = frames_per_sample
    if type(document.packets) == "table" then
        local timestamp_error
        timestamps, timestamp_error = packet_timestamps(document.packets)
        if not timestamps then
            return nil, timestamp_error
        end
        sample_size = math.max(1, #timestamps)
    else
        for index, frame in ipairs(document.frames or {}) do
            timestamps[index] = tonumber(frame.best_effort_timestamp) or false
        end
    end

    local deltas = {}
    local previous = nil
    local invalid_timestamps = 0
    local sample_count = 0
    for index, timestamp in ipairs(timestamps) do
        if (index - 1) % sample_size == 0 then
            previous = nil
            sample_count = sample_count + 1
        end

        if timestamp and previous then
            local delta = timestamp - previous
            if delta > 0 then
                deltas[#deltas + 1] = delta
            else
                invalid_timestamps = invalid_timestamps + 1
            end
        end
        if not timestamp and index % sample_size ~= 0 then
            invalid_timestamps = invalid_timestamps + 1
        end
        previous = timestamp
    end

    if #deltas < minimum_deltas then
        return {
            status = "unavailable",
            vfr = "unknown",
            source_num = 0,
            source_den = 0,
            avg_num = avg_num or 0,
            avg_den = avg_den or 0,
            reason = "недостаточно timestamp-дельт для проверки CFR/VFR",
        }
    end
    if invalid_timestamps > 0 then
        return {
            status = "unavailable",
            vfr = "unknown",
            source_num = 0,
            source_den = 0,
            avg_num = avg_num or 0,
            avg_den = avg_den or 0,
            reason = string.format(
                "не удалось надежно проверить %d timestamp-позиций",
                invalid_timestamps
            ),
        }
    end

    local timestamp_quantum = deltas[1]
    local shortest_delta = deltas[1]
    for index = 2, #deltas do
        timestamp_quantum = gcd(timestamp_quantum, deltas[index])
        shortest_delta = math.min(shortest_delta, deltas[index])
    end
    -- Whole-frame gaps share the nominal frame duration as their GCD. That
    -- duration is not a coarse timestamp clock and must not hide real VFR.
    if timestamp_quantum == shortest_delta then
        timestamp_quantum = 1
    end

    local source_num, source_den, _, _, lower_delta, upper_delta = choose_nominal(
        stream,
        deltas,
        time_base,
        timestamp_quantum
    )
    if not source_num then
        return {
            status = "unavailable",
            vfr = "unknown",
            source_num = 0,
            source_den = 0,
            avg_num = avg_num or 0,
            avg_den = avg_den or 0,
            reason = "timestamp-дельты не совпадают с r_frame_rate или avg_frame_rate",
        }
    end

    -- Some containers expose a finer stream time_base than their actual
    -- timestamp quantum. WebM VP9 commonly reports 1/16000 while its 1 ms
    -- timecodes produce 656/672 ticks for 24 fps. Quantize the expected CFR
    -- duration to the observed common tick, rather than calling this VFR.
    local outliers = 0
    local delta_sum = 0
    for _, delta in ipairs(deltas) do
        delta_sum = delta_sum + delta
        if delta ~= lower_delta and delta ~= upper_delta then
            outliers = outliers + 1
        end
    end

    local nominal_delta = 1 / (rate(source_num, source_den) * time_base)
    local mean_tolerance = math.max(
        0.05,
        timestamp_quantum * math.max(1, sample_count) / #deltas + 1e-6
    )
    if math.abs(delta_sum / #deltas - nominal_delta) > mean_tolerance then
        outliers = math.max(outliers, 1)
    end

    if outliers > 0 then
        return {
            status = "ready",
            vfr = true,
            source_num = source_num,
            source_den = source_den,
            avg_num = avg_num or 0,
            avg_den = avg_den or 0,
            reason = string.format(
                "VFR нормализуется: %d/%d timestamp-дельт отличаются от %.6g fps",
                outliers,
                #deltas,
                rate(source_num, source_den)
            ),
        }
    end

    return {
        status = "ready",
        vfr = false,
        source_num = source_num,
        source_den = source_den,
        avg_num = avg_num or 0,
        avg_den = avg_den or 0,
        reason = string.format(
            "CFR подтвержден %d timestamp-дельтами: %d/%d fps (шаг %d tick)",
            #deltas,
            source_num,
            source_den,
            timestamp_quantum
        ),
    }
end

local function build_intervals(duration)
    local intervals = {}
    for _, fraction in ipairs(sample_fractions) do
        intervals[#intervals + 1] = string.format(
            "%.3f%%+#%d",
            duration * fraction,
            frames_per_sample
        )
    end
    return table.concat(intervals, ",")
end

function M.start(path, duration, opts, callback)
    opts = opts or {}
    duration = tonumber(duration)
    local video_stream = opts.video_stream
    if type(path) ~= "string" or path == "" then
        return nil, "источник timing-probe недоступен"
    end
    if not opts.network and (not duration or duration <= 0) then
        return nil, "для локального timing-probe нужна известная длительность"
    end
    if type(callback) ~= "function" then
        return nil, "callback не задан"
    end
    if type(video_stream) ~= "string"
            or not (video_stream:match("^%d+$") or video_stream:match("^v:%d+$")) then
        return nil, "FFmpeg selector видеодорожки недоступен"
    end

    local ffprobe = opts.ffprobe
    if type(ffprobe) ~= "string" or ffprobe == "" then
        return nil, "bundled ffprobe не найден"
    end

    local timeout = math.max(1, math.min(tonumber(opts.timeout) or 8, 30))
    local task = {}
    local command
    local timer
    local done = false

    local args = { ffprobe, "-v", "error" }
    if opts.network then
        local headers = opts.http_headers or {}
        for _, option in ipairs({
            { "-rw_timeout", tostring(timeout * 1000000) },
            { "-http_proxy", opts.http_proxy },
            { "-user_agent", opts.user_agent },
            { "-referer", opts.referrer },
            { "-headers", #headers > 0 and table.concat(headers, "\r\n") .. "\r\n" },
            { "-request_size", opts.request_size and tostring(opts.request_size) },
        }) do
            if option[2] and option[2] ~= "" then
                args[#args + 1] = option[1]
                args[#args + 1] = option[2]
            end
        end
    end
    for _, argument in ipairs({
        "-select_streams", video_stream,
        "-show_entries", "stream=avg_frame_rate,r_frame_rate,time_base",
        "-show_entries", opts.network and "packet=pts,dts" or "frame=best_effort_timestamp",
        "-read_intervals", opts.network
            and string.format("%%+#%d", frames_per_sample * #sample_fractions)
            or build_intervals(duration),
        "-of", "json",
        "--", path,
    }) do
        args[#args + 1] = argument
    end

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
            local http_error = (result.stderr or ""):match("HTTP error (%d%d%d)")
            local reason = http_error and "timing-запрос: HTTP " .. http_error
                or result.error_string
            if not reason or reason == "" then
                reason = error or "ffprobe timing-probe завершился с ошибкой"
            end
            finish(nil, reason)
            return
        end

        local ok, document = pcall(require("mp.utils").parse_json, result.stdout or "")
        if not ok or type(document) ~= "table" then
            finish(nil, "ffprobe вернул некорректный JSON")
            return
        end

        local timing, analyze_error = M.analyze(document)
        finish(timing, analyze_error)
    end)

    timer = mp.add_timeout(timeout, function()
        if command then
            mp.abort_async_command(command)
            command = nil
        end
        finish(nil, string.format("timing-анализ превысил %.0f секунд", timeout))
    end)

    return task
end

return M
