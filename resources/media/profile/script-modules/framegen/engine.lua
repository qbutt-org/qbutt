local utils = require "mp.utils"
local msg = require "mp.msg"
local worker_path = mp.command_native({ "expand-path", "~~/framegen/prepare.py" })
local M = {}
local task

function M.reset()
    local previous = task
    task = nil
    if previous and previous.command then
        mp.abort_async_command(previous.command)
    end
end

local function run(request, command, callback)
    command.name = "subprocess"
    command.playback_only = true
    command.capture_stdout = true
    command.capture_stderr = true
    request.command = mp.command_native_async(command, function(success, result, error)
        if task ~= request then
            return
        end
        request.command = nil
        if not success or not result or result.status ~= 0 then
            request.status = "error"
            request.reason = "Не удалось подготовить TensorRT; видео продолжает играть без фреймгена"
            msg.error(tostring(error or (result and result.stderr) or "worker failed"))
            request.changed()
            return
        end
        callback(result)
    end)
end

function M.prepare(config, runtime, changed, benchmark)
    local key = table.concat({
        runtime.engine_dir, config.model, config.output_width, config.output_height,
        config.process_scale, config.device_id,
    }, ":")
    if task and task.key == key then
        return task.status, task.reason
    end
    M.reset()
    local request = {
        key = key,
        status = "loading",
        reason = "Готовим TensorRT в фоне; видео продолжает играть",
        changed = changed,
    }
    task = request
    local args = { utils.join_path(runtime.python_home, "python.exe"), "-B",
        worker_path, utils.format_json(config) }
    -- Let the caller remove the old playback graph before this worker takes
    -- the shared GPU lock. Otherwise a resolution change could race itself.
    mp.add_timeout(0, function()
        if task ~= request then
            return
        end
        run(request, { args = args }, function(result)
            local response = utils.parse_json(result.stdout)
            if type(response) ~= "table" or type(response.ready) ~= "boolean" then
                request.status = "error"
                request.reason = "TensorRT worker вернул некорректный результат"
                msg.error(result.stdout)
                changed()
                return
            end
            if response.ready then
                request.status = "ready"
                changed()
                return
            end
            args[#args + 1] = "--build"
            run(request, { args = args }, function()
                -- Preserve this video's working playback. Use the compiled engine
                -- on the next file or an explicit selection in the existing FX menu.
                request.status = benchmark and "ready" or "prepared"
                request.reason = benchmark and "" or
                    "TensorRT готов; включи Frame generation в FX или открой следующее видео"
                changed()
            end)
        end)
    end)
    return request.status, request.reason
end

return M
