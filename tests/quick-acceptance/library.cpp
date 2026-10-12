// SPDX-License-Identifier: GPL-2.0-or-later
#include "library.h"

#include <atomic>
#include <exception>
#include <functional>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThread>
#include <QTimer>
#include <QUrl>
#include <QUrlQuery>

#include "qbutt_quick/src/controller.cxxqt.h"
#include "qbutt_quick/src/models.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const int timeout, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < timeout))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller.property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Invalid typed model"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const auto roles = model->roleNames();
        for (auto it = roles.cbegin(); it != roles.cend(); ++it)
        {
            if (it.value() == name)
                return model->data(model->index(row, 0), it.key()).toString();
        }
        throw std::runtime_error("Missing role: " + name.toStdString());
    }

    void readyLibrary(Qbutt::ContentController &controller, const QString &profile)
    {
        controller.initLibrary(profile);
        waitFor([&]
        {
            const auto state = controller.property("libraryState").value<Qbutt::Library::LibraryState>();
            return (state == Qbutt::Library::LibraryState::Ready) || (state == Qbutt::Library::LibraryState::Failed);
        }, 10'000, u"Library open"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());
    }

    void saved(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, 10'000, u"Library write"_s);
        require(controller.property("libraryErrorText").toString().isEmpty(), u"Library write failed"_s);
    }

    void search(Qbutt::ContentController &controller)
    {
        controller.search(u"Frieren"_s);
        waitFor([&] { return !controller.property("busy").toBool(); }, 50'000, u"Catalog search"_s);
        require(controller.property("errorText").toString().isEmpty(), u"Catalog search failed"_s);
    }

    void inspect(Qbutt::ContentController &controller, const QString &id)
    {
        controller.selectWork(id);
        waitFor([&] { return !controller.property("inspecting").toBool(); }, 50'000, u"Publication inspection"_s);
        require(controller.property("errorText").toString().isEmpty(), u"Publication inspection failed"_s);
    }

    QJsonObject entry(QSqlDatabase &database, const QString &id, QString *key = nullptr)
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT reference, entry FROM library"_s), query.lastError().text());
        while (query.next())
        {
            const auto result = QJsonDocument::fromJson(query.value(1).toString().toUtf8()).object();
            if (result.value(u"work"_s).toObject().value(u"reference"_s).toObject().value(u"id"_s).toString() == id)
            {
                if (key)
                    *key = query.value(0).toString();
                return result;
            }
        }
        throw std::runtime_error("Saved work is missing");
    }

}

void exerciseLibrary()
{
    QQmlEngine engine;
    QQmlComponent enumProbe(&engine);
    enumProbe.setData("import QtQml; import Qbutt.Content; QtObject { property int searchFailed: Search.Failed; property int libraryFailed: Library.Failed }", QUrl());
    std::unique_ptr<QObject> enums(enumProbe.create());
    require(enums && (enums->property("searchFailed").toInt() == static_cast<int>(Qbutt::Search::SearchState::Failed))
        && (enums->property("libraryFailed").toInt() == static_cast<int>(Qbutt::Library::LibraryState::Failed)),
        u"QML enum namespaces are ambiguous"_s);

    QTemporaryDir profile;
    require(profile.isValid(), u"Temporary profile failed"_s);
    const QString publication = u"aniliberty:9542"_s;
    QString other;
    QString selectedVariant;
    QString expectedHash;
    int variantCount = 0;
    auto controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    auto *works = model(*controller, "works");
    auto *favorites = model(*controller, "favorites");
    auto *variants = model(*controller, "variants");
    require((works != favorites) && (favorites->rowCount() == 0), u"Fresh profile is not empty"_s);
    std::atomic_bool wrongThread = false;
    QObject observation;
    QObject::connect(favorites, &QAbstractItemModel::rowsInserted, &observation, [&]
    {
        if (QThread::currentThread() != qApp->thread())
            wrongThread.store(true);
    }, Qt::DirectConnection);
    search(*controller);
    for (int row = 0; row < works->rowCount(); ++row)
    {
        if (value(works, row, "workId") != publication)
            other = value(works, row, "workId");
    }
    require(!other.isEmpty(), u"Two publications are required"_s);
    inspect(*controller, publication);
    variantCount = variants->rowCount();
    require(variantCount >= 2, u"Two exact variants are required"_s);
    const auto *typedWorks = qobject_cast<Qbutt::WorkModel *>(works);
    const auto *typedVariants = qobject_cast<Qbutt::VariantModel *>(variants);
    require(typedWorks && typedVariants
        && (typedWorks->idAt(typedWorks->indexOfId(publication)) == publication)
        && typedWorks->idAt(-1).isEmpty() && typedWorks->idAt(works->rowCount()).isEmpty()
        && (typedWorks->indexOfId(u"missing-work"_s) == -1)
        && (typedVariants->idAt(0) == controller->property("selectedVariantId").toString())
        && (controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == Qbutt::Search::VariantSelectionState::Available), u"Typed lookup or initial native selection failed"_s);
    int lastTorrent = -1;
    int firstTorrent = -1;
    for (int row = 0; row < variantCount; ++row)
    {
        if (value(variants, row, "kind") == u"torrent"_s)
        {
            if (firstTorrent < 0)
                firstTorrent = row;
            lastTorrent = row;
        }
    }
    require((firstTorrent >= 0) && (firstTorrent != lastTorrent), u"Two exact torrents are required"_s);
    selectedVariant = value(variants, lastTorrent, "variantId");
    require((typedVariants->idAt(typedVariants->indexOfId(selectedVariant)) == selectedVariant)
        && typedVariants->idAt(-1).isEmpty() && typedVariants->idAt(variantCount).isEmpty()
        && (typedVariants->indexOfId(u"missing-variant"_s) == -1), u"Typed variant lookup failed"_s);
    expectedHash = value(variants, lastTorrent, "infoHash");
    controller->selectVariant(selectedVariant);
    controller->setSelectedFavorite(true);
    saved(*controller);
    require(controller->property("selectedFavorite").toBool() && (favorites->rowCount() == 1),
        u"Favorite did not retain the selected whole work"_s);

    controller->setSelectedFavorite(false);
    saved(*controller);
    {
        QSqlDatabase boundary = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"quick-library-boundary"_s);
        boundary.setDatabaseName(QDir(profile.path()).filePath(u"content-library.sqlite"_s));
        require(boundary.open(), boundary.lastError().text());
        QSqlQuery lock(boundary);
        bool rejected = false;
        bool erasedRejectedError = false;
        bool committedFavoritePublished = false;
        std::exception_ptr rejectionError;
        const auto rejectionConnection = QObject::connect(favorites, &QAbstractItemModel::rowsAboutToBeInserted,
            &observation, [&]
        {
            if (rejected)
                return;
            try
            {
                require(lock.exec(u"BEGIN IMMEDIATE"_s), lock.lastError().text());
                for (int request = 0; request < 67; ++request)
                    controller->setSelectedFavorite(false);
                rejected = !controller->property("libraryErrorText").toString().isEmpty();
                require(rejected, u"Bounded queue did not reject saturation"_s);
                require(lock.exec(u"ROLLBACK"_s), lock.lastError().text());
            }
            catch (...)
            {
                rejectionError = std::current_exception();
            }
        }, Qt::DirectConnection);
        const auto errorConnection = QObject::connect(controller.get(), &Qbutt::ContentController::libraryErrorTextChanged,
            &observation, [&]
        {
            erasedRejectedError |= rejected && controller->property("libraryErrorText").toString().isEmpty()
                && controller->property("selectedFavorite").toBool();
        }, Qt::DirectConnection);
        const auto favoriteConnection = QObject::connect(controller.get(), &Qbutt::ContentController::selectedFavoriteChanged,
            &observation, [&]
        {
            committedFavoritePublished |= rejected && controller->property("selectedFavorite").toBool();
        }, Qt::DirectConnection);
        controller->setSelectedFavorite(true);
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, 15'000, u"Queue saturation acknowledgements"_s);
        QObject::disconnect(rejectionConnection);
        QObject::disconnect(errorConnection);
        QObject::disconnect(favoriteConnection);
        if (rejectionError)
            std::rethrow_exception(rejectionError);
        require(rejected && !erasedRejectedError, u"Older successful acknowledgement erased queue rejection"_s);
        require(committedFavoritePublished, u"Queue rejection interrupted committed favorite publication"_s);
        require(!controller->property("libraryErrorText").toString().isEmpty(), u"Older queued writes hid rejected intent"_s);
        lock = QSqlQuery();
        boundary.close();
    }
    QSqlDatabase::removeDatabase(u"quick-library-boundary"_s);
    controller->setSelectedFavorite(true);
    saved(*controller);

    controller->setSelectedFavorite(false);
    saved(*controller);
    bool removedDuringInsert = false;
    std::exception_ptr insertionError;
    const auto removeDuringInsert = QObject::connect(favorites, &QAbstractItemModel::rowsAboutToBeInserted,
        &observation, [&]
    {
        if (removedDuringInsert)
            return;
        removedDuringInsert = true;
        try
        {
            controller->setSelectedFavorite(false);
            waitFor([&] { return !controller->property("libraryPending").toBool(); }, 10'000, u"Nested remove before insert"_s);
        }
        catch (...)
        {
            insertionError = std::current_exception();
        }
    }, Qt::DirectConnection);
    controller->setSelectedFavorite(true);
    saved(*controller);
    QObject::disconnect(removeDuringInsert);
    if (insertionError)
        std::rethrow_exception(insertionError);
    require(removedDuringInsert && (favorites->rowCount() == 0)
        && !controller->property("selectedFavorite").toBool(), u"Nested acknowledgement left a stale favorite row"_s);
    controller->setSelectedFavorite(true);
    saved(*controller);

    const QString previousVariant = value(variants, firstTorrent, "variantId");
    controller->selectVariant(previousVariant);
    saved(*controller);
    controller->selectVariant(selectedVariant);
    controller->selectWork(other);
    controller->selectWork(publication);
    require(controller->property("selectedVariantId").toString() == selectedVariant,
        u"Pending exact preference was lost before Qt completion"_s);
    saved(*controller);
    require(controller->property("selectedVariantId").toString() == selectedVariant,
        u"Earlier acknowledgement replaced the latest exact preference"_s);

    const int searchCount = works->rowCount();
    int searchNotifications = 0;
    QObject::connect(controller.get(), &Qbutt::ContentController::searchStateChanged, &observation,
        [&] { ++searchNotifications; });
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    controller->selectWork(publication);
    require(controller->property("inspecting").toBool(), u"Stored metadata became inspection authority"_s);
    require(controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
        == Qbutt::Search::VariantSelectionState::Inspecting, u"Cached variant became actionable during inspection"_s);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, 50'000, u"Saved publication reinspection"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Saved publication inspection failed"_s);
    require(controller->property("selectedVariantId").toString() == selectedVariant,
        u"Inspection substituted the preferred variant"_s);
    controller->setListMode(Qbutt::Search::ListMode::Search);
    require((works->rowCount() == searchCount) && (searchNotifications == 0), u"Mode switch repeated or lost search"_s);
    saved(*controller);

    controller->setSelectedFavorite(false);
    controller->selectWork(other);
    saved(*controller);
    require((controller->property("selectedWorkId").toString() == other)
        && !controller->property("selectedFavorite").toBool(), u"Late favorite completion changed another selected work"_s);
    inspect(*controller, publication);
    saved(*controller);
    require((favorites->rowCount() == 0) && (variants->rowCount() == variantCount)
        && (controller->property("selectedVariantId").toString() == selectedVariant),
        u"Removing a favorite lost variants or preference"_s);

    QSqlDatabase database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"quick-library-fixture"_s);
    database.setDatabaseName(QDir(profile.path()).filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    auto stored = entry(database, u"9542"_s);
    require(stored.value(u"last_opened"_s).isNull()
        && (stored.value(u"work"_s).toObject().value(u"variants"_s).toArray().size() >= variantCount),
        u"Selection wrote history or lost the whole known work"_s);
    int handoffs = 0;
    bool exactHandoff = false;
    QObject::connect(controller.get(), &Qbutt::ContentController::torrentReady, &observation, [&](const QString &magnet)
    {
        ++handoffs;
        exactHandoff = QUrlQuery(QUrl(magnet)).queryItemValue(u"xt"_s).compare(u"urn:btih:"_s + expectedHash,
            Qt::CaseInsensitive) == 0;
    });
    controller->resolveVariant(selectedVariant);
    waitFor([&] { return !controller->property("resolving").toBool(); }, 50'000, u"Saved exact resolution"_s);
    saved(*controller);
    require(exactHandoff && (handoffs == 1) && entry(database, u"9542"_s).value(u"last_opened"_s).isNull(),
        u"Resolution or torrentReady fabricated history"_s);
    controller.reset();

    controller = std::make_unique<Qbutt::ContentController>();
    search(*controller);
    inspect(*controller, publication);
    controller->initLibrary(profile.path());
    const auto stateBeforeEntry = controller->property("libraryState").value<Qbutt::Library::LibraryState>();
    controller->selectVariant(previousVariant);
    const bool acceptedBeforeEntry = controller->property("libraryPending").toBool();
    controller.reset();
    require(!acceptedBeforeEntry && (stateBeforeEntry == Qbutt::Library::LibraryState::Uninitialized),
        u"Unentered library worker accepted a preference"_s);
    require(entry(database, u"9542"_s).value(u"preferences"_s).toObject().value(u"selected"_s).toObject()
        .value(u"selector"_s).toObject().value(u"torrent"_s).toObject().value(u"id"_s)
        == selectedVariant.section(u':', -1).toInt(), u"Unaccepted preference changed saved metadata"_s);

    QSqlQuery loadLock(database);
    require(loadLock.exec(u"BEGIN IMMEDIATE"_s), loadLock.lastError().text());
    controller = std::make_unique<Qbutt::ContentController>();
    search(*controller);
    inspect(*controller, publication);
    controller->initLibrary(profile.path());
    controller->initLibrary(QDir(profile.path()).filePath(u"unexpected-second-owner"_s));
    waitFor([&]
    {
        return controller->property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Loading;
    }, 10'000, u"SQLite worker entry"_s);
    {
        controller->selectVariant(previousVariant);
        require(controller->property("libraryPending").toBool(), u"Entered worker did not accept Loading preference"_s);
        QElapsedTimer closeTime;
        closeTime.start();
        controller.reset();
        require(closeTime.elapsed() < 250, u"Loading shutdown waited for SQLite I/O"_s);
    }
    require(loadLock.exec(u"ROLLBACK"_s), loadLock.lastError().text());
    waitFor([&]
    {
        return entry(database, u"9542"_s).value(u"preferences"_s).toObject().value(u"selected"_s).toObject()
            .value(u"selector"_s).toObject().value(u"torrent"_s).toObject().value(u"id"_s)
            == previousVariant.section(u':', -1).toInt();
    }, 10'000, u"Accepted Loading preference after close"_s);
    require(!QDir(profile.path()).exists(u"unexpected-second-owner"_s),
        u"Repeated initialization created another SQLite owner"_s);
    controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    search(*controller);
    inspect(*controller, publication);
    saved(*controller);
    require(controller->property("selectedVariantId").toString() == previousVariant,
        u"Fresh controller lost the preference drained after Loading close"_s);
    controller->selectVariant(selectedVariant);
    saved(*controller);
    controller.reset();

    for (const bool explicitDuringLoad : {false, true})
    {
        controller = std::make_unique<Qbutt::ContentController>();
        search(*controller);
        inspect(*controller, publication);
        require(loadLock.exec(u"BEGIN IMMEDIATE"_s), loadLock.lastError().text());
        QString choiceDuringLoad;
        bool heldLoading = false;
        std::exception_ptr loadingError;
        const auto loadingConnection = QObject::connect(controller.get(), &Qbutt::ContentController::libraryStateChanged,
            &observation, [&]
        {
            if (controller->property("libraryState").value<Qbutt::Library::LibraryState>()
                != Qbutt::Library::LibraryState::Loading)
            {
                return;
            }
            try
            {
                controller->selectWork(other);
                controller->selectWork(publication);
                if (explicitDuringLoad)
                    controller->selectVariant(previousVariant);
                choiceDuringLoad = controller->property("selectedVariantId").toString();
                QEventLoop loop;
                QTimer::singleShot(100, &loop, &QEventLoop::quit);
                loop.exec();
                heldLoading = controller->property("libraryState").value<Qbutt::Library::LibraryState>()
                    == Qbutt::Library::LibraryState::Loading;
                require(loadLock.exec(u"ROLLBACK"_s), loadLock.lastError().text());
            }
            catch (...)
            {
                loadingError = std::current_exception();
            }
        }, Qt::DirectConnection);
        readyLibrary(*controller, profile.path());
        QObject::disconnect(loadingConnection);
        if (loadingError)
            std::rethrow_exception(loadingError);
        const auto expectedChoice = explicitDuringLoad ? previousVariant : selectedVariant;
        require(heldLoading && (controller->property("selectedVariantId").toString() == expectedChoice),
            u"Delayed library load replaced saved exact preference"_s);
        require(explicitDuringLoad ? (choiceDuringLoad == previousVariant) : choiceDuringLoad.isEmpty(),
            u"Automatic preference was accepted before library load"_s);
        saved(*controller);
        require(entry(database, u"9542"_s).value(u"preferences"_s).toObject().value(u"selected"_s).toObject()
            .value(u"selector"_s).toObject().value(u"torrent"_s).toObject().value(u"id"_s)
            == expectedChoice.section(u':', -1).toInt(), u"Explicit choice during Loading did not persist"_s);
        controller->selectVariant(selectedVariant);
        saved(*controller);
        controller.reset();
    }
    loadLock = QSqlQuery();

    controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    search(*controller);
    inspect(*controller, publication);
    saved(*controller);
    require((controller->property("selectedVariantId").toString() == selectedVariant)
        && !controller->property("selectedFavorite").toBool()
        && (model(*controller, "variants")->rowCount() == variantCount), u"Reopen lost exact preferences after favorite removal"_s);

    QSqlQuery lock(database);
    controller->selectVariant(previousVariant);
    saved(*controller);
    bool nestedFailure = false;
    std::exception_ptr nestedError;
    const auto nestedConnection = QObject::connect(controller.get(), &Qbutt::ContentController::selectedFavoriteChanged,
        &observation, [&]
    {
        if (nestedFailure || !controller->property("selectedFavorite").toBool())
            return;
        nestedFailure = true;
        try
        {
            require(lock.exec(u"BEGIN IMMEDIATE"_s), lock.lastError().text());
            controller->selectVariant(selectedVariant);
            controller->selectWork(other);
            controller->selectWork(publication);
            require(controller->property("selectedVariantId").toString() == selectedVariant,
                u"Locked preference was lost during reentrant selection"_s);
            waitFor([&] { return !controller->property("libraryPending").toBool(); }, 10'000, u"Reentrant library completion"_s);
            require(lock.exec(u"ROLLBACK"_s), lock.lastError().text());
        }
        catch (...)
        {
            nestedError = std::current_exception();
        }
    }, Qt::DirectConnection);
    controller->setSelectedFavorite(true);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, 15'000, u"Nested favorite mutation"_s);
    QObject::disconnect(nestedConnection);
    if (nestedError)
        std::rethrow_exception(nestedError);
    require(nestedFailure && controller->property("selectedFavorite").toBool()
        && !controller->property("libraryErrorText").toString().isEmpty()
        && (controller->property("selectedVariantId").toString() == selectedVariant),
        u"Earlier completion hid a reentrant preference failure or changed intent"_s);
    require(entry(database, u"9542"_s).value(u"preferences"_s).toObject().value(u"selected"_s).toObject()
        .value(u"selector"_s).toObject().value(u"torrent"_s).toObject().value(u"id"_s)
        == previousVariant.section(u':', -1).toInt(), u"Locked preference unexpectedly committed"_s);
    controller->selectWork(other);
    controller->selectWork(publication);
    require(controller->property("selectedVariantId").toString() == selectedVariant,
        u"Failed preference lost latest intent after switching work"_s);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    inspect(*controller, publication);
    saved(*controller);
    require(controller->property("selectedVariantId").toString() == selectedVariant,
        u"Forced inspection replaced failed but accepted preference"_s);
    controller->setListMode(Qbutt::Search::ListMode::Search);
    controller->setSelectedFavorite(false);
    saved(*controller);
    controller.reset();
    controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    search(*controller);
    inspect(*controller, publication);
    saved(*controller);
    require(controller->property("selectedVariantId").toString() == selectedVariant,
        u"Preference recovery did not persist across reopen"_s);

    require(lock.exec(u"BEGIN IMMEDIATE"_s), lock.lastError().text());
    controller->setSelectedFavorite(true);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, 10'000, u"Locked database failure"_s);
    require(!controller->property("libraryErrorText").toString().isEmpty()
        && !controller->property("selectedFavorite").toBool(), u"Failed write published success"_s);
    require(lock.exec(u"ROLLBACK"_s), lock.lastError().text());
    controller->setSelectedFavorite(true);
    saved(*controller);
    require(controller->property("selectedFavorite").toBool(), u"Retry did not recover after I/O failure"_s);
    controller->setSelectedFavorite(false);
    saved(*controller);

    require(lock.exec(u"BEGIN IMMEDIATE"_s), lock.lastError().text());
    controller->setSelectedFavorite(true);
    require(controller->property("libraryPending").toBool(), u"Shutdown has no pending write"_s);
    QElapsedTimer closeTime;
    closeTime.start();
    controller.reset();
    require(closeTime.elapsed() < 250, u"QObject shutdown waited for SQLite I/O"_s);
    require(lock.exec(u"ROLLBACK"_s), lock.lastError().text());
    waitFor([&] { return entry(database, u"9542"_s).value(u"favorite"_s).toBool(); }, 10'000, u"Accepted write after close"_s);
    QString rowKey;
    stored = entry(database, u"9542"_s, &rowKey);
    auto staleWork = stored.value(u"work"_s).toObject();
    auto staleVariants = staleWork.value(u"variants"_s).toArray();
    auto staleVariant = staleVariants.last().toObject();
    auto staleReference = staleVariant.value(u"reference"_s).toObject();
    staleReference.insert(u"selector"_s, QJsonObject {{u"torrent"_s, QJsonObject {{u"id"_s, 999999999}}}});
    staleVariant.insert(u"reference"_s, staleReference);
    staleVariants.replace(staleVariants.size() - 1, staleVariant);
    staleWork.insert(u"variants"_s, staleVariants);
    stored.insert(u"work"_s, staleWork);
    auto stalePreferences = stored.value(u"preferences"_s).toObject();
    stalePreferences.insert(u"selected"_s, staleReference);
    stored.insert(u"preferences"_s, stalePreferences);
    lock.prepare(u"UPDATE library SET entry = ?1 WHERE reference = ?2"_s);
    lock.addBindValue(QString::fromUtf8(QJsonDocument(stored).toJson(QJsonDocument::Compact)));
    lock.addBindValue(rowKey);
    require(lock.exec() && (lock.numRowsAffected() == 1), u"Stale publication fixture failed"_s);
    controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    require(model(*controller, "favorites")->rowCount() == 1, u"Accepted write was aborted on close"_s);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    inspect(*controller, publication);
    saved(*controller);
    require((controller->property("selectedVariantId").toString() == publication + u":torrent:999999999"_s)
        && (controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == Qbutt::Search::VariantSelectionState::Unavailable), u"Missing exact preference was silently substituted"_s);
    controller->resolveVariant(controller->property("selectedVariantId").toString());
    require(!controller->property("resolving").toBool(), u"Missing exact preference entered resolution"_s);
    controller.reset();
    lock = QSqlQuery();
    database.close();
    database = QSqlDatabase();
    QSqlDatabase::removeDatabase(u"quick-library-fixture"_s);

    for (const bool corruptEntry : {false, true})
    {
        QTemporaryDir corruptProfile;
        require(corruptProfile.isValid(), u"Corrupt profile fixture failed"_s);
        if (corruptEntry)
        {
            Qbutt::ContentController seed;
            readyLibrary(seed, corruptProfile.path());
        }
        {
            QSqlDatabase startup = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"quick-library-startup-fault"_s);
            startup.setDatabaseName(QDir(corruptProfile.path()).filePath(u"content-library.sqlite"_s));
            require(startup.open(), startup.lastError().text());
            QSqlQuery fault(startup);
            require(fault.exec(corruptEntry
                ? u"INSERT INTO library VALUES ('generated-corrupt', 'not-json', 0)"_s
                : u"CREATE TABLE foreign_profile (value TEXT)"_s), fault.lastError().text());
            require(fault.exec(u"BEGIN IMMEDIATE"_s), fault.lastError().text());
            controller = std::make_unique<Qbutt::ContentController>();
            search(*controller);
            inspect(*controller, publication);
            bool acceptedLoadingChoice = false;
            std::exception_ptr startupError;
            const auto startupConnection = QObject::connect(controller.get(), &Qbutt::ContentController::libraryStateChanged,
                &observation, [&]
            {
                if (controller->property("libraryState").value<Qbutt::Library::LibraryState>()
                    != Qbutt::Library::LibraryState::Loading)
                {
                    return;
                }
                try
                {
                    controller->selectVariant(selectedVariant);
                    acceptedLoadingChoice = controller->property("libraryPending").toBool();
                    require(fault.exec(u"ROLLBACK"_s), fault.lastError().text());
                }
                catch (...)
                {
                    startupError = std::current_exception();
                }
            }, Qt::DirectConnection);
            controller->initLibrary(corruptProfile.path());
            waitFor([&]
            {
                return controller->property("libraryState").value<Qbutt::Library::LibraryState>()
                    == Qbutt::Library::LibraryState::Failed;
            }, 10'000, u"Accepted choice at startup failure"_s);
            QObject::disconnect(startupConnection);
            if (startupError)
                std::rethrow_exception(startupError);
            require(acceptedLoadingChoice && !controller->property("libraryPending").toBool()
                && !controller->property("libraryErrorText").toString().isEmpty(),
                u"Startup failure left an accepted choice pending or hid its error"_s);
            controller.reset();
            fault = QSqlQuery();
            startup.close();
        }
        QSqlDatabase::removeDatabase(u"quick-library-startup-fault"_s);
    }

    QTemporaryDir failureProfile;
    QFile file(QDir(failureProfile.path()).filePath(u"not-a-directory"_s));
    require(file.open(QIODevice::WriteOnly), u"Failure fixture file could not be created"_s);
    file.close();
    Qbutt::ContentController failed;
    failed.initLibrary(file.fileName());
    waitFor([&]
    {
        return failed.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Failed;
    }, 10'000, u"Invalid profile failure"_s);
    require(!failed.property("libraryErrorText").toString().isEmpty(), u"Open failure has no localized error"_s);
    Qbutt::ContentController relative;
    relative.initLibrary(u"relative-profile-is-not-allowed"_s);
    require(relative.property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Failed,
        u"Relative profile reached filesystem initialization"_s);
    require(!wrongThread.load(), u"Library model mutated outside GUI thread"_s);
    qInfo().noquote() << u"PASS library favorites=ok search_retained=ok exact_preference=ok pending_choice=ok nested_model=ok error_order=ok typed_lookup=ok delayed_load=ok explicit_loading=ok worker_entry=ok loading_close=ok startup_failure=ok fresh_inspection=ok missing_exact=ok stale_completion=ok history_absent=ok reopen=ok io_failure=ok reentrant_preference=ok close_pending=ok variants=%1"_s.arg(variantCount);
}
