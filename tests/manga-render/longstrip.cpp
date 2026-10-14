// SPDX-License-Identifier: GPL-2.0-or-later
#include "longstrip.h"

#include <stdexcept>

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QImageIOHandler>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLinearGradient>
#include <QPainter>
#include <QRegion>

#include "gui/content/mangarender.h"

using namespace Content::MangaRender;
using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    QJsonObject jsonRect(const QRectF &rect)
    {
        return { { "x", rect.x() }, { "y", rect.y() }, { "width", rect.width() }, { "height", rect.height() } };
    }

    QRectF normalized(const QRectF &rect, const QSize &size)
    {
        return { rect.x() / size.width(), rect.y() / size.height(), rect.width() / size.width(),
                 rect.height() / size.height() };
    }

    QJsonObject proposalRegion(const Region &region, const QString &source)
    {
        QJsonArray runs;
        for (const TextRun &run : region.translation)
            runs.append(QJsonObject { { "text", run.text }, { "bold", run.bold }, { "italic", run.italic } });
        QJsonArray cleanup;
        for (const QRectF &rect : region.cleanupRects)
            cleanup.append(jsonRect(rect));
        return { { "id", region.id },
                 { "sourceText", source },
                 { "translation", runs },
                 { "family", "comic" },
                 { "angle", region.angle },
                 { "textColor", region.textColor.name() },
                 { "outlineColor", region.outlineColor.name() },
                 { "outlineWidth", region.outlineWidth },
                 { "bounds", jsonRect(region.bounds) },
                 { "layout", jsonRect(region.layout) },
                 { "cleanupRects", cleanup } };
    }
}

void exerciseLongStrip(const QString &directory)
{
    require(QDir().mkpath(directory), u"Longstrip output could not be created"_s);
    QImage original(800, 20'000, QImage::Format_RGBA8888);
    QPainter painter(&original);
    QLinearGradient gradient(0, 0, 800, 20'000);
    gradient.setColorAt(0, QColor(55, 90, 135));
    gradient.setColorAt(0.5, QColor(208, 118, 86));
    gradient.setColorAt(1, QColor(62, 105, 83));
    painter.fillRect(original.rect(), gradient);
    painter.setPen(QPen(QColor(35, 52, 61), 5));
    for (int y = 1000; y < original.height(); y += 1700)
        painter.drawEllipse(QRect(80, y, 570, 950));

    const QList<QRect> rectangles { { 80, 200, 640, 320 }, { 80, 14'450, 640, 320 }, { 80, 15'800, 640, 440 } };
    const QStringList source { u"Ren is my elder sister."_s, u"Keep the colors and outlines."_s,
                               u"This complete balloon crosses the window edge."_s };
    QImage generated = original.copy();
    QPainter clean(&generated);
    QList<Region> fullRegions;
    for (int index = 0; index < rectangles.size(); ++index)
    {
        const QRect rect = rectangles.at(index);
        painter.fillRect(rect, QColor(232, 196, 160));
        clean.fillRect(rect, QColor(232, 196, 160));
        const TextShape shape = typesetText({ { source.at(index) } }, QRectF(rect).adjusted(20, 20, -20, -20),
                                            mangaFont(FontFamily::Comic), 64);
        require(shape.status == Status::Rendered, u"Authored long dialogue did not fit"_s);
        painter.fillPath(shape.path, QColor(45, 59, 75));
        Region region;
        region.id = index == 2 ? u"t13952-crossing"_s : u"t0-%1"_s.arg(index);
        region.bounds = normalized(QRectF(rect).adjusted(2, 2, -2, -2), original.size());
        region.layout = region.bounds;
        region.cleanupRects = { normalized(shape.path.boundingRect().adjusted(-3, -3, 3, 3), original.size()) };
        region.textColor = QColor(49, 68, 85);
        region.outlineColor = QColor(255, 245, 215);
        region.outlineWidth = 3;
        region.translation =
            index == 2
                ? QList<TextRun> { { u"Эта реплика "_s },
                                   { u"целиком"_s, false, true },
                                   { u" пересекает "_s },
                                   { u"стык окон."_s, true, true } }
                : QList<TextRun> { { index == 0 ? u"Рен — моя старшая сестра."_s : u"Сохраняем цвет и обводку."_s } };
        fullRegions.append(region);
    }
    painter.end();
    clean.end();
    QImage composed = original.copy();
    QRegion allowed;
    for (const Region &region : fullRegions)
    {
        const QRectF rect = region.layout;
        const int left = qFloor(rect.left() * original.width());
        const int top = qFloor(rect.top() * original.height());
        const int right = qCeil(rect.right() * original.width());
        const int bottom = qCeil(rect.bottom() * original.height());
        allowed += QRect(left, top, right - left, bottom - top);
    }
    for (const int top : { 0, 13'952 })
    {
        const int height = qMin(16'000, original.height() - top);
        const PageImage crop = cropWindow(original, top, height);
        const PageImage cleanup = cropWindow(generated, top, height);
        require(crop.error.isEmpty() && cleanup.error.isEmpty(), u"Native long crop failed"_s);
        QList<Region> regions;
        QJsonArray proposal;
        for (int index = 0; index < fullRegions.size(); ++index)
        {
            if ((top == 0) != (index < 2))
                continue;
            Region region = fullRegions.at(index);
            const auto local = [&](const QRectF &rect) {
                return QRectF(rect.x(), (rect.y() * original.height() - top) / height, rect.width(),
                              rect.height() * original.height() / height);
            };
            region.bounds = local(region.bounds);
            region.layout = local(region.layout);
            for (QRectF &rect : region.cleanupRects)
                rect = local(rect);
            regions.append(region);
            proposal.append(proposalRegion(region, source.at(index)));
        }
        const Result result = renderCleaned(crop.image, cleanup.image, regions);
        require(result.error.isEmpty(), result.error);
        for (const Diagnostic &diagnostic : result.diagnostics)
            require(diagnostic.status == Status::Rendered, u"Longstrip lettering did not fit"_s);
        QImage noisy = result.rendered.copy();
        noisy.setPixelColor(0, 0, Qt::red);
        require(composeWindow(composed, noisy, top, regions).isEmpty(), u"Native long compose failed"_s);
        QJsonArray clipped;
        if (top == 0)
            clipped.append(jsonRect(normalized(QRectF(82, 15'802, 636, 198), crop.image.size())));
        QJsonArray terms;
        QJsonArray facts;
        if (top == 0)
        {
            const QJsonObject evidence { { "region_id", "t0-0" }, { "quote", "Ren is my elder sister." } };
            terms.append(QJsonObject {
                { "source", "Ren" }, { "target", "Рен" }, { "forms", QJsonArray() }, { "evidence", evidence } });
            facts.append(QJsonObject {
                { "subject", "Ren" }, { "attribute", "gender" }, { "value", "female" }, { "evidence", evidence } });
        }
        QFile file(directory + u"/window-%1-proposal.json"_s.arg(top));
        require(file.open(QIODevice::WriteOnly), u"Longstrip proposal could not be saved"_s);
        file.write(QJsonDocument(QJsonObject { { "regions", proposal },
                                               { "terms", terms },
                                               { "facts", facts },
                                               { "clippedRegions", clipped },
                                               { "unsupportedReason", QJsonValue::Null } })
                       .toJson());
        require(cleanup.image.save(directory + u"/t%1-cleanup.png"_s.arg(top)),
                u"Longstrip cleanup could not be saved"_s);
    }
    int changed = 0;
    int fillPixels = 0;
    int outlinePixels = 0;
    for (int y = 0; y < original.height(); ++y)
    {
        for (int x = 0; x < original.width(); ++x)
        {
            if (original.pixel(x, y) != composed.pixel(x, y))
            {
                require(allowed.contains({ x, y }), u"Longstrip composition changed non-region artwork"_s);
                ++changed;
            }
            fillPixels += composed.pixel(x, y) == fullRegions.constFirst().textColor.rgb();
            outlinePixels += composed.pixel(x, y) == fullRegions.constFirst().outlineColor.rgb();
        }
    }
    require(changed > 1000, u"Longstrip lettering did not change"_s);
    require(fillPixels > 100 && outlinePixels > 100, u"Longstrip lettering lost explicit colors or outline"_s);
    require(supportsOriginalSize({ 1000, 40'000 }) && !supportsOriginalSize({ 1001, 40'000 }) &&
                !supportsOriginalSize({ 1, 65'537 }) && !supportsImageSize(original.size()),
            u"Longstrip size bounds changed"_s);
    require(!cropWindow(original, 19'999, 2).error.isEmpty() && !cropWindow(original, 0, 16'001).error.isEmpty() &&
                !cropWindow(original, 0, 16'000, [] { return true; }).error.isEmpty(),
            u"Invalid or cancelled crop was accepted"_s);
    QImage cancelled = original.copy();
    const QImage crop = cropWindow(original, 0, 16'000).image;
    require(!composeWindow(cancelled, crop, 0, { fullRegions.constFirst() }, [] { return true; }).isEmpty(),
            u"Cancelled long composition succeeded"_s);
    require(original.save(directory + u"/original.png"_s) && composed.save(directory + u"/result.png"_s) &&
                original.copy(0, 14'000, 800, 2600).save(directory + u"/seam-original.png"_s) &&
                composed.copy(0, 14'000, 800, 2600).save(directory + u"/seam-result.png"_s),
            u"Longstrip images could not be saved"_s);
    for (const bool rotated : { true, false })
    {
        QImage image(rotated ? QSize(4000, 5000) : QSize(1200, 1500), QImage::Format_RGB32);
        image.fill(QColor(75, 135, 185));
        QPainter marker(&image);
        marker.fillRect(QRect(0, 0, 200, 300), QColor(235, 70, 45));
        marker.end();
        const QString path = directory + (rotated ? u"/orientation-rotated.jpg"_s : u"/orientation-mirrored.jpg"_s);
        require(image.save(path, "JPEG"), u"Authored orientation fixture could not be written"_s);
        QFile file(path);
        require(file.open(QIODevice::ReadWrite), u"Authored JPEG could not be opened"_s);
        QByteArray jpeg = file.readAll();
        // One little-endian TIFF Orientation tag in a legal EXIF APP1 segment.
        QByteArray exif =
            QByteArray::fromHex("ffe1002245786966000049492a0008000000010012010300010000000600000000000000");
        exif[28] = rotated ? 6 : 2;
        jpeg.insert(2, exif);
        require(file.seek(0) && file.write(jpeg) == jpeg.size(), u"Authored EXIF metadata could not be written"_s);
        file.close();
        QImageReader reader(path);
        require(reader.size() == image.size() && reader.transformation() != QImageIOHandler::TransformationNone &&
                    reader.read().size() == image.size(),
                u"Authored JPEG lost its orientation metadata"_s);
    }
    qInfo() << "PASS manga_long_native geometry=800x20000 regions=3 crossing=complete colors=preserved "
               "outside=unchanged cancelled=rejected";
}
