// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-manga-translation.h"

#include <functional>
#include <stdexcept>

#include <QColor>
#include <QCryptographicHash>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QFont>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLinearGradient>
#include <QMetaObject>
#include <QPainter>
#include <QPolygon>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSize>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTimer>
#include <QUrl>

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &condition, const QString &operation, const int timeoutMs = 20'000)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(25);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (condition() || elapsed.elapsed() >= timeoutMs)
                loop.quit();
        });
        if (!condition())
        {
            timer.start();
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    QQuickItem *readerItem(Content::ContentView *view)
    {
        auto *loader = view->rootObject()->findChild<QObject *>(u"contentReaderLoader"_s);
        return loader ? loader->property("item").value<QQuickItem *>() : nullptr;
    }

    QQuickItem *control(QQuickItem *reader, const QString &name)
    {
        auto *item = reader ? reader->findChild<QQuickItem *>(name) : nullptr;
        require(item, u"Reader control is missing: "_s + name);
        return item;
    }

    void activate(QQuickItem *choice, const int index)
    {
        require(choice->setProperty("currentIndex", index)
            && QMetaObject::invokeMethod(choice, "activated", Q_ARG(int, index)),
            u"Reader selection could not be activated"_s);
    }

    void drawVertical(QPainter &painter, const QString &text, const int x, const int y)
    {
        for (int index = 0; index < text.size(); ++index)
            painter.drawText(x, y + index * 39, text.mid(index, 1));
    }

    QImage originalPage()
    {
        QImage image(900, 1300, QImage::Format_RGB32);
        QPainter painter(&image);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.fillRect(image.rect(), QColor(247, 246, 239));

        QLinearGradient sky(0, 0, 0, 720);
        sky.setColorAt(0, QColor(164, 190, 203));
        sky.setColorAt(1, QColor(227, 228, 213));
        painter.fillRect(24, 24, 852, 666, sky);
        painter.setPen(QPen(QColor(41, 61, 67), 5));
        painter.drawRect(24, 24, 852, 666);
        painter.setBrush(QColor(94, 117, 111));
        painter.drawPolygon(QPolygon({QPoint(26, 690), QPoint(240, 390), QPoint(520, 690)}));
        painter.setBrush(QColor(72, 93, 96));
        painter.drawPolygon(QPolygon({QPoint(320, 690), QPoint(650, 330), QPoint(876, 690)}));
        painter.setPen(QPen(QColor(203, 218, 215, 120), 2));
        for (int x = 20; x < 910; x += 31)
            painter.drawLine(x, 45, x - 155, 690);

        painter.setPen(QPen(QColor(41, 61, 67), 5));
        painter.setBrush(QColor(238, 233, 215));
        painter.drawRect(24, 718, 852, 558);
        painter.setBrush(QColor(126, 145, 136));
        painter.drawPolygon(QPolygon({QPoint(70, 1070), QPoint(375, 785), QPoint(600, 1070)}));
        painter.setBrush(QColor(213, 205, 181));
        painter.drawRect(115, 1060, 420, 180);
        painter.setBrush(QColor(57, 72, 70));
        painter.drawEllipse(650, 958, 75, 108);
        painter.drawRect(667, 1043, 45, 145);
        painter.drawEllipse(755, 995, 65, 100);
        painter.drawRect(766, 1080, 43, 123);

        painter.setPen(QPen(QColor(41, 61, 67), 4));
        painter.setBrush(Qt::white);
        painter.drawEllipse(QRect(575, 60, 250, 365));
        painter.drawEllipse(QRect(76, 91, 250, 335));
        painter.drawEllipse(QRect(328, 777, 255, 360));
        painter.setFont(QFont(u"Yu Gothic"_s, 29, QFont::DemiBold));
        drawVertical(painter, u"レン、空を見て。"_s, 704, 130);
        drawVertical(painter, u"雨が来るね。"_s, 198, 160);
        drawVertical(painter, u"大丈夫、帰ろう。"_s, 455, 847);
        painter.end();
        return image;
    }

    QImage pageImage(Qbutt::ContentController *controller)
    {
        const QUrl url(controller->property("readerPageUrl").toString());
        require(url.isLocalFile(), u"Reader page is not a local image"_s);
        const QImage image(url.toLocalFile());
        require(!image.isNull(), u"Reader page could not be decoded"_s);
        return image.convertToFormat(QImage::Format_ARGB32);
    }

    void capture(Content::ContentView *view, const QString &path, const QSize &size)
    {
        view->resize(size);
        QEventLoop frame;
        QTimer::singleShot(150, &frame, &QEventLoop::quit);
        frame.exec();
        const QImage image = view->grabFramebuffer();
        require(image.size() == size && image.save(path), u"Reader screenshot failed: "_s + path);
    }
}

void exerciseReaderMangaTranslation(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Manga translation controller is missing"_s);
    waitFor([&]
    {
        return controller->property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready;
    }, u"Manga translation library"_s);

    const QImage generated = originalPage();
    const QString originalPath = profileDir + u"/manga-original.png"_s;
    require(generated.save(originalPath), u"Generated manga page could not be saved"_s);
    controller->importLocalBook(QUrl::fromLocalFile(originalPath));
    waitFor([&]
    {
        return controller->property("readerOpen").toBool()
            && !controller->property("readerBusy").toBool()
            && !controller->property("readerPageUrl").toString().isEmpty();
    }, u"Generated manga reader"_s);
    require(!controller->property("readerIsText").toBool()
        && controller->property("readerPageCount").toInt() == 1
        && controller->property("readerPageIndex").toInt() == 0
        && controller->property("readerErrorText").toString().isEmpty(),
        u"Generated manga opened at the wrong page"_s);
    const QImage original = pageImage(controller);
    require(original == generated.convertToFormat(QImage::Format_ARGB32),
        u"Original reader image differs from the generated page"_s);
    capture(view, profileDir + u"/manga-reader-original.png"_s, QSize(1280, 720));

    auto *reader = readerItem(view);
    auto *settings = control(reader, u"readerTranslationSettings"_s);
    auto *picker = reader->findChild<QObject *>(u"readerTranslationPicker"_s);
    require(picker, u"Manga translation settings picker is missing"_s);
    auto *modelChoice = control(reader, u"readerTranslationModel"_s);
    auto *effortChoice = control(reader, u"readerTranslationEffort"_s);
    auto *translation = control(reader, u"readerTranslation"_s);
    require(QMetaObject::invokeMethod(settings, "clicked"), u"Manga translation settings did not open"_s);
    waitFor([&]
    {
        return picker->property("visible").toBool()
            && !controller->property("readerTranslationOptionsLoading").toBool()
            && controller->property("readerTranslationModelCount").toInt() > 0;
    }, u"Installed Codex manga models"_s, 120'000);
    require(!controller->property("readerTranslationOptionsError").toBool(),
        u"Installed Codex models are unavailable"_s);
    int modelIndex = -1;
    for (int index = 0; index < controller->property("readerTranslationModelCount").toInt(); ++index)
    {
        if (controller->readerTranslationModelAt(index) == u"gpt-6.1-sol"_s)
            modelIndex = index;
    }
    require(modelIndex >= 0, u"gpt-6.1-sol is unavailable for image translation"_s);
    activate(modelChoice, modelIndex + 1);
    waitFor([&]
    {
        return controller->property("readerTranslationModel").toString() == u"gpt-6.1-sol"_s
            && !controller->property("readerTranslationOptionsLoading").toBool();
    }, u"Manga model selection"_s);
    int effortIndex = -1;
    for (int index = 0; index < controller->property("readerTranslationEffortCount").toInt(); ++index)
    {
        if (controller->readerTranslationEffortAt(index) == u"medium"_s)
            effortIndex = index;
    }
    require(effortIndex >= 0, u"Medium reasoning is unavailable"_s);
    activate(effortChoice, effortIndex);
    waitFor([&]
    {
        return controller->property("readerTranslationEffort").toString() == u"medium"_s;
    }, u"Manga reasoning selection"_s);
    require(QMetaObject::invokeMethod(control(reader, u"readerTranslationClose"_s), "clicked"),
        u"Manga translation settings did not close"_s);

    activate(translation, 1);
    waitFor([&] { return controller->property("readerTranslationBusy").toBool(); },
        u"Live manga translation start"_s);
    waitFor([&] { return !controller->property("readerTranslationBusy").toBool(); },
        u"Live manga translation"_s, 360'000);
    require(controller->property("readerTranslationRequested").toBool()
        && !controller->property("readerTranslationError").toBool()
        && controller->property("readerPageIndex").toInt() == 0,
        u"Live manga translation did not reach exact page zero"_s);
    const QImage translated = pageImage(controller);
    require(translated.size() == original.size(), u"Translated manga dimensions changed"_s);
    require(translated.save(profileDir + u"/manga-translated.png"_s),
        u"Translated manga page could not be saved"_s);
    capture(view, profileDir + u"/manga-reader-translated.png"_s, QSize(1440, 900));

    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"reader-manga-translation"_s);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = {};
        QSqlDatabase::removeDatabase(u"reader-manga-translation"_s);
    });
    database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
    require(database.open(), database.lastError().text());
    QSqlQuery cache(database);
    require(cache.exec(u"SELECT key, content FROM translated_content"_s) && cache.next(),
        u"Native manga translation was not cached"_s);
    const auto key = QJsonDocument::fromJson(cache.value(0).toByteArray()).object();
    const auto regions = QJsonDocument::fromJson(cache.value(1).toByteArray()).array();
    QFile originalFile(originalPath);
    require(originalFile.open(QIODevice::ReadOnly), u"Original manga file was lost"_s);
    const QString digest = QString::fromLatin1(
        QCryptographicHash::hash(originalFile.readAll(), QCryptographicHash::Sha256).toHex());
    const auto chapter = key.value(u"chapter"_s).toObject();
    const auto publication = chapter.value(u"variant"_s).toObject().value(u"publication"_s).toObject();
    require(!cache.next() && key.value(u"page"_s).toInt(-1) == 0
        && key.value(u"model"_s).toString() == u"gpt-6.1-sol"_s
        && key.value(u"effort"_s).toString() == u"medium"_s
        && key.value(u"target_language"_s).toString() == u"ru"_s
        && key.value(u"original_digest"_s).toString() == digest
        && chapter.value(u"id"_s).toString() == u"1"_s
        && publication.value(u"source"_s).toString() == u"local_book"_s
        && publication.value(u"id"_s).toString() == digest
        && regions.size() >= 3,
        u"Cached manga translation has the wrong page, selection or regions"_s);
    cache.finish();

    int changedInside = 0;
    for (int y = 0; y < original.height(); ++y)
    {
        for (int x = 0; x < original.width(); ++x)
        {
            bool inside = false;
            for (const auto &entry : regions)
            {
                const auto bounds = entry.toObject().value(u"bounds"_s).toObject();
                const double left = bounds.value(u"x"_s).toDouble() * original.width();
                const double top = bounds.value(u"y"_s).toDouble() * original.height();
                const double right = left + bounds.value(u"width"_s).toDouble() * original.width();
                const double bottom = top + bounds.value(u"height"_s).toDouble() * original.height();
                inside |= x >= left - 1 && x <= right + 1 && y >= top - 1 && y <= bottom + 1;
            }
            if (original.pixel(x, y) == translated.pixel(x, y))
                continue;
            require(inside, u"Native renderer changed artwork outside accepted regions"_s);
            ++changedInside;
        }
    }
    require(changedInside > 1000, u"Translated page did not alter the source lettering"_s);

    activate(translation, 0);
    require(!controller->property("readerTranslationRequested").toBool()
        && pageImage(controller) == original,
        u"Original choice did not restore exact original pixels"_s);
    controller->closeReader();
    const QByteArray originalProgramFiles = qgetenv("ProgramFiles");
    const QByteArray originalLocalAppData = qgetenv("LOCALAPPDATA");
    const auto restoreEnvironment = qScopeGuard([&]
    {
        qputenv("ProgramFiles", originalProgramFiles);
        qputenv("LOCALAPPDATA", originalLocalAppData);
    });
    qputenv("ProgramFiles", profileDir.toUtf8());
    qputenv("LOCALAPPDATA", profileDir.toUtf8());
    controller->openReader();
    waitFor([&]
    {
        return controller->property("readerOpen").toBool()
            && !controller->property("readerBusy").toBool()
            && !controller->property("readerPageUrl").toString().isEmpty();
    }, u"Original manga reopen"_s);
    require(pageImage(controller) == original, u"Reopened manga did not start with Original"_s);
    reader = readerItem(view);
    translation = control(reader, u"readerTranslation"_s);
    activate(translation, 1);
    waitFor([&]
    {
        return !controller->property("readerTranslationBusy").toBool()
            && controller->property("readerTranslationRequested").toBool();
    }, u"Offline cached manga translation"_s, 120'000);
    require(!controller->property("readerTranslationError").toBool()
        && pageImage(controller) == translated,
        u"Reopened manga did not reuse the exact translation offline"_s);
    qInfo().noquote() << u"PASS reader_manga_translation model=gpt-6.1-sol effort=medium page=0 regions=%1 changed_pixels=%2 original=ok offline_cache=ok"_s
        .arg(regions.size()).arg(changedInside);
}
