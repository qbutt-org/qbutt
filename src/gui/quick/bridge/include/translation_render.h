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
    struct MangaTypographyOutput;
    struct MangaRenderOutput;
    struct RenderCancellation;

    MangaTypographyOutput prepare_manga_typography(::rust::Slice<const std::uint8_t> original,
        ::rust::Slice<const MangaRegion> regions, const RenderCancellation &cancelled);
    MangaRenderOutput render_cleaned_manga_page(::rust::Slice<const std::uint8_t> original,
        ::rust::Slice<const std::uint8_t> cleaned, ::rust::Slice<const MangaRegion> regions,
        const RenderCancellation &cancelled);
}
