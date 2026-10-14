// SPDX-License-Identifier: GPL-2.0-or-later
#include "session-retry.h"

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
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>
#include <QUrl>
#include <QUrlQuery>

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    const QString publication = u"rutracker:42"_s;
    const QString exact = publication + u":torrent:0123456789abcdef0123456789abcdef01234567"_s;

    void require(const bool accepted, const QString &message)
    {
        if (!accepted)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 30'000))
        {
            QEventLoop loop;
            QTimer::singleShot(10, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QByteArray read(const QString &path)
    {
        QFile file(path);
        return file.open(QIODevice::ReadOnly) ? file.readAll() : QByteArray();
    }

    void phase(const QByteArray &value)
    {
        QFile file(qEnvironmentVariable("QBUTT_SESSION_RETRY_PHASE"));
        require(file.open(QIODevice::WriteOnly | QIODevice::Truncate)
            && (file.write(value) == value.size()), u"Cannot publish fixture phase"_s);
        file.close();
        waitFor([&] { return read(qEnvironmentVariable("QBUTT_SESSION_RETRY_READY")) == value; },
            u"Isolated browser phase"_s);
    }

    QAbstractItemModel *model(QObject *controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller->property(name).value<QObject *>());
        require(result, u"Missing typed model"_s);
        return result;
    }

    QVariant sourceValue(QObject *controller, const char *role)
    {
        auto *sources = model(controller, "sources");
        const auto roles = sources->roleNames();
        for (int row = 0; row < sources->rowCount(); ++row)
        {
            if (sources->data(sources->index(row, 0), roles.key("sourceId")).toString() == u"rutracker"_s)
                return sources->data(sources->index(row, 0), roles.key(role));
        }
        throw std::runtime_error("Missing RuTracker source");
    }

    QJsonObject failure(QSqlDatabase &database)
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT failure FROM source_settings WHERE source = 'rutracker'"_s)
            && query.next(), u"Missing persisted source settings"_s);
        return QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
    }

    void saved(QObject *controller)
    {
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Source write"_s);
        require(controller->property("libraryErrorText").toString().isEmpty(), u"Source persistence failed"_s);
    }
}

void exerciseSessionRetry()
{
    require(QGuiApplication::platformName() == u"offscreen"_s, u"Session fixture requires hidden Qt"_s);
    QTemporaryDir profile;
    require(profile.isValid(), u"Owned profile failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Session recovery owners did not drain");
    });
    auto view = std::make_unique<Content::ContentView>(profile.path());
    view->resize(1440, 900);
    view->show();
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Production controller missing"_s);
    controller->setBrowserChoice(u"edge"_s);
    controller->watchBrowser();
    waitFor([&] { return controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
        == Qbutt::Sources::SettingsState::Ready; }, u"Source preferences"_s);
    auto *sources = model(controller, "sources");
    const auto roles = sources->roleNames();
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const auto id = sources->data(sources->index(row, 0), roles.key("sourceId")).toString();
        require(controller->setSourceEnabled(id, id == u"rutracker"_s), u"Source isolation failed"_s);
    }
    saved(controller);
    phase("initial");
    waitFor([&] { return !controller->property("browserConnectedName").toString().isEmpty(); }, u"Selected browser"_s);

    const auto search = [&]
    {
        controller->search(u"generated"_s);
        waitFor([&] { return !controller->property("busy").toBool(); }, u"Generated search"_s);
        require(model(controller, "works")->rowCount() == 2, u"Exact publications missing"_s);
    };
    const auto select = [&](const QString &id)
    {
        controller->selectWork(id);
        waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Exact inspection"_s);
        require(controller->property("selectedWorkId").toString() == id
            && model(controller, "variants")->rowCount() == 1, u"Publication selection changed"_s);
    };
    search();
    select(publication);
    require(controller->property("selectedVariantId").toString() == exact, u"Exact torrent changed"_s);
    const QString title = controller->property("selectedWorkTitle").toString();
    require(title == u"Generated legal 42"_s, u"Authored publication title missing"_s);
    int handoffs = 0;
    QObject observer;
    QObject::connect(controller, &Qbutt::ContentController::torrentReady, &observer, [&](const QString &magnet)
    {
        ++handoffs;
        require(QUrlQuery(QUrl(magnet)).queryItemValue(u"xt"_s)
            == u"urn:btih:0123456789abcdef0123456789abcdef01234567"_s, u"Wrong torrent handoff"_s);
    });
    const auto unregister = qScopeGuard([] { QSqlDatabase::removeDatabase(u"session-retry"_s); });
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"session-retry"_s);
    database.setDatabaseName(QDir(profile.path()).filePath(u"content-library.sqlite"_s));
    require(database.open(), u"Fixture database failed"_s);

    const auto failOpen = [&](const QByteArray &state)
    {
        phase(state);
        controller->resolveVariant(exact);
        waitFor([&] { return !controller->property("resolving").toBool(); }, u"Expired exact Open"_s);
        saved(controller);
        require(sourceValue(controller, "failed").toBool() && sourceValue(controller, "enabled").toBool()
            && controller->property("selectedVariantId").toString() == exact
            && controller->property("selectedWorkTitle").toString() == title,
            u"Expired session corrupted identity or disabled its source"_s);
        require(failure(database).value(u"operation"_s).toObject().contains(u"resolve"_s),
            u"Failed action was not the exact resolve"_s);
    };
    failOpen("expired");
    require(handoffs == 0, u"Expired request emitted a handoff"_s);
    const auto failedOperation = failure(database).value(u"operation"_s);
    auto *sourceButton = view->rootObject()->findChild<QQuickItem *>(u"contentSources"_s);
    require(sourceButton && QMetaObject::invokeMethod(sourceButton, "clicked"), u"Native Sources action failed"_s);
    QObject *picker = nullptr;
    waitFor([&]
    {
        picker = view->rootObject()->findChild<QObject *>(u"contentSourcePicker"_s);
        return picker && picker->property("opened").toBool();
    }, u"Native source failure render"_s);
    QElapsedTimer entry;
    entry.start();
    waitFor([&] { return entry.elapsed() >= 200; }, u"Sources entry animation"_s);
    const QDir renders(qEnvironmentVariable("QBUTT_SESSION_RETRY_RENDER_DIR"));
    require(view->grab().save(renders.filePath(u"expired-sources.png"_s)), u"Failed source render missing"_s);
    require(QMetaObject::invokeMethod(picker, "close"), u"Sources close failed"_s);
    waitFor([&] { return !picker->property("opened").toBool(); }, u"Sources close"_s);

    phase("renewed");
    search();
    saved(controller);
    require(sourceValue(controller, "failed").toBool()
        && failure(database).value(u"operation"_s) == failedOperation, u"Unrelated successful search cleared Resolve"_s);
    select(publication);
    failOpen("expired-again");
    phase("renewed-again");
    const auto beforeCheck = read(qEnvironmentVariable("QBUTT_SESSION_CHECK_LOG"));
    require(controller->retrySource(u"rutracker"_s), u"Exact source continuation rejected"_s);
    require(!controller->retrySource(u"rutracker"_s), u"Concurrent continuation was accepted twice"_s);
    waitFor([&] { return !controller->property("resolving").toBool()
        && !sourceValue(controller, "retrying").toBool(); }, u"Browser session continuation"_s);
    saved(controller);
    require(handoffs == 1 && !sourceValue(controller, "failed").toBool() && failure(database).isEmpty()
        && controller->property("selectedVariantId").toString() == exact, u"Original Open did not resume exactly once"_s);
    require(read(qEnvironmentVariable("QBUTT_SESSION_CHECK_LOG")) == beforeCheck,
        u"Current browser session still opened a WebView check"_s);
    require(!controller->retrySource(u"rutracker"_s), u"Accepted action could be replayed from Sources"_s);
    require(view->grab().save(renders.filePath(u"recovered-selection.png"_s)), u"Recovered native render missing"_s);

    failOpen("expired-stale");
    phase("renewed-stale");
    select(u"rutracker:43"_s);
    require(sourceValue(controller, "failed").toBool(), u"Unrelated inspection cleared Resolve"_s);
    require(controller->retrySource(u"rutracker"_s), u"Persisted exact probe rejected"_s);
    waitFor([&] { return !sourceValue(controller, "retrying").toBool(); }, u"Stale action probe"_s);
    saved(controller);
    require(handoffs == 1 && !sourceValue(controller, "failed").toBool()
        && controller->property("selectedWorkId").toString() == u"rutracker:43"_s,
        u"Stale Open was replayed or current selection changed"_s);

    select(publication);
    const auto beforeFault = read(qEnvironmentVariable("QBUTT_SESSION_CHECK_LOG"));
    failOpen("fault");
    require(failure(database).value(u"reason"_s) == u"network"_s, u"Unknown browser outcome became an auth failure"_s);
    require(controller->retrySource(u"rutracker"_s), u"Unknown-outcome exact probe rejected"_s);
    waitFor([&] { return !sourceValue(controller, "retrying").toBool(); }, u"Unknown browser outcome"_s);
    saved(controller);
    require(handoffs == 1 && sourceValue(controller, "failed").toBool()
        && read(qEnvironmentVariable("QBUTT_SESSION_CHECK_LOG")) == beforeFault,
        u"Unknown outcome opened an auth check or accepted a side effect"_s);
    phase("renewed-fault");
    require(controller->retrySource(u"rutracker"_s), u"Recovered unknown-outcome probe rejected"_s);
    waitFor([&] { return !sourceValue(controller, "retrying").toBool(); }, u"Recovered exact probe"_s);
    saved(controller);
    require(handoffs == 1 && !sourceValue(controller, "failed").toBool(), u"Unknown-outcome probe replayed Open"_s);

    failOpen("manual");
    require(controller->retrySource(u"rutracker"_s), u"Manual continuation rejected"_s);
    waitFor([&] { return !controller->property("resolving").toBool()
        && !sourceValue(controller, "retrying").toBool(); }, u"Existing manual check continuation"_s);
    saved(controller);
    require(handoffs == 2 && !sourceValue(controller, "failed").toBool()
        && read(qEnvironmentVariable("QBUTT_SESSION_CHECK_LOG")).count("interactive\n") == 1,
        u"Still expired session did not complete through its existing check"_s);
    view.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Session fixture did not drain"_s);
    require(!QFile::exists(qEnvironmentVariable("QBUTT_BROWSER_BRIDGE_DISCOVERY")), u"Discovery owner leaked"_s);
    qInfo("PASS browser_session_retry exact_action=2 stale_action=0 unrelated_success=retained manual_check=1");
}
