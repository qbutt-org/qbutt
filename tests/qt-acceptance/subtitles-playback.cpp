/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "subtitles-playback.h"

#include <functional>
#include <stdexcept>

#include <QCoreApplication>
#include <QElapsedTimer>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QThread>

#include "base/global.h"
#include "base/content/client.h"
#include "base/media/player.h"

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const QString &label, const std::function<bool()> &ready, const int timeout = 15000)
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

    QJsonArray readTrace(const QString &path)
    {
        QFile file(path);
        if (!file.open(QIODevice::ReadOnly))
            return {};
        QJsonArray events;
        for (const QByteArray &line : file.readAll().split('\n'))
        {
            const QJsonDocument document = QJsonDocument::fromJson(line);
            if (document.isObject())
                events.append(document.object());
        }
        return events;
    }

    bool helperEvent(const QJsonObject &spec, const QString &identity, const QString &event)
    {
        for (const QJsonValue &value : readTrace(spec.value(u"helperTrace"_s).toString()))
        {
            const QJsonObject item = value.toObject();
            if ((item.value(u"identity"_s).toString() == identity) && (item.value(u"event"_s).toString() == event))
                return true;
        }
        return false;
    }

    QJsonObject selectedSubtitle(const QJsonArray &tracks)
    {
        for (const QJsonValue &value : tracks)
        {
            const QJsonObject track = value.toObject();
            if ((track.value(u"type"_s).toString() == u"sub") && track.value(u"selected"_s).toBool()
                && (track.value(u"main-selection"_s).toInt() == 0))
                return track;
        }
        return {};
    }

    QJsonValue mpvCommand(QLocalSocket &socket, const QJsonArray &command)
    {
        socket.write(QJsonDocument(QJsonObject {{u"command"_s, command}, {u"request_id"_s, 1}})
            .toJson(QJsonDocument::Compact) + '\n');
        socket.flush();
        QByteArray response;
        QJsonObject result;
        waitFor(u"Independent actual mpv IPC command"_s, [&]
        {
            if (!result.isEmpty())
                return true;
            response += socket.readAll();
            qsizetype newline = -1;
            while ((newline = response.indexOf('\n')) >= 0)
            {
                const QJsonObject message = QJsonDocument::fromJson(response.first(newline)).object();
                response.remove(0, newline + 1);
                if (message.value(u"request_id"_s).toInt() == 1)
                {
                    result = message;
                    return true;
                }
            }
            return false;
        });
        require(result.value(u"error"_s).toString() == u"success", u"Actual mpv rejected acceptance command"_s);
        return result.value(u"data"_s);
    }

    QString ipcEndpoint(const QJsonObject &spec, const QString &title)
    {
        QString pipe;
        waitFor(u"Actual player IPC trace"_s, [&]
        {
            for (const QJsonValue &value : readTrace(spec.value(u"mpvTrace"_s).toString()))
            {
                const QJsonObject item = value.toObject();
                if (item.value(u"title"_s).toString() == title)
                    pipe = item.value(u"ipc"_s).toString();
            }
            return !pipe.isEmpty();
        });
        pipe.remove(u"\\\\.\\pipe\\"_s);
        return pipe;
    }
}

void SubtitlesPlaybackAcceptance::run(QObject *errorReceiver, const QJsonObject &spec, QJsonObject &evidence)
{
    QObject helperObservations;
    QJsonArray helperEvents;
    int codexTurns = 0;
    bool configured = false;
    QString configurationError;
    qint64 configuration = 0;
    if (spec.value(u"realCodex"_s).toBool())
    {
        QObject::connect(Content::Client::instance(), &Content::Client::eventReceived, &helperObservations,
            [&](const qint64 id, const QString &event, const QJsonValue &data)
        {
            if (id == configuration)
            {
                configured = (event == u"complete");
                if (event == u"error")
                    configurationError = data.toObject().value(u"message"_s).toString();
            }
            if ((event == u"subtitle") || (event == u"subtitleProgress"))
            {
                helperEvents.append(QJsonObject {{u"id"_s, id}, {u"event"_s, event}, {u"data"_s, data}});
                evidence[u"helperEvents"_s] = helperEvents;
                if ((event == u"subtitleProgress") && (data.toObject().value(u"provider"_s).toString() == u"codex")
                    && (data.toObject().value(u"phase"_s).toString() == u"translating"))
                    ++codexTurns;
            }
        });
        configuration = Content::Client::instance()->request(u"configureSource"_s,
            {{u"id"_s, u"ai"_s}, {u"settings"_s, QJsonObject {{u"provider"_s, u"codex"_s}, {u"model"_s, u""_s}}}});
        waitFor(u"Real helper Codex configuration through native ContentClient"_s,
            [&] { return configured || !configurationError.isEmpty(); }, 30000);
        require(configured && configurationError.isEmpty(), u"Real helper configuration failed: "_s + configurationError);
    }
    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    for (const QJsonValue &value : spec.value(u"scenarios"_s).toArray())
    {
        const QJsonObject scenario = value.toObject();
        const QString name = scenario.value(u"name"_s).toString();
        const QString mode = scenario.value(u"mode"_s).toString();
        Media::PlaybackOptions options;
        options.identity = scenario.value(u"identity"_s).toString();
        options.startPosition = 0;
        const QUrl media = QUrl::fromLocalFile(scenario.value(u"mediaPath"_s).toString());
        if (scenario.contains(u"externalSubtitle"_s))
            options.subtitles.append(QUrl::fromLocalFile(scenario.value(u"externalSubtitle"_s).toString()));

        QObject observations;
        QStringList errors;
        QStringList statuses;
        QStringList started;
        QJsonArray tracks;
        double position = 0;
        auto *player = Media::Player::instance();
        const auto stop = qScopeGuard([] { Media::Player::freeInstance(); });
        QObject::disconnect(player, &Media::Player::error, errorReceiver, nullptr);
        QObject::connect(player, &Media::Player::error, &observations, [&](const QString &message) { errors.append(message); });
        QObject::connect(player, &Media::Player::subtitleTranslationChanged, &observations,
            [&](const QString &message) { statuses.append(message); });
        QObject::connect(player, &Media::Player::playbackStarted, &observations, [&](const QString &identity) { started.append(identity); });
        QObject::connect(player, &Media::Player::tracksChanged, &observations, [&](const QJsonArray &actual) { tracks = actual; });
        QObject::connect(player, &Media::Player::positionChanged, &observations, [&](const double actual) { position = actual; });
        require(player->playUrl(media, name, options), name + u": legal media fixture was rejected"_s);
        waitFor(name + u": playback started"_s, [&] { return !started.isEmpty() || !errors.isEmpty(); });
        require(errors.isEmpty() && (started == QStringList {options.identity}), name + u": playback failed or identity changed"_s);

        QLocalSocket user;
        user.connectToServer(ipcEndpoint(spec, name));
        waitFor(u"Independent actual mpv control"_s, [&] { return user.state() == QLocalSocket::ConnectedState; });
        QJsonObject check {{u"name"_s, name}, {u"identity"_s, options.identity}};
        check[u"demuxerStartTime"_s] = mpvCommand(user, {u"get_property"_s, u"demuxer-start-time"_s});
        const auto translated = [&]
        {
            return selectedSubtitle(tracks).value(u"external-filename"_s).toString().contains(u"translated-");
        };

        if (mode == u"real-codex")
        {
            mpvCommand(user, {u"set_property"_s, u"pause"_s, true});
            waitFor(u"Real Codex translated track selected by production Player"_s, translated, 180000);
            require(tracks.size() == scenario.value(u"expectedTracks"_s).toInt(), u"Real Codex translation lost the original track"_s);
            bool originalRetained = false;
            for (const QJsonValue &value : tracks)
            {
                const QJsonObject track = value.toObject();
                originalRetained = originalRetained || ((track.value(u"type"_s).toString() == u"sub")
                    && (track.value(u"id"_s).toInt() == 1) && !track.value(u"external"_s).toBool()
                    && (track.value(u"codec"_s).toString() == u"ass"));
            }
            require(originalRetained, u"Real Codex flow removed the embedded original subtitles"_s);
            require(selectedSubtitle(tracks).value(u"lang"_s).toString() == u"ru", u"Real Codex track did not declare Russian"_s);
            waitFor(u"Actual mpv decoded real Codex Russian text"_s, [&]
            {
                check[u"text"_s] = mpvCommand(user, {u"get_property"_s, u"sub-text"_s});
                return QRegularExpression(u"[А-Яа-яЁё]"_s).match(check.value(u"text"_s).toString()).hasMatch();
            });
            check[u"codexTurns"_s] = codexTurns;
        }
        else if ((mode == u"pending-original") || (mode == u"seek") || (mode == u"eof-rewind")
            || (mode == u"eof-off") || (mode == u"eof-hidden")
            || (mode == u"delay") || (mode == u"navigate") || (mode == u"stop"))
        {
            waitFor(u"Pending subtitle translation"_s, [&] { return helperEvent(spec, options.identity, u"request"_s); });
            if (mode == u"pending-original")
            {
                require(selectedSubtitle(tracks).value(u"id"_s).toInt() == 1, u"Original was not selected before the user's choice"_s);
                mpvCommand(user, {u"set_property"_s, u"sid"_s, 1});
                waitFor(u"Explicit original choice cancelled pending translation"_s,
                    [&] { return helperEvent(spec, options.identity, u"cancel"_s); });
                waitFor(u"Cancelled translation delivered its late result"_s,
                    [&] { return helperEvent(spec, options.identity, u"late"_s); });
                const double userPosition = position;
                waitFor(u"Playback continues with the explicit original choice"_s, [&] { return position >= userPosition + 0.5; });
                const QJsonArray actual = mpvCommand(user, {u"get_property"_s, u"track-list"_s}).toArray();
                require(selectedSubtitle(actual).value(u"id"_s).toInt() == 1, u"Late translation overrode the explicit original choice"_s);
                check[u"userSelectedId"_s] = 1;
            }
            else if ((mode == u"seek") || (mode == u"eof-rewind") || (mode == u"eof-off")
                || (mode == u"eof-hidden") || (mode == u"delay"))
            {
                if ((mode == u"eof-rewind") || (mode == u"eof-off") || (mode == u"eof-hidden"))
                {
                    waitFor(u"First translated subtitles before EOF"_s, translated);
                    mpvCommand(user, {u"seek"_s, 300, u"absolute+exact"_s});
                    waitFor(u"Actual mpv reached EOF while keeping the file open"_s,
                        [&] { return mpvCommand(user, {u"get_property"_s, u"eof-reached"_s}).toBool(); });
                }
                if (mode == u"eof-off")
                    mpvCommand(user, {u"set_property"_s, u"sid"_s, u"no"_s});
                else if (mode == u"eof-hidden")
                    mpvCommand(user, {u"set_property"_s, u"sub-visibility"_s, false});
                if (mode != u"delay")
                    mpvCommand(user, {u"seek"_s, 180, u"absolute+exact"_s});
                else
                    mpvCommand(user, {u"set_property"_s, u"sub-delay"_s, -180});
                if ((mode == u"eof-rewind") || (mode == u"eof-off") || (mode == u"eof-hidden"))
                    mpvCommand(user, {u"set_property"_s, u"pause"_s, false});
                waitFor(u"Seek cancelled the previous subtitle request"_s,
                    [&] { return helperEvent(spec, options.identity, u"cancel"_s); });
                if ((mode == u"eof-off") || (mode == u"eof-hidden"))
                {
                    waitFor(u"Playback resumed with subtitles off"_s, [&] { return position >= 180.5; });
                    const QJsonArray actual = mpvCommand(user, {u"get_property"_s, u"track-list"_s}).toArray();
                    if (mode == u"eof-off")
                        require(selectedSubtitle(actual).isEmpty(), u"Rewind after EOF overrode the user's subtitle Off choice"_s);
                    else
                        require(!mpvCommand(user, {u"get_property"_s, u"sub-visibility"_s}).toBool(), u"Rewind after EOF made hidden subtitles visible"_s);
                    check[u"userSelectedId"_s] = selectedSubtitle(actual).value(u"id"_s).toInt();
                }
                else
                {
                    waitFor(u"New subtitle window selected"_s, [&] { return translated() && ((mode == u"delay") || (position >= 180)); });
                    waitFor(u"Actual mpv decoded the new subtitle window"_s, [&]
                    {
                        check[u"text"_s] = mpvCommand(user, {u"get_property"_s, u"sub-text"_s});
                        return check.value(u"text"_s).toString().contains(u"180");
                    });
                }
            }
            else if (mode == u"navigate")
            {
                const QString previous = options.identity;
                options.identity += u":next"_s;
                require(player->playUrl(media, name + u":next"_s, options), u"Navigation fixture rejected"_s);
                waitFor(u"Navigation cancelled old subtitles"_s, [&] { return helperEvent(spec, previous, u"cancel"_s); });
                waitFor(u"Navigation started exact next identity"_s, [&] { return started.contains(options.identity); });
                user.abort();
                user.connectToServer(ipcEndpoint(spec, name + u":next"_s));
                waitFor(u"Next actual player IPC"_s, [&] { return user.state() == QLocalSocket::ConnectedState; });
                waitFor(u"Next identity selected translated subtitles"_s, translated);
                check[u"text"_s] = mpvCommand(user, {u"get_property"_s, u"sub-text"_s});
                require(check.value(u"text"_s).toString().contains(u"next"), u"Previous identity subtitles were applied to next playback"_s);
            }
            else
            {
                player->stop();
                waitFor(u"Stop cancelled old subtitles"_s, [&] { return helperEvent(spec, options.identity, u"cancel"_s); });
                waitFor(u"Stopped actual mpv"_s, [&] { return user.state() == QLocalSocket::UnconnectedState; });
                check[u"cancelled"_s] = true;
            }
        }
        else if ((mode == u"early-off") || (mode == u"unavailable") || (mode == u"already-target") || (mode == u"no-track") || (mode == u"existing-ru"))
        {
            waitFor(u"Playback remains usable without AI work"_s, [&] { return position >= 1.5; });
            require(!translated(), u"An unnecessary generated subtitle track was selected"_s);
            if (mode == u"unavailable")
            {
                int messages = 0;
                for (const QString &status : statuses)
                    messages += status.contains(u"Fixture AI unavailable") ? 1 : 0;
                require(messages == 1, u"Unavailable AI did not produce exactly one actionable nonfatal message"_s);
            }
            if ((mode == u"early-off") || (mode == u"no-track") || (mode == u"existing-ru"))
                require(!helperEvent(spec, options.identity, u"request"_s), u"A subtitle AI request was unnecessary"_s);
            if (mode == u"existing-ru")
                require(selectedSubtitle(tracks).value(u"lang"_s).toString() == u"rus", u"The available Russian subtitle track was not selected"_s);
            if (mode == u"early-off")
            {
                require(selectedSubtitle(tracks).isEmpty(), u"Initial user Off was overridden during subtitle preparation"_s);
                options.identity += u":next"_s;
                require(player->playUrl(media, name + u":next"_s, options), u"Navigation after early Off was rejected"_s);
                waitFor(u"New playback after user Off"_s, [&] { return started.contains(options.identity); });
                user.abort();
                user.connectToServer(ipcEndpoint(spec, name + u":next"_s));
                waitFor(u"New player IPC after user Off"_s, [&] { return user.state() == QLocalSocket::ConnectedState; });
                waitFor(u"New playback restores automatic subtitle translation"_s, translated);
                check[u"nextSelected"_s] = selectedSubtitle(tracks);
            }
        }
        else
        {
            waitFor(u"Actual translated Russian subtitle track selected"_s, translated);
            const QJsonObject first = selectedSubtitle(tracks);
            check[u"firstSelected"_s] = first;
            require(first.value(u"lang"_s).toString() == u"ru", u"Generated track does not declare Russian"_s);
            require(tracks.size() == scenario.value(u"expectedTracks"_s).toInt(), u"Original subtitle tracks were lost or duplicated"_s);
            bool original = false;
            for (const QJsonValue &track : tracks)
            {
                const QJsonObject actual = track.toObject();
                original = original || ((actual.value(u"type"_s).toString() == u"sub")
                    && (actual.value(u"id"_s).toInt() == 1));
            }
            require(original, u"Original subtitle track was not preserved"_s);
            if ((mode == u"original") || (mode == u"off"))
            {
                mpvCommand(user, {u"set_property"_s, u"sid"_s, (mode == u"off") ? QJsonValue(u"no"_s) : QJsonValue(1)});
                waitFor(u"User subtitle choice cancelled automatic translation"_s,
                    [&] { return helperEvent(spec, options.identity, u"cancel"_s); });
                waitFor(u"Deliberately late subtitle event was delivered"_s,
                    [&] { return helperEvent(spec, options.identity, u"late"_s); });
                const double userPosition = position;
                waitFor(u"Playback continues after the late event"_s, [&] { return position >= userPosition + 0.5; });
                const QJsonArray actual = mpvCommand(user, {u"get_property"_s, u"track-list"_s}).toArray();
                const int selectedId = selectedSubtitle(actual).value(u"id"_s).toInt();
                require(selectedId == ((mode == u"off") ? 0 : 1), u"Late automatic translation overrode the user's subtitle choice"_s);
                check[u"userSelectedId"_s] = selectedId;
            }
            else
            {
                waitFor(u"Immutable next subtitle revision selected"_s, [&]
                {
                    return translated() && (selectedSubtitle(tracks).value(u"external-filename"_s) != first.value(u"external-filename"_s));
                });
                waitFor(u"Replaced generated track removed"_s,
                    [&] { return tracks.size() == scenario.value(u"expectedTracks"_s).toInt(); });
                check[u"text"_s] = mpvCommand(user, {u"get_property"_s, u"sub-text"_s});
                require(check.value(u"text"_s).toString().contains(u"Вторая версия"), u"Translated text was not decoded by actual mpv"_s);
                check[u"replacedSelected"_s] = selectedSubtitle(tracks);
            }
        }
        require(errors.isEmpty(), name + u": subtitle processing interrupted normal playback"_s);
        check[u"position"_s] = position;
        check[u"selected"_s] = selectedSubtitle(tracks);
        check[u"statuses"_s] = QJsonArray::fromStringList(statuses);
        checks.append(check);
        evidence[u"checks"_s] = checks;
        if (mode == u"real-codex")
            require(codexTurns == spec.value(u"expectedCodexTurns"_s).toInt(1), u"Installed Codex turn count did not match the translation/cache scenario"_s);
    }
}
