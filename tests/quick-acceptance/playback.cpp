// SPDX-License-Identifier: GPL-2.0-or-later
#include "playback.h"

#include <functional>
#include <memory>
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
#include <QTcpSocket>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool accepted, const QString &message)
    {
        if (!accepted)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation, const int timeout = 50'000)
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
        require(result, u"Invalid playback model"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &role)
    {
        return model->data(model->index(row, 0), model->roleNames().key(role)).toString();
    }

    void loaded(Qbutt::ContentController &controller)
    {
        waitFor([&]
        {
            return controller.property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready;
        }, u"Playback profile"_s);
    }

    void saved(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, u"Playback FIFO acknowledgement"_s);
    }

    void exec(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    }

    QJsonObject entry(QSqlDatabase &database)
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT entry FROM library WHERE json_extract(entry, '$.group.anchor.id') = '9542'"_s)
            && query.next(), u"Owned playback entry is missing"_s);
        return QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
    }

    void listenerRetired(const QUrl &proxy)
    {
        waitFor([&]
        {
            QTcpSocket socket;
            socket.connectToHost(proxy.host(), proxy.port());
            return !socket.waitForConnected(100);
        }, u"Retired media listener"_s);
    }
}

void exercisePlayback()
{
    require(QGuiApplication::platformName() == u"offscreen"_s, u"Playback bridge fixture requires offscreen Qt"_s);
    QTemporaryDir profile;
    require(profile.isValid(), u"Owned playback profile failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Playback owners did not drain");
    });

    auto seed = std::make_unique<Qbutt::ContentController>();
    seed->initLibrary(profile.path());
    loaded(*seed);
    auto *sources = model(*seed, "sources");
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString source = value(sources, row, "sourceId");
        require(seed->setSourceEnabled(source, source == u"aniliberty"), u"Playback source fixture rejected"_s);
    }
    saved(*seed);
    seed->search(u"Frieren"_s);
    waitFor([&] { return !seed->property("busy").toBool(); }, u"Live playback metadata"_s);
    seed->selectWork(u"aniliberty:9542"_s);
    waitFor([&] { return !seed->property("inspecting").toBool(); }, u"Live publication variants"_s);
    QStringList videos;
    QStringList videoLabels;
    auto *variants = model(*seed, "variants");
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        const QString id = value(variants, row, "variantId");
        if ((value(variants, row, "kind") == u"video"_s) && id.contains(u":episode:"_s))
        {
            videos.append(id);
            videoLabels.append(value(variants, row, "label"));
        }
    }
    require(videos.size() >= 2, u"Exact direct video variants are absent"_s);
    require((videoLabels.first() != videoLabels.at(1))
        && videoLabels.first().startsWith(QCoreApplication::translate("ContentController", "Episode %1").arg(u"1"_s)),
        u"Exact episodes/qualities have indistinguishable labels"_s);
    seed->selectVariant(videos.first());
    seed->setSelectedFavorite(true);
    saved(*seed);
    require(model(*seed, "history")->rowCount() == 0, u"Selection recorded playback history"_s);
    seed.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Seed owner did not drain"_s);

    const auto unregister = qScopeGuard([] { QSqlDatabase::removeDatabase(u"playback-faults"_s); });
    QSqlDatabase database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"playback-faults"_s);
    database.setDatabaseName(QDir(profile.path()).filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    auto generated = entry(database);
    auto preferences = generated.value(u"preferences"_s).toObject();
    preferences.insert(u"filters"_s, QJsonObject {{u"episode"_s, u"generated episode"_s},
        {u"translation"_s, u"generated translation"_s}, {u"source"_s, u"ani_liberty"_s},
        {u"language"_s, u"ru"_s}, {u"player"_s, u"AniLiberty"_s}, {u"transport"_s, u"video"_s},
        {u"query"_s, QJsonValue::Null}});
    generated.insert(u"preferences"_s, preferences);
    QSqlQuery replace(database);
    replace.prepare(u"UPDATE library SET entry = ? WHERE json_extract(entry, '$.group.anchor.id') = '9542'"_s);
    replace.addBindValue(QString::fromUtf8(QJsonDocument(generated).toJson(QJsonDocument::Compact)));
    require(replace.exec(), replace.lastError().text());

    Content::ContentView view(profile.path());
    view.resize(1440, 900);
    view.show();
    auto *controller = view.findChild<Qbutt::ContentController *>();
    require(controller, u"Production playback controller is missing"_s);
    loaded(*controller);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    controller->selectWork(u"aniliberty:9542"_s);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Reopened exact video"_s);
    saved(*controller);
    waitFor([&] { return view.status() != QQuickWidget::Loading; }, u"Playback QML"_s);
    require(view.status() == QQuickWidget::Ready, u"Playback QML failed"_s);
    auto *primary = view.rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
    require(primary && (primary->property("text").toString() == QCoreApplication::translate("WorkPane", "Watch")),
        u"Direct video primary action is not Watch"_s);

    struct Launch
    {
        quint64 token;
        QString identity;
        QUrl url;
        QUrl proxy;
    };
    QList<Launch> launches;
    QObject observation;
    QObject::connect(controller, &Qbutt::ContentController::videoReady, &observation,
        [&](const quint64 token, const QString &identity, const QUrl &url, const QUrl &proxy, const QString &, const QString &kind)
    {
        require((token != 0) && identity.startsWith(u"video:aniliberty:9542:episode:"_s)
            && (url.scheme() == u"https"_s) && (proxy.host() == u"127.0.0.1"_s)
            && !proxy.userInfo().isEmpty() && (kind == u"anime"_s), u"Invalid typed playback handoff"_s);
        launches.append({token, identity, url, proxy});
    });
    const auto launch = [&](const QString &id)
    {
        controller->selectVariant(id);
        saved(*controller);
        const int before = launches.size();
        controller->resolveVariant(id);
        waitFor([&] { return !controller->property("resolving").toBool(); }, u"Direct video resolve/gateway"_s);
        require(launches.size() == before + 1, u"Direct video did not produce a launch"_s);
        return launches.last();
    };

    const auto rejected = launch(videos.first());
    require(model(*controller, "history")->rowCount() == 0, u"Resolution recorded history before mpv ACK"_s);
    require(controller->finishPlayback(rejected.token) && !controller->confirmPlayback(rejected.token),
        u"Rejected launch accepted a late playback ACK"_s);
    listenerRetired(rejected.proxy);
    const auto first = launch(videos.first());
    const auto firstExpected = entry(database);
    require((first.token != rejected.token) && !controller->confirmPlayback(rejected.token), u"Same-variant launch reused a stale token"_s);
    exec(database, u"BEGIN IMMEDIATE"_s);
    for (int request = 0; request < 66; ++request)
        controller->setSelectedFavorite(true);
    require(controller->confirmPlayback(first.token) && !controller->confirmPlayback(first.token), u"Playback ACK was not consumed exactly once"_s);
    require(controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>() == Qbutt::Library::LibraryErrorKind::HistoryWriting,
        u"Full FIFO lost the confirmed playback intent"_s);
    exec(database, u"ROLLBACK"_s);
    require(controller->finishPlayback(first.token), u"Confirmed playback gateway did not retire"_s);
    listenerRetired(first.proxy);
    saved(*controller);
    exec(database, u"CREATE TRIGGER reject_playback BEFORE UPDATE ON library WHEN json_extract(NEW.entry, '$.last_opened') IS NOT NULL BEGIN SELECT RAISE(ABORT, 'generated playback history fault'); END;"_s);
    require(controller->retryHistoryWrite(), u"Rejected FIFO playback retry was not retained"_s);
    saved(*controller);
    require((model(*controller, "history")->rowCount() == 0)
        && (controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>() == Qbutt::Library::LibraryErrorKind::HistoryWriting),
        u"Failed confirmed playback did not retain exact retry intent"_s);
    require(controller->refreshHistory() && controller->setReaderFit(Qbutt::Reader::ReaderFit::Width), u"Independent history/reader action rejected"_s);
    saved(*controller);
    require(controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>() == Qbutt::Library::LibraryErrorKind::HistoryWriting,
        u"Unrelated success cleared the playback retry"_s);

    const auto second = launch(videos.at(1));
    const auto secondExpected = entry(database);
    require(controller->confirmPlayback(second.token) && controller->finishPlayback(second.token), u"Later confirmed playback rejected"_s);
    listenerRetired(second.proxy);
    const QString imageDirectory = qEnvironmentVariable("QBUTT_PLAYBACK_QA_IMAGES");
    if (!imageDirectory.isEmpty())
    {
        require(QDir().mkpath(imageDirectory), u"Playback render directory failed"_s);
        for (const QSize &size : {QSize(1440, 900), QSize(1280, 720)})
        {
            view.resize(size);
            QEventLoop frame;
            QTimer::singleShot(150, &frame, &QEventLoop::quit);
            frame.exec();
            require(view.grabFramebuffer().save(QDir(imageDirectory).filePath(u"playback-%1x%2.png"_s.arg(size.width()).arg(size.height()))),
                u"Playback render failed"_s);
        }
    }
    controller->search(QString());
    require(model(*controller, "history")->rowCount() == 0, u"Later playback replaced the failed head"_s);
    auto *error = view.rootObject()->findChild<QQuickItem *>(u"contentLibraryError"_s);
    auto *retry = view.rootObject()->findChild<QQuickItem *>(u"contentLibraryRetry"_s);
    require(error && error->isVisible() && retry && retry->isVisible() && retry->isEnabled(), u"Playback history has no actionable recovery"_s);
    exec(database, u"DROP TRIGGER reject_playback"_s);
    exec(database, u"CREATE TABLE playback_audit (entry TEXT NOT NULL)"_s);
    exec(database, u"CREATE TRIGGER capture_playback AFTER UPDATE ON library WHEN json_extract(NEW.entry, '$.last_opened') IS NOT NULL BEGIN INSERT INTO playback_audit VALUES (NEW.entry); END;"_s);
    require(QMetaObject::invokeMethod(retry, "clicked"), u"Production playback Retry action failed"_s);
    saved(*controller);
    require((model(*controller, "history")->rowCount() == 1) && !error->isVisible(), u"Playback history retry did not recover"_s);
    QSqlQuery audit(database);
    require(audit.exec(u"SELECT entry FROM playback_audit ORDER BY rowid"_s), audit.lastError().text());
    for (const QJsonObject &expected : {firstExpected, secondExpected})
    {
        require(audit.next(), u"Immutable playback save was lost"_s);
        const auto recorded = QJsonDocument::fromJson(audit.value(0).toString().toUtf8()).object();
        require((recorded.value(u"group"_s) == expected.value(u"group"_s))
            && (recorded.value(u"preferences"_s) == expected.value(u"preferences"_s))
            && (recorded.value(u"last_opened"_s).toObject().value(u"variant"_s)
                == expected.value(u"preferences"_s).toObject().value(u"selected"_s))
            && !recorded.contains(u"launchToken"_s) && !recorded.contains(u"proxyUrl"_s),
            u"Playback retry changed full work, exact variant, filters or durable metadata"_s);
    }
    require(!audit.next() && !controller->confirmPlayback(first.token) && !controller->finishPlayback(second.token), u"Duplicate/terminal playback mutated history"_s);
    controller->selectWork(u"aniliberty:9542"_s);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Reentrant retry publication"_s);
    const auto third = launch(videos.first());
    const auto thirdExpected = entry(database);
    controller->selectVariant(videos.at(1));
    controller->setVariantQuery(u"before playback ACK"_s);
    saved(*controller);
    auto latestFilters = entry(database).value(u"preferences"_s).toObject().value(u"filters"_s).toObject();
    require(latestFilters.value(u"query"_s).toString() == u"before playback ACK"_s,
        u"Filter edit before playback ACK was not saved"_s);

    // Reject only a confirmation carrying the new filter, proving the first
    // history attempt already merges choices made after resolution.
    exec(database, u"CREATE TRIGGER reject_reentrant_playback BEFORE UPDATE ON library "
        "WHEN json_extract(NEW.entry, '$.preferences.filters.query') = 'before playback ACK' "
        "BEGIN SELECT RAISE(ABORT, 'generated reentrant playback fault'); END;"_s);
    bool reentered = false;
    const auto connection = QObject::connect(controller, &Qbutt::ContentController::libraryErrorTextChanged, &observation, [&]
    {
        if (!reentered && (controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>() == Qbutt::Library::LibraryErrorKind::HistoryWriting))
        {
            reentered = true;
            exec(database, u"DROP TRIGGER reject_reentrant_playback"_s);
            controller->setVariantQuery(u"after playback write failure"_s);
            require(controller->retryHistoryWrite(), u"Reentrant exact playback retry was rejected"_s);
        }
    }, Qt::DirectConnection);
    require(controller->confirmPlayback(third.token) && controller->finishPlayback(third.token), u"Reentrant launch confirmation failed"_s);
    saved(*controller);
    QObject::disconnect(connection);
    listenerRetired(third.proxy);
    require(reentered && controller->property("libraryErrorText").toString().isEmpty(), u"Older failure publication replaced a successful reentrant retry"_s);
    latestFilters.insert(u"query"_s, u"after playback write failure"_s);
    const auto thirdRecorded = entry(database);
    const auto recordedPreferences = thirdRecorded.value(u"preferences"_s).toObject();
    const auto expectedSelection = thirdExpected.value(u"preferences"_s).toObject().value(u"selected"_s);
    require((thirdRecorded.value(u"group"_s) == thirdExpected.value(u"group"_s))
        && (recordedPreferences.value(u"selected"_s) == expectedSelection)
        && (thirdRecorded.value(u"last_opened"_s).toObject().value(u"variant"_s) == expectedSelection)
        && (recordedPreferences.value(u"filters"_s).toObject() == latestFilters)
        && (controller->property("selectedVariantId").toString() == videos.at(1)),
        u"Playback retry lost newer filters or replaced the immutable played variant/current selection"_s);
    qInfo("PASS playback bridge=typed direct_variants=visible start_ACK=once rejection=no_history same_variant_token=unique full_work_prefs=immutable FIFO_pressure=retained SQL_retry=ordered reentrant_retry=retained newer_filters=preserved QML_recovery=working gateway_terminal=retired");
}
