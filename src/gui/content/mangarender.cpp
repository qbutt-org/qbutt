/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "mangarender.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstring>

#include <QFontDatabase>
#include <QGlyphRun>
#include <QPainter>
#include <QPen>
#include <QRawFont>
#include <QRegion>
#include <QTextLayout>
#include <QTextOption>
#include <QTransform>

namespace
{
    constexpr int MAX_PIXELS = 16'000'000;
    constexpr int MAX_REGIONS = 60;
    constexpr int MAX_CLEANUP_RECTS = 4096;

    bool isCancelled(const std::function<bool()> &cancelled)
    {
        return cancelled && cancelled();
    }

    bool validRect(const QRectF &rect)
    {
        return std::isfinite(rect.x()) && std::isfinite(rect.y()) && std::isfinite(rect.width())
            && std::isfinite(rect.height()) && (rect.left() >= -0.000001) && (rect.top() >= -0.000001)
            && (rect.width() > 0) && (rect.height() > 0) && (rect.right() <= 1.000001) && (rect.bottom() <= 1.000001);
    }

    bool containsRect(const QRectF &bounds, const QRectF &rect)
    {
        return (bounds.left() - 0.000001 <= rect.left()) && (bounds.top() - 0.000001 <= rect.top())
            && (bounds.right() + 0.000001 >= rect.right()) && (bounds.bottom() + 0.000001 >= rect.bottom());
    }

    QRect pixelRect(const QRectF &rect, const QSize &size)
    {
        const int left = qBound(0, qFloor(rect.left() * size.width()), size.width());
        const int top = qBound(0, qFloor(rect.top() * size.height()), size.height());
        const int right = qBound(0, qCeil(rect.right() * size.width()), size.width());
        const int bottom = qBound(0, qCeil(rect.bottom() * size.height()), size.height());
        return { left, top, right - left, bottom - top };
    }
}

bool Content::MangaRender::supportsImageSize(const QSize &size)
{
    constexpr int MAX_DIMENSION = 16'000;
    return !size.isEmpty() && (size.width() <= MAX_DIMENSION) && (size.height() <= MAX_DIMENSION)
        && (qint64(size.width()) * size.height() <= MAX_PIXELS);
}

bool Content::MangaRender::supportsOriginalSize(const QSize &size)
{
    return !size.isEmpty() && (size.width() <= 16'000) && (size.height() <= 65'536)
        && (qint64(size.width()) * size.height() <= 40'000'000);
}

Content::MangaRender::PageImage Content::MangaRender::cropWindow(const QImage &original,
    const int top, const int height, const std::function<bool()> &cancelled)
{
    PageImage result;
    if (isCancelled(cancelled))
        result.error = QStringLiteral("Manga window crop cancelled");
    else if (!supportsOriginalSize(original.size()) || (top < 0) || (height <= 0)
        || (qint64(top) + height > original.height()) || !supportsImageSize({original.width(), height}))
    {
        result.error = QStringLiteral("Invalid manga window geometry");
    }
    else
    {
        result.image = original.copy(0, top, original.width(), height);
        if (isCancelled(cancelled))
        {
            result.image = { };
            result.error = QStringLiteral("Manga window crop cancelled");
        }
        else if (result.image.isNull())
            result.error = QStringLiteral("Manga window could not be copied");
    }
    return result;
}

QString Content::MangaRender::composeWindow(QImage &page, const QImage &window, const int top,
    const QList<Region> &regions, const std::function<bool()> &cancelled)
{
    if (!supportsOriginalSize(page.size()) || !supportsImageSize(window.size())
        || (page.width() != window.width()) || (top < 0) || (qint64(top) + window.height() > page.height())
        || (page.format() != QImage::Format_RGBA8888) || (window.format() != QImage::Format_RGBA8888))
    {
        return QStringLiteral("Invalid manga window composition geometry");
    }
    QRegion allowed;
    for (const Region &region : regions)
    {
        if (!validRect(region.layout))
            return QStringLiteral("Invalid manga window layout");
        allowed += pixelRect(region.layout, window.size());
        for (const QRectF &rect : region.cleanupRects)
        {
            if (!validRect(rect))
                return QStringLiteral("Invalid manga window cleanup rectangle");
            allowed += pixelRect(rect, window.size());
        }
    }
    for (const QRect &rect : allowed)
    {
        for (int row = rect.top(); row <= rect.bottom(); ++row)
        {
            if (isCancelled(cancelled))
                return QStringLiteral("Manga window composition cancelled");
            std::memcpy(page.scanLine(top + row) + rect.left() * 4,
                window.constScanLine(row) + rect.left() * 4, static_cast<std::size_t>(rect.width()) * 4);
        }
    }
    return { };
}

Content::MangaRender::PageImage Content::MangaRender::normalizeGeneratedPage(
    const QSize &originalSize, const QImage &generated, const std::function<bool()> &cancelled)
{
    PageImage result;
    if (cancelled && cancelled())
    {
        result.error = QStringLiteral("Generated manga page validation cancelled");
        return result;
    }
    if (!supportsImageSize(originalSize) || !supportsImageSize(generated.size()))
    {
        result.error = QStringLiteral("Invalid or oversized generated manga page");
        return result;
    }

    // Permit integer rounding of at most one pixel on each generated axis.
    // A changed aspect ratio must not be hidden by stretching, cropping or padding.
    const qint64 difference = std::abs(
        qint64(generated.width()) * originalSize.height() - qint64(generated.height()) * originalSize.width());
    if (difference > qint64(originalSize.width()) + originalSize.height())
    {
        result.error = QStringLiteral("Generated manga page changed the original aspect ratio");
        return result;
    }

    result.image = generated.scaled(originalSize, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
    if (cancelled && cancelled())
    {
        result.image = { };
        result.error = QStringLiteral("Generated manga page validation cancelled");
    }
    else if (result.image.isNull())
        result.error = QStringLiteral("Generated manga page could not be resized");
    return result;
}

QFont Content::MangaRender::mangaFont(const FontFamily family)
{
    static const std::array<QString, 3> families = []
    {
        std::array<QString, 3> loaded;
        const std::array<QString, 3> prefixes { QStringLiteral("BalsamiqSans"), QStringLiteral("PT_Serif-Web"),
            QStringLiteral("FiraSansCondensed") };
        const QStringList faces { QStringLiteral("Regular"), QStringLiteral("Bold"), QStringLiteral("Italic"),
            QStringLiteral("BoldItalic") };
        for (std::size_t index = 0; index < prefixes.size(); ++index)
        {
            for (const QString &face : faces)
            {
                const QString path = QStringLiteral(":/reader/%1-%2.ttf").arg(prefixes[index], face);
                const int fontId = QFontDatabase::addApplicationFont(path);
                const QStringList names = QFontDatabase::applicationFontFamilies(fontId);
                if (names.isEmpty())
                    return std::array<QString, 3> { };
                loaded[index] = names.constFirst();
            }
        }
        return loaded;
    }();
    QFont font(families.at(static_cast<std::size_t>(family)));
    font.setStyleStrategy(QFont::NoFontMerging);
    return font;
}

Content::MangaRender::TextShape Content::MangaRender::typesetText(
    const QList<TextRun> &runs, const QRectF &rect, QFont font, const int maximumPixels, const qreal angle)
{
    QString text;
    QList<QTextLayout::FormatRange> formats;
    for (const TextRun &run : runs)
    {
        QTextLayout::FormatRange range;
        range.start = static_cast<int>(text.size());
        range.length = static_cast<int>(run.text.size());
        range.format.setFontWeight(run.bold ? QFont::Bold : QFont::Normal);
        range.format.setFontItalic(run.italic);
        formats.append(range);
        text += run.text;
    }
    text.replace(u'\n', QChar::LineSeparator);
    QTextOption option;
    option.setWrapMode(QTextOption::WordWrap);

    for (int pixels = maximumPixels; pixels > 0; --pixels)
    {
        font.setPixelSize(pixels);
        QTextLayout layout(text, font);
        layout.setFormats(formats);
        layout.setTextOption(option);
        layout.beginLayout();
        qreal height = 0;
        bool fits = true;
        while (true)
        {
            QTextLine line = layout.createLine();
            if (!line.isValid())
                break;

            line.setLineWidth(rect.width());
            line.setPosition(QPointF((rect.width() - line.naturalTextWidth()) / 2, height));
            height += line.height();
            fits = fits && (line.naturalTextWidth() <= rect.width());
        }
        layout.endLayout();
        if (!fits || (height > rect.height()))
            continue;

        QPainterPath path;
        const QPointF origin(rect.left(), rect.top() + ((rect.height() - height) / 2));
        for (const QGlyphRun &run : layout.glyphRuns())
        {
            const QRawFont rawFont = run.rawFont();
            const QList<quint32> glyphs = run.glyphIndexes();
            const QList<QPointF> positions = run.positions();
            for (qsizetype index = 0; index < glyphs.size(); ++index)
            {
                if (glyphs.at(index) == 0)
                    return { { }, 0, 0, Status::MissingGlyph };
                path.addPath(rawFont.pathForGlyph(glyphs.at(index)).translated(origin + positions.at(index)));
            }
        }
        QTransform transform;
        transform.translate(rect.center().x(), rect.center().y());
        transform.rotate(angle);
        transform.translate(-rect.center().x(), -rect.center().y());
        path = transform.map(path);
        if (!rect.contains(path.boundingRect()))
            continue;
        return { path, pixels, layout.lineCount() };
    }
    return { };
}

Content::MangaRender::Typography Content::MangaRender::prepareTypography(
    const QSize &size, const QList<Region> &regions, const std::function<bool()> &cancelled)
{
    Typography result;
    if (!supportsImageSize(size) || regions.isEmpty() || (regions.size() > MAX_REGIONS))
    {
        result.error = QStringLiteral("Invalid or oversized manga page");
        return result;
    }

    int rectCount = 0;
    qint64 cleanupPixels = 0;
    for (const Region &region : regions)
    {
        if (isCancelled(cancelled))
        {
            result.error = QStringLiteral("Manga typography cancelled");
            return result;
        }
        Diagnostic diagnostic;
        diagnostic.regionId = region.id;
        TextShape shape;
        QString translation;
        for (const TextRun &run : region.translation)
            translation += run.text;
        if (translation.trimmed().isEmpty())
            diagnostic.status = Status::EmptyTranslation;
        else if (!validRect(region.bounds) || !validRect(region.layout) || !containsRect(region.bounds, region.layout)
            || region.cleanupRects.isEmpty() || (region.cleanupRects.size() > 512) || !std::isfinite(region.angle)
            || (qAbs(region.angle) > 45) || (region.translation.size() > 32) || (translation.toUtf8().size() > 3000)
            || !region.textColor.isValid() || !std::isfinite(region.outlineWidth) || (region.outlineWidth < 0)
            || (region.outlineWidth > 16) || ((region.outlineWidth > 0) != region.outlineColor.isValid()))
        {
            diagnostic.status = Status::InvalidGeometry;
        }

        qint64 regionPixels = 0;
        if (diagnostic.status == Status::Rendered)
        {
            for (const QRectF &rect : region.cleanupRects)
            {
                if (!validRect(rect) || !containsRect(region.bounds, rect))
                {
                    diagnostic.status = Status::InvalidGeometry;
                    break;
                }
                const QRect pixels = pixelRect(rect, size);
                regionPixels += qint64(pixels.width()) * pixels.height();
            }
            if ((rectCount + region.cleanupRects.size() > MAX_CLEANUP_RECTS)
                || (cleanupPixels + regionPixels > qint64(size.width()) * size.height() / 2))
            {
                diagnostic.status = Status::InvalidGeometry;
            }
        }
        if (diagnostic.status == Status::Rendered)
        {
            const QRectF layout(region.layout.x() * size.width(), region.layout.y() * size.height(),
                region.layout.width() * size.width(), region.layout.height() * size.height());
            const qreal margin = qMax(
                region.outlineWidth + 1, qMin(qreal(12), qMax(qreal(2), qMin(layout.width(), layout.height()) * 0.04)));
            const QRectF textRect = layout.adjusted(margin, margin, -margin, -margin);
            if (textRect.isEmpty())
                diagnostic.status = Status::Overflow;
            else
            {
                const QFont font = mangaFont(region.family);
                if (font.family().isEmpty())
                {
                    result.error = QStringLiteral("Bundled manga fonts unavailable");
                    return result;
                }
                const int maximumPixels = qMin(256, qMax(1, qFloor(textRect.height())));
                shape = typesetText(region.translation, textRect, font, maximumPixels, region.angle);
                diagnostic.fontPixels = shape.fontPixels;
                diagnostic.layoutLines = shape.lines;
                if ((shape.status != Status::Rendered) || shape.path.isEmpty() || (shape.fontPixels < 9))
                    diagnostic.status = shape.status == Status::Rendered ? Status::Overflow : shape.status;
                else if (!layout.contains(shape.path.boundingRect().adjusted(
                             -region.outlineWidth, -region.outlineWidth, region.outlineWidth, region.outlineWidth)))
                {
                    diagnostic.status = Status::Overflow;
                }
            }
        }
        if (diagnostic.status == Status::Rendered)
        {
            rectCount += region.cleanupRects.size();
            cleanupPixels += regionPixels;
            diagnostic.cleanupPixels = static_cast<int>(regionPixels);
        }
        result.shapes.append(shape);
        result.diagnostics.append(diagnostic);
    }
    return result;
}

Content::MangaRender::Result Content::MangaRender::renderCleaned(const QImage &original, const QImage &generated,
    const QList<Region> &regions, const std::function<bool()> &cancelled)
{
    Result result;
    const Typography typography = prepareTypography(original.size(), regions, cancelled);
    result.diagnostics = typography.diagnostics;
    result.error = typography.error;
    if (!result.error.isEmpty())
        return result;

    const QImage source = original.convertToFormat(QImage::Format_RGBA8888);
    if (std::any_of(result.diagnostics.cbegin(), result.diagnostics.cend(),
            [](const Diagnostic &diagnostic) { return diagnostic.status != Status::Rendered; }))
    {
        result.cleaned = source;
        result.rendered = source;
        return result;
    }
    const PageImage normalized = normalizeGeneratedPage(source.size(), generated, cancelled);
    if (!normalized.error.isEmpty())
    {
        result.error = normalized.error;
        return result;
    }
    const QImage replacement = normalized.image.convertToFormat(QImage::Format_RGBA8888);
    result.cleaned = source.copy();
    for (const Region &region : regions)
    {
        for (const QRectF &rect : region.cleanupRects)
        {
            const QRect pixels = pixelRect(rect, source.size());
            for (int y = pixels.top(); y <= pixels.bottom(); ++y)
            {
                if ((y % 64 == 0) && isCancelled(cancelled))
                {
                    result.error = QStringLiteral("Manga cleanup composition cancelled");
                    result.cleaned = { };
                    return result;
                }
                std::memcpy(result.cleaned.scanLine(y) + pixels.left() * 4,
                    replacement.constScanLine(y) + pixels.left() * 4, pixels.width() * 4);
            }
        }
    }

    result.rendered = result.cleaned.copy();
    QPainter painter(&result.rendered);
    painter.setRenderHint(QPainter::Antialiasing);
    for (qsizetype index = 0; index < regions.size(); ++index)
    {
        if (isCancelled(cancelled))
        {
            painter.end();
            result.rendered = { };
            result.cleaned = { };
            result.error = QStringLiteral("Manga lettering cancelled");
            return result;
        }
        const Region &region = regions.at(index);
        const TextShape &shape = typography.shapes.at(index);
        painter.setClipRect(pixelRect(region.layout, source.size()));
        if (region.outlineWidth > 0)
        {
            painter.strokePath(shape.path,
                QPen(region.outlineColor, region.outlineWidth * 2, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        }
        painter.fillPath(shape.path, region.textColor);
    }
    painter.end();
    if (isCancelled(cancelled))
    {
        result.rendered = { };
        result.cleaned = { };
        result.error = QStringLiteral("Manga lettering cancelled");
    }
    return result;
}
