local M = {}

local function shader(name)
    return "~~/shaders/" .. name
end

local function chain(names)
    local shaders = {}
    for index, name in ipairs(names) do
        shaders[index] = shader(name)
    end
    return shaders
end

local quality_fallbacks = {
    ultra = { "hq", "fast" },
    high = { "hq", "fast" },
    balanced = { "fast", "hq" },
    lite = { "fast", "hq" },
    hq = { "hq", "fast" },
    fast = { "fast", "hq" }
}

local function preset(id, name, display, hint, shaders, shaders_by_quality)
    local item = {
        id = id,
        name = name,
        display = display,
        hint = hint,
        shaders = shaders
    }

    if shaders_by_quality then
        item.shaders_by_quality = shaders_by_quality
    end

    return item
end

local anime4k_hq = {
    a = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Restore_CNN_VL.glsl",
        "Anime4K_Upscale_CNN_x2_VL.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl"
    }),
    b = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Restore_CNN_Soft_VL.glsl",
        "Anime4K_Upscale_CNN_x2_VL.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl"
    }),
    c = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Upscale_Denoise_CNN_x2_VL.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl"
    }),
    aa = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Restore_CNN_VL.glsl",
        "Anime4K_Upscale_CNN_x2_VL.glsl",
        "Anime4K_Restore_CNN_M.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl"
    }),
    bb = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Restore_CNN_Soft_VL.glsl",
        "Anime4K_Upscale_CNN_x2_VL.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Restore_CNN_Soft_M.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl"
    }),
    ca = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Upscale_Denoise_CNN_x2_VL.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Restore_CNN_M.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl"
    })
}

local anime4k_fast = {
    a = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Restore_CNN_M.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Upscale_CNN_x2_S.glsl"
    }),
    b = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Restore_CNN_Soft_M.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Upscale_CNN_x2_S.glsl"
    }),
    c = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Upscale_Denoise_CNN_x2_M.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Upscale_CNN_x2_S.glsl"
    }),
    aa = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Restore_CNN_M.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl",
        "Anime4K_Restore_CNN_S.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Upscale_CNN_x2_S.glsl"
    }),
    bb = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Restore_CNN_Soft_M.glsl",
        "Anime4K_Upscale_CNN_x2_M.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Restore_CNN_Soft_S.glsl",
        "Anime4K_Upscale_CNN_x2_S.glsl"
    }),
    ca = chain({
        "Anime4K_Clamp_Highlights.glsl",
        "Anime4K_Upscale_Denoise_CNN_x2_M.glsl",
        "Anime4K_AutoDownscalePre_x2.glsl",
        "Anime4K_AutoDownscalePre_x4.glsl",
        "Anime4K_Restore_CNN_S.glsl",
        "Anime4K_Upscale_CNN_x2_S.glsl"
    })
}

M.presets = {
    preset(
        "off",
        "Off",
        "Выкл",
        "Без шейдеров: лучше для нативного качества, 4K-исходников или когда видео уже не меньше текущего дисплея.",
        {}
    ),
    preset(
        "anime4k_a",
        "Anime4K A",
        "Anime4K A",
        "Официальный high-end Mode A: основной выбор для 1080p-аниме, блюра, смаза и заметной компрессии.",
        anime4k_hq.a,
        { fast = anime4k_fast.a }
    ),
    preset(
        "anime4k_b",
        "Anime4K B",
        "Anime4K B",
        "Официальный high-end Mode B: чаще всего лучший для 720p-аниме, даунскейла и звона по контурам.",
        anime4k_hq.b,
        { fast = anime4k_fast.b }
    ),
    preset(
        "anime4k_c",
        "Anime4K C",
        "Anime4K C",
        "Официальный high-end Mode C: мягче для 480p, чистых артов, заставок и исходников без сильной деградации.",
        anime4k_hq.c,
        { fast = anime4k_fast.c }
    ),
    preset(
        "anime4k_aa",
        "Anime4K A+A",
        "Anime4K A+A",
        "Сильный x2+ вариант Mode A: максимум восстановления линий для мыльного аниме, но может перешарпить.",
        anime4k_hq.aa,
        { fast = anime4k_fast.aa }
    ),
    preset(
        "anime4k_bb",
        "Anime4K B+B",
        "Anime4K B+B",
        "Сильный x2+ вариант Mode B: лучший кандидат для 720p-аниме на 1440p/4K, если картинка мыльная.",
        anime4k_hq.bb,
        { fast = anime4k_fast.bb }
    ),
    preset(
        "anime4k_ca",
        "Anime4K C+A",
        "Anime4K C+A",
        "Сильный x2+ вариант Mode C: для 480p/SD, чистых линий и низкого разрешения без явного старого блюра.",
        anime4k_hq.ca,
        { fast = anime4k_fast.ca }
    ),
    preset(
        "anime4k_gan_2x",
        "Anime4K GAN x2",
        "Anime4K GAN x2",
        "Экспериментальный тяжелый профиль для очень низкого разрешения: больше деталей, больше риск артефактов.",
        chain({
            "Anime4K_Clamp_Highlights.glsl",
            "Anime4K_Restore_GAN_UL.glsl",
            "Anime4K_Upscale_GAN_x2_M.glsl"
        })
    ),
    preset(
        "anime4k_gan_3x",
        "Anime4K GAN x3",
        "Anime4K GAN x3",
        "Экспериментальный тяжелый профиль для 360p/480p на 1440p/4K, когда хочется сильнее SRGAN-эффект.",
        chain({
            "Anime4K_Clamp_Highlights.glsl",
            "Anime4K_Restore_GAN_UL.glsl",
            "Anime4K_Upscale_GAN_x3_L.glsl"
        })
    ),
    preset(
        "anime4k_gan_4x",
        "Anime4K GAN x4",
        "Anime4K GAN x4",
        "Самый тяжелый эксперимент для совсем мелкого исходника на 4K; включай вручную, если обычные режимы мало помогают.",
        chain({
            "Anime4K_Clamp_Highlights.glsl",
            "Anime4K_Restore_GAN_UUL.glsl",
            "Anime4K_Upscale_GAN_x4_UL.glsl"
        })
    ),
    preset(
        "fsr",
        "FSR",
        "FSR",
        "Быстрый универсальный апскейл для игр, 60fps, live-action, записей экрана и не-аниме контента.",
        chain({
            "FSR.glsl"
        })
    )
}

function M.join_shaders(shaders)
    return table.concat(shaders or {}, ";")
end

function M.resolve_quality(hardware_tier, shader_quality)
    if shader_quality and shader_quality ~= "" and shader_quality ~= "auto" then
        return shader_quality
    end

    local tier = hardware_tier or "high"
    if tier == "balanced" or tier == "lite" then
        return "fast"
    end

    return "hq"
end

function M.resolve_shaders(item, hardware_tier, shader_quality)
    if not item then
        return {}
    end

    local quality = M.resolve_quality(hardware_tier, shader_quality)
    local fallback = quality_fallbacks[quality] or quality_fallbacks[hardware_tier] or quality_fallbacks.high
    local variants = item.shaders_by_quality or {}

    for _, key in ipairs(fallback) do
        if key == "hq" and item.shaders then
            return item.shaders
        end

        if variants[key] then
            return variants[key]
        end
    end

    return item.shaders or {}
end

function M.find_by_name(name)
    if not name then
        return nil, nil
    end

    local needle = name:lower()
    for index, item in ipairs(M.presets) do
        if item.name:lower() == needle or item.id:lower() == needle then
            return item, index
        end
    end

    return nil, nil
end

function M.find_by_shader_string(shader_string)
    local current = shader_string or ""
    local qualities = { "hq", "fast" }

    for index, item in ipairs(M.presets) do
        if current == M.join_shaders(item.shaders) then
            return item, index
        end

        for _, quality in ipairs(qualities) do
            local shaders = item.shaders_by_quality and item.shaders_by_quality[quality]
            if shaders and current == M.join_shaders(shaders) then
                return item, index
            end
        end
    end

    return nil, nil
end

return M
