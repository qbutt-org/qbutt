// SPDX-License-Identifier: GPL-2.0-or-later
#include "media-save.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QCryptographicHash>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLocalSocket>
#include <QMouseEvent>
#include <QQuickItem>
#include <QTimer>

#include "base/media/player.h"
#include "base/media/runtime.h"
#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(bool accepted, const QString &message)
    {
        if (!accepted)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &message)
    {
        QElapsedTimer timer;
        timer.start();
        while (!ready() && timer.elapsed() < 50'000)
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), message + u" timed out"_s);
    }

    void settle()
    {
        QEventLoop loop;
        QTimer::singleShot(200, &loop, &QEventLoop::quit);
        loop.exec();
    }
}

void exerciseMediaRuntimeFailure(Content::ContentView &view, const QString &variantId,
    const QJsonObject &spec, QJsonObject &evidence)
{
    auto *controller = view.findChild<Qbutt::ContentController *>();
    require(controller && view.rootObject(), u"Runtime Save controller is missing"_s);
    waitFor([&] { return controller->property("downloadsState").toString() == u"ready"_s; }, u"Runtime Save queue"_s);
    auto *save = view.rootObject()->findChild<QQuickItem *>(u"contentSaveVideo"_s);
    auto *downloads = qobject_cast<QAbstractItemModel *>(controller->property("downloads").value<QObject *>());
    const QString expected = spec.value(u"saveRuntimeError"_s).toString();
    require(save && save->isEnabled() && downloads && downloads->rowCount() == 0 && !expected.isEmpty(),
        u"Runtime Save fixture needs an enabled exact video and an empty queue"_s);
    QList<quint64> requests;
    int rejected = 0;
    QObject observations;
    QObject::connect(controller, &Qbutt::ContentController::mediaToolsRequested, &observations,
        [&](const quint64 id) { requests.append(id); });
    QObject::connect(&view, &Content::ContentView::playbackDownloadFinished, &observations,
        [&](const quint64 token, const bool queued)
    {
        require(token == 0 && !queued, u"Runtime failure confirmed a download"_s);
        ++rejected;
    });
    const QPointF point = save->mapToScene(QPointF(save->width() / 2, save->height() / 2));
    const QPointF global = view.mapToGlobal(point.toPoint());
    QMouseEvent press(QEvent::MouseButtonPress, point, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
    QCoreApplication::sendEvent(&view, &press);
    QMouseEvent release(QEvent::MouseButtonRelease, point, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
    QCoreApplication::sendEvent(&view, &release);
    waitFor([&] { return rejected == 1; }, u"Actual runtime Save refusal"_s);
    require(requests.size() == 1 && controller->property("errorText").toString() == expected,
        u"Save lost the specific localized runtime reason"_s);
    const QString screenshots = spec.value(u"screenshots"_s).toString();
    if (!screenshots.isEmpty())
    {
        settle();
        require(view.grabFramebuffer().save(screenshots + u"/native-save-runtime-error.png"_s),
            u"Runtime Save render failed"_s);
    }

    // Control completion order after proving the real Runtime -> Rust -> UI path.
    QObject::disconnect(controller, &Qbutt::ContentController::mediaToolsRequested, &view, nullptr);
    require(controller->downloadSelected(variantId) && controller->property("errorText").toString().isEmpty(),
        u"New Save did not clear the previous runtime error"_s);
    waitFor([&] { return requests.size() == 2; }, u"Older Save tools request"_s);
    require(controller->downloadSelected(variantId), u"Concurrent Save was rejected"_s);
    waitFor([&] { return requests.size() == 3; }, u"Newer Save tools request"_s);
    controller->mediaToolsReady(requests.at(2), {}, {}, Qbutt::ContentError::MediaRuntimeTimeout);
    waitFor([&] { return rejected == 2; }, u"Newer Save failure"_s);
    const QString timeout = Media::Runtime::errorMessage(Media::Runtime::Error::Timeout, Media::Runtime::Purpose::Download);
    require(controller->property("errorText").toString() == timeout, u"Typed timeout reason was lost"_s);
    controller->mediaToolsReady(requests.at(1), {}, {}, Qbutt::ContentError::MediaRuntimeInvalidResponse);
    waitFor([&] { return rejected == 3; }, u"Older Save failure"_s);
    require(controller->property("errorText").toString() == timeout && downloads->rowCount() == 0,
        u"Late Save failure replaced the current error or queued a payload"_s);
    evidence[u"checks"_s] = QJsonArray {QJsonObject {
        {u"name"_s, u"native-save-runtime-error"_s}, {u"error"_s, expected},
        {u"reset"_s, true}, {u"staleIgnored"_s, true},
        {u"requests"_s, requests.size()}, {u"rejected"_s, rejected}, {u"jobs"_s, downloads->rowCount()}
    }};
}

void exerciseMediaSave(Content::ContentView &view, Media::Player &player, const quint64 token,
    const QUrl &url, const QString &variantId, const QJsonObject &spec, QJsonObject &evidence)
{
    auto *controller = view.findChild<Qbutt::ContentController *>();
    require(controller && view.canDownloadPlayback(token), u"Confirmed playback has no Save capability"_s);
    auto *downloads = qobject_cast<QAbstractItemModel *>(controller->property("downloads").value<QObject *>());
    require(downloads && downloads->rowCount() == 0, u"Save fixture queue is not empty"_s);
    const auto roles = downloads->roleNames();
    const auto data = [&](const QByteArray &role)
    {
        return downloads->data(downloads->index(0, 0), roles.key(role));
    };
    const QString pipeFile = spec.value(u"savePipeFile"_s).toString();
    QString pipe;
    waitFor([&]
    {
        QFile input(pipeFile);
        if (!input.open(QIODevice::ReadOnly))
            return false;
        pipe = QString::fromUtf8(input.readAll()).trimmed();
        return pipe.startsWith(u"qbutt-mpv-"_s);
    }, u"Actual qplay IPC path"_s);
    QLocalSocket receiver;
    receiver.connectToServer(pipe);
    waitFor([&] { return receiver.state() == QLocalSocket::ConnectedState; }, u"Save IPC observer"_s);
    QByteArray buffered;
    QJsonArray results;
    QObject observations;
    QObject::connect(&receiver, &QLocalSocket::readyRead, &observations, [&]
    {
        buffered += receiver.readAll();
        while (buffered.contains('\n'))
        {
            const int newline = buffered.indexOf('\n');
            const QJsonObject message = QJsonDocument::fromJson(buffered.left(newline)).object();
            const QJsonArray arguments = message.value(u"args"_s).toArray();
            buffered.remove(0, newline + 1);
            if (arguments.size() == 3 && arguments.first() == u"qplay-download-result"_s)
                results.append(arguments);
        }
    });
    int requests = 0;
    QList<bool> outcomes;
    QObject::connect(&player, &Media::Player::downloadRequested, &observations,
        [&](const quint64 requested)
    {
        require(requested == token, u"Save changed the launch token"_s);
        ++requests;
    });
    QObject::connect(&view, &Content::ContentView::playbackDownloadFinished, &observations,
        [&](const quint64 requested, const bool queued)
    {
        require(requested == token, u"DownloadQueue ACK changed token"_s);
        outcomes.append(queued);
    });
    const auto send = [&](quint64 requested, const QUrl &address)
    {
        const QJsonArray command {u"script-message"_s, u"qplay-download"_s, QString::number(requested),
            QString::fromUtf8(address.toEncoded())};
        receiver.write(QJsonDocument(QJsonObject {{u"command"_s, command}}).toJson(QJsonDocument::Compact) + '\n');
        receiver.flush();
    };
    auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
    require(variants && variants->rowCount() > 1, u"Save fixture needs another selectable exact video"_s);
    const int idRole = variants->roleNames().key("variantId");
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        const QString id = variants->data(variants->index(row, 0), idRole).toString();
        if (id != variantId)
        {
            controller->selectVariant(id);
            break;
        }
    }
    require(controller->property("selectedVariantId").toString() != variantId,
        u"Save fixture did not change catalogue selection"_s);
    send(token + 1, url);
    QUrl stale = url;
    stale.setQuery(u"stale=1"_s);
    send(token, stale);
    settle();
    require(requests == 0 && downloads->rowCount() == 0, u"Stale Save token or URL reached the queue"_s);
    send(token, url);
    send(token, url);
    if (spec.value(u"saveFaultRetry"_s).toBool())
    {
        waitFor([&] { return outcomes == QList<bool> {false}; }, u"Failed exact resolve ACK"_s);
        require(requests == 1 && downloads->rowCount() == 0, u"Failed resolve queued a payload or duplicate request"_s);
        send(token, url);
        send(token, url);
    }
    waitFor([&] { return !outcomes.isEmpty() && outcomes.last(); }, u"Durable DownloadQueue ACK"_s);
    waitFor([&]
    {
        return (downloads->rowCount() == 1) && (data("status").toString() == u"complete"_s);
    }, u"Verified legal media download"_s);
    QFile output(data("path").toString());
    require(output.open(QIODevice::ReadOnly), u"Save payload is missing"_s);
    const QByteArray bytes = output.readAll();
    const QString hash = QString::fromLatin1(QCryptographicHash::hash(bytes, QCryptographicHash::Sha256).toHex());
    require(hash == spec.value(u"saveSha256"_s).toString(), u"Save payload differs from the generated legal media"_s);
    settle();
    const int requestCount = requests;
    const int ackCount = results.size();
    send(token, url);
    waitFor([&] { return results.size() > ackCount; }, u"Repeated queued request ACK replay"_s);
    settle();
    require(requests == requestCount && downloads->rowCount() == 1
        && results.last().toArray() == QJsonArray {u"qplay-download-result"_s, QString::number(token), u"queued"_s},
        u"Repeated queued request started a second download"_s);
    evidence[u"save"_s] = QJsonObject {
        {u"exactVariant"_s, variantId},
        {u"requests"_s, requests},
        {u"jobs"_s, downloads->rowCount()},
        {u"sha256"_s, hash},
        {u"bytes"_s, bytes.size()},
        {u"payload"_s, output.fileName()},
        {u"queuedAckReplay"_s, true},
        {u"faultRetry"_s, spec.value(u"saveFaultRetry"_s)}
    };
}
