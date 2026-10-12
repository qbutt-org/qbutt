// SPDX-License-Identifier: GPL-2.0-or-later
#include "mangalib.h"

#include <functional>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QColor>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    const QString publication = u"mangalib:5432--yotsubaamp"_s;
    const QString exact = publication + u":branch:null"_s;

    void require(const bool accepted, const QString &message)
    {
        if (!accepted)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 80'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(QObject *controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller->property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Missing GUI-thread MangaLib model"_s);
        return result;
    }

    QVariant value(QAbstractItemModel *model, const int row, const QByteArray &role)
    {
        const int key = model->roleNames().key(role, -1);
        require(key >= 0, u"Missing MangaLib model role"_s);
        return model->data(model->index(row, 0), key);
    }

    QQuickItem *visualItem(QQuickItem *parent, const QString &name)
    {
        if (!parent || (parent->objectName() == name))
            return parent;
        for (auto *child : parent->childItems())
        {
            if (auto *item = visualItem(child, name))
                return item;
        }
        return nullptr;
    }

    void saved(Qbutt::ContentController *controller)
    {
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"MangaLib FIFO drain"_s);
    }

    void exec(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    }

    QJsonObject entry(QSqlDatabase &database)
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT entry FROM library WHERE json_extract(entry, '$.work.reference.id') = '5432--yotsubaamp'"_s)
            && query.next(), u"MangaLib full work is missing"_s);
        return QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
    }

    QUrl decoded(Qbutt::ContentController *controller)
    {
        waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"MangaLib decoded page"_s);
        require(controller->property("readerOpen").toBool() && controller->property("readerErrorText").toString().isEmpty(),
            u"MangaLib reader failed: "_s + controller->property("readerErrorText").toString());
        const QUrl url = controller->property("readerPageUrl").toUrl();
        require(url.isLocalFile(), u"MangaLib reader exposed a remote image"_s);
        QImageReader image(url.toLocalFile());
        require(image.canRead() && !image.read().isNull(), u"MangaLib native page did not decode"_s);
        return url;
    }

    void writeFailure(QSqlDatabase &database, const QJsonObject &operation)
    {
        const QJsonObject failure {{u"operation"_s, operation}, {u"reason"_s, QJsonObject {{u"http"_s, 404}}}};
        QSqlQuery query(database);
        query.prepare(u"UPDATE source_settings SET failure = ?, revision = revision + 1 WHERE source = 'mangalib'"_s);
        query.addBindValue(QString::fromUtf8(QJsonDocument(failure).toJson(QJsonDocument::Compact)));
        require(query.exec(), query.lastError().text());
    }
}

void exerciseMangaLib()
{
    require(QGuiApplication::platformName() == u"offscreen"_s, u"MangaLib fixture requires offscreen Qt"_s);
    QTemporaryDir profile;
    require(profile.isValid(), u"Owned MangaLib profile failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("MangaLib owners did not drain");
    });
    std::unique_ptr<Content::ContentView> view;
    Qbutt::ContentController *controller = nullptr;
    const auto reopen = [&]
    {
        view = std::make_unique<Content::ContentView>(profile.path());
        view->resize(1440, 900);
        view->show();
        controller = view->findChild<Qbutt::ContentController *>();
        require(controller, u"MangaLib production controller missing"_s);
        waitFor([&] { return controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>() == Qbutt::Sources::SettingsState::Ready; }, u"MangaLib source preferences"_s);
        require(controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready, u"MangaLib Library failed"_s);
    };
    const auto close = [&]
    {
        view.reset();
        controller = nullptr;
        require(QThreadPool::globalInstance()->waitForDone(100'000), u"MangaLib native owner did not drain"_s);
    };
    reopen();
    auto *sources = model(controller, "sources");
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString id = value(sources, row, "sourceId").toString();
        require(controller->setSourceEnabled(id, id == u"mangalib"_s), u"MangaLib isolated source preference rejected"_s);
    }
    saved(controller);
    controller->search(u"Yotsuba"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"MangaLib live search"_s);
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"MangaLib live inspect"_s);
    controller->selectVariant(exact);
    auto *variants = model(controller, "variants");
    require(variants->rowCount() > 0, u"MangaLib translation branch is missing"_s);
    const QString translationLabel = value(variants, 0, "label").toString();
    require(controller->property("selectedVariantId").toString() == exact, u"MangaLib exact null branch missing"_s);
    controller->setSelectedFavorite(true);
    saved(controller);
    require(model(controller, "history")->rowCount() == 0, u"MangaLib selection created History"_s);
    auto *primary = view->rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
    require(primary && primary->isEnabled() && (primary->property("text").toString() == QCoreApplication::translate("WorkPane", "Read")), u"MangaLib has no Read action"_s);
    require(QMetaObject::invokeMethod(primary, "clicked"), u"MangaLib production Read action failed"_s);
    decoded(controller);
    waitFor([&] { return model(controller, "history")->rowCount() == 1; }, u"MangaLib shown-page History"_s);
    require(controller->setReaderFit(Qbutt::Reader::ReaderFit::Width), u"MangaLib Width fit rejected"_s);
    require(model(controller, "readerChapters")->rowCount() > 100, u"MangaLib exact chapter list missing"_s);
    controller->selectReaderPage(1);
    const QUrl second = decoded(controller);
    saved(controller);

    const auto unregister = qScopeGuard([] { QSqlDatabase::removeDatabase(u"mangalib-faults"_s); });
    QSqlDatabase database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"mangalib-faults"_s);
    database.setDatabaseName(QDir(profile.path()).filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    waitFor([&] { return entry(database).value(u"reader_progress"_s).toObject().value(u"page"_s).toInt() == 1; }, u"MangaLib exact progress"_s);
    controller->scrollReaderPage(second, 0.625);
    saved(controller);
    const auto captured = entry(database);
    const auto storedVariant = captured.value(u"work"_s).toObject().value(u"variants"_s).toArray().first().toObject();
    require(storedVariant.value(u"languages"_s).toArray().isEmpty()
        && (storedVariant.value(u"translation"_s).toString() == translationLabel),
        u"MangaLib invented a translation language: "_s + translationLabel);
    const auto progress = captured.value(u"reader_progress"_s).toObject();
    const auto chapter = progress.value(u"chapter"_s).toObject();
    require((captured.value(u"work"_s).toObject().value(u"variants"_s).toArray().size() > 0)
        && (captured.value(u"preferences"_s).toObject().value(u"selected"_s) == chapter.value(u"variant"_s)), u"MangaLib lost full work or exact preference"_s);
    controller->selectReaderPage(40);
    require(controller->setSourceEnabled(u"mangalib"_s, false), u"MangaLib disable rejected"_s);
    saved(controller);
    require(!controller->property("readerOpen").toBool() && controller->property("readerPageUrl").toUrl().isEmpty(), u"Disabled MangaLib continued reading"_s);
    require(entry(database).value(u"reader_progress"_s) == progress, u"Disabled acquisition overwrote confirmed progress"_s);
    close();
    reopen();
    sources = model(controller, "sources");
    int sourceRow = -1;
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        if (value(sources, row, "sourceId").toString() == u"mangalib"_s)
            sourceRow = row;
        require(!value(sources, row, "enabled").toBool(), u"Source flags did not survive restart"_s);
    }
    require(sourceRow >= 0 && controller->setSourceEnabled(u"mangalib"_s, true), u"MangaLib re-enable failed"_s);
    saved(controller);
    controller->setListMode(Qbutt::Search::ListMode::History);
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"MangaLib history reinspection"_s);
    require(controller->property("selectedVariantId").toString() == exact, u"History silently changed MangaLib translation"_s);
    controller->openReader();
    decoded(controller);
    require((controller->property("readerChapterId").toString() == u"161943"_s)
        && (controller->property("readerPageIndex").toInt() == 1)
        && (controller->property("readerScrollOffset").toDouble() == 0.625), u"MangaLib restart changed exact position"_s);
    const QString imageDirectory = qEnvironmentVariable("QBUTT_MANGALIB_QA_IMAGES");
    if (!imageDirectory.isEmpty())
    {
        require(QDir().mkpath(imageDirectory), u"MangaLib render directory failed"_s);
        waitFor([&]
        {
            auto *width = view->rootObject()->findChild<QQuickItem *>(u"readerWidthFit"_s);
            auto *caption = width ? width->property("contentItem").value<QQuickItem *>() : nullptr;
            return caption && width->isEnabled() && width->property("checked").toBool()
                && (caption->property("color").value<QColor>() == QColor(Qt::white));
        }, u"Settled MangaLib Width caption"_s);
        for (const QSize &size : {QSize(1440, 900), QSize(1280, 720)})
        {
            view->resize(size);
            QEventLoop frame;
            QTimer::singleShot(150, &frame, &QEventLoop::quit);
            frame.exec();
            require(view->grabFramebuffer().save(QDir(imageDirectory).filePath(u"mangalib-%1x%2.png"_s.arg(size.width()).arg(size.height()))), u"MangaLib render failed"_s);
        }
    }
    close();
    auto missingChapter = chapter;
    missingChapter.insert(u"id"_s, u"18446744073709551615"_s);
    writeFailure(database, QJsonObject {{u"chapter"_s, missingChapter}});
    reopen();
    sources = model(controller, "sources");
    require(value(sources, sourceRow, "failed").toBool() && controller->retrySource(u"mangalib"_s), u"Persisted exact MangaLib failure missing"_s);
    waitFor([&] { return !value(sources, sourceRow, "retrying").toBool(); }, u"Missing MangaLib upload retry"_s);
    saved(controller);
    require(value(sources, sourceRow, "failed").toBool() && !controller->property("readerOpen").toBool(), u"Unavailable upload substituted a reader"_s);
    close();
    writeFailure(database, QJsonObject {{u"reader_page"_s, QJsonObject {{u"reference"_s, chapter}, {u"page"_s, 52}}}});
    reopen();
    sources = model(controller, "sources");
    auto *sourceButton = view->rootObject()->findChild<QQuickItem *>(u"contentSources"_s);
    require(sourceButton && QMetaObject::invokeMethod(sourceButton, "clicked"), u"MangaLib Sources action missing"_s);
    QQuickItem *sourceRetry = nullptr;
    waitFor([&]
    {
        auto *picker = view->rootObject()->findChild<QObject *>(u"contentSourcePicker"_s);
        sourceRetry = picker ? visualItem(picker->property("contentItem").value<QQuickItem *>(), u"contentSourceRetry_mangalib"_s) : nullptr;
        return sourceRetry && sourceRetry->isVisible();
    }, u"MangaLib source recovery picker"_s);
    require(sourceRetry->isEnabled() && QMetaObject::invokeMethod(sourceRetry, "clicked"), u"Exact MangaLib page retry is not actionable"_s);
    waitFor([&] { return !value(sources, sourceRow, "retrying").toBool() && !value(sources, sourceRow, "failed").toBool(); }, u"Exact MangaLib JPEG retry"_s);
    saved(controller);
    require(!controller->property("readerOpen").toBool() && (entry(database).value(u"reader_progress"_s) == progress), u"Sources retry changed reading position or launched a reader"_s);
    close();
    qInfo("PASS MangaLib native_search=ok exact_translation=ok decoded_QML_read=ok full_work_history=ok favorites=ok progress_restart=ok disable_inflight=ok flags_restart=ok exact_missing_upload_retry=retained exact_JPEG_retry=cleared Sources_QML_action=ok");
}
