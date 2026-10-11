local msg = require "mp.msg"
local options = require "mp.options"
local utils = require "mp.utils"

local user_opts = {
    buffered_frames = 8,
    concurrent_frames = 8,
    duplicate_threshold = 0.001,
    benchmark = false,
    benchmark_pipeline_id = "",
    benchmark_target_num = 0,
    benchmark_target_den = 0,
    benchmark_target_fps = 0,
    benchmark_source_num = 0,
    benchmark_source_den = 0,
    benchmark_process_scale = 0,
    benchmark_gpu_lock_owned = false,
    benchmark_gpu_lock_bypassed = false,
}
options.read_options(user_opts, "framegen")

local FILTER_LABEL = "mpv-framegen"
local runtime_dir = os.getenv("QBUTT_MEDIA_RUNTIME")
local manifest_path = runtime_dir and utils.join_path(runtime_dir, "runtime-manifest.json")
local script_path = mp.command_native({ "expand-path", "~~/framegen/framegen.vpy" })
local bootstrap_path = mp.command_native({
    "expand-path",
    "~~/tools/bin/framegen-bootstrap.dll",
})
local BOOTSTRAP_REQUEST_PROPERTY = "user-data/framegen-bootstrap-request"
local BOOTSTRAP_RESULT_PROPERTY = "user-data/framegen-bootstrap-result"
local BOOTSTRAP_TIMEOUT = 5

local function load_module(name)
    local path = mp.command_native({
        "expand-path",
        "~~/script-modules/framegen/" .. name .. ".lua",
    })
    local chunk, load_error = loadfile(path)
    if not chunk then
        error("Failed to load framegen module " .. name .. ": " .. tostring(load_error))
    end
    return chunk()
end

local catalog = load_module("catalog")
local policy = load_module("policy")
local session_quality = load_module("session_quality")
local engine = load_module("engine")
local framegen_override
local effective_framegen_enabled = false
local current_filter
local failed_filter
local failed_reason
local loaded = false
local applying = false
local reevaluate_timer
local interlaced_seen = false
local quality_state
local filter_generation = 0
local schedule_reevaluate
local mark_quality_transition
local bootstrap_request_id
local bootstrap_runtime_key
local bootstrap_started_at
local bootstrap_failure_key
local bootstrap_failure_reason
local bootstrap_failure_restart_required = false

local function read_json(path)
    if not path then
        return nil, "Каталог qbutt runtime недоступен"
    end
    local handle, open_error = io.open(path, "rb")
    if not handle then
        return nil, open_error
    end
    local contents = handle:read("*a")
    handle:close()
    local value, parse_error = utils.parse_json(contents)
    if type(value) ~= "table" then
        return nil, parse_error or "JSON root должен быть объектом"
    end
    return value
end

local function file_exists(path)
    local info = type(path) == "string" and utils.file_info(path) or nil
    return info and info.is_file or false
end

local function dir_exists(path)
    local info = type(path) == "string" and utils.file_info(path) or nil
    return info and info.is_dir or false
end

local function path_key(path)
    local normalized = tostring(path or ""):gsub("^%s+", ""):gsub("%s+$", "")
    if normalized:sub(1, 1) == "\"" and normalized:sub(-1) == "\"" then
        normalized = normalized:sub(2, -2)
    end
    normalized = normalized:gsub("/", "\\")
    local prefix = normalized:sub(1, 8):lower()
    if prefix == "\\\\?\\unc\\" then
        normalized = "\\\\" .. normalized:sub(9)
    elseif normalized:sub(1, 4) == "\\\\?\\" then
        normalized = normalized:sub(5)
    end
    return normalized
        :gsub("\\+$", "")
        :lower()
end

local function text_key(value)
    return tostring(value or ""):match("^%s*(.-)%s*$"):lower()
end

local function nonnegative_integer(value)
    return type(value) == "number" and value >= 0 and value == math.floor(value)
end

local function model_is_installed(runtime, pipeline)
    return runtime
        and pipeline
        and type(pipeline.model_file) == "string"
        and file_exists(utils.join_path(runtime.model_dir, pipeline.model_file))
end

local function runtime_key(runtime)
    local directories = {}
    for _, path in ipairs(runtime.dll_dirs or {}) do
        directories[#directories + 1] = path_key(path)
    end
    return table.concat({
        tostring(runtime.runtime_id or ""),
        path_key(runtime.version_dir),
        path_key(runtime.bootstrap_vsscript_path
            or utils.join_path(runtime.vsscript_dir or "", "vsscript.dll")),
        table.concat(directories, ";"),
    }, "|")
end

local function read_bootstrap_result()
    local raw = mp.get_property_native(BOOTSTRAP_RESULT_PROPERTY, "")
    if raw == "" then
        return nil
    end
    local result = utils.parse_json(raw)
    return type(result) == "table" and result or nil
end

local function matching_bootstrap_result(runtime)
    local result = read_bootstrap_result()
    if not result
        or result.schema_version ~= 1
        or result.ok ~= true
        or result.runtime_id ~= runtime.runtime_id
        or path_key(result.version_dir) ~= path_key(runtime.version_dir)
        or path_key(result.vsscript_path) ~= path_key(runtime.bootstrap_vsscript_path
            or utils.join_path(runtime.vsscript_dir, "vsscript.dll"))
        or type(result.dll_dirs) ~= "table"
        or #result.dll_dirs ~= #runtime.dll_dirs
    then
        return nil
    end
    for index, path in ipairs(runtime.dll_dirs) do
        if path_key(result.dll_dirs[index]) ~= path_key(path) then
            return nil
        end
    end
    return result
end

local function reset_bootstrap_attempt()
    bootstrap_request_id = nil
    bootstrap_runtime_key = nil
    bootstrap_started_at = nil
    bootstrap_failure_key = nil
    bootstrap_failure_reason = nil
    bootstrap_failure_restart_required = false
end

local function bootstrap_runtime(runtime)
    local key = runtime_key(runtime)
    local previous = matching_bootstrap_result(runtime)
    if previous then
        reset_bootstrap_attempt()
        return true
    end
    if bootstrap_failure_key == key then
        return false, bootstrap_failure_reason, true, bootstrap_failure_restart_required
    end

    if bootstrap_runtime_key ~= key then
        if not file_exists(bootstrap_path) then
            bootstrap_failure_key = key
            bootstrap_failure_reason = "Поврежден runtime: отсутствует framegen-bootstrap.dll"
            return false, bootstrap_failure_reason, true, false
        end
        bootstrap_request_id = string.format("%.6f:%s", mp.get_time(), key)
        local request = utils.format_json({
            schema_version = 1,
            request_id = bootstrap_request_id,
            runtime_id = runtime.runtime_id,
            version_dir = runtime.version_dir,
            vsscript_path = runtime.bootstrap_vsscript_path
                or utils.join_path(runtime.vsscript_dir, "vsscript.dll"),
        })
        if not request then
            bootstrap_failure_key = key
            bootstrap_failure_reason = "Не удалось подготовить запрос framegen bootstrap"
            return false, bootstrap_failure_reason, true, false
        end
        local cleared, clear_error = mp.set_property_native(BOOTSTRAP_RESULT_PROPERTY, "")
        if not cleared then
            bootstrap_failure_key = key
            bootstrap_failure_reason = "Не удалось очистить ответ framegen bootstrap: "
                .. tostring(clear_error)
            return false, bootstrap_failure_reason, true, false
        end
        local written, write_error = mp.set_property_native(BOOTSTRAP_REQUEST_PROPERTY, request)
        if not written then
            bootstrap_failure_key = key
            bootstrap_failure_reason = "Не удалось передать запрос framegen bootstrap: "
                .. tostring(write_error)
            return false, bootstrap_failure_reason, true, false
        end
        local called, loaded, load_error = pcall(
            mp.commandv,
            "load-script",
            bootstrap_path
        )
        if not called or not loaded then
            bootstrap_failure_key = key
            bootstrap_failure_reason = "Не удалось загрузить framegen bootstrap: "
                .. tostring(called and load_error or loaded)
            return false, bootstrap_failure_reason, true, false
        end
        bootstrap_runtime_key = key
        bootstrap_started_at = mp.get_time()
    end

    local result = read_bootstrap_result()
    if result and result.request_id == bootstrap_request_id then
        if matching_bootstrap_result(runtime) then
            reset_bootstrap_attempt()
            return true
        end
        if result.ok == false
            and result.schema_version == 1
            and result.runtime_id == runtime.runtime_id
            and path_key(result.version_dir) == path_key(runtime.version_dir)
        then
            bootstrap_failure_key = key
            bootstrap_failure_reason = result.error ~= "" and result.error
                or "Framegen bootstrap завершился с ошибкой"
            bootstrap_failure_restart_required = result.restart_required == true
            return false, bootstrap_failure_reason, true, bootstrap_failure_restart_required
        end
    end
    if mp.get_time() - (bootstrap_started_at or 0) >= BOOTSTRAP_TIMEOUT then
        bootstrap_failure_key = key
        bootstrap_failure_reason = "Framegen bootstrap не ответил за 5 секунд"
        return false, bootstrap_failure_reason, true, false
    end
    schedule_reevaluate(0.05)
    return false, "Подготавливаем frame generation", false, false
end

local function validate_runtime()
    local manifest, read_error = read_json(manifest_path)
    if not manifest then
        return nil, false, "Frame generation runtime не установлен: " .. tostring(read_error)
    end
    local backend = text_key(manifest.backend)
    if manifest.schema_version ~= 3
        or manifest.version ~= 3
        or type(manifest.runtime_id) ~= "string"
        or manifest.runtime_id == ""
        or manifest.ready ~= true
        or backend ~= "trt" and backend ~= "openvino"
        or type(manifest.files) ~= "table"
        or type(manifest.checksums) ~= "table"
        or manifest.checksums.algorithm ~= "sha256"
        or type(manifest.device_binding) ~= "table"
        or text_key(manifest.device_binding.vendor) == ""
        or text_key(manifest.device_binding.gpu_id) == ""
        or not nonnegative_integer(manifest.device_binding.discovery_index)
        or not nonnegative_integer(manifest.device_binding.device_ordinal)
        or manifest.device_binding.device_ordinal >= 32
        or text_key(manifest.device_binding.capability) == ""
        or text_key(manifest.device_binding.driver_version) == ""
    then
        return nil, true, "Frame generation runtime не готов или имеет несовместимую версию"
    end

    for _, field in ipairs({
        "vsscript_dir", "python_home", "plugin_dir", "model_dir", "engine_dir",
    }) do
        if not dir_exists(manifest[field]) then
            return nil, true, string.format(
                "Поврежден runtime: %s не указывает на каталог",
                field
            )
        end
    end

    local vsscript_path = utils.join_path(manifest.vsscript_dir, "vsscript.dll")
    local required_files = { vsscript_path }
    if backend == "trt" then
        for _, path in ipairs({
            "vstrt.dll", "vsmlrt.py", "vsmlrt-cuda\\nvinfer_10.dll",
            "vsmlrt-cuda\\nvinfer_plugin_10.dll",
            "vsmlrt-cuda\\nvonnxparser_10.dll",
            "vsmlrt-cuda\\nvinfer_builder_resource_ptx_10.dll",
            "vsmlrt-cuda\\trtexec.exe",
        }) do
            required_files[#required_files + 1] = utils.join_path(manifest.plugin_dir, path)
        end
    else
        required_files[#required_files + 1] = utils.join_path(manifest.plugin_dir, "vsov.dll")
    end
    for _, path in ipairs(required_files) do
        if not file_exists(path) then
            return nil, true, "Поврежден runtime: отсутствует файл " .. tostring(path)
        end
    end
    if backend == "trt" then
        local cuda_dir = utils.join_path(manifest.plugin_dir, "vsmlrt-cuda")
        local has_sm_builder = false
        for _, name in ipairs(utils.readdir(cuda_dir, "files") or {}) do
            if name:match("^nvinfer_builder_resource_sm%d+_10%.dll$") then
                has_sm_builder = true
                break
            end
        end
        if not has_sm_builder then
            return nil, true, "Поврежден runtime: отсутствует TensorRT builder для GPU"
        end
    end

    if type(manifest.version_dir) ~= "string" or not dir_exists(manifest.version_dir) then
        return nil, true, "Поврежден runtime: version_dir не указывает на каталог"
    end
    if type(manifest.dll_dirs) ~= "table" or #manifest.dll_dirs ~= 1 then
        return nil, true, "Поврежден runtime: dll_dirs отсутствует в manifest"
    end
    for _, path in ipairs(manifest.dll_dirs) do
        if not dir_exists(path) then
            return nil, true, "Поврежден runtime: отсутствует DLL-каталог " .. tostring(path)
        end
    end
    if backend == "trt" and text_key(os.getenv("CUDA_VISIBLE_DEVICES")) ~= "" then
        return nil, true,
            "Уберите CUDA_VISIBLE_DEVICES и перезапустите плеер: runtime сам выбирает GPU", true
    end
    manifest.bootstrap_vsscript_path = vsscript_path
    if not matching_bootstrap_result(manifest) then
        return nil, true, "Подготавливаем frame generation", false, manifest
    end
    return manifest, true
end

local function runtime_profile(runtime)
    local profile = {
        runtime_available = runtime ~= nil,
        pipelines = {},
    }
    for _, pipeline in ipairs(catalog.pipelines or {}) do
        profile.pipelines[pipeline.id] = {
            available = model_is_installed(runtime, pipeline),
        }
    end
    return profile
end

local function has_available_pipeline(profile, runtime)
    if not runtime then
        return false
    end
    for _, pipeline in ipairs(catalog.pipelines or {}) do
        local capability = profile.pipelines[pipeline.id]
        if type(capability) == "table"
                and capability.available == true
                and model_is_installed(runtime, pipeline) then
            return true
        end
    end
    return false
end

local function normalize_content(value)
    value = tostring(value or "unknown"):lower()
    if value == "film" or value == "video" or value == "live-action" then
        return "movie"
    end
    return value == "anime" and "anime" or value == "movie" and "movie" or "unknown"
end

local function first_number(...)
    for index = 1, select("#", ...) do
        local candidate = select(index, ...)
        local value = tonumber(candidate)
        if value and value > 0 then
            return value
        end
    end
end

local function gcd(a, b)
    while b ~= 0 do
        a, b = b, a % b
    end
    return math.abs(a)
end

local function rational(num, den)
    num = tonumber(num)
    den = tonumber(den)
    if not num or not den or num <= 0 or den <= 0 then
        return nil, nil
    end
    num = math.floor(num + 0.5)
    den = math.floor(den + 0.5)
    local divisor = gcd(num, den)
    return num / divisor, den / divisor
end

local function approximate_rational(value)
    value = tonumber(value)
    if not value or value <= 0 then
        return nil, nil
    end
    local common = {
        { 24000, 1001 }, { 30000, 1001 }, { 48000, 1001 },
        { 60000, 1001 }, { 120000, 1001 }, { 144000, 1001 },
    }
    for _, rate in ipairs(common) do
        if math.abs(value - rate[1] / rate[2]) < 0.01 then
            return rate[1], rate[2]
        end
    end
    return rational(math.floor(value * 1000 + 0.5), 1000)
end

local function collect_info()
    local params = mp.get_property_native("video-params", {}) or {}
    local decoded = mp.get_property_native("video-dec-params", {}) or {}
    local container_fps = first_number(mp.get_property_number("container-fps"))
    local display_fps = first_number(mp.get_property_number("display-fps"))
    local display_num, display_den = approximate_rational(display_fps)
    -- video_profile publishes a native map. Reading a string leaf through
    -- mp.get_property() returns its JSON representation (for example
    -- `"ready"`), so keep this contract native end to end.
    local profile_status = mp.get_property_native("user-data/video-profile/status", "idle")
    local timing_status = mp.get_property_native(
        "user-data/video-profile/timing-status",
        "unknown"
    )
    local vfr = mp.get_property_native("user-data/video-profile/vfr")
    local source_num, source_den = rational(
        mp.get_property_number("user-data/video-profile/source-num"),
        mp.get_property_number("user-data/video-profile/source-den")
    )
    local source_fps_source = "verified-profile"
    if timing_status ~= "ready" or type(vfr) ~= "boolean" then
        vfr = "unknown"
        source_num, source_den = nil, nil
        source_fps_source = "unknown"
    elseif not source_num then
        source_fps_source = "unknown"
    end
    if user_opts.benchmark then
        source_num, source_den = rational(
            user_opts.benchmark_source_num,
            user_opts.benchmark_source_den
        )
        source_fps_source = source_num and "benchmark-ffprobe" or "unknown"
    end
    local source_fps = source_num and source_num / source_den or nil

    return {
        has_video = mp.get_property("vid", "no") ~= "no",
        width = first_number(params.w, decoded.w, mp.get_property_number("width")),
        height = first_number(params.h, decoded.h, mp.get_property_number("height")),
        source_fps = source_fps,
        source_num = source_num,
        source_den = source_den,
        source_fps_source = source_fps_source,
        container_fps = container_fps,
        display_fps = display_fps,
        display_num = display_num,
        display_den = display_den,
        profile_status = profile_status,
        timing_status = timing_status,
        content_kind = profile_status == "ready"
            and normalize_content(mp.get_property_native("user-data/video-profile/kind"))
            or "unknown",
        content_confidence = mp.get_property_number("user-data/video-profile/confidence", 0),
        interlaced = interlaced_seen and true or false,
        vfr = vfr,
        matrix = params.colormatrix,
        range = params.colorlevels,
    }
end

local function user_summary(selection, info)
    if not selection.enabled then
        return ""
    end
    local kind = info.content_kind == "anime" and "Аниме"
        or info.content_kind == "movie" and "Фильм" or "Видео"
    return string.format("%s, %.1f FPS", kind, selection.target_fps)
end

local function desired_enabled(info)
    if user_opts.benchmark then
        return true
    end
    if framegen_override ~= nil then
        return framegen_override
    end
    return info.content_kind == "movie"
end

local function publish(selection, info, runtime, installed, restart_required)
    local pipeline = selection.pipeline_id and catalog.find(selection.pipeline_id)
        or catalog.pipelines and catalog.pipelines[1]
    local state = {
        selection = user_opts.benchmark and (selection.selection or "")
            or effective_framegen_enabled and "on" or "off",
        enabled = effective_framegen_enabled,
        status = selection.status or "error",
        summary = user_summary(selection, info),
        reason = selection.reason or "",
        available = selection.available == true,
        installed = installed == true,
        ["restart-required"] = restart_required == true,
        ["content-kind"] = info.content_kind or "unknown",
        interlaced = info.interlaced,
        ["pipeline-id"] = selection.pipeline_id or "",
        ["pipeline-name"] = pipeline and pipeline.name or "",
        backend = selection.backend or "",
        model = selection.model or "",
        ["target-fps"] = selection.target_fps or 0,
        ["target-num"] = selection.target_num or 0,
        ["target-den"] = selection.target_den or 0,
        ["source-fps"] = info.source_fps or 0,
        ["source-num"] = info.source_num or 0,
        ["source-den"] = info.source_den or 0,
        ["source-fps-source"] = info.source_fps_source or "unknown",
        ["container-fps"] = info.container_fps or 0,
        ["display-fps"] = info.display_fps or 0,
        ["display-num"] = info.display_num or 0,
        ["display-den"] = info.display_den or 0,
        ["process-scale"] = selection.process_scale or 0,
        ["output-width"] = selection.output_width or info.width or 0,
        ["output-height"] = selection.output_height or info.height or 0,
        ["fps-kind"] = selection.fps_kind or "",
    }
    if not installed then
        local setup = mp.get_property_native("user-data/qbutt-runtime")
        if type(setup) == "table" then
            if setup.status == "checking" or setup.status == "preparing" then
                state.status = "loading"
            elseif setup.status == "error" or setup.status == "unavailable" then
                state.status = setup.status
            end
            state.reason = setup.reason or state.reason
        end
    end
    mp.set_property_native("user-data/framegen", state)
end

local function has_filter()
    for _, entry in ipairs(mp.get_property_native("vf", {}) or {}) do
        if entry.label == FILTER_LABEL then
            return true
        end
    end
    return false
end

local function run_vf(operation, value)
    return mp.command_native({ "vf", operation, value })
end

local function remove_filter()
    if not has_filter() then
        current_filter = nil
        return true
    end

    local _, remove_error = run_vf("remove", "@" .. FILTER_LABEL)
    if remove_error or has_filter() then
        local reason = "Не удалось удалить frame generation filter: "
            .. tostring(remove_error or "filter remained active")
        msg.error(reason)
        return false, reason
    end

    current_filter = nil
    return true
end

local MATRIX_MAP = {
    ["bt.709"] = "709",
    ["bt.601"] = "470bg",
    ["smpte-240m"] = "240m",
    ["bt.2020-ncl"] = "2020ncl",
}

local function color_matrix(info)
    local source_matrix = tostring(info.matrix or ""):lower()
    if source_matrix == "" or source_matrix == "auto" or source_matrix == "unknown" then
        return info.width >= 1280 and "709" or "470bg"
    end
    return MATRIX_MAP[source_matrix]
end

local function build_filter(selection, info, runtime)
    local pipeline = catalog.find(selection.pipeline_id)
    if not pipeline then
        return nil, "Policy выбрал неизвестный пайплайн"
    end
    if not model_is_installed(runtime, pipeline) then
        return nil, "Выбранная модель не установлена"
    end
    local matrix = color_matrix(info)
    if not matrix then
        return nil, "Цветовая матрица видео не поддерживается: " .. tostring(info.matrix)
    end
    local duplicate_threshold = tonumber(user_opts.duplicate_threshold)
    if not duplicate_threshold
        or duplicate_threshold < 0
        or duplicate_threshold >= 1
    then
        return nil, "duplicate_threshold должен быть от 0 до 1"
    end
    local buffered_frames = tonumber(user_opts.buffered_frames)
    local concurrent_frames = tonumber(user_opts.concurrent_frames)
    if not buffered_frames or not concurrent_frames then
        return nil, "Параметры буфера frame generation должны быть числами"
    end
    local gpu_lock_owned = user_opts.benchmark_gpu_lock_owned == true
    local gpu_lock_bypassed = user_opts.benchmark_gpu_lock_bypassed == true
    if gpu_lock_owned and gpu_lock_bypassed then
        return nil, "GPU lock нельзя одновременно удерживать и обходить"
    end

    local config = {
        schema_version = 1,
        backend = runtime.backend,
        model = pipeline.model,
        source_num = info.source_num,
        source_den = info.source_den,
        target_num = selection.target_num,
        target_den = selection.target_den,
        process_scale = selection.process_scale,
        output_width = selection.output_width or info.width,
        output_height = selection.output_height or info.height,
        scene_threshold = selection.scene_threshold,
        duplicate_threshold = duplicate_threshold,
        gpu_lock_owned = gpu_lock_owned,
        gpu_lock_bypassed = gpu_lock_bypassed,
        matrix = matrix,
        range = info.range == "full" and "full" or "limited",
        plugin_dir = runtime.plugin_dir,
        model_dir = runtime.model_dir,
        engine_dir = runtime.engine_dir,
        device_id = runtime.device_binding.device_ordinal,
        gpu_id = runtime.device_binding.gpu_id,
    }
    local data = utils.format_json(config)
    if not data then
        return nil, "Не удалось сериализовать параметры framegen"
    end

    local function quote(value)
        return string.format("%%%d%%%s", #value, value)
    end
    return string.format(
        "@%s:vapoursynth=file=%s:buffered-frames=%d:concurrent-frames=%d:user-data=%s",
        FILTER_LABEL,
        quote(script_path),
        math.max(4, math.floor(buffered_frames)),
        math.max(1, math.floor(concurrent_frames)),
        quote(data)
    ), nil, config
end

local function apply_selection(selection, info, runtime, installed, restart_required)
    if not selection.enabled then
        engine.reset()
        local removed, remove_error = remove_filter()
        if not removed then
            selection.status = "error"
            selection.reason = remove_error
        end
        publish(selection, info, runtime, installed, restart_required)
        return
    end

    local filter, build_error, config = build_filter(selection, info, runtime)
    if not filter then
        local removed, remove_error = remove_filter()
        selection.status = removed and "unavailable" or "error"
        selection.enabled = false
        selection.reason = remove_error or build_error
        publish(selection, info, runtime, installed, restart_required)
        return
    end
    if current_filter == filter and has_filter() then
        selection.status = "active"
        publish(selection, info, runtime, installed, restart_required)
        return
    end
    if failed_filter == filter then
        local _, remove_error = remove_filter()
        selection.status = "error"
        selection.enabled = false
        selection.reason = remove_error or failed_reason
        publish(selection, info, runtime, installed, restart_required)
        return
    end

    if runtime.backend == "trt" then
        local status, reason = engine.prepare(config, runtime, function()
            schedule_reevaluate(0)
        end, user_opts.benchmark)
        if status ~= "ready" then
            local removed, remove_error = remove_filter()
            selection.status = not removed and "error"
                or status == "prepared" and "guarded" or status
            selection.enabled = removed and status == "loading"
            selection.reason = remove_error or reason
            publish(selection, info, runtime, installed, restart_required)
            return
        end
    end

    selection.status = "starting"
    publish(selection, info, runtime, installed, restart_required)
    applying = true
    local _, add_error = run_vf("add", filter)
    applying = false
    if not add_error and has_filter() then
        current_filter = filter
        failed_filter = nil
        failed_reason = nil
        selection.status = "active"
        mark_quality_transition()
    else
        current_filter = nil
        failed_filter = filter
        failed_reason = "Не удалось запустить frame generation: "
            .. tostring(add_error or "filter rejected")
        selection.status = "error"
        selection.enabled = false
        selection.reason = failed_reason
        msg.error(failed_reason)
        local _, remove_error = remove_filter()
        if remove_error then
            selection.reason = remove_error
        end
    end
    publish(selection, info, runtime, installed, restart_required)
end

local function quality_snapshot()
    return {
        time_pos = mp.get_property_number("time-pos", 0),
        estimated_vf_fps = mp.get_property_number("estimated-vf-fps", 0),
        frame_drops = mp.get_property_number("frame-drop-count", 0),
        decoder_drops = mp.get_property_number("decoder-frame-drop-count", 0),
        delayed_frames = mp.get_property_number("vo-delayed-frame-count", 0),
        mistimed_frames = mp.get_property_number("mistimed-frame-count", 0),
    }
end

local function reset_quality_sample()
    if quality_state then
        quality_state.sample_started = nil
        quality_state.first = nil
        if quality_state.counter_baseline_pending then
            quality_state.counter_settle_started_at = nil
            quality_state.counter_timeout_started_at = nil
        end
    end
end

local function defer_quality_counter_baseline(state, restart_timeout)
    state.counter_baseline_pending = true
    state.counter_settle_started_at = nil
    if restart_timeout then
        state.counter_timeout_started_at = nil
    end
    state.sample_started = nil
    state.first = nil
end

mark_quality_transition = function()
    filter_generation = filter_generation + 1
    if quality_state then
        quality_state.filter_generation = filter_generation
        defer_quality_counter_baseline(quality_state, true)
    end
end

local function same_rate(left_num, left_den, right_num, right_den)
    return left_num and left_den and right_num and right_den
        and left_num * right_den == right_num * left_den
end

local function quality_source_matches(state, info, backend)
    return state
        and state.source_width == info.width
        and state.source_height == info.height
        and state.backend == backend
        and same_rate(
            state.source_num,
            state.source_den,
            info.source_num,
            info.source_den
        )
end

local function target_ladder(info, backend)
    return session_quality.target_ladder(
        info.source_num,
        info.source_den,
        info.display_num,
        info.display_den,
        backend
    )
end

local function arm_session_quality(selection, info, runtime)
    if user_opts.benchmark or not selection.enabled or selection.status ~= "active" then
        return
    end

    local backend = runtime and runtime.backend
    if not quality_source_matches(quality_state, info, backend) then
        local targets = target_ladder(info, backend)
        local target_index = session_quality.find_target(
            targets, selection.target_num, selection.target_den
        )
        if not target_index then
            quality_state = nil
            return
        end
        quality_state = {
            source_width = info.width,
            source_height = info.height,
            source_num = info.source_num,
            source_den = info.source_den,
            backend = backend,
            output_width = selection.output_width,
            output_height = selection.output_height,
            targets = targets,
            target_index = target_index,
            bad_windows = 0,
        }
    end

    local state = quality_state
    if state.filter_generation ~= filter_generation then
        state.filter_generation = filter_generation
        defer_quality_counter_baseline(state, true)
    end
    state.display_target_num = info.display_num
    state.display_target_den = info.display_den
    state.target_fps = selection.target_fps
    state.target_num = selection.target_num
    state.target_den = selection.target_den
end

local function set_session_target(state, target_index, failures)
    local target = state.targets[target_index]
    if not target then
        return false
    end
    local previous = state.target_fps
    state.target_index = target_index
    state.target_num = target.num
    state.target_den = target.den
    state.target_fps = target.fps
    reset_quality_sample()
    failed_filter = nil
    failed_reason = nil
    msg.info(string.format(
        "Framegen target down: %.1f -> %.1f FPS%s",
        previous,
        state.target_fps,
        #failures > 0 and " (" .. table.concat(failures, ",") .. ")" or ""
    ))
    schedule_reevaluate(0)
    return true
end

local function unsustainable_reason(state)
    return string.format(
        "GPU не выдерживает минимальное качество Frame generation: %dx%d, %.1f FPS; "
            .. "выключен до следующего видео или включения в FX",
        state.output_width,
        state.output_height,
        state.target_fps
    )
end

local function enter_unsustainable(state, failures)
    state.unsustainable = true
    state.unsustainable_reason = unsustainable_reason(state)
    state.bad_windows = 0
    reset_quality_sample()
    msg.warn(state.unsustainable_reason
        .. (#failures > 0 and " (" .. table.concat(failures, ",") .. ")" or ""))
    schedule_reevaluate(0)
end

local function transition_wait_blocked()
    return mp.get_property_native("pause", false)
        or mp.get_property_native("seeking", false)
        or mp.get_property_native("paused-for-cache", false)
        or mp.get_property_native("eof-reached", false)
end

local function playback_blocked()
    return transition_wait_blocked()
        or mp.get_property_native("core-idle", false)
end

local function session_quality_filter_active(state)
    if not state
        or not current_filter
        or not has_filter()
        or state.filter_generation ~= filter_generation
    then
        return false
    end
    local published = mp.get_property_native("user-data/framegen", {}) or {}
    return published.status == "active"
        and same_rate(
            tonumber(published["target-num"]),
            tonumber(published["target-den"]),
            state.target_num,
            state.target_den
        )
end

local function session_quality_active(state)
    return session_quality_filter_active(state)
        and not playback_blocked()
        and mp.get_property_number("time-pos", 0) > 0
end

local function session_quality_output_ready(state)
    if not session_quality_filter_active(state) then
        return false
    end
    local params = mp.get_property_native("video-out-params", {}) or {}
    return tonumber(params.w) == state.output_width
        and tonumber(params.h) == state.output_height
end

local function session_quality_tick()
    local state = quality_state
    local now = mp.get_time()
    if not state or state.unsustainable then
        reset_quality_sample()
        return
    end
    local healthy
    local severe
    local failures
    if state.counter_baseline_pending then
        if not session_quality_filter_active(state) or transition_wait_blocked() then
            reset_quality_sample()
            return
        end
        state.counter_settle_started_at = state.counter_settle_started_at or now
        state.counter_timeout_started_at = state.counter_timeout_started_at or now
        local settle_elapsed = now - state.counter_settle_started_at
        local timeout_elapsed = now - state.counter_timeout_started_at
        if settle_elapsed < session_quality.TRANSITION_SETTLE_SECONDS then
            return
        end
        local output_ready = mp.get_property_number("time-pos", 0) > 0
            and session_quality_output_ready(state)
            and session_quality.output_rate_matches(
                mp.get_property_number("estimated-vf-fps", 0),
                state.target_fps
            )
        if output_ready then
            state.counter_baseline_pending = false
            state.counter_settle_started_at = nil
            state.counter_timeout_started_at = nil
            state.sample_started = now
            state.first = quality_snapshot()
            return
        end
        if timeout_elapsed < session_quality.TRANSITION_READY_TIMEOUT_SECONDS then
            return
        end
        state.counter_baseline_pending = false
        state.counter_settle_started_at = nil
        state.counter_timeout_started_at = nil
        reset_quality_sample()
        healthy = false
        severe = true
        failures = { "filter_output_rate" }
    else
        if not session_quality_active(state) or not session_quality_output_ready(state) then
            reset_quality_sample()
            return
        end
        if not state.sample_started then
            state.sample_started = now
            state.first = quality_snapshot()
            return
        end
        local elapsed = now - state.sample_started
        if elapsed < session_quality.SAMPLE_SECONDS then
            return
        end

        local last = quality_snapshot()
        if session_quality.counter_epoch_changed(state.first, last) then
            defer_quality_counter_baseline(state, true)
            return
        end
        healthy, severe, failures = session_quality.sample_verdict(
            state.first, last, elapsed, state.target_fps
        )
        reset_quality_sample()
    end

    -- Rebuilding the filter interrupts playback. Keep a working rung for this
    -- file; only sustained overload may lower it, never speculative headroom.
    if healthy then
        state.bad_windows = 0
        return
    end
    state.bad_windows = state.bad_windows + 1
    if not severe and state.bad_windows < 2 then
        return
    end
    state.bad_windows = 0
    if not set_session_target(state, state.target_index + 1, failures) then
        enter_unsustainable(state, failures)
    end
end

local function benchmark_selection(info)
    local pipeline_id = tostring(user_opts.benchmark_pipeline_id or ""):lower()
    local pipeline = catalog.find(pipeline_id)
    if not pipeline then
        return {
            status = "unavailable",
            enabled = false,
            selection = pipeline_id,
            reason = "Benchmark требует конкретный pipeline id",
        }
    end

    local target_num, target_den = rational(
        user_opts.benchmark_target_num,
        user_opts.benchmark_target_den
    )
    if not target_num then
        target_num, target_den = approximate_rational(user_opts.benchmark_target_fps)
    end
    local target_fps = target_num and target_num / target_den or nil
    local process_scale = tonumber(user_opts.benchmark_process_scale)
    if not process_scale or process_scale <= 0 then
        process_scale = pipeline.process_scale or 1
    end
    if not target_fps or target_fps <= (info.source_fps or 0) then
        return {
            status = "guarded",
            enabled = false,
            selection = pipeline_id,
            pipeline_id = pipeline_id,
            reason = "Benchmark target FPS должен быть выше FPS источника",
        }
    end
    if not info.source_num or not info.source_den then
        return {
            status = "guarded",
            enabled = false,
            selection = pipeline_id,
            pipeline_id = pipeline_id,
            reason = "Benchmark не смог определить FPS источника",
        }
    end
    if process_scale < 0.25 or process_scale > 1 then
        return {
            status = "guarded",
            enabled = false,
            selection = pipeline_id,
            pipeline_id = pipeline_id,
            reason = "Benchmark process scale должен быть от 0.25 до 1",
        }
    end
    return {
        status = "active",
        enabled = true,
        selection = pipeline_id,
        pipeline_id = pipeline.id,
        model = pipeline.model,
        target_num = target_num,
        target_den = target_den,
        target_fps = target_fps,
        process_scale = process_scale,
        output_width = info.width,
        output_height = info.height,
        scene_threshold = pipeline.scene_threshold,
        reason = "Изолированный benchmark",
    }
end

local function sync_session_display(state, info)
    local display_num, display_den = info.display_num, info.display_den
    if not state.unsustainable
            and state.target_num * display_den < display_num * state.target_den then
        display_num, display_den = state.target_num, state.target_den
    end
    local targets = session_quality.target_ladder(
        info.source_num, info.source_den, display_num, display_den, state.backend
    )
    local target_index = state.unsustainable and #targets or 1
    local target = targets[target_index]
    if not target then
        return false
    end
    state.targets = targets
    state.target_index = target_index
    state.target_num = target.num
    state.target_den = target.den
    state.target_fps = target.fps
    state.display_target_num = info.display_num
    state.display_target_den = info.display_den
    if state.unsustainable then
        state.unsustainable_reason = unsustainable_reason(state)
    end
    return true
end

local function unsustainable_selection(state)
    return {
        status = "guarded",
        enabled = false,
        selection = "off",
        reason = state.unsustainable_reason,
        backend = state.backend,
        target_num = state.target_num,
        target_den = state.target_den,
        target_fps = state.target_fps,
        output_width = state.output_width,
        output_height = state.output_height,
    }
end

local function reevaluate()
    local runtime, installed, runtime_reason, restart_required, pending_runtime = validate_runtime()
    if pending_runtime then
        local ready, reason, failed, bootstrap_restart = bootstrap_runtime(pending_runtime)
        if ready then
            runtime = pending_runtime
            runtime_reason = nil
        elseif not failed then
            remove_filter()
            effective_framegen_enabled = false
            local info = loaded and collect_info()
                or { has_video = false, content_kind = "unknown" }
            publish({
                status = "loading",
                selection = "off",
                reason = reason,
                available = true,
            }, info, pending_runtime, installed, false)
            return
        else
            runtime_reason = reason
            restart_required = bootstrap_restart
        end
    end
    local profile = runtime_profile(runtime)
    local available = has_available_pipeline(profile, runtime)
    if not loaded then
        effective_framegen_enabled = false
        local info = { has_video = false, content_kind = "unknown" }
        local benchmark_waiting = user_opts.benchmark and runtime ~= nil
        publish({
            status = benchmark_waiting and "loading"
                or user_opts.benchmark and "unavailable"
                or "off",
            selection = user_opts.benchmark and user_opts.benchmark_pipeline_id or "off",
            reason = benchmark_waiting and "Ожидаем загрузку файла для benchmark"
                or runtime and ""
                or runtime_reason,
            available = available,
        }, info, runtime, installed, restart_required)
        return
    end

    local info = collect_info()
    local backend = runtime and runtime.backend
    if quality_state and info.width and info.height
            and info.source_num and info.source_den and backend
            and not quality_source_matches(quality_state, info, backend) then
        quality_state = nil
    end
    if quality_state and info.display_num and info.display_den
            and not same_rate(
                quality_state.display_target_num,
                quality_state.display_target_den,
                info.display_num,
                info.display_den
            ) then
        if not sync_session_display(quality_state, info) then
            quality_state = nil
        end
    end
    if not user_opts.benchmark and not quality_state and backend then
        local initial = session_quality.initial_target(target_ladder(info, backend), backend)
        if initial then
            info.framegen_target_num = initial.num
            info.framegen_target_den = initial.den
        end
    end
    if quality_state then
        info.framegen_target_num = quality_state.target_num
        info.framegen_target_den = quality_state.target_den
    end
    local wanted_enabled = desired_enabled(info)
    effective_framegen_enabled = wanted_enabled
    local mode = wanted_enabled and "auto" or "off"
    local selection
    local profile_pending = info.profile_status == "idle"
        or info.profile_status == "analyzing"
        or info.timing_status == "idle"
        or info.timing_status == "analyzing"
    if not user_opts.benchmark and profile_pending then
        selection = {
            status = "loading",
            enabled = false,
            selection = mode,
            reason = "Анализируем временную шкалу видео",
        }
    elseif user_opts.benchmark then
        selection = benchmark_selection(info)
    elseif quality_state and quality_state.unsustainable and wanted_enabled then
        effective_framegen_enabled = false
        selection = unsustainable_selection(quality_state)
    else
        selection = policy.choose(info, profile, mode, catalog)
    end
    if not runtime then
        selection.status = effective_framegen_enabled and "unavailable" or "off"
        selection.enabled = false
        selection.reason = effective_framegen_enabled and runtime_reason or selection.reason
    end
    if runtime and selection.enabled then
        selection.backend = runtime.backend
    end
    selection.available = available
    apply_selection(selection, info, runtime, installed, restart_required)
    arm_session_quality(selection, info, runtime)
end

function schedule_reevaluate(delay)
    if reevaluate_timer then
        reevaluate_timer:kill()
    end
    reevaluate_timer = mp.add_timeout(delay or 0, function()
        reevaluate_timer = nil
        reevaluate()
    end)
end

local function set_enabled(value)
    value = tostring(value or ""):lower()
    if user_opts.benchmark then
        msg.warn("framegen-set-enabled ignored during isolated benchmark")
        return
    end
    if value == "toggle" then
        framegen_override = not effective_framegen_enabled
    elseif value == "yes" or value == "on" or value == "true" or value == "1" then
        framegen_override = true
    elseif value == "no" or value == "off" or value == "false" or value == "0" then
        framegen_override = false
    else
        mp.osd_message("Неизвестное состояние frame generation")
        return
    end
    if framegen_override then
        reset_bootstrap_attempt()
    end
    failed_filter = nil
    failed_reason = nil
    quality_state = nil
    engine.reset()
    schedule_reevaluate(0)
end

local function refresh()
    failed_filter = nil
    failed_reason = nil
    quality_state = nil
    engine.reset()
    reset_bootstrap_attempt()
    schedule_reevaluate(0)
end

mp.register_event("start-file", function()
    engine.reset()
    loaded = false
    framegen_override = nil
    effective_framegen_enabled = false
    interlaced_seen = false
    failed_filter = nil
    failed_reason = nil
    quality_state = nil
    remove_filter()
    schedule_reevaluate(0)
end)

mp.register_event("file-loaded", function()
    loaded = true
    interlaced_seen = mp.get_property_native("video-frame-info/interlaced") == true
    schedule_reevaluate(0.1)
end)

mp.register_event("playback-restart", function()
    reset_quality_sample()
    if loaded then
        schedule_reevaluate(0.1)
    end
end)

mp.register_event("seek", reset_quality_sample)

mp.register_event("end-file", function()
    engine.reset()
    loaded = false
    remove_filter()
    schedule_reevaluate(0)
end)

mp.register_event("video-reconfig", function()
    if not applying then
        if quality_state and current_filter and has_filter() then
            defer_quality_counter_baseline(quality_state, false)
        end
        if current_filter and not has_filter() then
            failed_filter = current_filter
            failed_reason = "Frame generation filter остановился во время воспроизведения"
            current_filter = nil
        end
        schedule_reevaluate(0.1)
    end
end)

mp.observe_property("video-frame-info/interlaced", "native", function(_, value)
    if loaded and value == true and not interlaced_seen then
        interlaced_seen = true
        schedule_reevaluate(0)
    end
end)

for _, property in ipairs({
    "user-data/qbutt-runtime",
    "display-fps",
    "container-fps",
    "video-params",
    "user-data/video-profile/status",
    "user-data/video-profile/kind",
    "user-data/video-profile/confidence",
    "user-data/video-profile/timing-status",
    "user-data/video-profile/vfr",
    "user-data/video-profile/source-num",
    "user-data/video-profile/source-den",
}) do
    mp.observe_property(property, "native", function()
        if loaded then
            schedule_reevaluate(0.1)
        end
    end)
end

mp.register_script_message("framegen-set-enabled", set_enabled)
mp.register_script_message("framegen-refresh", refresh)
mp.add_periodic_timer(0.5, session_quality_tick)
reevaluate()
