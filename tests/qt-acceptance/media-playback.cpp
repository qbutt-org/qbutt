/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "media-playback.h"

#include <functional>
#include <stdexcept>

#include <QCoreApplication>
#include <QElapsedTimer>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>
#include <QScopeGuard>
#include <QThread>

#include "base/global.h"
#include "base/media/player.h"

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const QString &label, const std::function<bool()> &ready)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 20000))
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
}

void MediaPlaybackAcceptance::run(QObject *errorReceiver, const QJsonObject &spec, QJsonObject &evidence)
{
    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    for (const QJsonValue &value : spec.value(u"selections"_s).toArray())
    {
        const QJsonObject selection = value.toObject();
        const QJsonObject wanted = selection.value(u"audioTrack"_s).toObject();
        Media::PlaybackOptions options;
        options.identity = selection.value(u"identity"_s).toString();
        options.audioTrack = {wanted.value(u"title"_s).toString(), wanted.value(u"type"_s).toString(), wanted.value(u"language"_s).toString(),
            wanted.value(u"codec"_s).toString(), wanted.value(u"channels"_s).toInt()};
        if (!selection.value(u"resume"_s).toBool())
            options.startPosition = 0;

        QStringList errors;
        QStringList started;
        QJsonArray tracks;
        double position = 0;
        double firstPosition = -1;
        QObject observations;
        auto *player = Media::Player::instance();
        const auto stop = qScopeGuard([] { Media::Player::freeInstance(); });
        // Capture the service error directly. Native error-dialog behavior has
        // its own GUI fault scenario; a modal dialog would block this runner.
        QObject::disconnect(player, &Media::Player::error, errorReceiver, nullptr);
        QObject::connect(player, &Media::Player::error, &observations, [&](const QString &message) { errors.append(message); });
        QObject::connect(player, &Media::Player::playbackStarted, &observations, [&](const QString &identity) { started.append(identity); });
        QObject::connect(player, &Media::Player::tracksChanged, &observations, [&](const QJsonArray &actual) { tracks = actual; });
        QObject::connect(player, &Media::Player::positionChanged, &observations, [&](const double actual)
        {
            position = actual;
            if ((actual >= 0) && (firstPosition < 0))
                firstPosition = actual;
        });
        require(player->playUrl(QUrl::fromLocalFile(spec.value(u"mediaPath"_s).toString()),
            selection.value(u"name"_s).toString(), options), u"Player rejected the legal media fixture"_s);
        waitFor(u"Actual mpv voice selection"_s, [&] { return !started.isEmpty() || !errors.isEmpty(); });

        QJsonObject check {{u"name"_s, selection.value(u"name"_s)}, {u"identity"_s, options.identity}};
        if (selection.value(u"rejected"_s).toBool())
        {
            require(started.isEmpty() && !errors.isEmpty(), u"Unavailable voice started playback"_s);
            require(position <= 0, u"Unavailable voice advanced before rejection"_s);
            check[u"rejected"_s] = true;
            check[u"error"_s] = errors.constFirst();
        }
        else
        {
            require(errors.isEmpty() && (started == QStringList {options.identity}), u"Wrong playback-start identity"_s);
            waitFor(u"Selected voice decoding"_s, [&] { return !errors.isEmpty() || (position >= selection.value(u"minimumPosition"_s).toDouble(1)); });
            require(errors.isEmpty(), u"Selected voice failed to decode"_s);
            const QJsonObject selected = selectedAudio(tracks);
            require(selected.value(u"id"_s).toInt() == selection.value(u"expectedId"_s).toInt(), u"The wrong actual mpv track was selected"_s);
            if (selection.value(u"resume"_s).toBool())
                require(firstPosition >= 1, u"Voice-specific playback position was not restored"_s);
            check[u"selected"_s] = selected;
            check[u"position"_s] = position;
            check[u"firstPosition"_s] = firstPosition;

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
        }
        checks.append(check);
        evidence[u"checks"_s] = checks;
    }
}
