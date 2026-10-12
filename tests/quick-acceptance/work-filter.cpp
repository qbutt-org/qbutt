// SPDX-License-Identifier: GPL-2.0-or-later
#include "work-filter.h"
#include "group-entry.h"

#include <functional>
#include <memory>
#include <stdexcept>

#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QPersistentModelIndex>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

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

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 10'000))
        {
            QEventLoop loop;
            QTimer::singleShot(10, &loop, &QEventLoop::quit);
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
        }, u"Generated library load"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());
    }

    Qbutt::WorkModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<Qbutt::WorkModel *>(controller.property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Missing production WorkModel"_s);
        return result;
    }

    QJsonObject generatedEntry(const int number, const bool favorite)
    {
        const bool manga = (number % 2) == 0;
        const QString id = manga ? u"00000000-0000-0000-0000-%1"_s.arg(number, 12, 10, u'0')
            : QString::number(900'000 + number);
        const QJsonObject publication {{u"source"_s, manga ? u"manga_dex"_s : u"ani_liberty"_s}, {u"id"_s, id}};
        const QJsonObject selector = manga
            ? QJsonObject {{u"manga_translation"_s, QJsonObject {{u"language"_s, u"en"_s}, {u"groups"_s, QJsonArray {}}}}}
            : QJsonObject {{u"torrent"_s, QJsonObject {{u"id"_s, number}}}};
        const QJsonObject variantReference {{u"publication"_s, publication}, {u"selector"_s, selector}};
        const QJsonObject variant {
            {u"reference"_s, variantReference}, {u"title"_s, QJsonValue::Null},
            {u"transport"_s, manga ? u"manga"_s : u"torrent"_s}, {u"episode"_s, QJsonValue::Null},
            {u"quality"_s, QJsonValue::Null}, {u"translation"_s, QJsonValue::Null}, {u"languages"_s, QJsonArray {}},
            {u"player"_s, QJsonValue::Null}, {u"size"_s, QJsonValue::Null}, {u"seeders"_s, QJsonValue::Null},
            {u"info_hash"_s, QJsonValue::Null}, {u"reported_video_ads"_s, false}
        };
        const QJsonObject work {
            {u"reference"_s, publication}, {u"title"_s, u"Generated publication %1"_s.arg(number)},
            {u"category"_s, manga ? u"manga"_s : u"anime"_s}, {u"url"_s, u"https://example.com/publication/%1"_s.arg(number)},
            {u"cover"_s, QJsonValue::Null}, {u"description"_s, QJsonValue::Null}, {u"year"_s, 2026},
            {u"aliases"_s, QJsonArray {}}, {u"identifiers"_s, QJsonObject {}}, {u"variants"_s, QJsonArray {variant}},
            {u"needs_inspection"_s, true}
        };
        QJsonObject filters;
        for (const auto *key : {"episode", "translation", "source", "language", "player", "transport"})
            filters.insert(QString::fromLatin1(key), QJsonValue::Null);
        return {
            {u"group"_s, GroupEntry::single(work)}, {u"preferences"_s, QJsonObject {{u"selected"_s, variantReference}, {u"filters"_s, filters}}},
            {u"favorite"_s, favorite}, {u"last_opened"_s, QJsonObject {{u"variant"_s, variantReference}, {u"at_unix_millis"_s, number * 1000}}},
            {u"reader_progress"_s, QJsonValue::Null}
        };
    }

    void writeEntry(QSqlDatabase &database, const int number, const bool favorite)
    {
        const auto metadata = generatedEntry(number, favorite);
        QSqlQuery query(database);
        require(query.prepare(u"INSERT INTO library VALUES (?, ?, ?)"_s), query.lastError().text());
        const auto reference = metadata.value(u"group"_s).toObject().value(u"anchor"_s).toObject();
        query.addBindValue(u"{\"source\":\"%1\",\"id\":\"%2\"}"_s.arg(reference.value(u"source"_s).toString(),
            reference.value(u"id"_s).toString()));
        query.addBindValue(QString::fromUtf8(QJsonDocument(metadata).toJson(QJsonDocument::Compact)));
        query.addBindValue(number);
        require(query.exec(), query.lastError().text());
    }

    void key(QQuickItem *item, const Qt::Key key)
    {
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(item, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(item, &release);
    }
}

void exerciseWorkFilter()
{
    QTemporaryDir profile;
    require(profile.isValid(), u"Work filter profile failed"_s);
    const auto drainOwners = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(10'000))
            qFatal("Work filter SQLite owner did not drain");
    });
    {
        Qbutt::ContentController empty;
        readyLibrary(empty, profile.path());
    }
    require(QThreadPool::globalInstance()->waitForDone(10'000), u"Empty library owner did not drain"_s);

    const QString connection = u"native-work-filter-fixture"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = QSqlDatabase();
        QSqlDatabase::removeDatabase(connection);
    });
    database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    for (int number = 1; number <= 5; ++number)
        writeEntry(database, number, true);

    auto controller = std::make_unique<Qbutt::ContentController>();
    auto *favorites = model(*controller, "favorites");
    auto *history = model(*controller, "history");
    auto *works = model(*controller, "works");
    QQmlEngine engine;
    engine.rootContext()->setContextProperty(u"filterController"_s, controller.get());
    int warnings = 0;
    QObject::connect(&engine, &QQmlEngine::warnings, &engine, [&](const QList<QQmlError> &) { ++warnings; });
    QQmlComponent component(&engine);
    component.setData(R"(
        import QtQuick
        import Qbutt.Content
        import Qbutt.Views
        Item {
            id: root
            width: 400
            height: 300
            property string categoryPattern: "^manga$"
            property ContentController backend: filterController
            property WorkModel currentSource: backend ? backend.favorites : null
            property string chosenWorkId: ""
            readonly property var proxyModel: filtered
            WorkFilterModel {
                id: filtered
                sourceModel: root.currentSource
                filterRegularExpression: new RegExp(root.categoryPattern)
            }
            ListView {
                objectName: "filteredList"
                anchors.fill: parent
                model: filtered
                delegate: Item {
                    required property string workId
                    required property string category
                    width: 400
                    height: 32
                }
                Keys.onDownPressed: incrementCurrentIndex()
                Keys.onReturnPressed: root.chosenWorkId = filtered.idAt(currentIndex)
            }
        }
    )", QUrl());
    std::unique_ptr<QObject> view(component.create());
    require(view != nullptr, component.errorString());
    auto *proxy = qobject_cast<Content::WorkFilterModel *>(view->property("proxyModel").value<QObject *>());
    auto *list = view->findChild<QQuickItem *>(u"filteredList"_s);
    require(proxy && list && (proxy->sourceModel() == favorites), u"QML proxy source binding failed"_s);
    require(proxy->filterRegularExpression().pattern() == u"^manga$"_s, u"QML regular expression binding failed"_s);
    int inserted = 0;
    QObject::connect(proxy, &QAbstractItemModel::rowsInserted, view.get(), [&](const QModelIndex &, const int first, const int last)
    {
        inserted += last - first + 1;
    });
    readyLibrary(*controller, profile.path());
    require((favorites->rowCount(QModelIndex()) == 5) && (history->rowCount(QModelIndex()) == 5)
        && (proxy->rowCount() == 2) && (inserted == 2), u"Native library inserts did not filter incrementally"_s);
    const QString firstManga = favorites->idAt(1);
    const QString secondManga = favorites->idAt(3);
    require((proxy->idAt(0) == firstManga) && (proxy->idAt(1) == secondManga)
        && (proxy->indexOfId(firstManga) == 0) && (proxy->indexOfId(secondManga) == 1)
        && (proxy->indexOfId(favorites->idAt(0)) == -1) && (proxy->indexOfId(u"missing"_s) == -1)
        && (proxy->indexOfId(QString()) == -1) && proxy->idAt(-1).isEmpty() && proxy->idAt(2).isEmpty(),
        u"Proxy lookup used source coordinates or accepted invalid identity"_s);
    require((proxy->mapToSource(proxy->index(0, 0)).row() == 1)
        && (proxy->mapFromSource(favorites->index(0, 0)).isValid() == false)
        && (proxy->roleNames() == favorites->roleNames()), u"Qt identity or role mapping changed"_s);
    waitFor([&] { return list->property("count").toInt() == 2; }, u"QML filtered list"_s);
    list->setProperty("currentIndex", 0);
    key(list, Qt::Key_Down);
    key(list, Qt::Key_Return);
    require((list->property("currentIndex").toInt() == 1) && (view->property("chosenWorkId").toString() == secondManga),
        u"Keyboard selection did not use exact proxy identity"_s);

    QPersistentModelIndex retained(proxy->index(0, 0));
    view->setProperty("categoryPattern", u"^anime$"_s);
    require((proxy->rowCount() == 3) && (proxy->indexOfId(firstManga) == -1) && !retained.isValid(),
        u"Category switch retained hidden manga indices"_s);
    for (const auto &pattern : {u"^man$"_s, u"^Manga$"_s, u"^missing$"_s})
    {
        view->setProperty("categoryPattern", pattern);
        require(proxy->rowCount() == 0, u"Exact category filter accepted a substring, case mismatch or absent key"_s);
    }
    view->setProperty("categoryPattern", QString());
    require((proxy->rowCount() == 5) && (favorites->rowCount(QModelIndex()) == 5), u"Empty filter lost accumulated source rows"_s);
    for (int row = 0; row < 5; ++row)
        require(proxy->idAt(row) == favorites->idAt(row), u"Empty filter changed source order"_s);

    view->setProperty("currentSource", QVariant::fromValue(works));
    require((proxy->sourceModel() == works) && (proxy->rowCount() == 0), u"Source switch retained old library rows"_s);
    view->setProperty("categoryPattern", u"^manga$"_s);
    view->setProperty("currentSource", QVariant::fromValue(history));
    require((proxy->rowCount() == 2) && (proxy->idAt(0) == firstManga), u"History source switch changed exact filter"_s);
    int resets = 0;
    QObject::connect(proxy, &QAbstractItemModel::modelReset, view.get(), [&] { ++resets; });
    writeEntry(database, 6, false);
    require(controller->refreshHistory(), u"History refresh rejected"_s);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"History snapshot update"_s);
    require((resets == 1) && (history->rowCount(QModelIndex()) == 6) && (proxy->rowCount() == 3)
        && (proxy->idAt(1) == firstManga), u"Production snapshot reset did not rebuild proxy mappings"_s);
    const QString newManga = proxy->idAt(0);
    require(controller->removeHistory(newManga), u"Exact history removal rejected"_s);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"History removal"_s);
    require((history->rowCount(QModelIndex()) == 5) && (proxy->rowCount() == 2)
        && (proxy->indexOfId(newManga) == -1) && (favorites->rowCount(QModelIndex()) == 5),
        u"Removal retained a hidden identity or changed the separate favorite collection"_s);
    view->setProperty("categoryPattern", QString());
    require(proxy->rowCount() == 5, u"Filter restoration lost confirmed History"_s);
    view->setProperty("currentSource", QVariant::fromValue<Qbutt::WorkModel *>(nullptr));
    require(!proxy->sourceModel() && (proxy->rowCount() == 0) && proxy->idAt(0).isEmpty(), u"Null source retained rows"_s);
    view->setProperty("currentSource", QVariant::fromValue(favorites));
    require(proxy->rowCount() == 5, u"Source restoration lost its accumulated collection"_s);
    controller.reset();
    require(!proxy->sourceModel() && (proxy->rowCount() == 0), u"Destroyed source was retained by proxy"_s);
    require(warnings == 0, u"QML work filtering emitted warnings"_s);
    qInfo("PASS work_filter production_library=qml exact_identity=ok keyboard=ok incremental=ok reset=ok source_switch=ok no_copy=ok destruction=ok");
}
