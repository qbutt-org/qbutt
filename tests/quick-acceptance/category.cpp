// SPDX-License-Identifier: GPL-2.0-or-later
#include "category.h"
#include "group-entry.h"

#include <functional>
#include <memory>
#include <stdexcept>
#include <utility>

#include <QAccessible>
#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QQmlEngine>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QStringList>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

#include "gui/quick/contentview.h"
#include "gui/quick/qml/workfiltermodel.h"
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

    void waitFor(const std::function<bool()> &ready, const QString &operation, const int timeout = 10'000)
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

    void readyLibrary(Qbutt::ContentController &controller, const QString &profile)
    {
        controller.initLibrary(profile);
        waitFor([&]
        {
            const auto state = controller.property("libraryState").value<Qbutt::Library::LibraryState>();
            return (state == Qbutt::Library::LibraryState::Ready) || (state == Qbutt::Library::LibraryState::Failed);
        }, u"Generated profile creation"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());
    }

    Qbutt::WorkModel *model(Qbutt::ContentController *controller, const char *name)
    {
        auto *result = qobject_cast<Qbutt::WorkModel *>(controller->property(name).value<QObject *>());
        require(result, u"Production typed WorkModel is missing"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const int role = model->roleNames().key(name, -1);
        require(role >= 0, u"Native model role is missing"_s);
        return model->data(model->index(row, 0), role).toString();
    }

    QStringList identities(QAbstractItemModel *model)
    {
        QStringList result;
        for (int row = 0; row < model->rowCount(); ++row)
            result.append(value(model, row, "workId"));
        return result;
    }

    QJsonObject generatedEntry(const int number)
    {
        const QString category = QStringList {u"books"_s, u"games"_s, u"software"_s}.at((number - 1) % 3);
        const QString text = uR"({
            "group": {
                "anchor": {"source": "ru_tor", "id": "%1"},
                "references": [{"source": "ru_tor", "id": "%1"}],
                "publications": [{
                    "reference": {"source": "ru_tor", "id": "%1"},
                    "title": "Generated category publication %1", "category": "%2",
                    "url": "https://example.com/generated/%1", "cover": null, "description": null,
                    "year": 2026, "aliases": [], "identifiers": {}, "needs_inspection": true,
                    "variants": [{
                        "reference": {"publication": {"source": "ru_tor", "id": "%1"},
                            "selector": {"torrent_hash": {"info_hash": "%3"}}},
                        "title": null, "transport": "torrent", "episode": null, "quality": null,
                        "translation": null, "languages": [], "player": null, "size": null,
                        "seeders": null, "info_hash": "%3", "reported_video_ads": false
                    }]
                }]
            },
            "preferences": {
                "selected": {"publication": {"source": "ru_tor", "id": "%1"},
                    "selector": {"torrent_hash": {"info_hash": "%3"}}},
                "filters": {"episode": null, "translation": null, "source": null,
                    "language": null, "player": null, "transport": null}
            },
            "favorite": true, "reader_progress": null,
            "last_opened": {"variant": {"publication": {"source": "ru_tor", "id": "%1"},
                "selector": {"torrent_hash": {"info_hash": "%3"}}}, "at_unix_millis": %4}
        })"_s.arg(QString::number(900'000'000 + number), category,
            QString::number(number, 16).rightJustified(40, u'0'), QString::number(number * 1000));
        const auto document = QJsonDocument::fromJson(text.toUtf8());
        require(document.isObject(), u"Generated legal metadata is invalid"_s);
        return document.object();
    }

    QString entryKey(const QJsonObject &metadata)
    {
        const auto reference = metadata.value(u"group"_s).toObject().value(u"anchor"_s).toObject();
        return u"{\"source\":\"%1\",\"id\":\"%2\"}"_s.arg(reference.value(u"source"_s).toString(),
            reference.value(u"id"_s).toString());
    }

    void writeEntry(QSqlDatabase &database, const QJsonObject &metadata, const int updatedAt)
    {
        QSqlQuery query(database);
        require(query.prepare(u"INSERT INTO library VALUES (?, ?, ?) ON CONFLICT (reference) "
            "DO UPDATE SET entry = excluded.entry, updated_at = excluded.updated_at"_s), query.lastError().text());
        query.addBindValue(entryKey(metadata));
        query.addBindValue(QString::fromUtf8(QJsonDocument(metadata).toJson(QJsonDocument::Compact)));
        query.addBindValue(updatedAt);
        require(query.exec(), query.lastError().text());
    }

    QJsonObject savedEntry(QSqlDatabase &database, const QString &publication)
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT entry FROM library"_s), query.lastError().text());
        while (query.next())
        {
            const auto metadata = QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
            const auto reference = metadata.value(u"group"_s).toObject().value(u"anchor"_s).toObject();
            if (u"rutor:"_s + reference.value(u"id"_s).toString() == publication)
                return metadata;
        }
        throw std::runtime_error("Confirmed exact RuTor entry is missing");
    }

    void settled(Qbutt::ContentController *controller)
    {
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Library publication"_s);
        require(controller->property("libraryErrorText").toString().isEmpty(), controller->property("libraryErrorText").toString());
    }

    struct Screen
    {
        Content::ContentView &view;
        Qbutt::ContentController *controller;
        QQuickItem *category;
        QQuickItem *results;
        QQuickItem *pane;
        QQuickItem *empty;
        Content::WorkFilterModel *proxy;

        explicit Screen(Content::ContentView &contentView)
            : view {contentView}
            , controller {view.findChild<Qbutt::ContentController *>()}
        {
            waitFor([&] { return view.status() != QQuickWidget::Loading; }, u"Production ContentView load"_s);
            require(controller && (view.status() == QQuickWidget::Ready) && view.rootObject(), u"Production category screen failed"_s);
            waitFor([&]
            {
                return controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready;
            }, u"Production library startup"_s);
            category = view.rootObject()->findChild<QQuickItem *>(u"contentCategory"_s);
            results = view.rootObject()->findChild<QQuickItem *>(u"contentResults"_s);
            pane = view.rootObject()->findChild<QQuickItem *>(u"contentWorkPane"_s);
            empty = view.rootObject()->findChild<QQuickItem *>(u"contentEmptyResults"_s);
            require(category && results && pane && empty, u"Production category controls are missing"_s);
            proxy = qobject_cast<Content::WorkFilterModel *>(results->property("model").value<QObject *>());
            require(proxy && (category->property("count").toInt() == 8), u"Production category proxy or selector is invalid"_s);
        }

        void choose(const int index, const QString &key)
        {
            require(category->setProperty("currentIndex", index) && (category->property("currentValue").toString() == key),
                u"Category selector did not expose its exact key"_s);
        }

        void selection(const QString &id, const bool visible)
        {
            try
            {
                waitFor([&]
                {
                    const int expected = proxy->indexOfId(id);
                    return (results->property("currentIndex").toInt() == expected)
                        && (pane->property("selectedInList").toBool() == visible);
                }, u"Exact visible work selection"_s);
            }
            catch (const std::exception &)
            {
                throw std::runtime_error(u"Exact selection failed: work=%1 category=%2 mode=%3 current=%4 expected=%5 pane=%6 visible=%7"_s
                    .arg(id, category->property("currentValue").toString())
                    .arg(static_cast<int>(controller->property("listMode").value<Qbutt::Search::ListMode>()))
                    .arg(results->property("currentIndex").toInt()).arg(proxy->indexOfId(id))
                    .arg(pane->property("selectedInList").toBool()).arg(visible).toStdString());
            }
            auto *details = pane->property("contentItem").value<QQuickItem *>();
            require(details && (details->isVisible() == visible), u"Hidden selection retained its details"_s);
            require(controller->property("selectedWorkId").toString() == id, u"Filtering replaced backend work identity"_s);
        }

        void visibleRows(const QString &key)
        {
            require(QMetaObject::invokeMethod(results, "forceLayout"), u"Category list layout failed"_s);
            require(results->property("count").toInt() == proxy->rowCount(), u"Visible category count differs from proxy"_s);
            int rendered = 0;
            for (int row = 0; row < proxy->rowCount(); ++row)
            {
                require(value(proxy, row, "category") == key, u"Category filter accepted a different key"_s);
                QQuickItem *delegate = nullptr;
                require(QMetaObject::invokeMethod(results, "itemAtIndex", Q_RETURN_ARG(QQuickItem *, delegate), Q_ARG(int, row)),
                    u"Production category row lookup failed"_s);
                if (!delegate)
                    continue;
                ++rendered;
                for (const auto *role : {"workId", "title", "source", "category", "year"})
                    require(delegate->property(role).toString() == value(proxy, row, role), u"Filtered delegate lost a native role"_s);
                auto *accessible = QAccessible::queryAccessibleInterface(delegate);
                require(accessible && (accessible->text(QAccessible::Name) == value(proxy, row, "title")),
                    u"Filtered category delegate lost its accessible title"_s);
            }
            require((proxy->rowCount() == 0) || (rendered > 0), u"Nonempty category did not render any actual delegate"_s);
        }
    };

    void key(Content::ContentView &view, const Qt::Key key)
    {
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(&view, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(&view, &release);
    }

    void storedLists(Screen &screen, QSqlDatabase &database)
    {
        auto *favorites = model(screen.controller, "favorites");
        auto *history = model(screen.controller, "history");
        const auto favoriteIds = identities(favorites);
        const auto historyIds = identities(history);
        require((favoriteIds.size() == 6) && (historyIds.size() == 6) && (favoriteIds != historyIds),
            u"Equal-count stored sources need distinct confirmed orders"_s);
        screen.controller->setListMode(Qbutt::Search::ListMode::Favorites);
        for (const auto &[index, category] : {std::pair {7, u"books"_s}, {5, u"games"_s}, {6, u"software"_s}})
        {
            screen.choose(index, category);
            waitFor([&] { return screen.results->property("count").toInt() == 2; }, u"Equal-count category switch"_s);
            screen.visibleRows(category);
            require((screen.proxy->sourceModel() == favorites) && (identities(favorites) == favoriteIds)
                && (identities(history) == historyIds), u"Category filtering damaged stored source collections"_s);
        }
        screen.choose(7, u"books"_s);
        screen.visibleRows(u"books"_s);
        const QString selected = u"rutor:900000001"_s;
        screen.controller->selectWork(selected);
        const QString exact = screen.controller->property("selectedVariantId").toString();
        require(!exact.isEmpty(), u"Stored exact preference is missing"_s);
        screen.selection(selected, true);
        const auto preferences = savedEntry(database, selected).value(u"preferences"_s);
        const int lastRow = screen.proxy->rowCount() - 1;
        const QString lastBook = screen.proxy->idAt(lastRow);
        require(favorites->indexOfId(lastBook) != lastRow, u"Keyboard fixture needs distinct source and proxy indices"_s);
        screen.results->forceActiveFocus();
        key(screen.view, Qt::Key_End);
        screen.selection(lastBook, true);
        const QString lastHash = savedEntry(database, lastBook).value(u"preferences"_s).toObject()
            .value(u"selected"_s).toObject().value(u"selector"_s).toObject().value(u"torrent_hash"_s).toObject()
            .value(u"info_hash"_s).toString();
        require(screen.controller->property("selectedVariantId").toString() == lastBook + u":torrent:"_s + lastHash,
            u"Keyboard selection retained another exact variant"_s);
        key(screen.view, Qt::Key_Home);
        screen.selection(selected, true);
        screen.choose(5, u"games"_s);
        screen.selection(selected, false);
        require(screen.proxy->rowCount() == 2, u"Hidden category switch changed the equal-count fixture"_s);
        screen.choose(7, u"books"_s);
        screen.selection(selected, true);
        require((screen.controller->property("selectedVariantId").toString() == exact)
            && (savedEntry(database, selected).value(u"preferences"_s) == preferences), u"Hidden/back filter changed exact preferences"_s);
        for (const auto mode : {Qbutt::Search::ListMode::History, Qbutt::Search::ListMode::Favorites,
            Qbutt::Search::ListMode::History})
        {
            screen.controller->setListMode(mode);
            screen.selection(selected, true);
            require((screen.proxy->rowCount() == 2) && (screen.proxy->sourceModel()
                == (mode == Qbutt::Search::ListMode::History ? history : favorites)), u"Equal-count source switch used another collection"_s);
            screen.visibleRows(u"books"_s);
        }
        auto changed = generatedEntry(1);
        auto work = GroupEntry::anchorWork(changed);
        work.insert(u"category"_s, u"software"_s);
        GroupEntry::replaceAnchorWork(changed, work);
        writeEntry(database, changed, 6);
        require(screen.controller->refreshHistory(), u"Category snapshot refresh rejected"_s);
        settled(screen.controller);
        screen.selection(selected, false);
        require((history->rowCount(QModelIndex()) == 6) && (screen.proxy->rowCount() == 1),
            u"Updated source category did not refilter the real snapshot"_s);
        screen.choose(6, u"software"_s);
        screen.selection(selected, true);
        screen.visibleRows(u"software"_s);
        for (int number = 1; number <= 6; ++number)
        {
            auto metadata = generatedEntry(number);
            metadata.insert(u"last_opened"_s, QJsonValue::Null);
            writeEntry(database, metadata, 7 - number);
        }
        require(screen.controller->refreshHistory(), u"Empty History refresh rejected"_s);
        settled(screen.controller);
        screen.choose(0, QString());
        waitFor([&] { return screen.empty->isVisible(); }, u"Empty History presentation"_s);
        require(screen.empty->property("text").toString() == QCoreApplication::translate("SearchPage", "History is empty"),
            u"All-category History used category-empty wording"_s);
        screen.choose(4, u"manga"_s);
        require(screen.empty->property("text").toString() == QCoreApplication::translate("SearchPage", "No titles in this category"),
            u"Filtered empty History lost its category wording"_s);
        screen.controller->setListMode(Qbutt::Search::ListMode::Favorites);
        screen.choose(1, u"movies"_s);
        waitFor([&] { return screen.empty->isVisible(); }, u"Empty favorite category"_s);
        require((favorites->rowCount(QModelIndex()) == 6) && (screen.proxy->rowCount() == 0)
            && (screen.empty->property("text").toString() == QCoreApplication::translate("SearchPage", "No titles in this category")),
            u"Empty favorite category erased its source or used the wrong wording"_s);
        screen.choose(0, QString());
        waitFor([&] { return (screen.proxy->rowCount() == 6) && !screen.empty->isVisible(); },
            u"All-category Favorites restoration"_s);
    }

    QString liveSearch(Screen &screen)
    {
        auto *works = model(screen.controller, "works");
        screen.controller->setListMode(Qbutt::Search::ListMode::Search);
        screen.choose(0, QString());
        int resets = 0;
        int searching = 0;
        bool filteredDuringSearch = false;
        QObject observer;
        QObject::connect(works, &QAbstractItemModel::modelReset, &observer, [&] { ++resets; });
        QObject::connect(screen.controller, &Qbutt::ContentController::searchStateChanged, &observer, [&]
        {
            searching += screen.controller->property("busy").toBool() ? 1 : 0;
        });
        QObject::connect(works, &QAbstractItemModel::rowsInserted, &observer, [&]
        {
            if (!filteredDuringSearch && screen.controller->property("busy").toBool())
            {
                const auto before = identities(works);
                screen.choose(5, u"games"_s);
                screen.choose(6, u"software"_s);
                require(identities(works) == before, u"Filtering during search removed source rows"_s);
                filteredDuringSearch = true;
            }
        });
        auto *query = screen.view.rootObject()->findChild<QQuickItem *>(u"contentQuery"_s);
        auto *search = screen.view.rootObject()->findChild<QQuickItem *>(u"contentSearch"_s);
        require(query && search && query->setProperty("text", u"Blender"_s)
            && QMetaObject::invokeMethod(search, "clicked"), u"Production search action did not run"_s);
        waitFor([&] { return !screen.controller->property("busy").toBool(); }, u"Live incremental category search"_s, 150'000);
        require(filteredDuringSearch && (searching == 1) && (resets == 0)
            && screen.controller->property("errorText").toString().isEmpty(), u"Category selection restarted or damaged active search"_s);
        require((screen.proxy->sourceModel() == works) && (screen.proxy->rowCount() > 0), u"Live software category is empty"_s);
        const auto all = identities(works);
        const auto software = identities(screen.proxy);
        screen.visibleRows(u"software"_s);
        screen.choose(0, QString());
        require((identities(works) == all) && (identities(screen.proxy) == all), u"All filter lost live accumulated results"_s);
        screen.choose(6, u"software"_s);
        require(identities(screen.proxy) == software, u"Software filter lost its previous results"_s);
        screen.results->forceActiveFocus();
        key(screen.view, Qt::Key_Home);
        const QString publication = screen.proxy->idAt(0);
        waitFor([&] { return !screen.controller->property("inspecting").toBool(); }, u"Exact live software inspection"_s, 50'000);
        screen.selection(publication, true);
        require(publication.startsWith(u"rutor:"_s) && screen.controller->property("errorText").toString().isEmpty()
            && (screen.controller->property("selectedWorkCategory").toString() == u"software"_s), u"Keyboard proxy selection inspected another publication"_s);
        screen.controller->setSelectedFavorite(true);
        settled(screen.controller);
        const QString exact = screen.controller->property("selectedVariantId").toString();
        screen.choose(5, u"games"_s);
        screen.selection(publication, false);
        screen.choose(6, u"software"_s);
        screen.selection(publication, true);
        require(screen.controller->property("selectedVariantId").toString() == exact, u"Live hidden/back category changed exact torrent preference"_s);
        return publication;
    }
}

void exerciseCategory()
{
    QAccessible::setActive(true);
    QTemporaryDir profile;
    require(profile.isValid(), u"Category profile creation failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Category fixture owners did not drain");
    });
    {
        Qbutt::ContentController creator;
        readyLibrary(creator, profile.path());
    }
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Initial profile owner did not drain"_s);
    const QString connection = u"production-category-fixture"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = QSqlDatabase();
        QSqlDatabase::removeDatabase(connection);
    });
    database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    for (int number = 1; number <= 6; ++number)
        writeEntry(database, generatedEntry(number), 7 - number);
    int warnings = 0;
    QString publication;
    {
        Content::ContentView view(profile.path());
        view.resize(1440, 900);
        view.show();
        QObject::connect(view.engine(), &QQmlEngine::warnings, &view, [&](const QList<QQmlError> &) { ++warnings; });
        Screen screen(view);
        storedLists(screen, database);
        publication = liveSearch(screen);
    }
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Category search owner did not drain"_s);
    auto cached = savedEntry(database, publication);
    const auto preferences = cached.value(u"preferences"_s);
    auto work = GroupEntry::anchorWork(cached);
    work.insert(u"category"_s, u"books"_s);
    GroupEntry::replaceAnchorWork(cached, work);
    writeEntry(database, cached, 20);
    {
        Content::ContentView view(profile.path());
        view.resize(1280, 720);
        view.show();
        QObject::connect(view.engine(), &QQmlEngine::warnings, &view, [&](const QList<QQmlError> &) { ++warnings; });
        Screen screen(view);
        screen.controller->setListMode(Qbutt::Search::ListMode::Favorites);
        screen.choose(7, u"books"_s);
        require(screen.proxy->indexOfId(publication) >= 0, u"Saved category fixture was not loaded"_s);
        auto *favorites = model(screen.controller, "favorites");
        int changed = 0;
        QObject observer;
        QObject::connect(favorites, &QAbstractItemModel::dataChanged, &observer, [&] { ++changed; });
        screen.controller->selectWork(publication);
        require(screen.controller->property("selectedWorkCategory").toString() == u"books"_s,
            u"Saved category was silently substituted before inspection"_s);
        waitFor([&] { return !screen.controller->property("inspecting").toBool(); }, u"Fresh authoritative category"_s, 50'000);
        settled(screen.controller);
        require(screen.controller->property("errorText").toString().isEmpty() && (changed > 0)
            && (screen.controller->property("selectedWorkCategory").toString() == u"software"_s)
            && (value(favorites, favorites->indexOfId(publication), "category") == u"software"_s),
            u"Fresh inspection did not update the native source category"_s);
        screen.selection(publication, false);
        screen.choose(6, u"software"_s);
        screen.selection(publication, true);
        require(savedEntry(database, publication).value(u"preferences"_s) == preferences,
            u"Category enrichment replaced exact saved variant preferences"_s);
        screen.visibleRows(u"software"_s);
        const auto favoriteIds = identities(favorites);
        for (const auto &id : favoriteIds)
        {
            screen.controller->selectWork(id);
            screen.controller->setSelectedFavorite(false);
            settled(screen.controller);
        }
        screen.choose(0, QString());
        waitFor([&] { return (favorites->rowCount(QModelIndex()) == 0) && screen.empty->isVisible(); },
            u"Empty Favorites presentation"_s);
        require(screen.empty->property("text").toString() == QCoreApplication::translate("SearchPage", "No favorites"),
            u"All-category empty Favorites used category-empty wording"_s);
        screen.choose(6, u"software"_s);
        require(screen.empty->property("text").toString() == QCoreApplication::translate("SearchPage", "No titles in this category"),
            u"Filtered empty Favorites lost its category wording"_s);
    }
    require(warnings == 0, u"Category interaction emitted QML warnings"_s);
    qInfo("PASS category production_ui=ok stored_equal_count=ok exact_hidden_restore=ok source_reset=ok live_incremental=ok no_search_restart=ok keyboard_proxy=ok data_changed=ok empty_semantics=ok preferences=ok");
}
