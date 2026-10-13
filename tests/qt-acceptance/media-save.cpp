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
#include <QTimer>

#include "base/media/player.h"
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
