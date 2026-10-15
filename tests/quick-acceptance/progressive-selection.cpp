// SPDX-License-Identifier: GPL-2.0-or-later
#include "progressive-selection.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
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

    QAbstractItemModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller.property(name).value<QObject *>());
        require(result, u"Production model is missing"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &role)
    {
        const int id = model->roleNames().key(role, -1);
        require(id >= 0, u"Production role is missing"_s);
        return model->data(model->index(row, 0), id).toString();
    }

    QJsonArray savedEntries(const QString &profile)
    {
        const QString connection = u"progressive-split-fixture"_s;
        const auto removeConnection = qScopeGuard([&] { QSqlDatabase::removeDatabase(connection); });
        auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
        database.setDatabaseName(QDir(profile).filePath(u"content-library.sqlite"_s));
        require(database.open(), database.lastError().text());
        QSqlQuery query(database);
        require(query.exec(u"SELECT entry FROM library"_s), query.lastError().text());
        QJsonArray entries;
        while (query.next())
            entries.append(QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object());
        return entries;
    }
}

void exerciseProgressiveSelection()
{
    const bool failure = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_FAILURE") == 1;
    const bool initial = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_INITIAL") == 1;
    const bool missing = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_MISSING") == 1;
    const bool split = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_SPLIT") == 1;
    const bool splitAnchor = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_SPLIT_ANCHOR") == 1;
    const bool pointer = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_POINTER") == 1;
    QTemporaryDir profile;
    require(profile.isValid(), u"Progressive profile failed"_s);
    const auto drainOwners = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(10'000))
            qFatal("Progressive library owner did not drain");
    });
    Content::ContentView view(profile.path());
    const QString renderDirectory = qEnvironmentVariable("QBUTT_PROGRESSIVE_RENDER_DIR");
    if (!renderDirectory.isEmpty())
        require(QDir().mkpath(renderDirectory), u"Render directory is unavailable"_s);
    if (pointer || !renderDirectory.isEmpty())
    {
        view.setWindowTitle(u"qbutt — progressive variants"_s);
        view.resize(1440, 900);
        view.show();
    }
    auto *controller = view.findChild<Qbutt::ContentController *>(u"contentController"_s);
    require(controller, u"Production controller is missing"_s);
    waitFor([&]
    {
        return controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
            == Qbutt::Sources::SettingsState::Ready;
    }, u"Source settings"_s);

    auto *sources = model(*controller, "sources");
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString id = value(sources, row, "sourceId");
        if ((id != u"aniliberty"_s) && (id != u"yani"_s))
            require(controller->setSourceEnabled(id, false), u"Source isolation failed"_s);
    }
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Source writes"_s);

    auto *works = model(*controller, "works");
    auto *variants = model(*controller, "variants");
    controller->search(u"Generated progressive"_s);
    waitFor([&] { return works->rowCount() == 1; }, u"Fast search publication"_s);
    if (initial)
        waitFor([&] { return !controller->property("busy").toBool(); }, u"Both search publications"_s);
    const QString work = value(works, 0, "workId");
    controller->selectWork(work);
    if (qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_EMPTY") == 1)
    {
        waitFor([&]
        {
            return !controller->property("inspecting").toBool() && (works->rowCount() == 0);
        }, u"Confirmed empty publication removal"_s);
        require(controller->property("selectedWorkId").toString().isEmpty()
            && (variants->rowCount() == 0), u"Removed empty work retained a dead selection"_s);
        qInfo("FIXTURE release_late_search");
        waitFor([&] { return works->rowCount() == 1; }, u"Late pending publication restores the work"_s);
        controller->selectWork(value(works, 0, "workId"));
        qInfo("FIXTURE release_late_inspection");
        waitFor([&]
        {
            return !controller->property("inspecting").toBool() && (variants->rowCount() == 1);
        }, u"Working publication beside confirmed empty source"_s);
        require((works->rowCount() == 1)
            && (controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
                == Qbutt::Search::VariantSelectionState::Available),
            u"Empty source hid the working alternative"_s);
        require(controller->setSelectedFavorite(true), u"Mixed work favorite rejected"_s);
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Mixed work favorite save"_s);
        const auto before = savedEntries(profile.path());
        controller->setListMode(Qbutt::Search::ListMode::Favorites);
        controller->selectWork(value(model(*controller, "favorites"), 0, "workId"));
        qInfo("FIXTURE release_cached_late");
        waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Favorite fresh inspection"_s);
        require((model(*controller, "favorites")->rowCount() == 1)
            && (savedEntries(profile.path()) == before), u"Empty source changed the saved library"_s);
        controller->setListMode(Qbutt::Search::ListMode::Search);
        require(works->rowCount() == 1, u"Search return lost the working alternative"_s);
        qInfo("PASS progressive_selection confirmed_empty=hidden dead_selection=cleared late_pending=visible mixed_group=retained favorites_and_library=unchanged");
        return;
    }
    waitFor([&]
    {
        return (controller->property("inspecting").toBool() == initial) && (variants->rowCount() == 2);
    }, u"Fast publication variants"_s);
    require(controller->property("selectedVariantId").toString() == value(variants, 0, "variantId"),
        u"First ready variant was not selected"_s);
    const QString selected = value(variants, 1, "variantId");
    controller->selectVariant(selected);
    qInfo("FIXTURE release_late_search");
    waitFor([&] { return controller->property("inspecting").toBool(); }, u"Late inspection started"_s);
    require(controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
        == Qbutt::Search::VariantSelectionState::Available,
        u"A pending source blocked the ready variant"_s);
    auto *download = view.rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
    require(download && download->isEnabled(), u"Ready variant action is disabled"_s);
    if (!renderDirectory.isEmpty())
    {
        QEventLoop frame;
        QTimer::singleShot(200, &frame, &QEventLoop::quit);
        frame.exec();
        require(view.grabFramebuffer().save(QDir(renderDirectory).filePath(u"ready-during-inspection.png"_s)),
            u"Progressive product render failed"_s);
    }

    int handoffs = 0;
    QString magnet;
    const QMetaObject::Connection capture = QObject::connect(controller, &Qbutt::ContentController::torrentReady,
        [&handoffs, &magnet](const QString &url)
        {
            ++handoffs;
            magnet = url;
        });
    const auto disconnectCapture = qScopeGuard([&] { QObject::disconnect(capture); });
    if (pointer)
        qInfo("FIXTURE click_ready_action");
    else
        controller->resolveVariant(selected);
    waitFor([&] { return handoffs == 1; }, u"Ready variant action during inspection"_s);
    require(magnet == u"magnet:?xt=urn:btih:0000000000000000000000000000000000009662"_s,
        u"Another variant was resolved"_s);
    require(controller->property("inspecting").toBool(), u"Inspection was cancelled by the action"_s);
    qInfo("FIXTURE release_late_inspection");

    if (failure)
    {
        waitFor([&]
        {
            for (int row = 0; row < sources->rowCount(); ++row)
            {
                if (value(sources, row, "sourceId") == u"yani"_s)
                    return value(sources, row, "failed") == u"true"_s;
            }
            return false;
        }, u"Late-publication failure"_s);
    }
    else
    {
        waitFor([&] { return variants->rowCount() == 3; }, u"Automatic late-publication inspection"_s);
    }
    waitFor([&]
    {
        return !controller->property("busy").toBool() && !controller->property("inspecting").toBool()
            && !controller->property("libraryPending").toBool();
    }, u"Progressive completion"_s);
    require(works->rowCount() == 1, u"Late source split the confirmed work"_s);
    require(controller->property("selectedWorkId").toString() == work,
        u"Late source replaced the selected work"_s);
    require(controller->property("selectedVariantId").toString() == selected,
        u"Late source replaced the exact selected variant"_s);
    require(value(variants, 1, "variantId") == selected, u"Late source reordered the selected row"_s);
    if (failure)
        require(variants->rowCount() == 2, u"Failed source replaced existing variants"_s);
    else
        require(value(variants, 2, "sourceId") == u"yani"_s, u"Late source variants are missing"_s);

    if (split)
    {
        const QString lateVariant = value(variants, 2, "variantId");
        const QString exactVariant = splitAnchor ? selected : lateVariant;
        const QString source = splitAnchor ? u"aniliberty"_s : u"yani"_s;
        const QString player = splitAnchor ? QString() : u"Kodik"_s;
        const QString episode = splitAnchor ? QString() : u"1"_s;
        const QString variantQuery = splitAnchor ? u"9662"_s : u"720"_s;
        controller->selectVariant(exactVariant);
        controller->setVariantProviderFilter(source, player);
        controller->setVariantEpisodeFilter(episode);
        controller->setVariantQuery(variantQuery);
        controller->setSelectedFavorite(true);
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Split preferences and favorite"_s);
        QJsonObject opened;
        {
            const QString connection = u"progressive-history-fixture"_s;
            const auto removeConnection = qScopeGuard([&] { QSqlDatabase::removeDatabase(connection); });
            auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
            database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
            require(database.open(), database.lastError().text());
            QSqlQuery query(database);
            require(query.exec(u"SELECT entry FROM library"_s) && query.next(), u"Saved split work is missing"_s);
            QJsonObject entry = QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
            require(entry.value(u"favorite"_s).toBool(), u"Split fixture favorite was not saved"_s);
            QJsonObject exact;
            for (const QJsonValue &work : entry.value(u"group"_s).toObject().value(u"publications"_s).toArray())
            {
                const QJsonObject publication = work.toObject();
                if (publication.value(u"reference"_s).toObject().value(u"source"_s) != u"yummy_anime"_s)
                    continue;
                const QJsonArray knownVariants = publication.value(u"variants"_s).toArray();
                require(!knownVariants.isEmpty(), u"Generated history publication has no variants"_s);
                exact = knownVariants.first().toObject().value(u"reference"_s).toObject();
                break;
            }
            require(exact.value(u"publication"_s).toObject() == QJsonObject {
                {u"source"_s, u"yummy_anime"_s}, {u"id"_s, u"9661"_s}},
                u"Generated history must use the exact selected late publication"_s);
            opened = {{u"variant"_s, exact},
                {u"at_unix_millis"_s, 123456}};
            entry.insert(u"last_opened"_s, opened);
            query.finish();
            require(query.prepare(u"UPDATE library SET entry = ?"_s), query.lastError().text());
            query.addBindValue(QString::fromUtf8(QJsonDocument(entry).toJson(QJsonDocument::Compact)));
            require(query.exec() && query.numRowsAffected() == 1, u"Generated exact history was not saved"_s);
        }
        controller->setListMode(Qbutt::Search::ListMode::Favorites);
        controller->selectWork(work);
        qInfo("FIXTURE release_cached_fast");
        qInfo("FIXTURE release_cached_late");
        waitFor([&]
        {
            return !controller->property("inspecting").toBool() && !controller->property("libraryPending").toBool();
        }, u"Conflicting publication inspection"_s);
        require(controller->property("selectedWorkId").toString() == (splitAnchor ? work : u"yani:9661"_s)
            && controller->property("selectedVariantId").toString() == exactVariant
            && controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
                == Qbutt::Search::VariantSelectionState::Available,
            u"Split did not follow the exact selected publication"_s);
        require(variants->rowCount() == (splitAnchor ? 2 : 1)
            && (splitAnchor || value(variants, 0, "variantId") == exactVariant),
            u"Split retained unrelated variants"_s);
        require(controller->property("variantSourceFilter").toString() == source
            && controller->property("variantPlayerFilter").toString() == player
            && controller->property("variantEpisodeFilter").toString() == episode
            && controller->property("variantQuery").toString() == variantQuery,
            u"Split lost the selected publication's filters"_s);

        const QJsonArray automatic = savedEntries(profile.path());
        require(automatic.size() == 1, u"Split refresh duplicated the saved library identity"_s);
        const QJsonObject original = automatic.first().toObject();
        require(original.value(u"favorite"_s).toBool() && original.value(u"last_opened"_s).toObject() == opened,
            u"Split inspection changed the favorite or exact confirmed history"_s);

        if (!splitAnchor)
        {
            controller->setListMode(Qbutt::Search::ListMode::Search);
            controller->selectWork(work);
            require(variants->rowCount() == 2 && value(variants, 0, "sourceId") == u"aniliberty"_s
                && value(variants, 1, "sourceId") == u"aniliberty"_s,
                u"Selecting the other split row restored a stale cached association"_s);
            require(controller->property("selectedVariantId").toString() != lateVariant,
                u"The other split row kept another publication's exact choice"_s);
        }
        controller->setVariantQuery(u"After split"_s);
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Explicit split preferences"_s);
        require(controller->property("libraryErrorText").toString().isEmpty(),
            u"Explicit split preferences failed"_s);
        const QJsonArray committed = savedEntries(profile.path());
        require(committed.size() == 2, u"Explicit split did not preserve separate confirmed history"_s);
        for (const QJsonValue &value : committed)
        {
            const QJsonObject entry = value.toObject();
            const QJsonObject group = entry.value(u"group"_s).toObject();
            const QString owner = group.value(u"anchor"_s).toObject().value(u"source"_s).toString();
            require(group.value(u"publications"_s).toArray().size() == 1,
                u"Explicit split retained a stale publication union"_s);
            if (owner == u"ani_liberty"_s)
                require(entry.value(u"favorite"_s).toBool() && entry.value(u"last_opened"_s).isNull(),
                    u"Explicit split changed the original favorite or substituted history"_s);
            else
                require(owner == u"yummy_anime"_s && !entry.value(u"favorite"_s).toBool()
                    && entry.value(u"last_opened"_s).toObject() == opened,
                    u"Detached exact history was not retained"_s);
        }
        controller->setListMode(Qbutt::Search::ListMode::History);
        controller->selectWork(u"yani:9661"_s);
        require(controller->property("selectedVariantId").toString() == lateVariant,
            u"History did not restore the detached exact publication"_s);
        waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Detached history inspection"_s);
        require(variants->rowCount() == 1 && controller->property("selectedVariantId").toString() == lateVariant,
            u"Detached history lost its full publication or changed exact selection"_s);
        qInfo("PASS progressive_selection split=exact filters=retained automatic_writes=none explicit_split=atomic favorite_history=retained history_reopen=exact");
        return;
    }

    controller->setSelectedFavorite(true);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Favorite write"_s);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    controller->selectWork(work);
    require(controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
        == Qbutt::Search::VariantSelectionState::Inspecting,
        u"Unverified cached variant became actionable"_s);
    controller->resolveVariant(selected);
    require(handoffs == 1 && !controller->property("resolving").toBool(),
        u"Cached variant started an action before verification"_s);
    qInfo("FIXTURE release_cached_fast");
    const auto refreshedState = missing ? Qbutt::Search::VariantSelectionState::Unavailable
                                       : Qbutt::Search::VariantSelectionState::Available;
    waitFor([&]
    {
        return controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == refreshedState;
    }, u"Exact saved variant refreshed"_s);
    require(controller->property("inspecting").toBool(), u"Slow cached source was not held"_s);
    require(controller->property("selectedVariantId").toString() == selected,
        u"Saved choice was silently substituted"_s);
    controller->resolveVariant(selected);
    if (missing)
    {
        require(handoffs == 1 && !controller->property("resolving").toBool(),
            u"Disappeared cached variant started an action"_s);
    }
    else
    {
        waitFor([&] { return handoffs == 2; }, u"Verified saved variant during slow inspection"_s);
        require(magnet.endsWith(u"9662"_s), u"Saved action resolved another variant"_s);
    }
    qInfo("FIXTURE release_cached_late");
    waitFor([&]
    {
        return !controller->property("inspecting").toBool() && !controller->property("libraryPending").toBool();
    }, u"Saved publication completion"_s);
    qInfo("PASS progressive_selection late=inspected ready=actionable cached=verified handoff=exact selection=stable");
}
