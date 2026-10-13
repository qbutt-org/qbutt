// SPDX-License-Identifier: GPL-2.0-or-later

#include <array>
#include <stdexcept>

#include <QDebug>
#include <QDir>
#include <QGuiApplication>
#include <QImage>
#include <QPainter>
#include <QRawFont>

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

    QRectF normalized(const QRectF &rect, const QSize &size)
    {
        return {rect.x() / size.width(), rect.y() / size.height(),
            rect.width() / size.width(), rect.height() / size.height()};
    }

    void verifyFonts()
    {
        const QString alphabet = u"АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя"_s;
        for (const FontFamily family : {FontFamily::Comic, FontFamily::Serif, FontFamily::Condensed})
        {
            for (int face = 0; face < 4; ++face)
            {
                QFont font = mangaFont(family);
                font.setPixelSize(32);
                font.setBold((face & 1) != 0);
                font.setItalic((face & 2) != 0);
                const QRawFont raw = QRawFont::fromFont(font);
                require(raw.isValid() && raw.familyName() == font.family(), u"Bundled font was substituted"_s);
                require(raw.weight() == ((face & 1) ? 700 : 400)
                    && ((raw.style() != QFont::StyleNormal) == ((face & 2) != 0)),
                    u"Requested physical font face was substituted"_s);
                for (const QChar character : alphabet)
                    require(raw.supportsCharacter(character), u"Missing Cyrillic glyph: "_s + character);
                qInfo().noquote() << u"FACE %1 [%2] weight=%3 Cyrillic=66"_s
                    .arg(raw.familyName(), raw.styleName()).arg(raw.weight());
            }
        }
    }

    void verifyUnchangedOutside(const QImage &original, const Result &result, const QList<Region> &regions)
    {
        int changed = 0;
        for (int y = 0; y < original.height(); ++y)
        {
            for (int x = 0; x < original.width(); ++x)
            {
                if (original.pixel(x, y) == result.rendered.pixel(x, y))
                    continue;
                const QPointF point(qreal(x) / original.width(), qreal(y) / original.height());
                bool inside = false;
                for (const Region &region : regions)
                    inside |= region.bounds.contains(point);
                require(inside, u"Lettering changed pixels outside accepted regions"_s);
                ++changed;
            }
        }
        require(changed > 10'000, u"Page lettering did not visibly change"_s);
    }

    void exercise(const QString &outputDir)
    {
        verifyFonts();
        QImage original(1200, 1500, QImage::Format_RGBA8888);
        original.fill(Qt::white);
        QPainter painter(&original);
        painter.setRenderHint(QPainter::Antialiasing);
        QList<Region> regions;
        const std::array<FontFamily, 3> families {FontFamily::Comic, FontFamily::Serif, FontFamily::Condensed};

        const auto addRegion = [&](const QString &id, const FontFamily family, const QRectF &rect,
                                   const QList<TextRun> &source, const QList<TextRun> &target,
                                   const qreal angle = 0, const Ink ink = Ink::Dark)
        {
            const QFont font = mangaFont(family);
            const TextShape shape = typesetText(source, rect.adjusted(12, 12, -12, -12), font, 100, angle);
            require(!shape.path.isEmpty(), u"Authored source lettering did not fit"_s);
            painter.fillRect(rect, ink == Ink::Dark ? Qt::white : Qt::black);
            painter.fillPath(shape.path, ink == Ink::Dark ? Qt::black : Qt::white);
            Region region;
            region.id = id;
            region.family = family;
            region.angle = angle;
            region.ink = ink;
            region.bounds = normalized(rect, original.size());
            region.layout = region.bounds;
            region.inkBoxes = {normalized(shape.path.boundingRect().adjusted(-2, -2, 2, 2), original.size())};
            region.translation = target;
            regions.append(region);
        };

        for (int column = 0; column < 3; ++column)
        {
            for (int face = 0; face < 4; ++face)
            {
                const bool bold = (face & 1) != 0;
                const bool italic = (face & 2) != 0;
                const QString id = u"family-%1-face-%2"_s.arg(column).arg(face);
                const QRectF rect(25 + column * 400, 40 + face * 235, 350, 185);
                painter.setPen(Qt::gray);
                painter.setFont(QFont(u"Segoe UI"_s, 10));
                painter.drawText(rect.topLeft() - QPointF(0, 8), id);
                addRegion(id, families[column], rect,
                    {{u"I'm still waiting for you, Ren."_s, bold, italic}},
                    {{u"Ёжик всё ещё ждёт тебя, Рен."_s, bold, italic}});
            }
        }

        addRegion(u"mixed-emphasis"_s, FontFamily::Comic, QRectF(25, 1000, 750, 200),
            {{u"Please, "_s}, {u"wait"_s, true}, {u" for me. "_s}, {u"Don't go."_s, false, true}},
            {{u"Пожалуйста, "_s}, {u"дождись"_s, true}, {u" меня. "_s}, {u"Не уходи."_s, false, true}});
        addRegion(u"rotated-emphasis"_s, FontFamily::Condensed, QRectF(825, 1000, 350, 200),
            {{u"HAAH!"_s, true, true}}, {{u"ХА-А!"_s, true, true}}, -20);
        addRegion(u"light-caption"_s, FontFamily::Serif, QRectF(25, 1250, 1150, 200),
            {{u"She had already left. "_s}, {u"He was still waiting."_s, false, true}},
            {{u"Она уже ушла. "_s}, {u"Он всё ещё ждал."_s, false, true}}, 0, Ink::Light);
        painter.end();

        const Result result = render(original, regions);
        require(result.error.isEmpty() && result.diagnostics.size() == regions.size(), result.error);
        for (const Diagnostic &diagnostic : result.diagnostics)
        {
            require(diagnostic.status == Status::Rendered && diagnostic.fontPixels >= 9
                && diagnostic.maskedPixels > 0, u"Region did not render: "_s + diagnostic.regionId);
            qInfo().noquote() << u"REGION %1 pixels=%2 lines=%3 mask=%4"_s
                .arg(diagnostic.regionId).arg(diagnostic.fontPixels).arg(diagnostic.layoutLines)
                .arg(diagnostic.maskedPixels);
        }
        verifyUnchangedOutside(original, result, regions);
        require(render(original, regions).rendered == result.rendered, u"Repeated native render changed pixels"_s);

        const QRectF inspectionBounds = normalized(QRectF(123.25, 250.25, 99.25, 100.25), original.size());
        const QRect expectedPixels(123, 250, 100, 101);
        for (const QImage &layer : {original, result.mask, result.cleaned, result.rendered})
        {
            const Crop crop = cropPage(layer, inspectionBounds);
            require(crop.error.isEmpty() && crop.image == layer.copy(expectedPixels),
                u"Inspection crop rescaled or changed actual layer pixels"_s);
            require(crop.bounds == normalized(expectedPixels, original.size()),
                u"Inspection crop lost its full-page coordinates"_s);
        }
        require(cropPage(original, QRectF(0, 0, 1, 1)).image == original,
            u"Full-page inspection changed pixels"_s);
        for (const QRectF &invalidCrop : {QRectF(1, 0, 0.0000001, 0.1), QRectF(-0.0000001, 0, 0.1, 0.1),
                                         QRectF(0, 0, 0, 0.1), QRectF(0.9, 0, 0.2, 0.1)})
        {
            const Crop crop = cropPage(original, invalidCrop);
            require(!crop.error.isEmpty() && crop.image.isNull(), u"Invalid inspection crop was accepted"_s);
        }
        int cropChecks = 0;
        const Crop cancelledCrop = cropPage(original, inspectionBounds, [&cropChecks] { return ++cropChecks == 2; });
        require(!cancelledCrop.error.isEmpty() && cancelledCrop.image.isNull(),
            u"Cancelled inspection returned an image"_s);

        const Region &mixed = regions.at(12);
        Region plain = mixed;
        for (TextRun &run : plain.translation)
        {
            run.bold = false;
            run.italic = false;
        }
        require(render(original, {mixed}).rendered != render(original, {plain}).rendered,
            u"Mixed emphasis was lost during shaping"_s);

        Region level = regions.at(13);
        level.angle = 0;
        require(render(original, {regions.at(13)}).rendered != render(original, {level}).rendered,
            u"Lettering rotation was lost"_s);

        Region unsupported = regions.constFirst();
        const char32_t missingCharacter[] {0x10FFFF};
        unsupported.translation = {{QString::fromUcs4(missingCharacter, 1)}};
        const Result missing = render(original, {unsupported});
        require(missing.diagnostics.constFirst().status == Status::MissingGlyph && missing.rendered == original,
            u"Missing target glyph erased the original lettering"_s);

        Region invalid = regions.constFirst();
        invalid.angle = 46;
        const Result rejected = render(original, {invalid});
        require(rejected.diagnostics.constFirst().status == Status::InvalidGeometry && rejected.rendered == original,
            u"Unbounded lettering rotation was accepted"_s);

        const Result cancelled = render(original, regions, [] { return true; });
        require(!cancelled.error.isEmpty() && cancelled.rendered.isNull(), u"Cancelled render completed"_s);
        require(original.save(outputDir + u"/original.png"_s)
            && result.rendered.save(outputDir + u"/result.png"_s)
            && result.cleaned.save(outputDir + u"/cleaned.png"_s)
            && result.mask.save(outputDir + u"/mask.png"_s), u"Native render artifacts could not be saved"_s);
        qInfo() << "PASS manga_lettering families=3 physical_faces=12 regions=15 mixed=ok rotation=ok outside=unchanged crops=4";
    }
}

int main(int argc, char **argv)
{
    QGuiApplication application(argc, argv);
    try
    {
        require(application.arguments().size() == 2, u"Pass an isolated output directory"_s);
        const QString outputDir = application.arguments().at(1);
        require(QDir().mkpath(outputDir), u"Output directory could not be created"_s);
        exercise(outputDir);
        return 0;
    }
    catch (const std::exception &error)
    {
        qCritical() << error.what();
        return 1;
    }
}
