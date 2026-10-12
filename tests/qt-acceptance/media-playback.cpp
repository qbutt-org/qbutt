/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "media-playback.h"

#include <cmath>
#include <functional>
#include <stdexcept>

#include <QCoreApplication>
#include <QCryptographicHash>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>
#include <QScopeGuard>
#include <QThread>

#include "base/global.h"
#include "base/media/player.h"
#include "base/media/runtime.h"
#include "base/profile.h"
#include "base/settingsstorage.h"

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const QString &label, const std::function<bool()> &ready, const int timeout = 20000)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < timeout))
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
        require(ready(), label + u" timed out"_s);
    }

    QJsonObject selectedAudio(const QJsonArray &tracks)
    {
        for (const QJsonValue &value : tracks)
        {
            const QJsonObject track = value.toObject();
            if ((track.value(u"type"_s).toString() == u"audio") && track.value(u"selected"_s).toBool())
                return track;
        }
        return {};
    }

    QString positionKey(const QByteArray &identity)
    {
        return u"Media/PlaybackPosition/"_s
            + QString::fromLatin1(QCryptographicHash::hash(identity, QCryptographicHash::Sha256).toHex());
    }
}

void MediaPlaybackAcceptance::run(QObject *errorReceiver, const QJsonObject &spec, QJsonObject &evidence)
{
    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    quint64 launchToken = 0;
    for (const QJsonValue &value : spec.value(u"selections"_s).toArray())
    {
        const QJsonObject selection = value.toObject();
        const QJsonObject wanted = selection.value(u"audioTrack"_s).toObject();
        Media::PlaybackOptions options;
        options.identity = selection.value(u"identity"_s).toString();
        options.contentKind = selection.value(u"contentKind"_s).toString();
        options.launchToken = selection.value(u"legacyLaunch"_s).toBool() ? 0 : ++launchToken;
        options.legacyIdentity = selection.value(u"legacyIdentity"_s).toString();
        options.proxyUrl = QUrl(selection.value(u"proxyUrl"_s).toString());
        options.audioTrack = {wanted.value(u"title"_s).toString(), wanted.value(u"type"_s).toString(), wanted.value(u"language"_s).toString(),
            wanted.value(u"codec"_s).toString(), wanted.value(u"channels"_s).toInt()};
        if (!selection.value(u"resume"_s).toBool())
            options.startPosition = 0;

        const QUrl url = selection.contains(u"url"_s) ? QUrl(selection.value(u"url"_s).toString())
            : QUrl::fromLocalFile(spec.value(u"mediaPath"_s).toString());
        const QString key = positionKey(options.identity.isEmpty() ? url.toEncoded() : options.identity.toUtf8());
        const QString legacyKey = positionKey(options.legacyIdentity.toUtf8());
        auto *settings = SettingsStorage::instance();
        if (selection.value(u"seedLegacyPosition"_s).isDouble())
            settings->storeValue(legacyKey, selection.value(u"seedLegacyPosition"_s).toDouble());
        settings->save();

        QStringList errors;
        QStringList started;
        QList<quint64> startedTokens;
        QList<quint64> finishedTokens;
        QList<quint64> expectedFinished;
        int downloads = 0;
        QJsonArray tracks;
        double position = 0;
        double firstPosition = -1;
        QObject observations;
        auto *player = Media::Player::instance();
        const auto stop = qScopeGuard([] { Media::Player::freeInstance(); });
        const quint64 firstToken = options.launchToken;
        // Capture the service error directly. Native error-dialog behavior has
        // its own GUI fault scenario; a modal dialog would block this runner.
        QObject::disconnect(player, &Media::Player::error, errorReceiver, nullptr);
        QObject::connect(player, &Media::Player::error, &observations, [&](const QString &message) { errors.append(message); });
        QObject::connect(player, &Media::Player::playbackStarted, &observations, [&](const QString &identity, const quint64 token)
        {
            started.append(identity);
            startedTokens.append(token);
        });
        QObject::connect(player, &Media::Player::playbackFinished, &observations, [&](const quint64 token)
        {
            finishedTokens.append(token);
            if ((token == firstToken) && selection.value(u"reentrantSupersede"_s).toBool())
            {
                expectedFinished.append(options.launchToken);
                options.launchToken = ++launchToken;
                require(player->playUrl(url, selection.value(u"name"_s).toString(), options),
                    u"Player rejected reentrant replacement playback"_s);
            }
        });
        QObject::connect(player, &Media::Player::tracksChanged, &observations, [&](const QJsonArray &actual) { tracks = actual; });
        QObject::connect(player, &Media::Player::downloadRequested, &observations, [&] { ++downloads; });
        QObject::connect(player, &Media::Player::positionChanged, &observations, [&](const double actual)
        {
            position = actual;
            if ((actual >= 0) && (firstPosition < 0))
                firstPosition = actual;
        });
        bool toolsReady = false;
        Media::Runtime::Result toolsResult;
        if (selection.value(u"sharedProbe"_s).toBool())
        {
            Media::Runtime::instance()->request(&observations, [&](const Media::Runtime::Result &result)
            {
                toolsResult = result;
                toolsReady = true;
            });
        }
        require(player->playUrl(url, selection.value(u"name"_s).toString(), options),
            u"Player rejected the legal media fixture"_s);
        if (selection.value(u"stopDuringPreparation"_s).toBool() || selection.value(u"cancelPreparationAndRetry"_s).toBool())
        {
            waitFor(u"Actual runtime probe process"_s, [&]
            {
                QFile trace(spec.value(u"probeTracePath"_s).toString());
                return trace.exists() && (trace.size() > 0);
            });
        }
        if (selection.value(u"cancelPreparationAndRetry"_s).toBool())
        {
            player->stop();
            expectedFinished.append(options.launchToken);
            require(startedTokens.isEmpty() && (finishedTokens == expectedFinished),
                u"Cancelled preparation lost or confirmed its token"_s);
            qputenv("QBUTT_LAB_QPLAY", spec.value(u"retryPlayerPath"_s).toString().toUtf8());
            options.launchToken = ++launchToken;
            require(player->playUrl(url, selection.value(u"name"_s).toString(), options),
                u"Player rejected retry after preparation cancellation"_s);
        }
        if (selection.value(u"stopBeforeLoaded"_s).toBool() || selection.value(u"stopDuringPreparation"_s).toBool())
        {
            player->stop();
            Media::Player::freeInstance();
            require(startedTokens.isEmpty() && (finishedTokens == QList<quint64> {options.launchToken}),
                u"Stopping pending playback lost or confirmed its launch token"_s);
            QJsonObject check {{u"name"_s, selection.value(u"name"_s)}, {u"pendingStop"_s, true}};
            if (selection.value(u"sharedProbe"_s).toBool())
            {
                waitFor(u"Shared media tools probe after player cancellation"_s, [&] { return toolsReady; });
                require((toolsResult.error == Media::Runtime::Error::None)
                    && QFileInfo(toolsResult.info.ffmpeg).isFile()
                    && QFileInfo(toolsResult.info.ffprobe).isFile(),
                    u"Player cancellation invalidated a concurrent tools request"_s);
                check[u"sharedTools"_s] = true;
            }
            checks.append(check);
            evidence[u"checks"_s] = checks;
            continue;
        }
        if (selection.value(u"supersedePending"_s).toBool())
        {
            expectedFinished.append(options.launchToken);
            options.launchToken = ++launchToken;
            require(player->playUrl(url, selection.value(u"name"_s).toString(), options), u"Player rejected replacement playback"_s);
            require(startedTokens.isEmpty() && (finishedTokens == expectedFinished),
                u"Replacing pending playback lost or confirmed its previous token"_s);
        }
        waitFor(u"Actual mpv voice selection"_s, [&] { return !started.isEmpty() || !errors.isEmpty(); },
            selection.value(u"timeoutMs"_s).toInt(20000));

        QJsonObject check {{u"name"_s, selection.value(u"name"_s)}, {u"identity"_s, options.identity}};
        if (selection.value(u"rejected"_s).toBool())
        {
            require(started.isEmpty() && !errors.isEmpty(), u"Unavailable voice started playback"_s);
            require(position <= 0, u"Unavailable voice advanced before rejection"_s);
            check[u"rejected"_s] = true;
            check[u"error"_s] = errors.constFirst();
            Media::Player::freeInstance();
        }
        else
        {
            require(errors.isEmpty(), u"Playback failed before confirmation: "_s + errors.join(u"; "_s));
            require(started == QStringList {options.identity}, u"Wrong playback-start identity: "_s + started.join(u", "_s));
            require(startedTokens == QList<quint64> {options.launchToken}, u"Playback confirmed another launch token"_s);
            waitFor(u"Selected voice decoding"_s, [&] { return !errors.isEmpty() || (position >= selection.value(u"minimumPosition"_s).toDouble(1)); });
            require(errors.isEmpty(), u"Selected voice failed to decode"_s);
            if (selection.value(u"supersedePlaying"_s).toBool())
            {
                expectedFinished.append(options.launchToken);
                options.launchToken = ++launchToken;
                started.clear();
                startedTokens.clear();
                require(player->playUrl(url, selection.value(u"name"_s).toString(), options),
                    u"Player rejected a replacement after file-loaded"_s);
                require(finishedTokens == expectedFinished, u"Replacing current playback lost its token"_s);
                waitFor(u"Replacement mpv file-loaded"_s, [&] { return !startedTokens.isEmpty() || !errors.isEmpty(); });
                require(errors.isEmpty() && (startedTokens == QList<quint64> {options.launchToken}),
                    u"Replacement playback confirmed another token"_s);
                waitFor(u"Replacement decoding"_s, [&] { return !errors.isEmpty() || (position >= 1); });
                require(errors.isEmpty(), u"Replacement playback failed to decode"_s);
            }
            const QJsonObject selected = selectedAudio(tracks);
            require(selected.value(u"id"_s).toInt() == selection.value(u"expectedId"_s).toInt(), u"The wrong actual mpv track was selected"_s);
            if (selection.value(u"resume"_s).toBool())
                require(firstPosition >= 1, u"Saved playback position was not restored"_s);
            const QJsonArray range = selection.value(u"firstPositionRange"_s).toArray();
            if (!range.isEmpty())
                require((firstPosition >= range.at(0).toDouble()) && (firstPosition <= range.at(1).toDouble()),
                    u"Playback started outside the expected bookmark range: "_s + QString::number(firstPosition));
            check[u"selected"_s] = selected;
            check[u"position"_s] = position;
            check[u"firstPosition"_s] = firstPosition;
            if (selection.value(u"checkDownload"_s).toBool())
            {
                require(downloads == (options.launchToken == 0 ? 1 : 0), u"Download IPC bypassed the playback capability"_s);
                check[u"downloads"_s] = downloads;
            }

            if (selection.value(u"switchTrack"_s).isDouble())
            {
                QFile trace(spec.value(u"tracePath"_s).toString());
                require(trace.open(QIODevice::ReadOnly), u"Cannot read actual mpv IPC trace"_s);
                QString pipe;
                for (const QByteArray &line : trace.readAll().split('\n'))
                {
                    const QJsonObject event = QJsonDocument::fromJson(line).object();
                    if (event.value(u"title"_s).toString() == selection.value(u"name"_s).toString())
                        pipe = event.value(u"ipc"_s).toString();
                }
                require(!pipe.isEmpty(), u"The actual mpv IPC endpoint was not recorded"_s);
                pipe.remove(u"\\\\.\\pipe\\"_s);
                QLocalSocket user;
                user.connectToServer(pipe);
                waitFor(u"Actual mpv user-control IPC"_s, [&] { return user.state() == QLocalSocket::ConnectedState; });
                user.write(QJsonDocument(QJsonObject {{u"command"_s, QJsonArray {u"set_property"_s, u"aid"_s,
                    selection.value(u"switchTrack"_s)}}}).toJson(QJsonDocument::Compact) + '\n');
                user.flush();
                const double beforeSwitch = position;
                waitFor(u"User audio selection remains active"_s, [&]
                {
                    return !errors.isEmpty() || ((selectedAudio(tracks).value(u"id"_s) == selection.value(u"switchTrack"_s))
                        && (position >= beforeSwitch + 1));
                });
                require(errors.isEmpty(), u"User audio switching failed"_s);
                check[u"userSelected"_s] = selectedAudio(tracks);
                check[u"userSwitchPosition"_s] = position;
            }

            // Stop the real player before reading the persisted bookmark. A
            // subsequent selection creates a fresh qplay process and IPC session.
            Media::Player::freeInstance();
            settings->save();
            const auto persisted = Profile::instance()->applicationSettings(u"qbutt"_s);
            const double savedPosition = persisted->value(key).toDouble();
            require(std::abs(savedPosition - position) < 0.5, u"The stopped player's bookmark was not persisted"_s);
            check[u"savedPosition"_s] = savedPosition;
            if (!options.legacyIdentity.isEmpty())
                check[u"legacyPosition"_s] = persisted->contains(legacyKey)
                    ? QJsonValue(persisted->value(legacyKey).toDouble()) : QJsonValue(QJsonValue::Null);
        }
        if (options.launchToken != 0)
            expectedFinished.append(options.launchToken);
        require(finishedTokens == expectedFinished, u"An accepted playback launch has no unique terminal outcome"_s);
        check[u"launchToken"_s] = static_cast<qint64>(options.launchToken);
        check[u"uniquePlaybackFinish"_s] = true;
        checks.append(check);
        evidence[u"checks"_s] = checks;
    }
}
