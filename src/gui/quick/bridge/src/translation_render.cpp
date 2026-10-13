/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qbutt_quick/translation_render.h"

#include <utility>

#include <QBuffer>
#include <QColor>
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

    bool readLetteringColor(const ::rust::String &value, QColor &color)
    {
        const QString text = fromRust(value);
        color = QColor::fromString(text);
        return text.isEmpty() || ((text.size() == 7) && text.startsWith(u'#') && color.isValid());
    }

    QImage readMangaPage(const ::rust::Slice<const std::uint8_t> bytes, QString &error)
    {
        if (bytes.size() > 96 * 1024 * 1024)
        {
            error = QStringLiteral("Manga image exceeds the encoded size limit");
            return {};
        }

        QByteArray input(reinterpret_cast<const char *>(bytes.data()), static_cast<qsizetype>(bytes.size()));
        QBuffer buffer(&input);
        if (!buffer.open(QIODevice::ReadOnly))
        {
            error = QStringLiteral("Manga image could not be opened");
            return {};
        }
        QImageReader reader(&buffer);
        reader.setAutoTransform(true);
        const QSize size = reader.size();
        if (!size.isValid() || (qint64(size.width()) * size.height() > 16'000'000))
        {
            error = QStringLiteral("Invalid or oversized manga page");
            return {};
        }
        const QImage image = reader.read();
        if (image.isNull())
            error = QStringLiteral("Manga image could not be decoded");
        return image;
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

    QString error;
    const QImage image = readMangaPage(original, error);
    if (image.isNull())
    {
        output.error = toRust(error);
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
        if (!readLetteringColor(value.text_color, region.textColor)
            || !readLetteringColor(value.outline_color, region.outlineColor))
        {
            output.error = "Invalid manga lettering color";
            return output;
        }
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

Qbutt::MangaCropOutput Qbutt::crop_manga_page(
    const ::rust::Slice<const std::uint8_t> encoded,
    const MangaRect &bounds, const RenderCancellation &cancelled)
{
    MangaCropOutput output;
    if (render_cancelled(cancelled))
    {
        output.error = "Manga inspection cancelled";
        return output;
    }
    QString error;
    const QImage image = readMangaPage(encoded, error);
    if (image.isNull())
    {
        output.error = toRust(error);
        return output;
    }
    const Content::MangaRender::Crop crop = Content::MangaRender::cropPage(image, rectangle(bounds),
        [&cancelled] { return render_cancelled(cancelled); });
    if (!crop.error.isEmpty())
    {
        output.error = toRust(crop.error);
        return output;
    }
    output.png = png(crop.image);
    if (output.png.empty())
    {
        output.error = "Manga crop PNG encoding failed";
        return output;
    }
    if (render_cancelled(cancelled))
    {
        output.png.clear();
        output.error = "Manga inspection cancelled";
        return output;
    }
    output.bounds.x = crop.bounds.x();
    output.bounds.y = crop.bounds.y();
    output.bounds.width = crop.bounds.width();
    output.bounds.height = crop.bounds.height();
    output.width = static_cast<std::uint32_t>(crop.image.width());
    output.height = static_cast<std::uint32_t>(crop.image.height());
    return output;
}
