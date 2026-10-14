// SPDX-License-Identifier: GPL-2.0-or-later
#include "video-actions.h"
#include "group-entry.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFileInfo>
#include <QHash>
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

    void waitFor(const std::function<bool()> &condition, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!condition() && (elapsed.elapsed() < 10'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    QJsonObject generatedEntry()
    {
        const QJsonObject publication {{"source", "yummy_anime"}, {"id", "900001"}};
        QJsonArray variants;
        for (const QString &player : {u"Плеер Aksor"_s, u"Плеер CVH"_s, u"Плеер Kodik"_s,
            u"Плеер VK"_s, u"Плеер Alloha"_s, u"Alloha"_s, u"CVH"_s})
        {
            const QJsonObject selector {{"yummy_anime_video", QJsonObject {
                {"id", variants.size() + 1}, {"episode", "3"}, {"dubbing", "Fixture Studio"},
                {"player", player}, {"player_id", variants.size() + 1}
            }}};
            variants.append(QJsonObject {
                {"reference", QJsonObject {{"publication", publication}, {"selector", selector}}},
                {"transport", "video"}, {"episode", "3"}, {"translation", "Fixture Studio"},
                {"languages", QJsonArray {"ru"}}, {"player", player}, {"reported_video_ads", false}
            });
        }
        const QJsonObject work {
            {"reference", publication}, {"title", "Generated video action fixture"},
            {"category", "anime"}, {"url", "https://example.com/generated-video"},
            {"aliases", QJsonArray()}, {"identifiers", QJsonObject()},
            {"needs_inspection", false}, {"variants", variants}
        };
        return {
            {"group", GroupEntry::single(work)}, {"favorite", true},
            {"preferences", QJsonObject {
                {"selected", variants.first().toObject().value("reference")}, {"filters", QJsonObject()}
            }}
        };
    }
}

void exerciseVideoActions(const QString &requestedProfile)
{
    QTemporaryDir temporary;
    const QString profile = requestedProfile.isEmpty() ? temporary.path() : requestedProfile;
    require(QDir::isAbsolutePath(profile) && QDir().mkpath(profile)
        && !QFileInfo::exists(profile + u"/content-library.sqlite"_s), u"Use a fresh isolated profile"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Video action owners did not drain");
    });
    {
        Qbutt::ContentController creator;
        creator.initLibrary(profile);
        waitFor([&] { return creator.getLibraryState() == Qbutt::Library::LibraryState::Ready; }, u"Library"_s);
    }
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Initial library owner did not drain"_s);
    {
        auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"video-actions"_s);
        const auto closeDatabase = qScopeGuard([&]
        {
            database.close();
            database = {};
            QSqlDatabase::removeDatabase(u"video-actions"_s);
        });
        database.setDatabaseName(profile + u"/content-library.sqlite"_s);
        require(database.open(), database.lastError().text());
        QSqlQuery insert(database);
        require(insert.prepare(u"INSERT INTO library VALUES (?, ?, ?)"_s), insert.lastError().text());
        insert.addBindValue(u"{\"source\":\"yummy_anime\",\"id\":\"900001\"}"_s);
        insert.addBindValue(QString::fromUtf8(QJsonDocument(generatedEntry()).toJson(QJsonDocument::Compact)));
        insert.addBindValue(1);
        require(insert.exec(), insert.lastError().text());
    }
    Content::ContentView view(profile);
    view.resize(1440, 900);
    view.show();
    auto *controller = view.findChild<Qbutt::ContentController *>();
    require(controller, u"Missing controller"_s);
    waitFor([&] { return controller->getLibraryState() == Qbutt::Library::LibraryState::Ready; }, u"Library reopen"_s);
    controller->initDownloads(profile + u"/downloads"_s);
    waitFor([&] { return controller->property("downloadsState").toString() == u"ready"; }, u"Download queue"_s);
    waitFor([&]
    {
        return controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
            == Qbutt::Sources::SettingsState::Ready;
    }, u"Source settings"_s);
    int resolutions = 0;
    QObject::connect(controller, &Qbutt::ContentController::resolvingChanged, &view,
        [&] { resolutions += controller->property("resolving").toBool() ? 1 : 0; });
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    controller->selectWork(u"yani:900001"_s);
    auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
    require(variants && variants->rowCount() == 7, u"Stored variants lost"_s);
    auto *watch = view.rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
    auto *save = view.rootObject()->findChild<QQuickItem *>(u"contentSaveVideo"_s);
    auto *notice = view.rootObject()->findChild<QQuickItem *>(u"contentUnavailableVariant"_s);
    auto *list = view.rootObject()->findChild<QQuickItem *>(u"contentVariants"_s);
    require(watch && save && notice && list, u"Video actions missing"_s);
    const auto roles = variants->roleNames();
    QHash<QString, QString> rawPlayers;
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        const auto index = variants->index(row, 0);
        rawPlayers.insert(variants->data(index, roles.key("variantId")).toString(),
            variants->data(index, roles.key("player")).toString());
    }
    const QString selected = controller->property("selectedVariantId").toString();
    require(controller->property("inspecting").toBool()
        && (controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == Qbutt::Search::VariantSelectionState::Inspecting), u"Cached variant skipped reinspection"_s);
    waitFor([&] { return !watch->isEnabled() && !save->isEnabled(); }, u"Cached action guards"_s);
    require(!controller->downloadSelected(selected), u"Uninspected download accepted"_s);
    controller->resolveVariant(selected);
    require(resolutions == 0 && !controller->property("resolving").toBool(),
        u"Uninspected player started resolution"_s);
    qInfo("FIXTURE release_video_inspection");
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Generated HTTPS inspection"_s);
    require(controller->property("errorText").toString().isEmpty()
        && (controller->property("selectedWorkDescription").toString() == u"Generated HTTPS inspection completed"_s),
        u"Fresh publication was not inspected"_s);
    require(controller->property("selectedVariantId").toString() == selected && variants->rowCount() == 7,
        u"Fresh inspection replaced the exact player selection"_s);
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        const auto index = variants->index(row, 0);
        const QString player = variants->data(index, roles.key("player")).toString();
        const QString id = variants->data(index, roles.key("variantId")).toString();
        require(rawPlayers.contains(id), u"Inspection changed a raw player identity"_s);
        const QString rawPlayer = rawPlayers.value(id);
        QString displayPlayer = rawPlayer;
        displayPlayer.remove(u"Плеер "_s);
        require(player == displayPlayer, u"Inspected player label differs: "_s + rawPlayer);
        const bool supported = !rawPlayer.contains(u"Alloha"_s);
        controller->selectVariant(id);
        waitFor([&] { return watch->isEnabled() == supported && save->isVisible() == supported; }, player);
        require((controller->property("selectedVariantSaveable").toBool() == supported)
            && (save->isEnabled() == supported), u"Watch and Save capabilities differ: "_s + player);
        require(controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == (supported ? Qbutt::Search::VariantSelectionState::Available
                          : Qbutt::Search::VariantSelectionState::UnsupportedPlayback), u"Wrong selection state"_s);
        require(notice->isVisible() != supported, u"Unsupported player notice mismatch"_s);
        require(QMetaObject::invokeMethod(list, "forceLayout"), u"Variant layout failed"_s);
        QQuickItem *delegate = nullptr;
        require(QMetaObject::invokeMethod(list, "itemAtIndex", Q_RETURN_ARG(QQuickItem *, delegate), Q_ARG(int, row))
            && delegate && delegate->property("highlighted").toBool(), u"Selected variant lost its highlight"_s);
        if (!supported)
        {
            require(notice->property("text").toString() == QCoreApplication::translate("WorkPane",
                "This player is not supported yet. Choose another."), u"Missing localized next action"_s);
        }
        if (rawPlayer == u"Плеер CVH" || rawPlayer == u"Плеер Alloha")
        {
            for (const QSize &size : {QSize(1440, 900), QSize(1280, 720)})
            {
                view.resize(size);
                QEventLoop frame;
                QTimer::singleShot(200, &frame, &QEventLoop::quit);
                frame.exec();
                const QString path = u"%1/video-%2-%3x%4.png"_s.arg(profile,
                    supported ? u"supported"_s : u"unsupported"_s).arg(size.width()).arg(size.height());
                require(view.grabFramebuffer().save(path), u"Product render failed"_s);
            }
        }
        if (!supported)
        {
            require(!controller->downloadSelected(id), u"Unsupported download accepted"_s);
            controller->resolveVariant(id);
            require(!controller->property("resolving").toBool(), u"Unsupported player started resolution"_s);
        }
    }
    require(resolutions == 0 && !controller->property("inspecting").toBool(), u"Rejected actions started resolution"_s);
    auto *downloads = qobject_cast<QAbstractItemModel *>(controller->property("downloads").value<QObject *>());
    auto *history = qobject_cast<QAbstractItemModel *>(controller->property("history").value<QObject *>());
    require(downloads && history && downloads->rowCount() == 0 && history->rowCount() == 0,
        u"Rejected actions created jobs or history"_s);
    qInfo("PASS video_actions native_inspection cached_guard raw_player save watch unsupported_no_request no_jobs no_history");
}
