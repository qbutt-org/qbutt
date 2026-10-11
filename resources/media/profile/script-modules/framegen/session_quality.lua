local M = {}

M.SAMPLE_SECONDS = 4
-- A successful vf command precedes asynchronous filter/VO reconfiguration.
M.TRANSITION_SETTLE_SECONDS = 1
-- Slow graph initialization gets active playback time to expose its real rate.
M.TRANSITION_READY_TIMEOUT_SECONDS = 15
M.OUTPUT_RATE_TOLERANCE = 0.03

local initial_multipliers = {
    trt = 4,
}

local function positive(value)
    value = tonumber(value)
    return value and value > 0 and value or nil
end

local function positive_integer(value)
    value = positive(value)
    if not value or value == math.huge or value ~= math.floor(value) then
        return nil
    end
    return value
end

local function gcd(left, right)
    while right ~= 0 do
        left, right = right, left % right
    end
    return math.abs(left)
end

local function rate(num, den)
    num = positive_integer(num)
    den = positive_integer(den)
    if not num or not den then
        return nil
    end
    local divisor = gcd(num, den)
    num = num / divisor
    den = den / divisor
    return { num = num, den = den, fps = num / den }
end

local function compare_rates(left, right)
    local difference = left.num * right.den - right.num * left.den
    return difference < 0 and -1 or difference > 0 and 1 or 0
end

local function adjacent_target(source, display)
    -- For target/source = (n + 1) / n, exactly one output in every n + 1
    -- coincides with a decoded frame. The remaining n RIFE calls therefore
    -- cost exactly source FPS while keeping the highest useful aligned target.
    for n = 2, 8 do
        local candidate = rate(source.num * (n + 1), source.den * n)
        if compare_rates(candidate, display) <= 0 then
            return candidate
        end
    end
end

function M.target_ladder(source_num, source_den, display_num, display_den, backend)
    if backend ~= "trt" and backend ~= "openvino" then
        return {}
    end
    local source = rate(source_num, source_den)
    local display = rate(display_num, display_den)
    if not source or not display or compare_rates(display, source) <= 0 then
        return {}
    end

    local result = {}
    local seen = {}
    local function add(candidate, multiplier)
        local key = candidate.num .. "/" .. candidate.den
        if seen[key] then
            return
        end
        seen[key] = true
        candidate.multiplier = multiplier
        result[#result + 1] = candidate
    end

    local double = rate(source.num * 2, source.den)
    if compare_rates(display, double) < 0 then
        add(display)
        local adjacent = adjacent_target(source, display)
        if adjacent then
            add(adjacent)
        end
        return result
    end

    if backend == "openvino" then
        add(double, 2)
        return result
    end

    -- Exact source multiples make RIFE demand decrease monotonically between
    -- rungs; arbitrary lower output rates can require more interpolations.
    local ratio_num = display.num * source.den
    local ratio_den = display.den * source.num
    local display_multiplier
    if ratio_num % ratio_den == 0 then
        display_multiplier = ratio_num / ratio_den
    end
    add(display, display_multiplier)

    for multiplier = math.floor(ratio_num / ratio_den), 2, -1 do
        local candidate = rate(source.num * multiplier, source.den)
        if compare_rates(candidate, display) < 0 then
            add(candidate, multiplier)
        end
    end
    return result
end

function M.initial_target(ladder, backend)
    ladder = type(ladder) == "table" and ladder or {}
    if backend == "openvino" then
        return ladder[1], #ladder > 0 and 1 or nil
    end
    local maximum_multiplier = initial_multipliers[backend]
    if not maximum_multiplier then
        return nil
    end
    for index, candidate in ipairs(ladder) do
        if candidate.multiplier and candidate.multiplier <= maximum_multiplier then
            return candidate, index
        end
    end
    return ladder[1], #ladder > 0 and 1 or nil
end

function M.find_target(ladder, num, den)
    local wanted = rate(num, den)
    if not wanted then
        return nil
    end
    for index, candidate in ipairs(type(ladder) == "table" and ladder or {}) do
        if compare_rates(candidate, wanted) == 0 then
            return index
        end
    end
    return nil
end

local counter_rules = {
    { "frame_drops", 1 },
    { "decoder_drops", 0 },
    { "mistimed_frames", 1 },
    { "delayed_frames", 2 },
}

local function delta(first, last, key)
    local before = tonumber(first[key])
    local after = tonumber(last[key])
    if not before or not after or after < before then
        return nil
    end
    return after - before
end

function M.counter_epoch_changed(first, last)
    if type(first) ~= "table" or type(last) ~= "table" then
        return false
    end
    for _, rule in ipairs(counter_rules) do
        local before = tonumber(first[rule[1]])
        local after = tonumber(last[rule[1]])
        if before and after and after < before then
            return true
        end
    end
    return false
end

function M.output_rate_matches(actual_fps, target_fps)
    actual_fps = positive(actual_fps)
    target_fps = positive(target_fps)
    return actual_fps and target_fps
        and math.abs(actual_fps - target_fps) <= target_fps * M.OUTPUT_RATE_TOLERANCE
        or false
end

function M.sample_verdict(first, last, seconds, target_fps)
    seconds = positive(seconds)
    target_fps = positive(target_fps)
    if type(first) ~= "table" or type(last) ~= "table" or not seconds or not target_fps then
        return false, true, { "invalid_sample" }
    end

    local failures = {}
    local severe = false
    local progress = (tonumber(last.time_pos) or 0) - (tonumber(first.time_pos) or 0)
    if progress < seconds * 0.90 then
        failures[#failures + 1] = "playback_progress"
        severe = progress < seconds * 0.75
    end
    local output_fps = tonumber(last.estimated_vf_fps) or 0
    if output_fps < target_fps * (1 - M.OUTPUT_RATE_TOLERANCE) then
        failures[#failures + 1] = "filter_output_rate"
        severe = severe or output_fps < target_fps * 0.90
    end
    for _, rule in ipairs(counter_rules) do
        local value = delta(first, last, rule[1])
        if value == nil or value > rule[2] then
            failures[#failures + 1] = rule[1]
            severe = severe or rule[1] == "frame_drops"
                and value and value > math.max(4, target_fps * seconds * 0.03)
        end
    end
    return #failures == 0, severe, failures
end

return M
