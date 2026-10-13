/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <cstdint>

#include "rust/cxx.h"

namespace Qbutt
{
    struct MangaRegion;
    struct MangaRect;
    struct MangaRenderOutput;
    struct MangaCropOutput;
    struct MangaImageOutput;
    struct RenderCancellation;

    MangaRenderOutput render_manga_page(::rust::Slice<const std::uint8_t> original,
        ::rust::Slice<const MangaRegion> regions, const RenderCancellation &cancelled);
    MangaCropOutput crop_manga_page(::rust::Slice<const std::uint8_t> image,
        const MangaRect &bounds, const RenderCancellation &cancelled);
    MangaImageOutput normalize_generated_manga_page(::rust::Slice<const std::uint8_t> original,
        ::rust::Slice<const std::uint8_t> generated, const RenderCancellation &cancelled);
}
