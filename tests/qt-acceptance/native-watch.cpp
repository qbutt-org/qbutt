// SPDX-License-Identifier: GPL-2.0-or-later
#include "native-watch.h"
#include "media-save.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDir>
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

void NativeWatchAcceptance::run(Application &application, const QJsonObject &spec, QJsonObject &evidence)
{
    const QString platform = QGuiApplication::platformName();
    const bool native = qEnvironmentVariableIsSet("QBUTT_QT_ACCEPTANCE_NATIVE");
    require((platform == u"offscreen"_s) || ((platform == u"windows"_s) && native),
        u"Native Watch requires offscreen Qt or an explicit native fixture"_s);
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

    const QString sourceId = spec.value(u"source"_s).toString();
    const QString query = spec.value(u"query"_s).toString();
    const QString workId = spec.value(u"workId"_s).toString();
    const QString episode = spec.value(u"episode"_s).toString();
    const QString translation = spec.value(u"translation"_s).toString();
    const QString variantSuffix = spec.value(u"variantSuffix"_s).toString();
    require(!sourceId.isEmpty() && !query.isEmpty() && workId.startsWith(sourceId + u":"_s),
        u"Native Watch selection is incomplete"_s);

    auto *sources = qobject_cast<QAbstractItemModel *>(controller->property("sources").value<QObject *>());
    require(sources, u"Native source settings are missing"_s);
    const int sourceIdRole = sources->roleNames().key("sourceId");
    require(sourceIdRole != 0, u"Native source IDs are unavailable"_s);
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString id = sources->data(sources->index(row, 0), sourceIdRole).toString();
        if (id != sourceId)
            require(controller->setSourceEnabled(id, false), u"Native Watch source fixture rejected "_s + id);
    }
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Source preferences"_s);
    controller->search(query);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Native search"_s);
    auto *works = qobject_cast<QAbstractItemModel *>(controller->property("works").value<QObject *>());
    require(works, u"Native search results are missing"_s);
    QModelIndex selectedSource;
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const auto index = sources->index(row, 0);
        if (sources->data(index, sourceIdRole).toString() == sourceId)
            selectedSource = index;
    }
    const auto sourceRoles = sources->roleNames();
    const bool sourceEnabled = sources->data(selectedSource, sourceRoles.key("enabled")).toBool();
    const bool sourceFailed = sources->data(selectedSource, sourceRoles.key("failed")).toBool();
    const QString sourceFailure = sources->data(selectedSource, sourceRoles.key("failureText")).toString();
    const QString searchError = controller->property("errorText").toString();
    QJsonArray workIds;
    const int workIdRole = works->roleNames().key("workId");
    for (int row = 0; row < works->rowCount(); ++row)
    {
        const QString id = works->data(works->index(row, 0), workIdRole).toString();
        workIds.append(id);
    }
    QJsonObject selection {
        {u"source"_s, sourceId},
        {u"workId"_s, workId},
        {u"sourceEnabled"_s, sourceEnabled},
        {u"sourceFailed"_s, sourceFailed},
        {u"sourceFailure"_s, sourceFailure},
        {u"searchError"_s, searchError},
        {u"workCount"_s, works->rowCount()},
        {u"workIds"_s, workIds}
    };
    evidence[u"selection"_s] = selection;
    require(selectedSource.isValid() && sourceEnabled, u"Native Watch source is disabled or missing"_s);
    require(!sourceFailed, u"Native search source failed: "_s + sourceFailure);
    require(searchError.isEmpty(), u"Native search failed: "_s + searchError);
    require(workIds.contains(workId), u"Native search did not return the selected publication"_s);
    if (sourceId == u"vkvideo"_s)
    {
        auto *category = view->rootObject()->findChild<QQuickItem *>(u"contentCategory"_s);
        auto *results = view->rootObject()->findChild<QQuickItem *>(u"contentResults"_s);
        require(category && results && (results->property("count").toInt() > 0),
            u"Native VK search returned no visible publications"_s);
        const int count = results->property("count").toInt();
        require(category->setProperty("currentIndex", 1), u"Native movie category is unavailable"_s);
        waitFor([&] { return results->property("count").toInt() == 0; }, u"VK category exclusion"_s);
        require(category->setProperty("currentIndex", 3)
            && (category->property("currentValue").toString() == u"video"_s),
            u"Native Video category is unavailable"_s);
        waitFor([&] { return results->property("count").toInt() == count; }, u"VK category restoration"_s);
    }
    controller->selectWork(workId);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Native video variants"_s);
    const QString inspectionError = controller->property("errorText").toString();
    selection[u"inspectionError"_s] = inspectionError;
    evidence[u"selection"_s] = selection;
    require(inspectionError.isEmpty(), u"Native video inspection failed: "_s + inspectionError);
    auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
    auto *history = qobject_cast<QAbstractItemModel *>(controller->property("history").value<QObject *>());
    require(variants && history && (history->rowCount() == 0), u"Native inspection recorded premature history"_s);
    QString variantId;
    int matches = 0;
    const auto roles = variants->roleNames();
    QJsonArray variantRows;
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        const auto index = variants->index(row, 0);
        const QString id = variants->data(index, roles.key("variantId")).toString();
        const QString variantEpisode = variants->data(index, roles.key("episode")).toString();
        const QString variantTranslation = variants->data(index, roles.key("translation")).toString();
        variantRows.append(QJsonObject {
            {u"variantId"_s, id},
            {u"episode"_s, variantEpisode},
            {u"translation"_s, variantTranslation}
        });
        if (id.startsWith(workId + u":"_s)
            && (variantSuffix.isEmpty() || id.endsWith(variantSuffix))
            && (episode.isEmpty() || variantEpisode == episode)
            && (translation.isEmpty() || variantTranslation == translation))
        {
            if (variantId.isEmpty())
                variantId = id;
            ++matches;
        }
    }
    selection[u"variantCount"_s] = variants->rowCount();
    selection[u"variantMatches"_s] = matches;
    selection[u"variants"_s] = variantRows;
    evidence[u"selection"_s] = selection;
    require(!variantId.isEmpty() && ((episode.isEmpty() && translation.isEmpty()) || matches == 1),
        u"Native source has no unique selected video"_s);
    const int variantCount = variants->rowCount();
    controller->selectVariant(variantId);
    auto *button = view->rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
    require(button && button->isEnabled(), u"Native Watch action is unavailable"_s);
    const QString screenshots = spec.value(u"screenshots"_s).toString();
    if (!screenshots.isEmpty())
    {
        require(QDir().mkpath(screenshots), u"Native Watch screenshot directory failed"_s);
        for (const QSize &size : {QSize(1440, 900), QSize(1280, 720)})
        {
            window->resize(size);
            QEventLoop frame;
            QTimer::singleShot(250, &frame, &QEventLoop::quit);
            frame.exec();
            require(window->grab().save(QDir(screenshots).filePath(
                u"native-watch-%1x%2.png"_s.arg(size.width()).arg(size.height()))),
                u"Native Watch product render failed"_s);
        }
    }

    QObject observations;
    auto *player = Media::Player::instance();
    const auto stop = qScopeGuard([player] { player->stop(); });
    QObject::disconnect(player, &Media::Player::error, window, nullptr);
    QStringList errors;
    quint64 readyToken = 0;
    QString readyIdentity;
    QUrl gatewayListener;
    QUrl playbackUrl;
    bool historyEmptyBeforeAck = false;
    QList<quint64> started;
    QStringList startedIdentities;
    QList<quint64> finished;
    double position = 0;
    QObject::connect(player, &Media::Player::error, &observations, [&](const QString &message) { errors.append(message); });
    bool descriptorIsHttps = false;
    bool descriptorIsLocal = false;
    QString contentKind;
    QJsonArray tracks;
    QObject::connect(view, &Content::ContentView::videoReady, &observations,
        [&](const quint64 token, const QString &identity, const QUrl &url, const QUrl &proxyUrl, const QString &, const QString &kind)
    {
        readyToken = token;
        readyIdentity = identity;
        playbackUrl = url;
        gatewayListener = proxyUrl.isValid() ? proxyUrl : url;
        descriptorIsHttps = url.scheme() == u"https"_s;
        descriptorIsLocal = (url.scheme() == u"http"_s) && (url.host() == u"127.0.0.1"_s)
            && (url.port() > 0) && !proxyUrl.isValid();
        contentKind = kind;
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
    QObject::connect(player, &Media::Player::tracksChanged, &observations, [&](const QJsonArray &actual) { tracks = actual; });
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
    require((descriptorIsHttps || descriptorIsLocal)
        && (contentKind == spec.value(u"contentKind"_s).toString(u"anime"_s)),
        u"Main Watch did not hand the exact video descriptor to qplay"_s);
    require(gatewayListener.isValid() && gatewayListener.host() == u"127.0.0.1"_s
        && gatewayListener.port() > 0,
        u"Main Watch has no isolated media gateway"_s);
    waitFor([&] { return (position >= 0.5) || !errors.isEmpty(); }, u"Main Watch actual decoding"_s);
    require(errors.isEmpty(), u"Main Watch decoding failed"_s);
    require(!tracks.isEmpty(), u"qplay did not report decoded media tracks"_s);
    waitFor([&] { return !controller->property("libraryPending").toBool() && (history->rowCount() == 1); },
        u"Main Watch confirmed history"_s);
    if (spec.contains(u"savePipeFile"_s))
        exerciseMediaSave(*view, *player, readyToken, playbackUrl, variantId, spec, evidence);
    player->stop();
    require(finished == QList<quint64> {readyToken}, u"Main Watch terminal outcome is not unique"_s);
    waitFor([&]
    {
        QTcpSocket socket;
        socket.connectToHost(gatewayListener.host(), gatewayListener.port());
        return !socket.waitForConnected(100);
    }, u"Main Watch media listener retirement"_s);
    evidence[u"checks"_s] = QJsonArray {QJsonObject {{u"name"_s, u"native-watch"_s},
        {u"source"_s, sourceId}, {u"workId"_s, workId}, {u"episode"_s, episode},
        {u"translation"_s, translation}, {u"variantId"_s, variantId},
        {u"variantCount"_s, variantCount}, {u"identity"_s, readyIdentity},
        {u"trackCount"_s, static_cast<int>(tracks.size())}, {u"descriptorIsHttps"_s, descriptorIsHttps},
        {u"descriptorIsLocal"_s, descriptorIsLocal}, {u"contentKind"_s, contentKind},
        {u"launchToken"_s, static_cast<qint64>(readyToken)}, {u"position"_s, position},
        {u"historyAfterActualAck"_s, true}, {u"listenerRetired"_s, true},
        {u"libraryPath"_s, specialFolderLocation(SpecialFolder::Data).toString()}}};
}
