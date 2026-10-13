/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qbutt_quick/translation_render.h"

#include <utility>

#include <QBuffer>
#include <QImageReader>
#include <QRectF>

#include "qbutt_quick/src/translation_render.cxx.h"
#include "../../../content/mangarender.h"

namespace
{
    QString fromRust(const ::rust::String &text)
    {
        return QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size()));
    }

    ::rust::String toRust(const QString &text)
    {
        const QByteArray bytes = text.toUtf8();
        return ::rust::String(bytes.constData(), static_cast<std::size_t>(bytes.size()));
    }

    ::rust::Vec<std::uint8_t> png(const QImage &image)
    {
        ::rust::Vec<std::uint8_t> result;
        QByteArray bytes;
        QBuffer buffer(&bytes);
        if (!buffer.open(QIODevice::WriteOnly) || !image.save(&buffer, "PNG"))
            return result;
        result.reserve(static_cast<std::size_t>(bytes.size()));
        for (const char byte : bytes)
            result.push_back(static_cast<std::uint8_t>(byte));
        return result;
    }

    QRectF rectangle(const Qbutt::MangaRect &rect)
    {
        return {rect.x, rect.y, rect.width, rect.height};
    }

    ::rust::String statusName(const Content::MangaRender::Status status)
    {
        using Status = Content::MangaRender::Status;
        switch (status)
        {
        case Status::Rendered:
            return "Rendered";
        case Status::EmptyTranslation:
            return "EmptyTranslation";
        case Status::InvalidGeometry:
            return "InvalidGeometry";
        case Status::Overflow:
            return "Overflow";
        case Status::NoInkMask:
            return "NoInkMask";
        case Status::MissingGlyph:
            return "MissingGlyph";
        }
        return "Unknown";
    }
}

Qbutt::MangaRenderOutput Qbutt::render_manga_page(
    ::rust::Slice<const std::uint8_t> original,
    ::rust::Slice<const MangaRegion> regions,
    const RenderCancellation &cancelled)
{
    MangaRenderOutput output;
    if (render_cancelled(cancelled))
    {
        output.error = "Manga render cancelled";
        return output;
    }

    if (original.size() > 96 * 1024 * 1024)
    {
        output.error = "Manga image exceeds the encoded size limit";
        return output;
    }

    QByteArray input(reinterpret_cast<const char *>(original.data()), static_cast<qsizetype>(original.size()));
    QBuffer buffer(&input);
    if (!buffer.open(QIODevice::ReadOnly))
    {
        output.error = "Manga image could not be opened";
        return output;
    }
    QImageReader reader(&buffer);
    reader.setAutoTransform(true);
    const QSize size = reader.size();
    if (!size.isValid() || (qint64(size.width()) * size.height() > 16'000'000))
    {
        output.error = "Invalid or oversized manga page";
        return output;
    }

    const QImage image = reader.read();
    if (image.isNull())
    {
        output.error = "Manga image could not be decoded";
        return output;
    }

    QList<Content::MangaRender::Region> pageRegions;
    pageRegions.reserve(static_cast<qsizetype>(regions.size()));
    for (const MangaRegion &value : regions)
    {
        Content::MangaRender::Region region;
        region.id = fromRust(value.id);
        for (const MangaTextRun &run : value.translation)
            region.translation.append({fromRust(run.text), run.bold, run.italic});
        switch (value.family)
        {
        case MangaFontFamily::Comic:
            region.family = Content::MangaRender::FontFamily::Comic;
            break;
        case MangaFontFamily::Serif:
            region.family = Content::MangaRender::FontFamily::Serif;
            break;
        case MangaFontFamily::Condensed:
            region.family = Content::MangaRender::FontFamily::Condensed;
            break;
        default:
            output.error = "Invalid manga font family";
            return output;
        }
        region.bounds = rectangle(value.bounds);
        region.layout = rectangle(value.layout);
        region.angle = value.angle;
        region.ink = value.ink == 1 ? Content::MangaRender::Ink::Light : Content::MangaRender::Ink::Dark;
        for (const MangaRect &box : value.ink_boxes)
            region.inkBoxes.append(rectangle(box));
        pageRegions.append(std::move(region));
    }

    const Content::MangaRender::Result rendered = Content::MangaRender::render(image, pageRegions,
        [&cancelled] { return render_cancelled(cancelled); });
    if (!rendered.error.isEmpty())
    {
        output.error = toRust(rendered.error);
        return output;
    }

    output.width = static_cast<std::uint32_t>(image.width());
    output.height = static_cast<std::uint32_t>(image.height());
    for (const Content::MangaRender::Diagnostic &value : rendered.diagnostics)
    {
        MangaDiagnostic diagnostic;
        diagnostic.region_id = toRust(value.regionId);
        diagnostic.status = statusName(value.status);
        diagnostic.font_pixels = static_cast<std::uint32_t>(value.fontPixels);
        diagnostic.layout_lines = static_cast<std::uint32_t>(value.layoutLines);
        diagnostic.masked_pixels = static_cast<std::uint32_t>(value.maskedPixels);
        output.diagnostics.push_back(std::move(diagnostic));
    }

    output.result_png = png(rendered.rendered);
    output.cleaned_png = png(rendered.cleaned);
    output.mask_png = png(rendered.mask);
    if (output.result_png.empty() || output.cleaned_png.empty() || output.mask_png.empty())
        output.error = "Manga PNG encoding failed";
    return output;
}
