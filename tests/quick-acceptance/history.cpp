// SPDX-License-Identifier: GPL-2.0-or-later
#include "history.h"

#include <exception>
#include <functional>
#include <memory>
#include <stdexcept>

#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QStringList>
#include <QTemporaryDir>
#include <QThread>
#include <QThreadPool>
#include <QTimer>

#include "qbutt_quick/src/controller.cxxqt.h"
#include "qbutt_quick/src/models.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    const QString publication = u"mangadex:b0b721ff-c388-4486-aa0f-c2b0bb321512"_s;
    const QString exact = publication + u":translation:ru:9e383226-2af3-4af0-a69d-3d3b630212b3"_s;
    const QString generatedId = u"11111111-1111-1111-1111-111111111111"_s;
    const QString generated = u"mangadex:"_s + generatedId;

    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 100'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    Qbutt::WorkModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<Qbutt::WorkModel *>(controller.property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Missing typed GUI-thread work model"_s);
        return result;
    }

    void readyLibrary(Qbutt::ContentController &controller, const QString &profile)
    {
        controller.initLibrary(profile);
        waitFor([&]
        {
            const auto state = controller.property("libraryState").value<Qbutt::Library::LibraryState>();
            return (state == Qbutt::Library::LibraryState::Ready) || (state == Qbutt::Library::LibraryState::Failed);
        }, u"Library startup"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());
    }

    void settled(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, u"Library FIFO completion"_s);
    }

    void saved(Qbutt::ContentController &controller)
    {
        settled(controller);
        require(controller.property("libraryErrorText").toString().isEmpty(), u"Library operation failed"_s);
    }

    void inspect(Qbutt::ContentController &controller)
    {
        controller.selectWork(publication);
        require(controller.property("inspecting").toBool(), u"Stored history skipped fresh inspection"_s);
        waitFor([&] { return !controller.property("inspecting").toBool(); }, u"Fresh publication inspection"_s);
        require(controller.property("errorText").toString().isEmpty(), controller.property("errorText").toString());
        saved(controller);
    }

    QUrl readyPage(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("readerBusy").toBool(); }, u"Reader acquisition"_s);
        require(controller.property("readerOpen").toBool() && controller.property("readerErrorText").toString().isEmpty(),
            controller.property("readerErrorText").toString());
        const QUrl url = controller.property("readerPageUrl").toUrl();
        require(url.isLocalFile(), u"Page acquisition did not produce a local file"_s);
        QImageReader decoder(url.toLocalFile());
        require(decoder.canRead() && !decoder.read().isNull(), u"Actual acquired page did not decode"_s);
        return url;
    }

    void execute(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    }

    QJsonObject entry(QSqlDatabase &database, const QString &key)
    {
        QSqlQuery query(database);
        require(query.prepare(u"SELECT entry FROM library WHERE reference = ?"_s), query.lastError().text());
        query.addBindValue(key);
        require(query.exec() && query.next(), u"Expected exact library entry is missing"_s);
        const auto result = QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
        require(!result.isEmpty() && !query.next(), u"Exact library metadata is invalid"_s);
        return result;
    }

    void writeEntry(QSqlDatabase &database, const QString &key, const QJsonObject &metadata, const qint64 updatedAt = 0)
    {
        QSqlQuery query(database);
        require(query.prepare(u"INSERT INTO library VALUES (?, ?, ?) ON CONFLICT (reference) "
            "DO UPDATE SET entry = excluded.entry, updated_at = excluded.updated_at"_s), query.lastError().text());
        query.addBindValue(key);
        query.addBindValue(QString::fromUtf8(QJsonDocument(metadata).toJson(QJsonDocument::Compact)));
        query.addBindValue(updatedAt);
        require(query.exec() && (query.numRowsAffected() == 1), query.lastError().text());
    }

    QJsonObject variantReference(const QJsonObject &metadata, const QString &id)
    {
        const auto variants = metadata.value(u"work"_s).toObject().value(u"variants"_s).toArray();
        for (const auto &value : variants)
        {
            const auto reference = value.toObject().value(u"reference"_s).toObject();
            const auto translation = reference.value(u"selector"_s).toObject().value(u"manga_translation"_s).toObject();
            QStringList groups;
            for (const auto &group : translation.value(u"groups"_s).toArray())
                groups.append(group.toString());
            if (publication + u":translation:"_s + translation.value(u"language"_s).toString() + u":"_s + groups.join(u'+') == id)
                return reference;
        }
        throw std::runtime_error("Exact variant reference is missing from the saved work");
    }
}

void exerciseHistory()
{
    QTemporaryDir profile;
    require(profile.isValid(), u"History profile creation failed"_s);
    // The profile must outlive retired SQLite owners even when a scenario fails.
    const auto drainOwners = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("History fixture owners did not drain");
    });
    QFile payload(profile.filePath(u"generated-unrelated-payload.txt"_s));
    require(payload.open(QIODevice::WriteOnly) && (payload.write("generated unrelated payload") == 27), u"Payload fixture failed"_s);
    payload.close();
    auto controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    auto *history = model(*controller, "history");
    auto *favorites = model(*controller, "favorites");
    auto *works = model(*controller, "works");
    require((history != favorites) && (history != works) && (history->rowCount(QModelIndex()) == 0), u"Fresh History is not isolated"_s);
    require(controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>()
        == Qbutt::Library::LibraryErrorKind::None, u"Fresh library has an error kind"_s);
    controller->search(u"Frieren"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Live catalog search"_s);
    require(controller->property("errorText").toString().isEmpty() && (works->indexOfId(publication) >= 0), u"Live manga is missing"_s);
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Live manga inspection"_s);
    controller->selectVariant(exact);
    controller->setSelectedFavorite(true);
    saved(*controller);
    require((favorites->rowCount(QModelIndex()) == 1) && (history->rowCount(QModelIndex()) == 0), u"Search, inspection or bookmark invented History"_s);
    const int searchCount = works->rowCount(QModelIndex());
    const QString searchQuery = controller->property("query").toString();
    controller->openReader();
    const QUrl firstPage = readyPage(*controller);
    require(history->rowCount(QModelIndex()) == 0, u"Acquisition or decode invented History"_s);
    controller->readerPageShown(firstPage);
    controller->scrollReaderPage(firstPage, 0.25);
    saved(*controller);
    require((history->rowCount(QModelIndex()) == 1) && (history->idAt(0) == publication), u"Confirmed reading did not publish History"_s);
    controller->closeReader();
    for (const auto mode : {Qbutt::Search::ListMode::History, Qbutt::Search::ListMode::Favorites})
    {
        controller->setListMode(mode);
        inspect(*controller);
        require(controller->property("selectedVariantId").toString() == exact, u"Stored view changed exact identity"_s);
    }
    controller->setListMode(Qbutt::Search::ListMode::Search);
    controller->selectWork(publication);
    require((works->rowCount(QModelIndex()) == searchCount) && (controller->property("query").toString() == searchQuery)
        && (controller->property("selectedVariantId").toString() == exact), u"Stored views lost search context or exact identity"_s);

    const QString connectionName = u"native-history-fixture"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connectionName);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = QSqlDatabase();
        QSqlDatabase::removeDatabase(connectionName);
    });
    database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    QSqlQuery identity(database);
    require(identity.exec(u"SELECT reference FROM library"_s) && identity.next(), u"Confirmed work key is missing"_s);
    const QString key = identity.value(0).toString();
    identity.finish();
    auto metadata = entry(database, key);
    require(metadata.value(u"favorite"_s).toBool()
        && (metadata.value(u"work"_s).toObject().value(u"variants"_s).toArray().size() > 1)
        && (metadata.value(u"last_opened"_s).toObject().value(u"variant"_s).toObject() == variantReference(metadata, exact)),
        u"History lost full Work, bookmark or exact opened branch"_s);
    const QJsonObject filters {
        {u"episode"_s, QJsonValue::Null},
        {u"translation"_s, u"Generated saved translation choice"_s},
        {u"source"_s, u"manga_dex"_s},
        {u"language"_s, u"ru"_s},
        {u"player"_s, QJsonValue::Null},
        {u"transport"_s, u"manga"_s}
    };
    auto preferences = metadata.value(u"preferences"_s).toObject();
    preferences.insert(u"filters"_s, filters);
    metadata.insert(u"preferences"_s, preferences);
    writeEntry(database, key, metadata,
        metadata.value(u"last_opened"_s).toObject().value(u"at_unix_millis"_s).toInteger() + 1000);

    // A second generated historical entry makes updated_at and last_opened
    // disagree. No generated publication is resolved or acquired.
    const QString originalId = metadata.value(u"work"_s).toObject().value(u"reference"_s).toObject().value(u"id"_s).toString();
    const QString generatedKey = QString(key).replace(originalId, generatedId);
    auto historical = QJsonDocument::fromJson(QJsonDocument(metadata).toJson(QJsonDocument::Compact)
        .replace(originalId.toUtf8(), generatedId.toUtf8())).object();
    auto historicalWork = historical.value(u"work"_s).toObject();
    historicalWork.insert(u"title"_s, u"Generated historical publication"_s);
    historical.insert(u"work"_s, historicalWork);
    historical.insert(u"favorite"_s, false);
    historical.insert(u"reader_progress"_s, QJsonValue::Null);
    auto opened = historical.value(u"last_opened"_s).toObject();
    opened.insert(u"at_unix_millis"_s, metadata.value(u"last_opened"_s).toObject().value(u"at_unix_millis"_s).toInteger() + 10);
    historical.insert(u"last_opened"_s, opened);
    writeEntry(database, generatedKey, historical);
    controller.reset();
    controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    history = model(*controller, "history");
    favorites = model(*controller, "favorites");
    require((history->rowCount(QModelIndex()) == 2) && (history->idAt(0) == generated) && (history->idAt(1) == publication),
        u"Restart did not use successful action time ordering"_s);
    controller->setListMode(Qbutt::Search::ListMode::History);
    inspect(*controller);
    require(controller->property("selectedVariantId").toString() == exact, u"History restart substituted the exact branch"_s);
    auto *variants = qobject_cast<Qbutt::VariantModel *>(controller->property("variants").value<QObject *>());
    require(variants && (variants->rowCount(QModelIndex()) > 1), u"Fresh history inspection lost other variants"_s);
    QString other;
    for (int row = 0; row < variants->rowCount(QModelIndex()); ++row)
    {
        if (variants->idAt(row) != exact)
            other = variants->idAt(row);
    }
    require(!other.isEmpty(), u"Another exact reading branch is required"_s);
    const auto originalOpened = entry(database, key).value(u"last_opened"_s);
    controller->selectVariant(other);
    saved(*controller);
    require((history->idAt(0) == generated) && (entry(database, key).value(u"last_opened"_s) == originalOpened),
        u"Explicit variant choice recorded false reading or reordered History"_s);
    controller->openReader();
    const QUrl otherPage = readyPage(*controller);
    execute(database, u"BEGIN IMMEDIATE"_s);
    controller->readerPageShown(otherPage);
    require(controller->property("libraryPending").toBool(), u"Confirmed delayed write was not accepted"_s);
    controller->selectVariant(exact);
    require(controller->property("selectedVariantId").toString() == exact, u"Explicit pending choice failed"_s);
    execute(database, u"COMMIT"_s);
    saved(*controller);
    metadata = entry(database, key);
    require((history->idAt(0) == publication)
        && (metadata.value(u"last_opened"_s).toObject().value(u"variant"_s).toObject() == variantReference(metadata, other))
        && (metadata.value(u"preferences"_s).toObject().value(u"selected"_s).toObject() == variantReference(metadata, exact))
        && (metadata.value(u"preferences"_s).toObject().value(u"filters"_s).toObject() == filters)
        && (controller->property("selectedVariantId").toString() == exact), u"Delayed acknowledgement replaced current exact intent"_s);

    // A completion delivered inside begin/end model reset must wait in the
    // controller's existing publication queue, then remove the exact second row.
    bool reentered = false;
    bool nestedReset = false;
    bool wrongThread = false;
    int resetDepth = 0;
    std::exception_ptr notificationError;
    QObject observation;
    QObject::connect(history, &QAbstractItemModel::modelAboutToBeReset, &observation, [&]
    {
        nestedReset |= (++resetDepth > 1);
        wrongThread |= (QThread::currentThread() != qApp->thread());
        if (reentered)
            return;
        reentered = true;
        try
        {
            require(controller->removeHistory(publication), u"Reentrant exact removal was rejected"_s);
            settled(*controller);
            require((resetDepth == 1) && (history->rowCount(QModelIndex()) == 2), u"Nested completion mutated an open model reset"_s);
        }
        catch (...)
        {
            notificationError = std::current_exception();
        }
    }, Qt::DirectConnection);
    QObject::connect(history, &QAbstractItemModel::modelReset, &observation, [&] { --resetDepth; }, Qt::DirectConnection);
    require(controller->removeHistory(generated), u"Generated history removal was rejected"_s);
    saved(*controller);
    if (notificationError)
        std::rethrow_exception(notificationError);
    require(reentered && !nestedReset && !wrongThread && (resetDepth == 0) && (history->rowCount(QModelIndex()) == 0)
        && (favorites->rowCount(QModelIndex()) == 1) && controller->property("selectedFavorite").toBool()
        && (controller->property("selectedVariantId").toString() == exact), u"Reentrant History removal damaged bookmark or selection"_s);
    QObject::disconnect(history, nullptr, &observation, nullptr);
    const auto retained = entry(database, key);
    require(retained.value(u"last_opened"_s).isNull()
        && (retained.value(u"reader_progress"_s) == metadata.value(u"reader_progress"_s))
        && (retained.value(u"preferences"_s) == metadata.value(u"preferences"_s)), u"Removal erased progress or preferences"_s);
    require(!controller->removeHistory(publication) && !controller->removeHistory(u"missing-work"_s), u"Absent history submitted a removal"_s);

    controller->openReader();
    const QUrl confirmed = readyPage(*controller);
    controller->readerPageShown(confirmed);
    saved(*controller);
    const auto beforeAbort = entry(database, key);
    execute(database, u"CREATE TRIGGER reject_history_remove BEFORE UPDATE ON library "
        "BEGIN SELECT RAISE(ABORT, 'generated history removal fault'); END"_s);
    require(controller->removeHistory(publication), u"Faulted history removal was not queued"_s);
    settled(*controller);
    require(!controller->property("libraryErrorText").toString().isEmpty()
        && (controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>()
            == Qbutt::Library::LibraryErrorKind::Writing)
        && (history->rowCount(QModelIndex()) == 1) && (favorites->rowCount(QModelIndex()) == 1) && (entry(database, key) == beforeAbort),
        u"SQLite abort reported false removal or damaged saved metadata"_s);
    execute(database, u"DROP TRIGGER reject_history_remove"_s);
    require(controller->removeHistory(publication), u"Exact removal retry was rejected"_s);
    saved(*controller);
    require((history->rowCount(QModelIndex()) == 0) && (favorites->rowCount(QModelIndex()) == 1), u"Successful retry removed the bookmark"_s);
    controller->scrollReaderPage(confirmed, 0.5);
    saved(*controller);
    controller->setSelectedFavorite(false);
    saved(*controller);
    require((history->rowCount(QModelIndex()) == 1) && (favorites->rowCount(QModelIndex()) == 0), u"Removing bookmark erased History"_s);
    controller->setSelectedFavorite(true);
    saved(*controller);

    // Confirmed writes commit, but another corrupt fixture entry makes the
    // ordered snapshot fail. Read failure cannot undo their acknowledgement.
    const auto beforeReadFault = entry(database, generatedKey);
    QStringList staleBookmarks;
    for (const bool favorite : {false, true})
    {
        writeEntry(database, generatedKey, beforeReadFault);
        controller->setSelectedFavorite(!favorite);
        saved(*controller);
        require(controller->property("selectedFavorite").toBool() == !favorite, u"Bookmark fault setup did not settle"_s);
        writeEntry(database, generatedKey, QJsonObject {{u"generated_invalid"_s, true}});
        const auto beforeBookmark = entry(database, key);
        controller->setSelectedFavorite(favorite);
        settled(*controller);
        const auto afterBookmark = entry(database, key);
        require((controller->property("libraryErrorText").toString()
                == QCoreApplication::translate("ContentController", "Could not load reading history."))
            && (controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>()
                == Qbutt::Library::LibraryErrorKind::HistoryRead)
            && (afterBookmark.value(u"favorite"_s).toBool() == favorite)
            && (favorites->rowCount(QModelIndex()) == (favorite ? 1 : 0))
            && ((favorites->indexOfId(publication) >= 0) == favorite)
            && (afterBookmark.value(u"last_opened"_s) == beforeBookmark.value(u"last_opened"_s))
            && (afterBookmark.value(u"reader_progress"_s) == beforeBookmark.value(u"reader_progress"_s))
            && (afterBookmark.value(u"preferences"_s) == beforeBookmark.value(u"preferences"_s))
            && (controller->property("selectedVariantId").toString() == exact)
            && (history->rowCount(QModelIndex()) == 1), u"Failed History read damaged a committed bookmark write"_s);
        if (controller->property("selectedFavorite").toBool() != favorite)
            staleBookmarks.append(favorite ? u"add"_s : u"remove"_s);
    }
    require(staleBookmarks.isEmpty(), u"Committed bookmark property stayed stale after History read failure: "_s
        + staleBookmarks.join(u", "_s));
    controller->scrollReaderPage(confirmed, 0.75);
    settled(*controller);
    require((controller->property("libraryErrorText").toString()
            == QCoreApplication::translate("ContentController", "Could not load reading history."))
        && (controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>()
            == Qbutt::Library::LibraryErrorKind::HistoryRead)
        && controller->property("readerProgressErrorText").toString().isEmpty()
        && !controller->property("readerProgressPending").toBool()
        && (entry(database, key).value(u"reader_progress"_s).toObject().value(u"scroll_offset"_s).toDouble() == 0.75),
        u"Failed snapshot masked committed reading or invented a rejected write"_s);
    const auto committed = entry(database, key);
    controller->retryReaderProgress();
    require(!controller->property("libraryPending").toBool() && (entry(database, key) == committed),
        u"Snapshot read error made successful reader persistence retryable"_s);
    writeEntry(database, generatedKey, beforeReadFault);
    require(controller->refreshHistory(), u"Ordered history read retry was rejected"_s);
    saved(*controller);
    require((history->rowCount(QModelIndex()) == 1) && (history->idAt(0) == publication) && (entry(database, key) == committed),
        u"History read retry changed already committed reading"_s);

    // An actual blocked SQLite writer holds the sole worker while its bounded
    // queue fills. Rejected reads must report a read error and remain retryable.
    execute(database, u"BEGIN IMMEDIATE"_s);
    controller->setSelectedFavorite(true);
    require(controller->property("libraryPending").toBool(), u"Queue fault did not accept its blocked writer"_s);
    bool readRejected = false;
    for (int request = 0; (request < 80) && !readRejected; ++request)
        readRejected = !controller->refreshHistory();
    const bool correctRejection = readRejected && (controller->property("libraryErrorText").toString()
        == QCoreApplication::translate("ContentController", "Could not load reading history."))
        && (controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>()
            == Qbutt::Library::LibraryErrorKind::HistoryRead);
    execute(database, u"COMMIT"_s);
    settled(*controller);
    require(correctRejection, u"Rejected History read reported a write error"_s);
    require((controller->property("libraryErrorText").toString()
            == QCoreApplication::translate("ContentController", "Could not load reading history."))
        && (history->rowCount(QModelIndex()) == 1) && controller->property("selectedFavorite").toBool()
        && (entry(database, key) == committed), u"Older accepted completion hid rejected History read or changed metadata"_s);
    require(controller->refreshHistory(), u"Rejected History read could not retry after the queue drained"_s);
    saved(*controller);
    require((controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>()
            == Qbutt::Library::LibraryErrorKind::None)
        && (history->rowCount(QModelIndex()) == 1) && (entry(database, key) == committed),
        u"History queue retry changed confirmed metadata"_s);
    controller->closeReader();
    controller.reset();

    auto stale = entry(database, key);
    auto staleWork = stale.value(u"work"_s).toObject();
    auto staleVariants = staleWork.value(u"variants"_s).toArray();
    auto missing = staleVariants.first().toObject();
    auto missingReference = missing.value(u"reference"_s).toObject();
    missingReference.insert(u"selector"_s, QJsonObject {{u"manga_translation"_s, QJsonObject {
        {u"language"_s, u"ru"_s}, {u"groups"_s, QJsonArray {generatedId}}}}});
    missing.insert(u"reference"_s, missingReference);
    staleVariants.append(missing);
    staleWork.insert(u"variants"_s, staleVariants);
    stale.insert(u"work"_s, staleWork);
    auto stalePreferences = stale.value(u"preferences"_s).toObject();
    stalePreferences.insert(u"selected"_s, missingReference);
    stale.insert(u"preferences"_s, stalePreferences);
    writeEntry(database, key, stale);
    controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    controller->setListMode(Qbutt::Search::ListMode::History);
    inspect(*controller);
    require((controller->property("selectedVariantId").toString() == publication + u":translation:ru:"_s + generatedId)
        && (controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == Qbutt::Search::VariantSelectionState::Unavailable), u"Missing history variant was silently substituted"_s);
    controller->openReader();
    require(!controller->property("readerOpen").toBool(), u"Missing variant became actionable"_s);
    controller->selectVariant(exact);
    saved(*controller);
    controller->openReader();
    readyPage(*controller);
    require(controller->property("readerScrollOffset").toDouble() == 0.75, u"Explicit real branch lost exact retained progress"_s);
    controller->closeReader();
    require(payload.open(QIODevice::ReadOnly) && (payload.readAll() == "generated unrelated payload"), u"History removal changed unrelated payload"_s);
    payload.close();
    controller.reset();
    qInfo().noquote() << u"PASS history confirmed_only=ok full_work=ok opened_order=ok restart=ok fresh_inspect=ok other_variant=ok delayed_intent=ok reentrant_publication=ok remove_retry=ok bookmark_independence=ok bookmark_snapshot_fault=ok committed_snapshot_fault=ok snapshot_retry=ok queue_read_retry=ok missing_exact=ok exact_resume=ok payload_preserved=ok"_s;
}
