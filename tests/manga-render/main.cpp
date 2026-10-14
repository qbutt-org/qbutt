// SPDX-License-Identifier: GPL-2.0-or-later

#include <array>
#include <stdexcept>

#include <QDebug>
#include <QDir>
#include <QGuiApplication>
#include <QImage>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPainter>
#include <QRawFont>
#include <QRegion>
#include <limits>

#include "gui/content/mangarender.h"
#include "longstrip.h"

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
        return { rect.x() / size.width(), rect.y() / size.height(), rect.width() / size.width(),
            rect.height() / size.height() };
    }

    void verifyFonts()
    {
        const QString alphabet = u"АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя"_s;
        for (const FontFamily family : { FontFamily::Comic, FontFamily::Serif, FontFamily::Condensed })
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
                qInfo().noquote()
                    << u"FACE %1 [%2] weight=%3 Cyrillic=66"_s.arg(raw.familyName(), raw.styleName()).arg(raw.weight());
            }
        }
    }

    QRect pixels(const QRectF &rect, const QSize &size)
    {
        const int left = qFloor(rect.left() * size.width());
        const int top = qFloor(rect.top() * size.height());
        const int right = qCeil(rect.right() * size.width());
        const int bottom = qCeil(rect.bottom() * size.height());
        return { left, top, right - left, bottom - top };
    }

    void verifyUnchangedOutside(const QImage &original, const Result &result, const QList<Region> &regions)
    {
        QRegion cleanup;
        QRegion allowed;
        for (const Region &region : regions)
        {
            for (const QRectF &rect : region.cleanupRects)
                cleanup += pixels(rect, original.size());
            allowed += pixels(region.layout, original.size());
        }
        allowed += cleanup;
        int changed = 0;
        for (int y = 0; y < original.height(); ++y)
        {
            for (int x = 0; x < original.width(); ++x)
            {
                require(original.pixel(x, y) == result.cleaned.pixel(x, y) || cleanup.contains({ x, y }),
                    u"Cleanup changed pixels outside cleanupRects"_s);
                if (original.pixel(x, y) == result.rendered.pixel(x, y))
                    continue;
                require(allowed.contains({ x, y }), u"Lettering changed pixels outside cleanupRects/layout"_s);
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
        QImage generated = original.copy();
        QPainter background(&generated);
        QPainter painter(&original);
        painter.setRenderHint(QPainter::Antialiasing);
        QList<Region> regions;
        QJsonArray proposalRegions;
        const std::array<FontFamily, 3> families { FontFamily::Comic, FontFamily::Serif, FontFamily::Condensed };

        const auto addRegion
            = [&](const QString &id, const FontFamily family, const QRectF &rect, const QList<TextRun> &source,
                  const QList<TextRun> &target, const qreal angle = 0, const bool light = false)
        {
            const QFont font = mangaFont(family);
            const TextShape shape = typesetText(source, rect.adjusted(12, 12, -12, -12), font, 100, angle);
            require(!shape.path.isEmpty(), u"Authored source lettering did not fit"_s);
            painter.fillRect(rect, light ? Qt::black : Qt::white);
            background.fillRect(rect, light ? Qt::black : Qt::white);
            painter.fillPath(shape.path, light ? Qt::white : Qt::black);
            Region region;
            region.id = id;
            region.family = family;
            region.angle = angle;
            region.textColor = light ? Qt::white : Qt::black;
            region.outlineColor = light ? Qt::black : Qt::white;
            region.outlineWidth = 0.75;
            region.bounds = normalized(rect, original.size());
            region.layout = region.bounds;
            region.cleanupRects = { normalized(shape.path.boundingRect().adjusted(-2, -2, 2, 2), original.size()) };
            region.translation = target;
            regions.append(region);
            const auto serializeRect = [](const QRectF &value)
            {
                return QJsonObject { { "x", value.x() }, { "y", value.y() }, { "width", value.width() },
                    { "height", value.height() } };
            };
            QString sourceText;
            for (const TextRun &run : source)
                sourceText += run.text;
            QJsonArray translation;
            for (const TextRun &run : target)
                translation.append(
                    QJsonObject { { "text", run.text }, { "bold", run.bold }, { "italic", run.italic } });
            const QStringList familyNames { u"comic"_s, u"serif"_s, u"condensed"_s };
            proposalRegions.append(QJsonObject { { "id", id }, { "sourceText", sourceText },
                { "translation", translation }, { "family", familyNames.at(static_cast<int>(family)) },
                { "angle", angle }, { "textColor", region.textColor.name() },
                { "outlineColor", region.outlineColor.name() }, { "outlineWidth", region.outlineWidth },
                { "bounds", serializeRect(region.bounds) }, { "layout", serializeRect(region.layout) },
                { "cleanupRects", QJsonArray { serializeRect(region.cleanupRects.constFirst()) } } });
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
                addRegion(id, families[column], rect, { { u"I'm still waiting for you, Ren."_s, bold, italic } },
                    { { u"Ёжик всё ещё ждёт тебя, Рен."_s, bold, italic } });
            }
        }

        addRegion(u"mixed-emphasis"_s, FontFamily::Comic, QRectF(25, 1000, 750, 200),
            { { u"Please, "_s }, { u"wait"_s, true }, { u" for me. "_s }, { u"Don't go."_s, false, true } },
            { { u"Пожалуйста, "_s }, { u"дождись"_s, true }, { u" меня. "_s }, { u"Не уходи."_s, false, true } });
        addRegion(u"rotated-emphasis"_s, FontFamily::Condensed, QRectF(825, 1000, 350, 200),
            { { u"HAAH!"_s, true, true } }, { { u"ХА-А!"_s, true, true } }, -20);
        addRegion(u"light-caption"_s, FontFamily::Serif, QRectF(25, 1250, 1150, 200),
            { { u"She had already left. "_s }, { u"He was still waiting."_s, false, true } },
            { { u"Она уже ушла. "_s }, { u"Он всё ещё ждал."_s, false, true } }, 0, true);
        painter.end();
        background.end();
        generated.setPixelColor(0, 0, Qt::red);
        const Typography typography = prepareTypography(original.size(), regions);
        require(
            typography.error.isEmpty() && typography.shapes.size() == regions.size(), u"Typography preflight failed"_s);
        for (const Diagnostic &diagnostic : typography.diagnostics)
            require(diagnostic.status == Status::Rendered, u"Typography preflight rejected a region"_s);

        const Preparation prepared = preparePage(original, regions, 16'000'000, 24);
        require(prepared.error.isEmpty() && !prepared.previews.isEmpty(), prepared.error);
        quint64 previewPixels = 0;
        for (qsizetype index = 0; index < prepared.previews.size(); ++index)
        {
            const QImage &preview = prepared.previews.at(index);
            require(preview.width() <= 2048 && preview.height() <= 2048,
                u"Preview contact sheet exceeds its readable image bounds"_s);
            previewPixels += quint64(preview.width()) * preview.height();
            require(preview.save(outputDir + u"/preview-%1.png"_s.arg(index)), u"Preview PNG could not be saved"_s);
        }
        require(previewPixels <= 16'000'000, u"Preview batch exceeded its pixel budget"_s);
        require(!preparePage(original, regions, previewPixels - 1, 24).error.isEmpty(),
            u"Preview allocation ignored its remaining pixel budget"_s);
        require(!preparePage(original, regions, 16'000'000, 0).error.isEmpty(),
            u"Preview allocation ignored its image count budget"_s);
        int previewChecks = 0;
        const Preparation cancelledPreview = preparePage(original, regions, 16'000'000, 24,
            [&previewChecks, &regions] { return ++previewChecks > 2 * regions.size() + 2; });
        require(!cancelledPreview.error.isEmpty() && cancelledPreview.previews.isEmpty(),
            u"Cancelled preparation returned partial previews"_s);
        qInfo() << "PASS manga_preview sheets=" << prepared.previews.size() << "pixels=" << previewPixels;

        Region previewRegion = regions.at(1);
        previewRegion.textColor = QColor(0, 0, 0);
        previewRegion.outlineColor = QColor(255, 0, 0);
        previewRegion.outlineWidth = 4;
        const QRect previewCrop = pixels(previewRegion.bounds, original.size()).adjusted(-16, -16, 16, 16)
                                      .intersected(original.rect());
        QImage previewBackground(original.size(), QImage::Format_RGBA8888);
        previewBackground.fill(QColor(248, 248, 248));
        const Result previewReference = renderCleaned(previewBackground, previewBackground, { previewRegion });
        const Preparation exactPreview = preparePage(original, { previewRegion }, 16'000'000, 24);
        require(exactPreview.error.isEmpty() && exactPreview.previews.size() == 1, exactPreview.error);
        const QImage sheet = exactPreview.previews.constFirst();
        const QImage sourceTile = sheet.copy(8, 32, previewCrop.width(), previewCrop.height());
        const QImage coverageTile = sheet.copy(16 + previewCrop.width(), 32, previewCrop.width(), previewCrop.height());
        const QImage letteringTile = sheet.copy(24 + 2 * previewCrop.width(), 32, previewCrop.width(), previewCrop.height());
        require(sourceTile == original.copy(previewCrop), u"Preview changed its original crop"_s);
        require(letteringTile == previewReference.rendered.copy(previewCrop),
            u"Preview lettering differs from final native paths, bold face or outline"_s);
        const QRect cleanup = pixels(previewRegion.cleanupRects.constFirst(), original.size());
        int covered = 0;
        for (int y = 0; y < sourceTile.height(); ++y)
        {
            for (int x = 0; x < sourceTile.width(); ++x)
            {
                const bool changed = sourceTile.pixel(x, y) != coverageTile.pixel(x, y);
                require(changed == cleanup.contains(previewCrop.topLeft() + QPoint(x, y)),
                    u"Preview coverage does not match cleanup pixels"_s);
                covered += changed;
            }
        }
        require(covered == cleanup.width() * cleanup.height(), u"Preview missed cleanup coverage"_s);
        Region missedCoverage = previewRegion;
        missedCoverage.cleanupRects[0].setWidth(previewRegion.cleanupRects.constFirst().width() / 2);
        const Preparation missedPreview = preparePage(original, { missedCoverage }, 16'000'000, 24);
        require(missedPreview.error.isEmpty() && missedPreview.previews.constFirst() != sheet,
            u"Preview concealed missing source-glyph coverage"_s);
        require(missedPreview.previews.constFirst().save(outputDir + u"/preview-missed-coverage.png"_s)
                && sheet.save(outputDir + u"/preview-corrected-coverage.png"_s),
            u"Coverage comparison could not be saved"_s);
        Region weakLettering = previewRegion;
        for (TextRun &run : weakLettering.translation)
            run.bold = false;
        require(preparePage(original, { weakLettering }, 16'000'000, 24).previews.constFirst() != sheet,
            u"Preview concealed lost bold emphasis"_s);
        Region rotatedPreviewRegion = regions.at(13);
        rotatedPreviewRegion.textColor = QColor(49, 68, 85);
        rotatedPreviewRegion.outlineWidth = 4;
        const QRect rotatedCrop = pixels(rotatedPreviewRegion.bounds, original.size()).adjusted(-16, -16, 16, 16)
                                      .intersected(original.rect());
        const Preparation rotatedPreview = preparePage(original, { rotatedPreviewRegion }, 16'000'000, 24);
        require(rotatedPreview.error.isEmpty() && rotatedPreview.previews.size() == 1, rotatedPreview.error);
        const QImage rotatedSheet = rotatedPreview.previews.constFirst();
        require(rotatedSheet.copy(24 + 2 * rotatedCrop.width(), 32, rotatedCrop.width(), rotatedCrop.height())
                == renderCleaned(previewBackground, previewBackground, { rotatedPreviewRegion }).rendered.copy(rotatedCrop),
            u"Preview rotation, color, outline or clipping differs from final painting"_s);
        require(rotatedSheet.save(outputDir + u"/preview-rotated-color-outline.png"_s),
            u"Rotated preview could not be saved"_s);
        Region unreadablePreview = previewRegion;
        unreadablePreview.bounds = QRectF(0, 0, 1, 1);
        unreadablePreview.layout = normalized(QRectF(20, 20, 100, 20), original.size());
        unreadablePreview.translation = { { u"Hello"_s } };
        unreadablePreview.outlineWidth = 0;
        unreadablePreview.outlineColor = QColor();
        require(prepareTypography(original.size(), { unreadablePreview }).diagnostics.constFirst().status == Status::Rendered
                && !preparePage(original, { unreadablePreview }, 16'000'000, 24).error.isEmpty(),
            u"Unreadable full-page preview was silently attached"_s);
        qInfo() << "PASS manga_preview_pixels original=exact coverage=exact lettering=final bold=distinct "
                   "rotation_color_outline_clip=exact unreadable=rejected";

        const Result result = renderCleaned(original, generated, regions);
        require(result.error.isEmpty() && result.diagnostics.size() == regions.size(), result.error);
        for (const Diagnostic &diagnostic : result.diagnostics)
        {
            require(diagnostic.status == Status::Rendered && diagnostic.fontPixels >= 9 && diagnostic.cleanupPixels > 0,
                u"Region did not render: "_s + diagnostic.regionId);
            qInfo().noquote() << u"REGION %1 pixels=%2 lines=%3 cleanup=%4"_s.arg(diagnostic.regionId)
                                     .arg(diagnostic.fontPixels)
                                     .arg(diagnostic.layoutLines)
                                     .arg(diagnostic.cleanupPixels);
        }
        verifyUnchangedOutside(original, result, regions);
        require(renderCleaned(original, generated, regions).rendered == result.rendered,
            u"Repeated native render changed pixels"_s);

        QList<Region> coloredRegions = regions;
        coloredRegions[0].textColor = QColor(49, 68, 85);
        coloredRegions[0].outlineColor = QColor(230, 186, 67);
        const Result colored = renderCleaned(original, generated, coloredRegions);
        require(colored.error.isEmpty() && colored.diagnostics.constFirst().status == Status::Rendered
                && colored.cleaned == result.cleaned,
            u"Lettering colors changed cleanup or prevented rendering"_s);
        int fillPixels = 0;
        int outlinePixels = 0;
        for (int y = 0; y < colored.rendered.height(); ++y)
        {
            for (int x = 0; x < colored.rendered.width(); ++x)
            {
                const QRgb pixel = colored.rendered.pixel(x, y);
                fillPixels += pixel == coloredRegions[0].textColor.rgb();
                outlinePixels += (qRed(pixel) > qGreen(pixel)) && (qGreen(pixel) > qBlue(pixel));
            }
        }
        require(fillPixels > 100 && outlinePixels > 10, u"Rendered pixels lost the explicit text or outline color"_s);
        qInfo() << "COLOR fill_pixels=" << fillPixels << "antialiased_outline_pixels=" << outlinePixels;
        verifyUnchangedOutside(original, colored, coloredRegions);
        require(colored.rendered.save(outputDir + u"/colored.png"_s), u"Colored lettering render could not be saved"_s);

        const Region &mixed = regions.at(12);
        Region plain = mixed;
        for (TextRun &run : plain.translation)
        {
            run.bold = false;
            run.italic = false;
        }
        require(renderCleaned(original, generated, { mixed }).rendered
                != renderCleaned(original, generated, { plain }).rendered,
            u"Mixed emphasis was lost during shaping"_s);

        Region narrow = regions.constFirst();
        narrow.layout = normalized(QRectF(165, 70, 64, 113), original.size());
        narrow.translation = { { u"Нельзя доверять сборщикам трупов."_s } };
        const Result unbroken = renderCleaned(original, generated, { narrow });
        narrow.translation = { { u"Нельзя до\u00ADверять сбор\u00ADщикам трупов."_s } };
        const Result hyphenated = renderCleaned(original, generated, { narrow });
        require(unbroken.error.isEmpty() && hyphenated.error.isEmpty()
                && unbroken.diagnostics.constFirst().status == Status::Rendered
                && hyphenated.diagnostics.constFirst().status == Status::Rendered
                && hyphenated.diagnostics.constFirst().fontPixels > unbroken.diagnostics.constFirst().fontPixels
                && hyphenated.cleaned == unbroken.cleaned && hyphenated.rendered != unbroken.rendered,
            u"Discretionary hyphens did not improve narrow lettering without changing cleanup"_s);

        Region level = regions.at(13);
        level.angle = 0;
        require(renderCleaned(original, generated, { regions.at(13) }).rendered
                != renderCleaned(original, generated, { level }).rendered,
            u"Lettering rotation was lost"_s);

        Region unsupported = regions.constFirst();
        const char32_t missingCharacter[] { 0x10FFFF };
        unsupported.translation = { { QString::fromUcs4(missingCharacter, 1) } };
        const Result missing = renderCleaned(original, generated, { unsupported });
        require(missing.diagnostics.constFirst().status == Status::MissingGlyph && missing.rendered == original,
            u"Missing target glyph erased the original lettering"_s);

        Region invalid = regions.constFirst();
        invalid.angle = 46;
        const Result rejected = renderCleaned(original, generated, { invalid });
        require(rejected.diagnostics.constFirst().status == Status::InvalidGeometry && rejected.rendered == original,
            u"Unbounded lettering rotation was accepted"_s);

        for (const qreal width : { -1.0, 17.0, std::numeric_limits<qreal>::quiet_NaN() })
        {
            Region badOutline = regions.constFirst();
            badOutline.outlineWidth = width;
            const Typography rejectedOutline = prepareTypography(original.size(), { badOutline });
            require(rejectedOutline.diagnostics.constFirst().status == Status::InvalidGeometry,
                u"Invalid outline width was accepted"_s);
        }
        Region unoutlined = regions.constFirst();
        unoutlined.outlineWidth = 0;
        unoutlined.outlineColor = QColor();
        const Result noOutline = renderCleaned(original, generated, { unoutlined });
        require(noOutline.diagnostics.constFirst().status == Status::Rendered,
            u"Explicit no-outline lettering was rejected"_s);
        Region thickOutline = coloredRegions.constFirst();
        thickOutline.outlineWidth = 4;
        const Result thick = renderCleaned(original, generated, { thickOutline });
        require(thick.diagnostics.constFirst().status == Status::Rendered
                && thick.rendered != renderCleaned(original, generated, { coloredRegions.constFirst() }).rendered,
            u"Visible SFX outline width was lost"_s);
        Region mismatchedOutline = regions.constFirst();
        mismatchedOutline.outlineWidth = 0;
        require(prepareTypography(original.size(), { mismatchedOutline }).diagnostics.constFirst().status
                == Status::InvalidGeometry,
            u"Outline color without width was accepted"_s);
        const Result badGeometry = renderCleaned(original, QImage(100, 100, QImage::Format_RGBA8888), regions);
        require(
            !badGeometry.error.isEmpty() && badGeometry.rendered.isNull(), u"Changed cleanup aspect was accepted"_s);
        int checks = 0;
        const Result cancelledComposition
            = renderCleaned(original, generated, regions, [&checks] { return ++checks >= 22; });
        require(!cancelledComposition.error.isEmpty() && cancelledComposition.rendered.isNull(),
            u"Cancellation during cleanup/lettering returned a result"_s);

        const Result cancelled = renderCleaned(original, generated, regions, [] { return true; });
        require(!cancelled.error.isEmpty() && cancelled.rendered.isNull(), u"Cancelled render completed"_s);
        require(original.save(outputDir + u"/original.png"_s) && result.rendered.save(outputDir + u"/result.png"_s)
                && result.cleaned.save(outputDir + u"/cleaned.png"_s)
                && generated.save(outputDir + u"/generated-cleanup.png"_s),
            u"Native render artifacts could not be saved"_s);
        QFile proposalFile(outputDir + u"/proposal.json"_s);
        require(proposalFile.open(QIODevice::WriteOnly), u"Fixture proposal could not be opened"_s);
        proposalFile.write(QJsonDocument(
            QJsonObject { { "regions", proposalRegions }, { "terms", QJsonArray() }, { "facts", QJsonArray() } })
                .toJson());
        qInfo() << "PASS manga_lettering families=3 physical_faces=12 regions=15 mixed=ok rotation=ok soft_hyphens=ok "
                   "colors=ok cleanup_copy=bounded outline=variable outside=unchanged";
    }
}

int main(int argc, char **argv)
{
    QGuiApplication application(argc, argv);
    try
    {
        const QStringList arguments = application.arguments();
        if (arguments.size() >= 5 && arguments.at(1) == u"--normalize"_s)
        {
            QSize originalSize { 0, 0 };
            for (int index = 2; index < arguments.size() - 2; ++index)
            {
                const QImage original(arguments.at(index));
                require(!original.isNull(), u"Original image could not be decoded"_s);
                require(originalSize.isEmpty() || originalSize.width() == original.width(),
                    u"Vertical source segments must have the same width"_s);
                originalSize = { original.width(), originalSize.height() + original.height() };
            }
            const QImage generated(arguments.at(arguments.size() - 2));
            const PageImage normalized = normalizeGeneratedPage(originalSize, generated);
            require(normalized.error.isEmpty(), normalized.error);
            require(normalized.image.save(arguments.constLast()), u"Normalized page could not be saved"_s);
            qInfo() << "PASS generated_page_normalized" << generated.size() << "to" << normalized.image.size();
            return 0;
        }
        require(application.arguments().size() == 2, u"Pass an isolated output directory"_s);
        const QString outputDir = application.arguments().at(1);
        require(QDir().mkpath(outputDir), u"Output directory could not be created"_s);
        exercise(outputDir);
        exerciseLongStrip(outputDir + u"/longstrip"_s);
        return 0;
    }
    catch (const std::exception &error)
    {
        qCritical() << error.what();
        return 1;
    }
}
