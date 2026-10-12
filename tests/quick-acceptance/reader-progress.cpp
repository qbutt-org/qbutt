// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-progress.h"

#include <functional>
#include <limits>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QList>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QTimer>
#include <QUrl>

#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    const QString publication = u"mangadex:b0b721ff-c388-4486-aa0f-c2b0bb321512"_s;
    const QString exact = publication + u":translation:ru:9e383226-2af3-4af0-a69d-3d3b630212b3"_s;

    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &condition, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!condition() && (elapsed.elapsed() < 100'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller.property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Missing GUI-thread model"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const int role = model->roleNames().key(name, -1);
        require(role >= 0, u"Missing model role"_s);
        return model->data(model->index(row, 0), role).toString();
    }

    void readyLibrary(Qbutt::ContentController &controller, const QString &profile)
    {
        controller.initLibrary(profile);
        waitFor([&]
        {
            return controller.property("libraryState").value<Qbutt::Library::LibraryState>()
                != Qbutt::Library::LibraryState::Uninitialized
                && controller.property("libraryState").value<Qbutt::Library::LibraryState>()
                != Qbutt::Library::LibraryState::Loading;
        }, u"Library startup"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());
    }

    void saved(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, u"Library completion"_s);
        require(controller.property("libraryErrorText").toString().isEmpty(), u"Library write failed"_s);
    }

    void selectSaved(Qbutt::ContentController &controller)
    {
        controller.setListMode(Qbutt::Search::ListMode::Favorites);
        controller.selectWork(publication);
        waitFor([&] { return !controller.property("inspecting").toBool(); }, u"Saved publication refresh"_s);
        require(controller.property("errorText").toString().isEmpty(), u"Saved inspection failed"_s);
        require(controller.property("selectedVariantId").toString() == exact,
            u"Restart substituted the exact translation"_s);
        saved(controller);
    }

    QUrl readyPage(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("readerBusy").toBool(); }, u"Reader page"_s);
        require(controller.property("readerOpen").toBool(), u"Reader unexpectedly closed"_s);
        require(controller.property("readerErrorText").toString().isEmpty(),
            controller.property("readerErrorText").toString());
        const QUrl url = controller.property("readerPageUrl").toUrl();
        require(url.isLocalFile(), u"Reader did not prepare a local page"_s);
        QImageReader image(url.toLocalFile());
        require(image.canRead() && !image.read().isNull(), u"Actual reader page did not decode"_s);
        return url;
    }

    void execute(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    }

    QJsonObject entry(QSqlDatabase &database, QString *reference = nullptr)
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT reference, entry FROM library"_s), query.lastError().text());
        require(query.next(), u"Saved reader work is missing"_s);
        const auto result = QJsonDocument::fromJson(query.value(1).toString().toUtf8()).object();
        if (reference)
            *reference = query.value(0).toString();
        require(!result.isEmpty() && !query.next(), u"Reader fixture must contain exactly one work"_s);
        return result;
    }

    void writeEntry(QSqlDatabase &database, const QJsonObject &entry)
    {
        QSqlQuery query(database);
        require(query.prepare(u"UPDATE library SET entry = ?"_s), query.lastError().text());
        query.addBindValue(QString::fromUtf8(QJsonDocument(entry).toJson(QJsonDocument::Compact)));
        require(query.exec() && (query.numRowsAffected() == 1), query.lastError().text());
    }

    void changeSavedPosition(QSqlDatabase &database, const QString &chapter, const int page)
    {
        auto metadata = entry(database);
        auto progress = metadata.value(u"reader_progress"_s).toObject();
        auto reference = progress.value(u"chapter"_s).toObject();
        reference.insert(u"id"_s, chapter);
        progress.insert(u"chapter"_s, reference);
        progress.insert(u"page"_s, page);
        metadata.insert(u"reader_progress"_s, progress);
        writeEntry(database, metadata);
    }

    void readerFailed(Qbutt::ContentController &controller, const char *message)
    {
        waitFor([&] { return !controller.property("readerBusy").toBool(); }, u"Reader failure"_s);
        require(controller.property("readerErrorText").toString()
            == QCoreApplication::translate("ContentController", message), u"Reader reported the wrong failure"_s);
        require(controller.property("readerPageUrl").toUrl().isEmpty(), u"Failure substituted a page"_s);
    }

    void exerciseInitialFailure(QJsonObject metadata, const QString &reference, const bool invalidMetadata)
    {
        QTemporaryDir profile;
        require(profile.isValid(), u"Could not create the initial failure profile"_s);
        const QString connectionName = u"reader-progress-initial-failure"_s;
        auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connectionName);
        const auto closeDatabase = qScopeGuard([&]
        {
            database.close();
            database = QSqlDatabase();
            QSqlDatabase::removeDatabase(connectionName);
        });
        database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
        require(database.open(), database.lastError().text());
        execute(database, u"CREATE TABLE library (reference TEXT PRIMARY KEY NOT NULL, "
            "entry TEXT NOT NULL, updated_at INTEGER NOT NULL CHECK (updated_at >= 0)) STRICT"_s);
        execute(database, u"PRAGMA application_id = 1363299394"_s);
        execute(database, u"CREATE TABLE reader_settings (id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1), "
            "fit INTEGER NOT NULL CHECK (fit IN (0, 1))) STRICT"_s);
        execute(database, u"INSERT INTO reader_settings VALUES (1, 0)"_s);
        execute(database, u"PRAGMA user_version = 3"_s);
        metadata.insert(u"last_opened"_s, QJsonValue::Null);
        metadata.insert(u"reader_progress"_s, QJsonValue::Null);
        QSqlQuery insert(database);
        require(insert.prepare(u"INSERT INTO library VALUES (?, ?, 0)"_s), insert.lastError().text());
        insert.addBindValue(reference);
        auto initial = metadata;
        if (invalidMetadata)
            initial.insert(u"reader_progress"_s, QJsonObject {{u"page"_s, -1}});
        insert.addBindValue(QString::fromUtf8(QJsonDocument(initial).toJson(QJsonDocument::Compact)));
        require(insert.exec(), insert.lastError().text());

        Qbutt::ContentController controller;
        controller.search(u"Frieren"_s);
        waitFor([&] { return !controller.property("busy").toBool(); }, u"Initial failure search"_s);
        controller.selectWork(publication);
        waitFor([&] { return !controller.property("inspecting").toBool(); }, u"Initial failure inspection"_s);
        controller.selectVariant(exact);
        require(controller.property("selectedVariantId").toString() == exact, u"Initial failure variant is missing"_s);
        if (!invalidMetadata)
            execute(database, u"BEGIN EXCLUSIVE"_s);
        bool acceptedWhileLoading = false;
        QObject observations;
        QObject::connect(&controller, &Qbutt::ContentController::libraryStateChanged, &observations, [&]
        {
            if (controller.property("libraryState").value<Qbutt::Library::LibraryState>()
                != Qbutt::Library::LibraryState::Loading)
            {
                return;
            }
            if (!acceptedWhileLoading)
            {
                controller.openReader();
                acceptedWhileLoading = controller.property("libraryPending").toBool();
            }
        }, Qt::DirectConnection);
        controller.initLibrary(profile.path());
        waitFor([&]
        {
            return controller.property("libraryState").value<Qbutt::Library::LibraryState>()
                == Qbutt::Library::LibraryState::Failed;
        }, u"Initial library failure"_s);
        require(acceptedWhileLoading && !controller.property("libraryPending").toBool(),
            u"Initial failure did not drain an accepted resume read"_s);
        readerFailed(controller, "Could not open the content library.");
        require(!controller.property("readerHasChapters").toBool()
            && entry(database).value(u"last_opened"_s).isNull(), u"Initial failure invented chapters or history"_s);
        if (invalidMetadata)
            writeEntry(database, metadata);
        else
            execute(database, u"ROLLBACK"_s);
        if (invalidMetadata)
            require(controller.retryLibrary(), u"Explicit library retry was rejected"_s);
        else
            controller.retryReader();
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            != Qbutt::Library::LibraryState::Failed, u"Temporary startup failure could not retry the same owner"_s);
        const QUrl recovered = readyPage(controller);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready && controller.property("libraryErrorText").toString().isEmpty(),
            u"Retry did not reopen the same profile successfully"_s);
        require(entry(database).value(u"last_opened"_s).isNull(), u"Startup retry invented reading history"_s);
        controller.readerPageShown(recovered);
        controller.scrollReaderPage(recovered, 0.375);
        saved(controller);
        require(entry(database).value(u"reader_progress"_s).toObject().value(u"scroll_offset"_s).toDouble() == 0.375,
            u"Recovered owner did not accept exact FIFO progress writes"_s);
        controller.closeReader();
    }
}

void exerciseReaderProgress()
{
    QTemporaryDir profile;
    require(profile.isValid(), u"Could not create an isolated reader profile"_s);
    auto controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    controller->search(u"Frieren"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Live reader search"_s);
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Live reader inspection"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Live reader inspection failed"_s);
    controller->selectVariant(exact);
    require(controller->property("selectedVariantId").toString() == exact, u"Exact reader branch is missing"_s);
    controller->setSelectedFavorite(true);
    saved(*controller);

    const QString connectionName = u"reader-progress-fixture"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connectionName);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = QSqlDatabase();
        QSqlDatabase::removeDatabase(connectionName);
    });
    database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    require(entry(database).value(u"last_opened"_s).isNull()
        && entry(database).value(u"reader_progress"_s).isNull(), u"Selection created reading history"_s);

    struct Restore
    {
        QUrl url;
        double offset;
    };
    QList<Restore> restores;
    QObject observations;
    const auto observe = [&]
    {
        QObject::connect(controller.get(), &Qbutt::ContentController::restoreReaderScroll, &observations,
            [&](const QUrl &url, const double offset) { restores.append({url, offset}); }, Qt::DirectConnection);
    };
    observe();
    controller->openReader();
    const QUrl firstPage = readyPage(*controller);
    const QString firstChapter = controller->property("readerChapterId").toString();
    require(controller->property("readerPageCount").toInt() > 1, u"Live fixture needs two pages"_s);
    require(restores.isEmpty() && entry(database).value(u"last_opened"_s).isNull(),
        u"Acquisition or decode alone created reading history"_s);
    controller->readerPageShown(QUrl(u"file:///generated-stale-page"_s));
    controller->scrollReaderPage(firstPage, 0.8);
    require(restores.isEmpty() && !controller->property("libraryPending").toBool(),
        u"Unshown or stale page accepted progress"_s);
    controller->readerPageShown(firstPage);
    saved(*controller);
    require((restores.size() == 1) && (restores.last().url == firstPage) && (restores.last().offset == 0),
        u"Initial render did not restore once"_s);
    const QJsonObject firstShown = entry(database);
    const QJsonObject knownWork = firstShown.value(u"work"_s).toObject();
    const QJsonObject opened = firstShown.value(u"last_opened"_s).toObject();
    require(!knownWork.value(u"title"_s).toString().isEmpty()
        && (knownWork.value(u"variants"_s).toArray().size() == model(*controller, "variants")->rowCount())
        && (opened.value(u"variant"_s) == firstShown.value(u"reader_progress"_s).toObject()
            .value(u"chapter"_s).toObject().value(u"variant"_s))
        && (opened.value(u"at_unix_millis"_s).toDouble() > 0),
        u"Confirmed render did not retain the full work and exact history"_s);
    controller->readerPageShown(firstPage);
    require((restores.size() == 1) && !controller->property("libraryPending").toBool()
        && (entry(database) == firstShown), u"Duplicate render rewrote history"_s);
    for (const double offset : {std::numeric_limits<double>::quiet_NaN(),
             std::numeric_limits<double>::infinity(), -0.1, 1.1})
    {
        controller->scrollReaderPage(firstPage, offset);
    }
    require(!controller->property("libraryPending").toBool() && (entry(database) == firstShown),
        u"Invalid scroll changed persisted progress"_s);

    controller->turnReaderPage(1);
    const QUrl secondPage = readyPage(*controller);
    controller->readerPageShown(secondPage);
    controller->scrollReaderPage(secondPage, 0.625);
    saved(*controller);
    const QJsonObject secondShown = entry(database);
    const QJsonObject progress = secondShown.value(u"reader_progress"_s).toObject();
    require((progress.value(u"page"_s).toInt() == 1)
        && (progress.value(u"scroll_offset"_s).toDouble() == 0.625)
        && (progress.value(u"chapter"_s).toObject().value(u"id"_s).toString() == firstChapter),
        u"The confirmed exact second page was not saved"_s);
    controller->readerPageShown(firstPage);
    controller->scrollReaderPage(firstPage, 0.9);
    require(!controller->property("libraryPending").toBool() && (entry(database) == secondShown),
        u"Stale URL overwrote the confirmed page"_s);

    controller->selectReaderPage(0);
    controller->closeReader();
    controller->openReader();
    const QUrl reopened = readyPage(*controller);
    require((controller->property("readerChapterId").toString() == firstChapter)
        && (controller->property("readerPageIndex").toInt() == 1), u"Reopen resumed an unshown selection"_s);
    controller->readerPageShown(reopened);
    saved(*controller);
    require((restores.last().url == reopened) && (restores.last().offset == 0.625),
        u"Reopen discarded the saved scroll fraction"_s);
    controller->closeReader();
    controller.reset();
    controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    selectSaved(*controller);
    observe();
    controller->openReader();
    const QUrl restarted = readyPage(*controller);
    require((controller->property("readerPageIndex").toInt() == 1)
        && (controller->property("readerChapterId").toString() == firstChapter)
        && (controller->property("readerScrollOffset").toDouble() == 0.625), u"Controller restart lost exact progress"_s);
    controller->readerPageShown(restarted);
    saved(*controller);
    require((restores.last().url == restarted) && (restores.last().offset == 0.625),
        u"Controller restart lost the scroll offset"_s);

    controller->selectReaderPage(0);
    const QUrl faultPage = readyPage(*controller);
    const QJsonObject beforeFault = entry(database);
    execute(database, u"CREATE TRIGGER reject_reader_update BEFORE UPDATE ON library "
        "BEGIN SELECT RAISE(ABORT, 'generated reader write fault'); END"_s);
    controller->readerPageShown(faultPage);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"SQLite abort completion"_s);
    require(!controller->property("libraryErrorText").toString().isEmpty()
        && !controller->property("readerProgressErrorText").toString().isEmpty()
        && !controller->property("readerProgressPending").toBool()
        && !controller->property("readerBusy").toBool() && (entry(database) == beforeFault),
        u"SQLite abort invented history or left reading busy"_s);
    execute(database, u"DROP TRIGGER reject_reader_update"_s);
    const int restoresBeforeRetry = restores.size();
    controller->retryReaderProgress();
    saved(*controller);
    require((controller->property("readerPageUrl").toUrl() == faultPage)
        && (restores.size() == restoresBeforeRetry)
        && controller->property("readerProgressErrorText").toString().isEmpty()
        && (entry(database).value(u"reader_progress"_s).toObject().value(u"page"_s).toInt() == 0),
        u"Render retry did not save the exact page"_s);

    controller->selectReaderPage(1);
    const QUrl queuePage = readyPage(*controller);
    const QJsonObject beforeQueueFault = entry(database);
    execute(database, u"BEGIN EXCLUSIVE"_s);
    for (int count = 0; count < 70; ++count)
        controller->setSelectedFavorite(true);
    controller->readerPageShown(queuePage);
    require(!controller->property("readerProgressPending").toBool()
        && !controller->property("readerProgressErrorText").toString().isEmpty()
        && (entry(database) == beforeQueueFault), u"Rejected queue submission invented confirmed progress"_s);
    execute(database, u"ROLLBACK"_s);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Accepted queue drain"_s);
    require(entry(database) == beforeQueueFault, u"Other queued writes invented reading history"_s);
    controller->retryReaderProgress();
    saved(*controller);
    require((controller->property("readerPageUrl").toUrl() == queuePage)
        && controller->property("readerProgressErrorText").toString().isEmpty()
        && (entry(database).value(u"reader_progress"_s).toObject().value(u"page"_s).toInt() == 1),
        u"Rejected queue progress could not be retried on the displayed page"_s);

    execute(database, u"CREATE TRIGGER reject_stale_progress BEFORE UPDATE ON library "
        "BEGIN SELECT RAISE(ABORT, 'generated stale write fault'); END"_s);
    controller->scrollReaderPage(queuePage, 0.75);
    controller->closeReader();
    controller->openReader();
    const QUrl afterStaleWrite = readyPage(*controller);
    require(controller->property("readerProgressErrorText").toString().isEmpty()
        && !controller->property("readerProgressPending").toBool(),
        u"A late write failure changed the new reader session"_s);
    execute(database, u"DROP TRIGGER reject_stale_progress"_s);
    controller->readerPageShown(afterStaleWrite);
    saved(*controller);
    require(restores.last().offset == 0, u"New session resumed the rejected transient scroll offset"_s);

    controller->closeReader();
    bool reopenedDuringRead = false;
    const auto reentrant = QObject::connect(controller.get(), &Qbutt::ContentController::libraryPendingChanged,
        &observations, [&]
        {
            if (reopenedDuringRead || !controller->property("libraryPending").toBool())
                return;
            reopenedDuringRead = true;
            controller->closeReader();
            controller->openReader();
        }, Qt::DirectConnection);
    controller->openReader();
    const QUrl afterStaleRead = readyPage(*controller);
    QObject::disconnect(reentrant);
    require(reopenedDuringRead && (controller->property("readerPageIndex").toInt() == 1),
        u"Reentrant close/reopen applied a stale library read"_s);
    controller->readerPageShown(afterStaleRead);
    saved(*controller);

    controller->closeReader();
    execute(database, u"BEGIN EXCLUSIVE"_s);
    controller->openReader();
    readerFailed(*controller, "Could not open the content library.");
    execute(database, u"ROLLBACK"_s);
    controller->retryReader();
    const QUrl afterReadFault = readyPage(*controller);
    controller->readerPageShown(afterReadFault);
    saved(*controller);

    execute(database, u"CREATE TRIGGER reject_previous_page BEFORE UPDATE ON library "
        "BEGIN SELECT RAISE(ABORT, 'generated pending page fault'); END"_s);
    controller->scrollReaderPage(afterReadFault, 0.375);
    controller->selectReaderPage(0);
    const QUrl unshownPage = readyPage(*controller);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Previous page write failure"_s);
    require(!controller->property("readerProgressErrorText").toString().isEmpty(), u"Previous shown write failure was hidden"_s);
    execute(database, u"DROP TRIGGER reject_previous_page"_s);
    const int restoresBeforePreviousRetry = restores.size();
    controller->retryReaderProgress();
    saved(*controller);
    const QJsonObject previousPage = entry(database).value(u"reader_progress"_s).toObject();
    require((previousPage.value(u"page"_s).toInt() == 1)
        && (previousPage.value(u"scroll_offset"_s).toDouble() == 0.375)
        && (controller->property("readerPageIndex").toInt() == 0)
        && (controller->property("readerPageUrl").toUrl() == unshownPage)
        && (restores.size() == restoresBeforePreviousRetry),
        u"Retry lost the immutable shown page after an unshown selection"_s);
    controller->readerPageShown(unshownPage);
    saved(*controller);

    auto *chapterChoices = model(*controller, "readerChapters");
    require(chapterChoices->rowCount() > 1, u"Pending chapter fault needs another chapter"_s);
    const QString acquiringChapter = value(chapterChoices, 1, "chapterId");
    QTemporaryDir cacheFault;
    const QString blockedCache = cacheFault.filePath(u"cache-is-a-file"_s);
    QFile blocked(blockedCache);
    require(cacheFault.isValid() && blocked.open(QIODevice::WriteOnly), u"Could not create the acquiring chapter cache fault"_s);
    blocked.close();
    const QByteArray originalTemp = qgetenv("TEMP");
    const QByteArray originalTmp = qgetenv("TMP");
    const auto restoreEnvironment = qScopeGuard([&]
    {
        qputenv("TEMP", originalTemp);
        qputenv("TMP", originalTmp);
    });
    execute(database, u"CREATE TRIGGER reject_previous_chapter BEFORE UPDATE ON library "
        "BEGIN SELECT RAISE(ABORT, 'generated pending chapter fault'); END"_s);
    controller->scrollReaderPage(unshownPage, 0.8125);
    qputenv("TEMP", blockedCache.toLocal8Bit());
    qputenv("TMP", blockedCache.toLocal8Bit());
    controller->selectChapter(acquiringChapter);
    readerFailed(*controller, "Could not load this page.");
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Previous chapter write failure"_s);
    qputenv("TEMP", originalTemp);
    qputenv("TMP", originalTmp);
    execute(database, u"DROP TRIGGER reject_previous_chapter"_s);
    const int restoresBeforeFailedChapterRetry = restores.size();
    controller->retryReaderProgress();
    saved(*controller);
    const QJsonObject previousChapter = entry(database).value(u"reader_progress"_s).toObject();
    require((previousChapter.value(u"chapter"_s).toObject().value(u"id"_s).toString() == firstChapter)
        && (previousChapter.value(u"page"_s).toInt() == 0)
        && (previousChapter.value(u"scroll_offset"_s).toDouble() == 0.8125)
        && (controller->property("readerChapterId").toString() == acquiringChapter)
        && (restores.size() == restoresBeforeFailedChapterRetry),
        u"Retry replaced the confirmed snapshot with a failed acquiring chapter"_s);
    readerFailed(*controller, "Could not load this page.");

    controller->closeReader();
    changeSavedPosition(database, u"11111111-1111-1111-1111-111111111111"_s, 0);
    controller->openReader();
    readerFailed(*controller, "This chapter is no longer available.");
    require(controller->property("readerChapterId").toString().isEmpty(), u"Missing saved chapter was substituted"_s);
    auto *chapters = model(*controller, "readerChapters");
    require(chapters->rowCount() > 0, u"Missing saved chapter discarded the current chapter choices"_s);
    controller->setChapterQuery(u"generated-query-with-no-chapter-match"_s);
    require((chapters->rowCount() == 0) && controller->property("readerHasChapters").toBool(),
        u"Chapter filtering disabled recovery from a missing saved chapter"_s);
    controller->setChapterQuery(u""_s);
    controller->retryReader();
    readerFailed(*controller, "This chapter is no longer available.");
    controller->selectChapter(firstChapter);
    const QUrl chosen = readyPage(*controller);
    controller->readerPageShown(chosen);
    saved(*controller);

    controller->closeReader();
    changeSavedPosition(database, firstChapter, 1'000'000);
    controller->openReader();
    readerFailed(*controller, "This page is no longer available.");
    require(controller->property("readerPageIndex").toInt() == 1'000'000, u"Missing saved ordinal was clamped"_s);
    controller->retryReader();
    readerFailed(*controller, "This page is no longer available.");
    controller->selectChapter(firstChapter);
    const QUrl recovered = readyPage(*controller);
    controller->readerPageShown(recovered);
    controller->scrollReaderPage(recovered, 0.625);
    saved(*controller);
    controller->closeReader();

    auto *variants = model(*controller, "variants");
    QString other;
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        const QString candidate = value(variants, row, "variantId");
        if (candidate != exact)
        {
            other = candidate;
            break;
        }
    }
    require(!other.isEmpty(), u"Live work needs another exact translation branch"_s);
    controller->selectVariant(other);
    saved(*controller);
    const QJsonObject beforeOther = entry(database);
    controller->openReader();
    const QUrl otherPage = readyPage(*controller);
    require((controller->property("selectedVariantId").toString() == other)
        && (controller->property("readerPageIndex").toInt() == 0)
        && (entry(database) == beforeOther), u"Explicit other branch inherited or rewrote old progress"_s);
    controller->readerPageShown(otherPage);
    saved(*controller);
    require((restores.last().url == otherPage) && (restores.last().offset == 0),
        u"Explicit other branch inherited the old scroll fraction"_s);
    controller->closeReader();
    QString reference;
    const QJsonObject startupMetadata = entry(database, &reference);
    exerciseInitialFailure(startupMetadata, reference, false);
    exerciseInitialFailure(startupMetadata, reference, true);
    controller.reset();
    qInfo().noquote() << u"PASS reader_progress decoded_confirmation=ok full_work=ok duplicate_shown=ok finite_bounds=ok stale_url=ok reopen=ok controller_restart=ok sqlite_abort_retry=ok queue_retry=ok stale_write=ok stale_read=ok sqlite_read_retry=ok immutable_snapshot_retry=ok loading_failure_retry=ok entries_failure_retry=ok missing_chapter=ok missing_ordinal=ok explicit_branch=ok"_s;
}
