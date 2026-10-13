/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "mangarender.h"

#include <algorithm>
#include <array>
#include <cmath>

#include <QFontDatabase>
#include <QGlyphRun>
#include <QPainter>
#include <QPen>
#include <QRawFont>
#include <QTextLayout>
#include <QTextOption>
#include <QTransform>

namespace
{
    constexpr int MAX_PIXELS = 16'000'000;
    constexpr int MAX_REGIONS = 60;
    constexpr int MAX_INK_BOXES = 4096;

    struct PixelBox
    {
        int left;
        int top;
        int right;
        int bottom;
    };

    struct PendingText
    {
        Content::MangaRender::TextShape shape;
        QColor textColor;
        QColor outlineColor;
    };

    bool validRect(const QRectF &rect)
    {
        return std::isfinite(rect.x()) && std::isfinite(rect.y())
            && std::isfinite(rect.width()) && std::isfinite(rect.height())
            && (rect.left() >= -0.000001) && (rect.top() >= -0.000001)
            && (rect.width() > 0) && (rect.height() > 0)
            && (rect.right() <= 1.000001) && (rect.bottom() <= 1.000001);
    }

    bool containsRect(const QRectF &bounds, const QRectF &rect)
    {
        return (bounds.left() - 0.000001 <= rect.left())
            && (bounds.top() - 0.000001 <= rect.top())
            && (bounds.right() + 0.000001 >= rect.right())
            && (bounds.bottom() + 0.000001 >= rect.bottom());
    }

    PixelBox pixelBox(const QRectF &rect, const QSize &size)
    {
        return {
            qBound(0, qFloor(rect.left() * size.width()), size.width()),
            qBound(0, qFloor(rect.top() * size.height()), size.height()),
            qBound(0, qCeil(rect.right() * size.width()), size.width()),
            qBound(0, qCeil(rect.bottom() * size.height()), size.height())
        };
    }

    int luminance(const uchar *pixel)
    {
        const int color = (pixel[0] * 77 + pixel[1] * 150 + pixel[2] * 29) / 256;
        return (color * pixel[3] + 255 * (255 - pixel[3])) / 255;
    }

    int percentile(const std::array<int, 256> &histogram, const int count, const double part)
    {
        const int target = qMax(1, qCeil(count * part));
        int cumulative = 0;
        for (int value = 0; value < 256; ++value)
        {
            cumulative += histogram[value];
            if (cumulative >= target)
                return value;
        }
        return 255;
    }

    bool isCancelled(const std::function<bool()> &cancelled)
    {
        return cancelled && cancelled();
    }

    bool markInk(const QImage &source, QImage &mask, const PixelBox &box,
        const Content::MangaRender::Ink ink, int &markedPixels,
        const std::function<bool()> &cancelled)
    {
        std::array<int, 256> histogram {};
        int samples = 0;
        for (int y = box.top; y < box.bottom; ++y)
        {
            const uchar *row = source.constScanLine(y);
            for (int x = box.left; x < box.right; ++x)
            {
                ++histogram[luminance(row + x * 4)];
                ++samples;
            }
            if (((y - box.top) % 64 == 0) && isCancelled(cancelled))
                return false;
        }
        if (samples == 0)
            return true;

        const int background = percentile(histogram, samples, ink == Content::MangaRender::Ink::Dark ? 0.8 : 0.2);
        const int foreground = percentile(histogram, samples, ink == Content::MangaRender::Ink::Dark ? 0.02 : 0.98);
        if (qAbs(background - foreground) < 20)
            return true;

        const int threshold = (background + foreground) / 2;
        for (int y = box.top; y < box.bottom; ++y)
        {
            const uchar *row = source.constScanLine(y);
            for (int x = box.left; x < box.right; ++x)
            {
                const int value = luminance(row + x * 4);
                if (ink == Content::MangaRender::Ink::Dark ? value >= threshold : value <= threshold)
                    continue;

                for (int dy = -1; dy <= 1; ++dy)
                {
                    if ((y + dy < box.top) || (y + dy >= box.bottom))
                        continue;
                    uchar *maskRow = mask.scanLine(y + dy);
                    for (int dx = -1; dx <= 1; ++dx)
                    {
                        if ((x + dx >= box.left) && (x + dx < box.right)
                            && (maskRow[x + dx] == 0))
                        {
                            maskRow[x + dx] = 255;
                            ++markedPixels;
                        }
                    }
                }
            }
            if (((y - box.top) % 64 == 0) && isCancelled(cancelled))
                return false;
        }
        return true;
    }

    bool flatBackground(const QImage &source, const QImage &mask, const PixelBox &box,
        std::array<uchar, 4> &color)
    {
        std::array<QList<uchar>, 4> channels;
        QList<uchar> lightness;
        const int left = qMax(0, box.left - 3);
        const int top = qMax(0, box.top - 3);
        const int right = qMin(source.width(), box.right + 3);
        const int bottom = qMin(source.height(), box.bottom + 3);
        for (int y = top; y < bottom; ++y)
        {
            const uchar *row = source.constScanLine(y);
            const uchar *maskRow = mask.constScanLine(y);
            for (int x = left; x < right; ++x)
            {
                if ((x >= box.left) && (x < box.right) && (y >= box.top) && (y < box.bottom))
                    continue;
                if (maskRow[x] != 0)
                    continue;

                const uchar *pixel = row + x * 4;
                lightness.append(luminance(pixel));
                for (int channel = 0; channel < 4; ++channel)
                    channels[channel].append(pixel[channel]);
            }
        }
        if (lightness.size() < 8)
            return false;

        std::sort(lightness.begin(), lightness.end());
        if ((lightness.at(lightness.size() * 9 / 10) - lightness.at(lightness.size() / 10)) > 24)
            return false;

        for (int channel = 0; channel < 4; ++channel)
        {
            std::sort(channels[channel].begin(), channels[channel].end());
            if ((channel < 3) && ((channels[channel].at(channels[channel].size() * 9 / 10)
                - channels[channel].at(channels[channel].size() / 10)) > 24))
                return false;
            color[channel] = channels[channel].at(channels[channel].size() / 2);
        }
        return true;
    }

    bool repairMask(QImage &cleaned, const QImage &mask, const QList<PixelBox> &boxes,
        const std::function<bool()> &cancelled)
    {
        const int width = cleaned.width();
        const int height = cleaned.height();
        QList<uchar> known(width * height, 1);
        for (int y = 0; y < height; ++y)
        {
            const uchar *maskRow = mask.constScanLine(y);
            for (int x = 0; x < width; ++x)
                known[y * width + x] = maskRow[x] == 0;
        }

        for (const PixelBox &box : boxes)
        {
            std::array<uchar, 4> color {};
            if (!flatBackground(cleaned, mask, box, color))
                continue;
            for (int y = box.top; y < box.bottom; ++y)
            {
                uchar *row = cleaned.scanLine(y);
                const uchar *maskRow = mask.constScanLine(y);
                for (int x = box.left; x < box.right; ++x)
                {
                    if (maskRow[x] == 0)
                        continue;
                    std::copy(color.begin(), color.end(), row + x * 4);
                    known[y * width + x] = 1;
                }
            }
            if (isCancelled(cancelled))
                return false;
        }

        QList<int> queue;
        for (int y = 0; y < height; ++y)
        {
            for (int x = 0; x < width; ++x)
            {
                const int index = y * width + x;
                if (known[index] != 0)
                    continue;
                const bool edge = ((x > 0) && (known[index - 1] == 1))
                    || ((x + 1 < width) && (known[index + 1] == 1))
                    || ((y > 0) && (known[index - width] == 1))
                    || ((y + 1 < height) && (known[index + width] == 1));
                if (edge)
                {
                    known[index] = 2;
                    queue.append(index);
                }
            }
            if ((y % 64 == 0) && isCancelled(cancelled))
                return false;
        }

        for (qsizetype head = 0; head < queue.size(); ++head)
        {
            const int index = queue.at(head);
            const int x = index % width;
            const int y = index / width;
            std::array<double, 4> sums {};
            double weightSum = 0;
            for (int dy = -3; dy <= 3; ++dy)
            {
                if ((y + dy < 0) || (y + dy >= height))
                    continue;
                const uchar *row = cleaned.constScanLine(y + dy);
                for (int dx = -3; dx <= 3; ++dx)
                {
                    if (((dx == 0) && (dy == 0)) || (x + dx < 0) || (x + dx >= width))
                        continue;
                    if (known[(y + dy) * width + x + dx] != 1)
                        continue;

                    const double distance = dx * dx + dy * dy;
                    const double weight = 1 / (distance * std::sqrt(distance));
                    const uchar *pixel = row + (x + dx) * 4;
                    for (int channel = 0; channel < 4; ++channel)
                        sums[channel] += pixel[channel] * weight;
                    weightSum += weight;
                }
            }
            if (weightSum == 0)
                return false;

            uchar *pixel = cleaned.scanLine(y) + x * 4;
            for (int channel = 0; channel < 4; ++channel)
                pixel[channel] = qRound(sums[channel] / weightSum);
            known[index] = 1;
            const std::array<int, 4> neighbours {
                x > 0 ? index - 1 : -1,
                x + 1 < width ? index + 1 : -1,
                y > 0 ? index - width : -1,
                y + 1 < height ? index + width : -1
            };
            for (const int next : neighbours)
            {
                if ((next >= 0) && (known[next] == 0))
                {
                    known[next] = 2;
                    queue.append(next);
                }
            }
            if ((head % 8192 == 0) && isCancelled(cancelled))
                return false;
        }
        return !isCancelled(cancelled);
    }
}

Content::MangaRender::NormalizedPage Content::MangaRender::normalizeGeneratedPage(
    const QSize &originalSize, const QImage &generated, const std::function<bool()> &cancelled)
{
    NormalizedPage result;
    if (cancelled && cancelled())
    {
        result.error = QStringLiteral("Generated manga page validation cancelled");
        return result;
    }
    if (originalSize.isEmpty() || generated.isNull()
        || (qint64(originalSize.width()) * originalSize.height() > MAX_PIXELS)
        || (qint64(generated.width()) * generated.height() > MAX_PIXELS))
    {
        result.error = QStringLiteral("Invalid or oversized generated manga page");
        return result;
    }

    // Permit integer rounding of at most one pixel on each generated axis.
    // A changed aspect ratio must not be hidden by stretching, cropping or padding.
    const qint64 difference = std::abs(qint64(generated.width()) * originalSize.height()
        - qint64(generated.height()) * originalSize.width());
    if (difference > qint64(originalSize.width()) + originalSize.height())
    {
        result.error = QStringLiteral("Generated manga page changed the original aspect ratio");
        return result;
    }

    result.image = generated.scaled(originalSize, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
    if (cancelled && cancelled())
    {
        result.image = {};
        result.error = QStringLiteral("Generated manga page validation cancelled");
    }
    else if (result.image.isNull())
        result.error = QStringLiteral("Generated manga page could not be resized");
    return result;
}

Content::MangaRender::Crop Content::MangaRender::cropPage(
    const QImage &image, const QRectF &bounds, const std::function<bool()> &cancelled)
{
    Crop crop;
    if (isCancelled(cancelled))
    {
        crop.error = QStringLiteral("Manga inspection cancelled");
        return crop;
    }
    if (image.isNull() || (qint64(image.width()) * image.height() > MAX_PIXELS))
    {
        crop.error = QStringLiteral("Invalid or oversized manga page");
        return crop;
    }
    if (!validRect(bounds) || (bounds.left() < 0) || (bounds.top() < 0)
        || (bounds.right() > 1) || (bounds.bottom() > 1))
    {
        crop.error = QStringLiteral("Invalid manga inspection crop");
        return crop;
    }

    const PixelBox box = pixelBox(bounds, image.size());
    const int width = box.right - box.left;
    const int height = box.bottom - box.top;
    crop.image = image.copy(box.left, box.top, width, height);
    crop.bounds = QRectF(qreal(box.left) / image.width(), qreal(box.top) / image.height(),
        qreal(width) / image.width(), qreal(height) / image.height());
    if (isCancelled(cancelled))
    {
        crop.image = {};
        crop.error = QStringLiteral("Manga inspection cancelled");
    }
    return crop;
}

QFont Content::MangaRender::mangaFont(const FontFamily family)
{
    static const std::array<QString, 3> families = []
    {
        std::array<QString, 3> loaded;
        const std::array<QString, 3> prefixes {
            QStringLiteral("BalsamiqSans"),
            QStringLiteral("PT_Serif-Web"),
            QStringLiteral("FiraSansCondensed")
        };
        const QStringList faces {
            QStringLiteral("Regular"), QStringLiteral("Bold"),
            QStringLiteral("Italic"), QStringLiteral("BoldItalic")
        };
        for (std::size_t index = 0; index < prefixes.size(); ++index)
        {
            for (const QString &face : faces)
            {
                const QString path = QStringLiteral(":/reader/%1-%2.ttf").arg(prefixes[index], face);
                const int fontId = QFontDatabase::addApplicationFont(path);
                const QStringList names = QFontDatabase::applicationFontFamilies(fontId);
                if (names.isEmpty())
                    return std::array<QString, 3> {};
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
    const QList<TextRun> &runs, const QRectF &rect, QFont font, const int maximumPixels,
    const qreal angle)
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
                    return {{}, 0, 0, Status::MissingGlyph};
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
        return {path, pixels, layout.lineCount()};
    }
    return {};
}

Content::MangaRender::Result Content::MangaRender::render(const QImage &original,
    const QList<Region> &regions, const std::function<bool()> &cancelled)
{
    Result result;
    if (original.isNull() || (original.width() <= 0) || (original.height() <= 0)
        || (qint64(original.width()) * original.height() > MAX_PIXELS)
        || (regions.size() > MAX_REGIONS))
    {
        result.error = QStringLiteral("Invalid or oversized manga page");
        return result;
    }

    const QImage source = original.convertToFormat(QImage::Format_RGBA8888);
    result.mask = QImage(source.size(), QImage::Format_Grayscale8);
    result.mask.fill(0);
    QList<PixelBox> boxes;
    QList<PendingText> pending;
    pending.reserve(regions.size());
    int boxCount = 0;
    qint64 boxPixels = 0;
    for (const Region &region : regions)
    {
        Diagnostic diagnostic;
        diagnostic.regionId = region.id;
        if (isCancelled(cancelled))
        {
            result.error = QStringLiteral("Manga render cancelled");
            return result;
        }
        QString translation;
        for (const TextRun &run : region.translation)
            translation += run.text;
        if (translation.trimmed().isEmpty())
        {
            diagnostic.status = Status::EmptyTranslation;
            result.diagnostics.append(diagnostic);
            continue;
        }
        if (!validRect(region.bounds) || !validRect(region.layout)
            || !containsRect(region.bounds, region.layout)
            || region.inkBoxes.isEmpty() || !std::isfinite(region.angle)
            || (qAbs(region.angle) > 45) || (region.translation.size() > 32)
            || (translation.toUtf8().size() > 3000))
        {
            diagnostic.status = region.inkBoxes.isEmpty() ? Status::NoInkMask : Status::InvalidGeometry;
            result.diagnostics.append(diagnostic);
            continue;
        }

        QList<PixelBox> regionBoxes;
        qint64 regionBoxPixels = 0;
        for (const QRectF &rect : region.inkBoxes)
        {
            if (!validRect(rect) || !containsRect(region.bounds, rect))
            {
                diagnostic.status = Status::InvalidGeometry;
                break;
            }
            const PixelBox box = pixelBox(rect, source.size());
            regionBoxPixels += qint64(box.right - box.left) * (box.bottom - box.top);
            regionBoxes.append(box);
        }
        if ((diagnostic.status == Status::InvalidGeometry)
            || (boxCount + regionBoxes.size() > MAX_INK_BOXES)
            || (boxPixels + regionBoxPixels > qint64(source.width()) * source.height() / 2))
        {
            diagnostic.status = Status::InvalidGeometry;
            result.diagnostics.append(diagnostic);
            continue;
        }

        const QRectF layout(region.layout.x() * source.width(), region.layout.y() * source.height(),
            region.layout.width() * source.width(), region.layout.height() * source.height());
        const qreal margin = qMin(qreal(12), qMax(qreal(2), qMin(layout.width(), layout.height()) * 0.04));
        const QRectF textRect = layout.adjusted(margin, margin, -margin, -margin);
        if (textRect.isEmpty())
        {
            diagnostic.status = Status::Overflow;
            result.diagnostics.append(diagnostic);
            continue;
        }
        const QFont font = mangaFont(region.family);
        if (font.family().isEmpty())
        {
            result.error = QStringLiteral("Bundled manga fonts unavailable");
            return result;
        }
        const int maximumPixels = qMin(256, qMax(1, qFloor(textRect.height())));
        const TextShape shape = typesetText(region.translation, textRect, font, maximumPixels, region.angle);
        diagnostic.fontPixels = shape.fontPixels;
        diagnostic.layoutLines = shape.lines;
        if (shape.status != Status::Rendered || shape.path.isEmpty() || (shape.fontPixels < 9))
        {
            diagnostic.status = shape.status == Status::Rendered ? Status::Overflow : shape.status;
            result.diagnostics.append(diagnostic);
            continue;
        }

        for (const PixelBox &box : regionBoxes)
        {
            if (!markInk(source, result.mask, box, region.ink, diagnostic.maskedPixels, cancelled))
            {
                result.error = QStringLiteral("Manga render cancelled");
                return result;
            }
        }
        if (diagnostic.maskedPixels == 0)
        {
            diagnostic.status = Status::NoInkMask;
            result.diagnostics.append(diagnostic);
            continue;
        }

        boxCount += regionBoxes.size();
        boxPixels += regionBoxPixels;
        boxes.append(regionBoxes);
        const bool light = region.ink == Ink::Light;
        const QColor textColor = region.textColor.isValid() ? region.textColor
            : QColor(light ? Qt::white : Qt::black);
        const QColor outlineColor = region.outlineColor.isValid() ? region.outlineColor
            : QColor(light ? Qt::black : Qt::white);
        pending.append({shape, textColor, outlineColor});
        result.diagnostics.append(diagnostic);
    }

    result.cleaned = source.copy();
    if (!repairMask(result.cleaned, result.mask, boxes, cancelled))
    {
        result.error = isCancelled(cancelled) ? QStringLiteral("Manga render cancelled")
            : QStringLiteral("Manga mask cannot be repaired");
        result.cleaned = {};
        return result;
    }

    result.rendered = result.cleaned.copy();
    QPainter painter(&result.rendered);
    painter.setRenderHint(QPainter::Antialiasing);
    for (const PendingText &text : pending)
    {
        painter.strokePath(text.shape.path, QPen(text.outlineColor,
            1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.fillPath(text.shape.path, text.textColor);
    }
    painter.end();
    return result;
}
