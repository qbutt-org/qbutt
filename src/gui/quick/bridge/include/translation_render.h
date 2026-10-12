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
    struct MangaRenderOutput;
    struct RenderCancellation;

    MangaRenderOutput render_manga_page(::rust::Slice<const std::uint8_t> original,
        ::rust::Slice<const MangaRegion> regions, const RenderCancellation &cancelled);
}
