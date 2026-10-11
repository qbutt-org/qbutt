/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

namespace Content
{
    enum class ReaderMode
    {
        Paged,
        Continuous
    };

    enum class ImageFit
    {
        Height,
        Width
    };

    struct ReaderSettings
    {
        ReaderMode mode = ReaderMode::Paged;
        ImageFit imageFit = ImageFit::Height;
        bool enlargeSmallImages = true;
        int brightness = 100;
        int pageGap = 0;
        int contentWidth = 100;

        bool operator==(const ReaderSettings &) const = default;
    };
}
