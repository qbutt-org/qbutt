/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qbutt_quick/translation_render.h"

#include <utility>

#include <QBuffer>
#include <QColor>
#include <QImageIOHandler>
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
        return { rect.x, rect.y, rect.width, rect.height };
    }

    bool readLetteringColor(const ::rust::String &value, QColor &color)
    {
        const QString text = fromRust(value);
        color = QColor::fromString(text);
        return (text.size() == 7) && text.startsWith(u'#') && color.isValid();
    }

    QImage readMangaPage(const ::rust::Slice<const std::uint8_t> bytes, QString &error, const bool original = false)
    {
        if (bytes.size() > 96 * 1024 * 1024)
        {
            error = QStringLiteral("Manga image exceeds the encoded size limit");
            return { };
        }

        QByteArray input(reinterpret_cast<const char *>(bytes.data()), static_cast<qsizetype>(bytes.size()));
        QBuffer buffer(&input);
        if (!buffer.open(QIODevice::ReadOnly))
        {
            error = QStringLiteral("Manga image could not be opened");
            return { };
        }
        QImageReader reader(&buffer);
        const QSize size = reader.size();
        if (!(original ? Content::MangaRender::supportsOriginalSize(size) : Content::MangaRender::supportsImageSize(size)))
        {
            error = QStringLiteral("Invalid or oversized manga page");
            return { };
        }
        // The reader uses untransformed source coordinates. Metadata must not
        // change the canvas seen by Qt or the model.
        if (reader.transformation() != QImageIOHandler::TransformationNone)
        {
            error = QStringLiteral("Manga image orientation metadata is not supported for translation");
            return { };
        }
        const QImage image = reader.read();
        if (image.isNull() || (image.size() != size))
        {
            error = QStringLiteral("Manga image could not be decoded");
            return { };
        }
        return image;
    }

    QList<Content::MangaRender::Region> nativeRegions(
        const ::rust::Slice<const Qbutt::MangaRegion> regions, QString &error)
    {
        QList<Content::MangaRender::Region> result;
        result.reserve(static_cast<qsizetype>(regions.size()));
        for (const Qbutt::MangaRegion &value : regions)
        {
            Content::MangaRender::Region region;
            region.id = fromRust(value.id);
            for (const Qbutt::MangaTextRun &run : value.translation)
                region.translation.append({ fromRust(run.text), run.bold, run.italic });
            switch (value.family)
            {
            case Qbutt::MangaFontFamily::Comic:
                region.family = Content::MangaRender::FontFamily::Comic;
                break;
            case Qbutt::MangaFontFamily::Serif:
                region.family = Content::MangaRender::FontFamily::Serif;
                break;
            case Qbutt::MangaFontFamily::Condensed:
                region.family = Content::MangaRender::FontFamily::Condensed;
                break;
            default:
                error = QStringLiteral("Invalid manga font family");
                return { };
            }
            region.bounds = rectangle(value.bounds);
            region.layout = rectangle(value.layout);
            region.angle = value.angle;
            region.outlineWidth = value.outline_width;
            if (!readLetteringColor(value.text_color, region.textColor)
                || (!value.outline_color.empty() && !readLetteringColor(value.outline_color, region.outlineColor)))
            {
                error = QStringLiteral("Invalid manga lettering color");
                return { };
            }
            if (value.outline_color.empty())
                region.outlineColor = QColor();
            for (const Qbutt::MangaRect &rect : value.cleanup_rects)
                region.cleanupRects.append(rectangle(rect));
            result.append(std::move(region));
        }
        return result;
    }

    ::rust::Vec<Qbutt::MangaDiagnostic> nativeDiagnostics(const QList<Content::MangaRender::Diagnostic> &items)
    {
        ::rust::Vec<Qbutt::MangaDiagnostic> result;
        for (const Content::MangaRender::Diagnostic &item : items)
        {
            Qbutt::MangaDiagnostic diagnostic;
            diagnostic.region_id = toRust(item.regionId);
            diagnostic.status = static_cast<std::uint8_t>(item.status);
            diagnostic.font_pixels = static_cast<std::uint32_t>(item.fontPixels);
            diagnostic.layout_lines = static_cast<std::uint32_t>(item.layoutLines);
            diagnostic.cleanup_pixels = static_cast<std::uint32_t>(item.cleanupPixels);
            result.push_back(std::move(diagnostic));
        }
        return result;
    }
}

Qbutt::MangaImageOutput Qbutt::manga_image_geometry(const ::rust::Slice<const std::uint8_t> original,
    const RenderCancellation &cancelled)
{
    MangaImageOutput output;
    if (render_cancelled(cancelled))
    {
        output.error = "Manga geometry cancelled";
        return output;
    }
    QString error;
    const QImage image = readMangaPage(original, error, true);
    output.error = toRust(error);
    output.width = static_cast<std::uint32_t>(image.width());
    output.height = static_cast<std::uint32_t>(image.height());
    return output;
}

Qbutt::MangaImageOutput Qbutt::crop_manga_window(const ::rust::Slice<const std::uint8_t> original,
    const std::uint32_t top, const std::uint32_t height, const RenderCancellation &cancelled)
{
    MangaImageOutput output;
    if (render_cancelled(cancelled))
    {
        output.error = "Manga crop cancelled";
        return output;
    }
    QString error;
    const QImage image = readMangaPage(original, error, true);
    if (image.isNull())
    {
        output.error = toRust(error);
        return output;
    }
    const Content::MangaRender::PageImage crop = Content::MangaRender::cropWindow(image,
        static_cast<int>(top), static_cast<int>(height), [&cancelled] { return render_cancelled(cancelled); });
    output.error = toRust(crop.error);
    if (!crop.error.isEmpty())
        return output;
    output.width = static_cast<std::uint32_t>(crop.image.width());
    output.height = static_cast<std::uint32_t>(crop.image.height());
    output.png = png(crop.image);
    if (output.png.empty())
        output.error = "Manga crop PNG encoding failed";
    return output;
}

Qbutt::MangaImageOutput Qbutt::compose_manga_windows(const ::rust::Slice<const std::uint8_t> original,
    const ::rust::Slice<const MangaWindow> windows, const RenderCancellation &cancelled)
{
    MangaImageOutput output;
    if (render_cancelled(cancelled) || (windows.size() > 8))
    {
        output.error = "Manga composition cancelled or window count exceeded";
        return output;
    }
    QString error;
    QImage page = readMangaPage(original, error, true).convertToFormat(QImage::Format_RGBA8888);
    if (page.isNull())
    {
        output.error = toRust(error);
        return output;
    }
    for (const MangaWindow &window : windows)
    {
        if (render_cancelled(cancelled))
        {
            output.error = "Manga composition cancelled";
            return output;
        }
        const QImage image = readMangaPage({window.png.data(), window.png.size()}, error)
            .convertToFormat(QImage::Format_RGBA8888);
        if (image.isNull() || (image.height() != static_cast<int>(window.height)))
        {
            output.error = "Invalid manga window image";
            return output;
        }
        const QList<Content::MangaRender::Region> regions = nativeRegions({window.regions.data(), window.regions.size()}, error);
        if (!error.isEmpty())
        {
            output.error = toRust(error);
            return output;
        }
        error = Content::MangaRender::composeWindow(page, image, static_cast<int>(window.top), regions,
            [&cancelled] { return render_cancelled(cancelled); });
        if (!error.isEmpty())
        {
            output.error = toRust(error);
            return output;
        }
    }
    output.width = static_cast<std::uint32_t>(page.width());
    output.height = static_cast<std::uint32_t>(page.height());
    output.png = png(page);
    if (output.png.empty())
        output.error = "Manga full-page PNG encoding failed";
    return output;
}

Qbutt::MangaPreparationOutput Qbutt::prepare_manga_page(const ::rust::Slice<const std::uint8_t> original,
    const ::rust::Slice<const MangaRegion> regions, const std::uint64_t previewPixels,
    const std::uint32_t maximumImages, const std::uint64_t maximumBytes, const RenderCancellation &cancelled)
{
    MangaPreparationOutput output;
    if (render_cancelled(cancelled))
    {
        output.error = "Manga typography cancelled";
        return output;
    }
    QString error;
    const QImage image = readMangaPage(original, error);
    if (image.isNull())
    {
        output.error = toRust(error);
        return output;
    }
    const QList<Content::MangaRender::Region> pageRegions = nativeRegions(regions, error);
    if (!error.isEmpty())
    {
        output.error = toRust(error);
        return output;
    }
    const Content::MangaRender::Preparation prepared = Content::MangaRender::preparePage(
        image, pageRegions, previewPixels, static_cast<int>(maximumImages),
        [&cancelled] { return render_cancelled(cancelled); });
    output.error = toRust(prepared.error);
    output.diagnostics = nativeDiagnostics(prepared.diagnostics);
    if (!prepared.error.isEmpty())
        return output;
    std::size_t imageBytes = 0;
    for (const QImage &preview : prepared.previews)
    {
        if (render_cancelled(cancelled))
        {
            output.previews.clear();
            output.error = "Manga preview cancelled";
            return output;
        }
        MangaImageOutput item;
        item.png = png(preview);
        imageBytes += item.png.size();
        if (item.png.empty() || (imageBytes > maximumBytes))
        {
            output.previews.clear();
            output.error = "Manga previews exceed the encoded image batch limit";
            return output;
        }
        output.previews.push_back(std::move(item));
    }
    return output;
}

Qbutt::MangaRenderOutput Qbutt::render_cleaned_manga_page(const ::rust::Slice<const std::uint8_t> original,
    const ::rust::Slice<const std::uint8_t> cleaned, const ::rust::Slice<const MangaRegion> regions,
    const RenderCancellation &cancelled)
{
    MangaRenderOutput output;
    if (render_cancelled(cancelled))
    {
        output.error = "Manga composition cancelled";
        return output;
    }
    QString error;
    const QImage originalImage = readMangaPage(original, error);
    if (originalImage.isNull())
    {
        output.error = toRust(error);
        return output;
    }
    const QImage cleanedImage = readMangaPage(cleaned, error);
    if (cleanedImage.isNull())
    {
        output.error = toRust(error);
        return output;
    }
    const QList<Content::MangaRender::Region> pageRegions = nativeRegions(regions, error);
    if (!error.isEmpty())
    {
        output.error = toRust(error);
        return output;
    }
    const Content::MangaRender::Result rendered = Content::MangaRender::renderCleaned(
        originalImage, cleanedImage, pageRegions, [&cancelled] { return render_cancelled(cancelled); });
    output.error = toRust(rendered.error);
    output.diagnostics = nativeDiagnostics(rendered.diagnostics);
    if (!rendered.error.isEmpty())
        return output;
    output.png = png(rendered.rendered);
    if (output.png.empty())
        output.error = "Manga PNG encoding failed";
    else if (render_cancelled(cancelled))
    {
        output.png.clear();
        output.error = "Manga composition cancelled";
    }
    return output;
}
