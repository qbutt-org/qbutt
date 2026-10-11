local M = {}
local validated_scene_histogram_threshold = 0.08917166076596267

local pipelines = {
    {
        id = "rife-v4.26",
        name = "RIFE 4.26",
        model = "4.26",
        model_file = "rife_v2\\rife_v4.26.onnx",
        process_scale = 1,
        scene_threshold = validated_scene_histogram_threshold,
        quality_tier = 100,
        content = { anime = true, movie = true, unknown = true },
    },
}

local by_id = {}
for _, pipeline in ipairs(pipelines) do
    by_id[pipeline.id] = pipeline
end

M.pipelines = pipelines

function M.find(id)
    return by_id[id]
end

function M.supports_content(pipeline, kind)
    if not pipeline then
        return false
    end

    kind = kind == "anime" and "anime" or kind == "movie" and "movie" or "unknown"
    return pipeline.content and pipeline.content[kind] == true
end

return M
