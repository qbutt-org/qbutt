// SPDX-License-Identifier: GPL-2.0-or-later
#include "native-watch.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QJsonArray>
#include <QJsonObject>
#include <QMouseEvent>
#include <QQuickItem>
#include <QScopeGuard>
#include <QTcpSocket>
#include <QTimer>

#include "app/application.h"
#include "base/bittorrent/session.h"
#include "base/media/player.h"
#include "base/path.h"
#include "base/profile.h"
#include "gui/mainwindow.h"
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
}

void NativeWatchAcceptance::run(Application &application, QJsonObject &evidence)
{
    require(QGuiApplication::platformName() == u"offscreen"_s, u"Native Watch fixture requires offscreen Qt"_s);
    auto *session = BitTorrent::Session::instance();
    require(session->torrents().isEmpty() && (session->networkInterfaceAddress() == u"127.0.0.1")
        && !session->isDHTEnabled() && !session->isLSDEnabled() && !session->isPeXEnabled(),
        u"Native Watch torrent session is not isolated"_s);
    auto *window = application.mainWindow();
    const auto views = window->findChildren<Content::ContentView *>();
    require((views.size() == 1) && (window->currentTabWidget() == views.first()), u"Main native search is missing"_s);
    auto *view = views.first();
    window->resize(1440, 900);
    window->show();
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Main content controller is missing"_s);
    waitFor([&] { return controller->getLibraryState() == Qbutt::Library::LibraryState::Ready; }, u"Native library"_s);
    require(controller->setSourceEnabled(u"rutor"_s, false) && controller->setSourceEnabled(u"mangadex"_s, false),
        u"Native Watch source fixture rejected"_s);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Source preferences"_s);
    controller->search(u"Frieren"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Native search"_s);
    controller->selectWork(u"aniliberty:9542"_s);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Native video variants"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Native video inspection failed"_s);
    auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
    auto *history = qobject_cast<QAbstractItemModel *>(controller->property("history").value<QObject *>());
    require(variants && history && (history->rowCount() == 0), u"Native inspection recorded premature history"_s);
    QString variantId;
    const auto roles = variants->roleNames();
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        const QString id = variants->data(variants->index(row, 0), roles.key("variantId")).toString();
        if (id.contains(u":episode:"_s) && id.endsWith(u":480"_s))
        {
            variantId = id;
            break;
        }
    }
    require(!variantId.isEmpty(), u"Native source has no exact 480p video"_s);
    const int variantCount = variants->rowCount();
    controller->selectVariant(variantId);
    auto *button = view->rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
    require(button && button->isEnabled(), u"Native Watch action is unavailable"_s);

    QObject observations;
    auto *player = Media::Player::instance();
    const auto stop = qScopeGuard([player] { player->stop(); });
    QObject::disconnect(player, &Media::Player::error, window, nullptr);
    QStringList errors;
    quint64 readyToken = 0;
    QString readyIdentity;
    QUrl proxy;
    bool historyEmptyBeforeAck = false;
    QList<quint64> started;
    QStringList startedIdentities;
    QList<quint64> finished;
    double position = 0;
    QObject::connect(player, &Media::Player::error, &observations, [&](const QString &message) { errors.append(message); });
    QObject::connect(view, &Content::ContentView::videoReady, &observations,
        [&](const quint64 token, const QString &identity, const QUrl &, const QUrl &proxyUrl, const QString &, const QString &)
    {
        readyToken = token;
        readyIdentity = identity;
        proxy = proxyUrl;
        historyEmptyBeforeAck = history->rowCount() == 0;
    });
    QObject::connect(player, &Media::Player::playbackStarted, &observations,
        [&](const QString &identity, const quint64 token)
    {
        startedIdentities.append(identity);
        started.append(token);
    });
    QObject::connect(player, &Media::Player::playbackFinished, &observations, [&](const quint64 token) { finished.append(token); });
    QObject::connect(player, &Media::Player::positionChanged, &observations, [&](const double seconds) { position = seconds; });
    const QPointF point = button->mapToScene(QPointF(button->width() / 2, button->height() / 2));
    const QPointF global = view->mapToGlobal(point.toPoint());
    QMouseEvent press(QEvent::MouseButtonPress, point, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
    QCoreApplication::sendEvent(view, &press);
    QMouseEvent release(QEvent::MouseButtonRelease, point, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
    QCoreApplication::sendEvent(view, &release);
    waitFor([&] { return !started.isEmpty() || !errors.isEmpty(); }, u"Main Watch actual mpv file-loaded"_s);
    require(errors.isEmpty(), u"Main Watch failed: "_s + errors.join(u"; "_s));
    require(readyToken && historyEmptyBeforeAck && (readyIdentity == u"video:"_s + variantId)
        && (started == QList<quint64> {readyToken}) && (startedIdentities == QStringList {readyIdentity}),
        u"Main Watch did not confirm its exact launch once"_s);
    waitFor([&] { return (position >= 0.5) || !errors.isEmpty(); }, u"Main Watch actual decoding"_s);
    require(errors.isEmpty(), u"Main Watch decoding failed"_s);
    waitFor([&] { return !controller->property("libraryPending").toBool() && (history->rowCount() == 1); },
        u"Main Watch confirmed history"_s);
    player->stop();
    require(finished == QList<quint64> {readyToken}, u"Main Watch terminal outcome is not unique"_s);
    waitFor([&]
    {
        QTcpSocket socket;
        socket.connectToHost(proxy.host(), proxy.port());
        return !socket.waitForConnected(100);
    }, u"Main Watch media listener retirement"_s);
    evidence[u"checks"_s] = QJsonArray {QJsonObject {{u"name"_s, u"native-watch"_s},
        {u"variantId"_s, variantId}, {u"variantCount"_s, variantCount}, {u"identity"_s, readyIdentity},
        {u"launchToken"_s, static_cast<qint64>(readyToken)}, {u"position"_s, position},
        {u"historyAfterActualAck"_s, true}, {u"listenerRetired"_s, true},
        {u"libraryPath"_s, specialFolderLocation(SpecialFolder::Data).toString()}}};
}
