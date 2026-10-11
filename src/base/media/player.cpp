/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "player.h"

#include <algorithm>
#include <cmath>
#include <utility>

#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocale>
#include <QMimeDatabase>
#include <QProcessEnvironment>
#include <QRegularExpression>
#include <QSet>
#include <QUuid>

#include "base/bittorrent/infohash.h"
#include "base/bittorrent/torrent.h"
#include "base/content/client.h"
#include "base/global.h"
#include "base/path.h"
#include "base/profile.h"
#include "base/settingsstorage.h"
#include "streamserver.h"

using namespace Media;

namespace
{
    const QRegularExpression HTTP_TOKEN_PATTERN {u"^[!#$%&'*+.^_`|~0-9A-Za-z-]+$"_s};
    constexpr int AUDIO_TRACKS_REQUEST = 1;
    constexpr int AUDIO_SELECT_REQUEST = 2;
    constexpr int AUDIO_VERIFY_REQUEST = 3;

    bool isRussian(const QString &language)
    {
        return QLocale::codeToLanguage(language.section(u'-', 0, 0).section(u'_', 0, 0)) == QLocale::Russian;
    }

    int selectedSubtitle(const QJsonArray &tracks)
    {
        for (const QJsonValue &value : tracks)
        {
            const QJsonObject track = value.toObject();
            if ((track.value(u"type"_s).toString() == u"sub") && track.value(u"selected"_s).toBool()
                && (track.value(u"main-selection"_s).toInt() == 0))
                return track.value(u"id"_s).toInt();
        }
        return 0;
    }

    QUrl subtitleUrl(const QString &fileName)
    {
        const QUrl url(fileName);
        return url.scheme().isEmpty() || QFileInfo(fileName).isAbsolute() ? QUrl::fromLocalFile(fileName) : url;
    }
}

QPointer<Player> Player::m_instance;

Player::Player(QObject *parent)
    : QObject(parent)
{
    connect(StreamServer::instance(), &StreamServer::error, this, &Player::error);
    connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, this, &Player::stop);
    connect(Content::Client::instance(), &Content::Client::eventReceived, this,
        [this](const qint64 id, const QString &event, const QJsonValue &data)
    {
        if ((id != 0) && (id == m_subtitles.request))
        {
            receiveSubtitles(event, data.toObject());
            return;
        }
        if ((id == 0) || (id != m_preparationRequest))
            return;

        if ((event != u"complete") && (event != u"error"))
            return;

        m_preparationRequest = 0;
        emit preparationChanged({});
        if (event == u"error")
        {
            StreamServer::instance()->closeStream(m_pending.url);
            m_pending = {};
            emit error(data.toObject().value(u"message"_s).toString());
            return;
        }
        if (m_pending.url.isValid())
            startPrepared(data.toObject());
    });

    m_process.setStandardOutputFile(QProcess::nullDevice());
    m_process.setStandardErrorFile(QProcess::nullDevice());

    m_connectTimer.setInterval(100);
    connect(&m_connectTimer, &QTimer::timeout, this, &Player::connectIpc);
    connect(&m_ipc, &QLocalSocket::connected, this, &Player::loadMedia);
    connect(&m_ipc, &QLocalSocket::readyRead, this, &Player::readMessages);

    m_stopTimer.setSingleShot(true);
    m_stopTimer.setInterval(2000);
    connect(&m_stopTimer, &QTimer::timeout, &m_process, &QProcess::kill);

    m_saveTimer.setInterval(10000);
    connect(&m_saveTimer, &QTimer::timeout, this, &Player::savePosition);

    connect(&m_process, &QProcess::started, this, [this]
    {
        if (m_stopping)
            return;

        m_startupTime.start();
        m_connectTimer.start();
        connectIpc();
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](const QProcess::ProcessError processError)
    {
        if (processError == QProcess::FailedToStart)
        {
            if (!m_stopping)
                emit error(tr("The player could not start: %1").arg(m_process.errorString()));

            finishPlayback();
        }
    });
    connect(&m_process, &QProcess::finished, this, [this](const int exitCode, const QProcess::ExitStatus exitStatus)
    {
        if (!m_stopping && ((exitStatus == QProcess::CrashExit) || (exitCode != 0)))
            emit error(tr("The player stopped unexpectedly (exit code %1).").arg(exitCode));

        finishPlayback();
    });
}

Player::~Player()
{
    stop();
    if ((m_process.state() != QProcess::NotRunning) && !m_process.waitForFinished(1000))
    {
        m_process.kill();
        m_process.waitForFinished(1000);
    }
}

void Player::initInstance(QObject *parent)
{
    if (!m_instance)
        m_instance = new Player(parent ? parent : QCoreApplication::instance());
}

void Player::freeInstance()
{
    delete m_instance;
}

Player *Player::instance()
{
    initInstance();
    return m_instance;
}

bool Player::isMediaFile(const QString &fileName)
{
    static const QStringList playlists {u"m3u"_s, u"m3u8"_s, u"pls"_s, u"xspf"_s, u"asx"_s,
        u"wax"_s, u"wvx"_s, u"ram"_s, u"rpm"_s, u"smil"_s};
    if (playlists.contains(QFileInfo(fileName).suffix().toLower()))
        return false;

    const QString mime = QMimeDatabase().mimeTypeForFile(fileName, QMimeDatabase::MatchExtension).name();
    return mime.startsWith(u"video/") || mime.startsWith(u"audio/") || (mime == u"application/ogg");
}

bool Player::playUrl(const QUrl &url, const QString &title, const PlaybackOptions &options)
{
    const QString scheme = url.scheme().toLower();
    if (!url.isValid() || !((scheme == u"file") || (scheme == u"http") || (scheme == u"https")))
    {
        emit error(tr("Choose a local media file or an HTTP video stream."));
        return false;
    }
    if ((scheme == u"file") && !isMediaFile(url.fileName()))
    {
        emit error(tr("Choose a local audio or video file."));
        return false;
    }
    if ((options.audioTrack.channels < 0)
        || (!options.audioTrack.title.isEmpty() && options.audioTrack.title.simplified().isEmpty())
        || (!options.audioTrack.type.isEmpty() && options.audioTrack.type.simplified().isEmpty())
        || (options.audioTrack.title.isEmpty() && (!options.audioTrack.type.isEmpty() || !options.audioTrack.language.isEmpty()
            || !options.audioTrack.codec.isEmpty() || (options.audioTrack.channels != 0))))
    {
        emit error(tr("The video source returned incomplete voice information. Choose the voice again."));
        return false;
    }

    bool network = (scheme == u"http") || (scheme == u"https");
    for (const QUrl &source : options.subtitles + QList<QUrl> {options.audioUrl})
    {
        if (source.isEmpty())
            continue;

        const QString sourceScheme = source.scheme().toLower();
        if (!source.isValid() || !((sourceScheme == u"file") || (sourceScheme == u"http") || (sourceScheme == u"https")))
        {
            emit error(tr("The video source returned an invalid audio or subtitle URL."));
            return false;
        }
        network = network || (sourceScheme == u"http") || (sourceScheme == u"https");
    }
    if (network && ((options.proxyUrl.scheme() != u"http") || (options.proxyUrl.host() != u"127.0.0.1")
        || (options.proxyUrl.port() <= 0) || (options.proxyUrl.userName() != u"qbutt") || options.proxyUrl.password().isEmpty()
        || options.proxyUrl.hasQuery() || options.proxyUrl.hasFragment()
        || (!options.proxyUrl.path().isEmpty() && (options.proxyUrl.path() != u"/"))))
    {
        emit error(tr("The video source did not provide a protected connection. Try opening it again."));
        return false;
    }

    static const QRegularExpression controlPattern(u"[\\x00-\\x1f\\x7f]"_s);
    for (auto it = options.headers.cbegin(); it != options.headers.cend(); ++it)
    {
        if ((it.key().compare(u"Authorization", Qt::CaseInsensitive) == 0)
            || (it.key().compare(u"Proxy-Authorization", Qt::CaseInsensitive) == 0))
        {
            emit error(tr("This video source requires an unsupported authorization header."));
            return false;
        }
        if (!HTTP_TOKEN_PATTERN.match(it.key()).hasMatch() || controlPattern.match(it.value()).hasMatch())
        {
            emit error(tr("The video source returned an invalid HTTP header."));
            return false;
        }
    }

    return startPlayback(url, title, options);
}

bool Player::startPlayback(const QUrl &url, const QString &title, const PlaybackOptions &options)
{
    const QByteArray positionIdentity = options.identity.isEmpty() ? url.toEncoded() : options.identity.toUtf8();
    StreamServer::instance()->closeStream(m_pending.url);
    m_pending = {url, title, options, u"Media/PlaybackPosition/"_s
        + QString::fromLatin1(QCryptographicHash::hash(positionIdentity, QCryptographicHash::Sha256).toHex())};
    if (m_process.state() == QProcess::NotRunning)
        return startPending();

    stopProcess();
    return true;
}

QString Player::torrentIdentity(const BitTorrent::Torrent *torrent, const int fileIndex)
{
    return u"torrent:"_s + torrent->infoHash().toString() + u':' + QString::number(fileIndex);
}

bool Player::playTorrent(BitTorrent::Torrent *torrent, const int fileIndex, const QString &contentKind)
{
    if (!torrent || (fileIndex < 0) || (fileIndex >= torrent->filePaths().size()))
    {
        emit error(tr("Choose a media file in the torrent."));
        return false;
    }

    const QUrl url = StreamServer::instance()->openTorrent(torrent, fileIndex);
    if (!url.isValid())
        return false;

    PlaybackOptions options;
    options.contentKind = contentKind;
    options.identity = torrentIdentity(torrent, fileIndex);
    return startPlayback(url, torrent->filePaths().at(fileIndex).filename(), options);
}

void Player::stop()
{
    if (m_preparationRequest != 0)
    {
        Content::Client::instance()->cancel(std::exchange(m_preparationRequest, 0));
        emit preparationChanged({});
    }
    StreamServer::instance()->closeStream(m_pending.url);
    m_pending = {};
    stopProcess();
}

bool Player::startPending()
{
    if (m_preparationRequest == 0)
    {
        emit preparationChanged(tr("Preparing player…"));
        m_preparationRequest = Content::Client::instance()->request(u"mediaRuntime"_s);
    }
    return true;
}

bool Player::startPrepared(const QJsonObject &runtime)
{
    const QDir mediaDir(QCoreApplication::applicationDirPath() + u"/media"_s);
    const QString executable = runtime.value(u"mpv"_s).toString();
    const QString profile = mediaDir.filePath(u"profile"_s);
    if (!QFileInfo::exists(executable) || !QFileInfo::exists(profile + u"/scripts/modernx.lua"_s))
    {
        StreamServer::instance()->closeStream(m_pending.url);
        m_pending = {};
        emit error(tr("The player is missing. Try opening the video again."));
        return false;
    }

    m_current = std::exchange(m_pending, {});
    m_position = 0;
    m_duration = 0;
    m_stopping = false;
    m_subtitles = {};
    if (!prepareCookies())
    {
        stop();
        return false;
    }
    m_pipeName = u"qbutt-mpv-"_s + QUuid::createUuid().toString(QUuid::WithoutBraces);
#ifdef Q_OS_WIN
    const QString ipcPath = u"\\\\.\\pipe\\"_s + m_pipeName;
#else
    const QString ipcPath = QDir::tempPath() + u'/' + m_pipeName;
    m_pipeName = ipcPath;
#endif
    QStringList arguments {
        u"--config-dir="_s + profile,
        u"--input-ipc-server="_s + ipcPath,
        u"--idle=yes"_s,
        u"--save-position-on-quit=no"_s,
        u"--resume-playback=no"_s,
        u"--cookies=no"_s,
        u"--http-proxy="_s + QString::fromUtf8(m_current.options.proxyUrl.toEncoded()),
        u"--terminal=no"_s
    };
    if ((m_current.options.contentKind == u"anime") || (m_current.options.contentKind == u"movie"))
        arguments.append(u"--script-opts=video_profile-override="_s + m_current.options.contentKind);

    const QString framegenRuntime = specialFolderLocation(SpecialFolder::Data).toString() + u"/media/framegen"_s;
    if (!QDir().mkpath(framegenRuntime))
    {
        emit error(tr("The player data folder could not be created."));
        stop();
        return false;
    }
    QProcessEnvironment environment = QProcessEnvironment::systemEnvironment();
    environment.insert(u"QBUTT_MEDIA_RUNTIME"_s, QDir::toNativeSeparators(framegenRuntime));
    environment.insert(u"QBUTT_FFPROBE"_s, QDir::toNativeSeparators(runtime.value(u"ffprobe"_s).toString()));
    environment.insert(u"PATH"_s, QDir::toNativeSeparators(runtime.value(u"tools"_s).toString())
        + QDir::listSeparator() + environment.value(u"PATH"_s));
    environment.remove(u"VSSCRIPT_PATH"_s);
    m_process.setProcessEnvironment(environment);

    m_process.setWorkingDirectory(mediaDir.absolutePath());
    m_process.start(executable, arguments);
    return m_process.state() != QProcess::NotRunning;
}

bool Player::prepareCookies()
{
    QByteArray jar;
    static const QRegularExpression valuePattern(u"^[\\x21\\x23-\\x2b\\x2d-\\x3a\\x3c-\\x5b\\x5d-\\x7e]*$"_s);
    const QString host = m_current.url.host(QUrl::FullyEncoded);
    for (auto it = m_current.options.headers.cbegin(); it != m_current.options.headers.cend(); ++it)
    {
        if (it.key().compare(u"Cookie", Qt::CaseInsensitive) != 0)
            continue;

        if (host.isEmpty() || host.startsWith(u'.')
            || !((m_current.url.scheme() == u"http") || (m_current.url.scheme() == u"https")))
        {
            emit error(tr("The video source returned cookies without an HTTP host."));
            return false;
        }

        for (const QString &field : it.value().split(u';'))
        {
            const QString pair = field.trimmed();
            if (pair.isEmpty())
                continue;

            const qsizetype equals = pair.indexOf(u'=');
            const QString name = pair.left(equals).trimmed();
            const QString value = pair.mid(equals + 1).trimmed();
            const bool quoted = value.startsWith(u'"') && value.endsWith(u'"') && (value.size() >= 2);
            const QString unquoted = quoted ? value.mid(1, value.size() - 2) : value;
            if ((equals <= 0) || !HTTP_TOKEN_PATTERN.match(name).hasMatch() || !valuePattern.match(unquoted).hasMatch())
            {
                emit error(tr("The video source returned an invalid cookie."));
                return false;
            }

            jar += host.toUtf8() + "\tFALSE\t/\t"
                + ((m_current.url.scheme() == u"https") ? "TRUE" : "FALSE")
                + "\t0\t" + name.toUtf8() + '\t' + value.toUtf8() + '\n';
        }
    }
    if (jar.isEmpty())
        return true;

    jar.prepend("# Netscape HTTP Cookie File\n");
    m_cookies = std::make_unique<QTemporaryFile>(QDir::tempPath() + u"/qbutt-mpv-cookies-XXXXXX"_s);
    if (!m_cookies->open() || (m_cookies->write(jar) != jar.size()) || !m_cookies->flush())
    {
        m_cookies.reset();
        emit error(tr("The temporary video cookies could not be created."));
        return false;
    }
    m_cookies->close();
    return true;
}

void Player::stopProcess()
{
    cancelSubtitleRequest();
    m_subtitles.enabled = false;
    m_subtitles.queued = {};
    emit subtitleTranslationChanged({});
    if (m_stopping)
        return;

    savePosition();
    StreamServer::instance()->closeStream(m_current.url);
    m_connectTimer.stop();
    m_saveTimer.stop();
    m_stopping = true;
    setPlaying(false);
    if (m_process.state() != QProcess::NotRunning)
    {
        if (m_ipc.state() == QLocalSocket::ConnectedState)
        {
            sendCommand({u"quit"_s});
            m_ipc.flush();
        }
        else
        {
            m_process.terminate();
        }
        m_stopTimer.start();
    }
    else
    {
        m_cookies.reset();
    }
}

void Player::finishPlayback()
{
    cancelSubtitleRequest();
    m_subtitles = {};
    emit subtitleTranslationChanged({});
    savePosition();
    StreamServer::instance()->closeStream(m_current.url);
    m_connectTimer.stop();
    m_stopTimer.stop();
    m_saveTimer.stop();
    m_ipc.abort();
    m_messages.clear();
    m_cookies.reset();
    m_current = {};
    m_initialAudioId = 0;
    m_stopping = false;
    setPlaying(false);
    if (m_pending.url.isValid())
        startPending();
}

void Player::connectIpc()
{
    if (m_startupTime.elapsed() > 10000)
    {
        emit error(tr("The player did not become ready. Try opening the video again."));
        stop();
        return;
    }

    if (m_ipc.state() == QLocalSocket::UnconnectedState)
        m_ipc.connectToServer(m_pipeName);
}

void Player::loadMedia()
{
    if (m_stopping)
        return;

    m_connectTimer.stop();
    m_messages.clear();
    sendCommand({u"observe_property"_s, 1, u"time-pos"_s});
    sendCommand({u"observe_property"_s, 2, u"duration"_s});
    sendCommand({u"observe_property"_s, 3, u"track-list"_s});
    sendCommand({u"observe_property"_s, 4, u"eof-reached"_s});
    sendCommand({u"observe_property"_s, 5, u"sub-visibility"_s});
    sendCommand({u"observe_property"_s, 6, u"sub-delay"_s});
    sendCommand({u"observe_property"_s, 7, u"demuxer-start-time"_s});
    sendCommand({u"observe_property"_s, 8, u"options/sid"_s});

    QJsonArray headers;
    for (auto it = m_current.options.headers.cbegin(); it != m_current.options.headers.cend(); ++it)
    {
        if (it.key().compare(u"Cookie", Qt::CaseInsensitive) == 0)
            continue;

        const QString header = it.key() + u": "_s + it.value();
        headers.append(header);
    }
    sendCommand({u"set_property"_s, u"http-header-fields"_s, headers});

    QJsonObject options;
    options.insert(u"http-proxy"_s, QString::fromUtf8(m_current.options.proxyUrl.toEncoded()));
    if (m_cookies)
    {
        options.insert(u"cookies"_s, u"yes"_s);
        options.insert(u"cookies-file"_s, m_cookies->fileName());
    }
    if (!m_current.title.isEmpty())
        options.insert(u"force-media-title"_s, m_current.title);
    if (m_current.options.identity.startsWith(u"torrent:"))
        options.insert(u"network-timeout"_s, u"120"_s);
    m_initialAudioId = m_current.options.audioTrack.title.isEmpty() ? 0 : -1;
    if (m_initialAudioId != 0)
        options.insert(u"pause"_s, u"yes"_s);

    if (!m_current.options.audioUrl.isEmpty())
    {
        sendCommand({u"set_property"_s, u"audio-files"_s,
            QJsonArray {QString::fromUtf8(m_current.options.audioUrl.toEncoded())}});
    }
    QJsonArray subtitles;
    for (const QUrl &subtitle : m_current.options.subtitles)
        subtitles.append(QString::fromUtf8(subtitle.toEncoded()));
    sendCommand({u"set_property"_s, u"sub-files"_s, subtitles});

    const double position = (m_current.options.startPosition >= 0) ? m_current.options.startPosition
        : SettingsStorage::instance()->loadValue<double>(m_current.positionKey);
    if (std::isfinite(position) && (position > 0))
        options.insert(u"start"_s, QString::number(position, 'f', 3));

    sendCommand({u"loadfile"_s, QString::fromUtf8(m_current.url.toEncoded()), u"replace"_s, -1, options});
    m_saveTimer.start();
}

void Player::readMessages()
{
    m_messages.append(m_ipc.readAll());
    if (m_messages.size() > 1024 * 1024)
    {
        emit error(tr("The player sent an invalid response."));
        stop();
        return;
    }

    qsizetype newline = -1;
    while ((newline = m_messages.indexOf('\n')) >= 0)
    {
        const QByteArray line = m_messages.first(newline);
        m_messages.remove(0, newline + 1);
        if (m_stopping)
            continue;

        const QJsonObject message = QJsonDocument::fromJson(line).object();
        const int requestId = message.value(u"request_id"_s).toInt();
        if ((requestId != 0) && (requestId == m_subtitles.commandId))
        {
            finishSubtitleCommand(message);
            continue;
        }
        if (requestId >= 10)
            continue;
        const QString commandError = message.value(u"error"_s).toString();
        if (!commandError.isEmpty() && (commandError != u"success"))
        {
            emit error(tr("The bundled player rejected a playback command."));
            stop();
            return;
        }

        const QString event = message.value(u"event"_s).toString();
        if (requestId == AUDIO_TRACKS_REQUEST)
        {
            const AudioTrack &wanted = m_current.options.audioTrack;
            const auto metadataPattern = [](const QString &value)
            {
                return QRegularExpression(u"(?<![\\p{L}\\p{N}])"_s
                    + QRegularExpression::escape(value.normalized(QString::NormalizationForm_C).simplified())
                    + u"(?![\\p{L}\\p{N}])"_s, QRegularExpression::CaseInsensitiveOption);
            };
            const QRegularExpression titlePattern = metadataPattern(wanted.title);
            const QRegularExpression typePattern = metadataPattern(wanted.type);
            const QLocale::Language wantedLanguage = QLocale::codeToLanguage(wanted.language);
            QJsonArray matches;
            for (const QJsonValue &value : message.value(u"data"_s).toArray())
            {
                const QJsonObject track = value.toObject();
                const QString title = track.value(u"title"_s).toString().normalized(QString::NormalizationForm_C).simplified();
                if ((track.value(u"type"_s).toString() != u"audio") || !titlePattern.match(title).hasMatch()
                    || (!wanted.type.isEmpty() && !typePattern.match(title).hasMatch()))
                    continue;

                const QString language = track.value(u"lang"_s).toString();
                const QLocale::Language actualLanguage = QLocale::codeToLanguage(language);
                if (!wanted.language.isEmpty()
                    && !((wantedLanguage != QLocale::AnyLanguage) && (wantedLanguage == actualLanguage))
                    && (wanted.language.compare(language, Qt::CaseInsensitive) != 0))
                    continue;
                if (!wanted.codec.isEmpty()
                    && (wanted.codec.compare(track.value(u"codec"_s).toString(), Qt::CaseInsensitive) != 0))
                    continue;
                if ((wanted.channels > 0) && (wanted.channels != track.value(u"demux-channel-count"_s).toInt()))
                    continue;

                matches.append(track);
            }
            if ((matches.size() != 1) || (matches.at(0).toObject().value(u"id"_s).toInt() <= 0))
            {
                emit error(tr("The selected voice “%1” is unavailable or ambiguous in this stream. Choose another voice or open the source player.")
                    .arg(wanted.title));
                stop();
                return;
            }
            m_initialAudioId = matches.at(0).toObject().value(u"id"_s).toInt();
            sendCommand({u"set_property"_s, u"aid"_s, m_initialAudioId}, AUDIO_SELECT_REQUEST);
        }
        else if (requestId == AUDIO_SELECT_REQUEST)
        {
            sendCommand({u"get_property"_s, u"track-list"_s}, AUDIO_VERIFY_REQUEST);
        }
        else if (requestId == AUDIO_VERIFY_REQUEST)
        {
            bool selected = false;
            for (const QJsonValue &value : message.value(u"data"_s).toArray())
            {
                const QJsonObject track = value.toObject();
                selected = selected || ((track.value(u"type"_s).toString() == u"audio")
                    && (track.value(u"id"_s).toInt() == m_initialAudioId) && track.value(u"selected"_s).toBool());
            }
            if (!selected)
            {
                emit error(tr("The player could not select the requested voice. Choose another voice or open the source player."));
                stop();
                return;
            }
            m_initialAudioId = 0;
            sendCommand({u"set_property"_s, u"pause"_s, false});
            setPlaying(true);
            emit playbackStarted(m_current.options.identity);
        }
        else if (event == u"file-loaded")
        {
            m_subtitles.loaded = true;
            sendSubtitleCommand({u"get_property"_s, u"demuxer-start-time"_s}, SubtitleCommand::StartTime);
            if (m_initialAudioId == -1)
            {
                sendCommand({u"get_property"_s, u"track-list"_s}, AUDIO_TRACKS_REQUEST);
            }
            else
            {
                setPlaying(true);
                emit playbackStarted(m_current.options.identity);
            }
        }
        else if ((event == u"playback-restart") && (m_initialAudioId == 0))
        {
            setPlaying(true);
            m_subtitles.seeking = false;
            requestSubtitles();
        }
        else if (event == u"seek")
        {
            cancelSubtitleRequest();
            ++m_subtitles.generation;
            m_subtitles.seeking = true;
            m_subtitles.requestedBucket = -1;
            m_subtitles.windowStart = 0;
            m_subtitles.windowEnd = 0;
            m_subtitles.queued = {};
        }
        else if ((event == u"client-message")
            && (message.value(u"args"_s).toArray().at(0).toString() == u"qbutt-download"))
        {
            emit downloadRequested(m_current.url, m_current.title, m_current.options);
        }
        else if ((event == u"end-file") && (message.value(u"reason"_s).toString() == u"error"))
        {
            emit error(tr("The video could not be opened. Check that the source is still available."));
            stop();
        }
        else if (event == u"end-file")
        {
            cancelSubtitleRequest();
            m_subtitles.loaded = false;
            savePosition();
            setPlaying(false);
        }
        else if (event == u"property-change")
        {
            const QString name = message.value(u"name"_s).toString();
            const QJsonValue data = message.value(u"data"_s);
            if ((name == u"time-pos") && data.isDouble())
            {
                m_position = data.toDouble();
                emit positionChanged(m_position);
                requestSubtitles();
            }
            else if ((name == u"duration") && data.isDouble())
            {
                m_duration = data.toDouble();
                emit durationChanged(m_duration);
            }
            else if (name == u"track-list")
            {
                emit tracksChanged(data.toArray());
            }
            else if (name == u"options/sid")
            {
                updateSubtitleSelection(data);
            }
            else if ((name == u"sub-visibility") && data.isBool() && !data.toBool())
            {
                m_subtitles.enabled = false;
                cancelSubtitleRequest();
                m_subtitles.queued = {};
                emit subtitleTranslationChanged({});
            }
            else if ((name == u"sub-delay") && data.isDouble() && std::isfinite(data.toDouble()))
            {
                const double delay = data.toDouble();
                if (delay != m_subtitles.delay)
                {
                    cancelSubtitleRequest();
                    ++m_subtitles.generation;
                    m_subtitles.requestedBucket = -1;
                    m_subtitles.windowStart = 0;
                    m_subtitles.windowEnd = 0;
                    m_subtitles.queued = {};
                    m_subtitles.delay = delay;
                    requestSubtitles();
                }
            }
            else if ((name == u"demuxer-start-time") && data.isDouble() && std::isfinite(data.toDouble()))
            {
                m_subtitles.startTime = data.toDouble();
            }
            else if ((name == u"eof-reached") && data.isBool())
            {
                if (data.toBool())
                {
                    cancelSubtitleRequest();
                    m_subtitles.loaded = false;
                    savePosition();
                    setPlaying(false);
                }
                else if (!m_subtitles.source.isEmpty())
                {
                    m_subtitles.loaded = true;
                }
            }
        }
    }
}

void Player::sendCommand(const QJsonArray &command, const int requestId)
{
    QJsonObject message {{u"command"_s, command}};
    if (requestId != 0)
        message.insert(u"request_id"_s, requestId);
    m_ipc.write(QJsonDocument(message).toJson(QJsonDocument::Compact) + '\n');
}

void Player::cancelSubtitleRequest()
{
    if (m_subtitles.request != 0)
        Content::Client::instance()->cancel(std::exchange(m_subtitles.request, 0));
}

void Player::updateSubtitleSelection(const QJsonValue &selection)
{
    if (!m_subtitles.enabled || (selection.toString() == u"auto"))
        return;

    const int selectedId = selection.toInt();
    if ((selectedId > 0) && ((selectedId == m_subtitles.translatedId)
        || (selectedId == m_subtitles.loading.value(u"trackId"_s).toInt())))
        return;

    m_subtitles.enabled = false;
    cancelSubtitleRequest();
    m_subtitles.queued = {};
    emit subtitleTranslationChanged({});
}

void Player::initializeSubtitles(const QJsonArray &tracks)
{
    if (!m_subtitles.enabled)
        return;

    const int selectedId = selectedSubtitle(tracks);
    static const QSet<QString> textCodecs {u"ass"_s, u"ssa"_s, u"subrip"_s, u"srt"_s,
        u"webvtt"_s, u"mov_text"_s, u"text"_s, u"ttml"_s};
    int bestScore = -1;
    int russianId = 0;
    for (const QJsonValue &value : tracks)
    {
        const QJsonObject track = value.toObject();
        if ((track.value(u"type"_s).toString() != u"sub") || (track.value(u"id"_s).toInt() <= 0)
            || !textCodecs.contains(track.value(u"codec"_s).toString().toLower()))
            continue;
        if ((track.value(u"id"_s).toInt() == selectedId) && isRussian(track.value(u"lang"_s).toString()))
        {
            m_subtitles.enabled = false;
            return;
        }
        if (isRussian(track.value(u"lang"_s).toString()))
            russianId = track.value(u"id"_s).toInt();
        const QString external = track.value(u"external-filename"_s).toString();
        if (!external.isEmpty())
        {
            const QUrl url = subtitleUrl(external);
            if (!url.isValid() || !((url.scheme() == u"file") || (url.scheme() == u"http") || (url.scheme() == u"https")))
                continue;
        }
        else if (!track.value(u"ff-index"_s).isDouble() || (track.value(u"ff-index"_s).toInt(-1) < 0))
        {
            continue;
        }
        const int score = ((track.value(u"id"_s).toInt() == selectedId) ? 4 : 0)
            + (track.value(u"default"_s).toBool() ? 2 : 0) + (!track.value(u"forced"_s).toBool() ? 1 : 0);
        if (score > bestScore)
        {
            bestScore = score;
            m_subtitles.source = {{u"id"_s, track.value(u"id"_s)},
                {u"ffIndex"_s, track.value(u"ff-index"_s).toInt(0)},
                {u"codec"_s, track.value(u"codec"_s)}, {u"language"_s, track.value(u"lang"_s).toString()}};
            if (!external.isEmpty())
                m_subtitles.source.insert(u"url"_s, QString::fromUtf8(subtitleUrl(external).toEncoded()));
        }
    }
    if (russianId > 0)
    {
        m_subtitles.source = {};
        m_subtitles.loading = {{u"trackId"_s, russianId}};
        m_subtitles.loadingGeneration = m_subtitles.generation;
        sendSubtitleCommand({u"set_property"_s, u"sid"_s, russianId}, SubtitleCommand::Select);
        return;
    }
    if (!m_subtitles.source.isEmpty() && !m_subtitles.source.contains(u"url"_s) && !m_subtitles.startTime)
    {
        subtitleFailure(tr("The player could not identify the subtitle timing. Choose an external subtitle file."));
        return;
    }
    requestSubtitles();
}

void Player::requestSubtitles()
{
    if (!m_subtitles.loaded || !m_subtitles.enabled || m_subtitles.seeking || m_stopping
        || m_subtitles.source.isEmpty() || (m_subtitles.request != 0) || !std::isfinite(m_position))
        return;

    const double position = std::max(0.0, m_position - m_subtitles.delay);
    const qint64 bucket = static_cast<qint64>(std::floor(position / 60));
    if ((bucket == m_subtitles.requestedBucket)
        || ((position >= m_subtitles.windowStart) && (m_subtitles.windowEnd - position > 60)))
        return;

    QJsonObject headers;
    for (auto it = m_current.options.headers.cbegin(); it != m_current.options.headers.cend(); ++it)
        headers.insert(it.key(), it.value());
    const QString identity = m_current.options.identity.isEmpty()
        ? QString::fromUtf8(m_current.url.toEncoded()) : m_current.options.identity;
    m_subtitles.requestedBucket = bucket;
    m_subtitles.request = Content::Client::instance()->request(u"subtitles.ahead"_s,
        {{u"identity"_s, identity}, {u"url"_s, QString::fromUtf8(m_current.url.toEncoded())},
            {u"headers"_s, headers}, {u"proxyUrl"_s, QString::fromUtf8(m_current.options.proxyUrl.toEncoded())},
            {u"track"_s, m_subtitles.source}, {u"position"_s, position},
            {u"startTime"_s, m_subtitles.source.contains(u"url"_s) ? 0 : m_subtitles.startTime.value()},
            {u"targetLanguage"_s, u"ru"_s}});
    if (!m_subtitles.announced)
    {
        m_subtitles.announced = true;
        emit subtitleTranslationChanged(tr("Translating subtitles…"));
        sendCommand({u"show-text"_s, tr("Translating subtitles…"), 3000});
    }
}

void Player::receiveSubtitles(const QString &event, const QJsonObject &data)
{
    if (event == u"error")
    {
        m_subtitles.request = 0;
        subtitleFailure(data.value(u"message"_s).toString());
        return;
    }
    if ((event != u"subtitle") && (event != u"complete"))
        return;
    if (event == u"complete")
        m_subtitles.request = 0;

    const QString identity = m_current.options.identity.isEmpty()
        ? QString::fromUtf8(m_current.url.toEncoded()) : m_current.options.identity;
    if (!m_subtitles.enabled || !m_subtitles.loaded || (data.value(u"identity"_s).toString() != identity))
        return;

    if (event == u"complete")
    {
        const QString status = data.value(u"status"_s).toString();
        if ((status == u"already-target") || (status == u"unsupported"))
        {
            m_subtitles.enabled = false;
            emit subtitleTranslationChanged({});
            return;
        }
        if (status != u"ready")
        {
            subtitleFailure(data.value(u"message"_s).toString());
            return;
        }
    }

    const double windowStart = data.value(u"windowStart"_s).toDouble(-1);
    const double windowEnd = data.value(u"windowEnd"_s).toDouble(-1);
    if (!std::isfinite(windowStart) || !std::isfinite(windowEnd) || (windowStart < 0)
        || (windowEnd <= windowStart) || (windowEnd - windowStart > 120.01))
    {
        subtitleFailure(tr("The subtitle translation returned an invalid time window."));
        return;
    }
    m_subtitles.windowStart = windowStart;
    m_subtitles.windowEnd = windowEnd;
    if (event == u"complete")
    {
        emit subtitleTranslationChanged({});
        return;
    }

    const QUrl url(data.value(u"url"_s).toString());
    const QFileInfo file(url.toLocalFile());
    const QDir cache(QDir(Profile::instance()->location(SpecialFolder::Data).toString()).filePath(u"content/subtitles"_s));
    const QString relative = cache.relativeFilePath(file.canonicalFilePath());
    static const QRegularExpression cachePattern(u"^[0-9a-f]{64}/translated-([0-9a-f]{64})\\.(ass|srt|vtt)$"_s);
    const QRegularExpressionMatch match = cachePattern.match(QDir::fromNativeSeparators(relative));
    if (!url.isLocalFile() || !file.isFile() || file.isSymLink() || (file.size() <= 0) || (file.size() > 16 * 1024 * 1024)
        || !match.hasMatch() || (match.captured(1) != data.value(u"revision"_s).toString())
        || !isRussian(data.value(u"language"_s).toString()))
    {
        subtitleFailure(tr("The subtitle translation returned an invalid cached file."));
        return;
    }
    if ((data.value(u"revision"_s).toString() == m_subtitles.revision)
        || (data.value(u"revision"_s) == m_subtitles.loading.value(u"revision"_s)))
        return;

    m_subtitles.queued = data;
    applySubtitles();
}

void Player::applySubtitles()
{
    if (!m_subtitles.enabled || m_subtitles.queued.isEmpty() || !m_subtitles.loading.isEmpty()
        || (m_subtitles.command != SubtitleCommand::None))
        return;

    m_subtitles.loading = std::exchange(m_subtitles.queued, {});
    m_subtitles.loadingGeneration = m_subtitles.generation;
    const QUrl url(m_subtitles.loading.value(u"url"_s).toString());
    sendSubtitleCommand({u"sub-add"_s, url.toLocalFile(), u"auto"_s,
        tr("Russian [AI translation]"), u"ru"_s}, SubtitleCommand::Add);
}

void Player::sendSubtitleCommand(const QJsonArray &command, const SubtitleCommand stage)
{
    m_subtitles.command = stage;
    m_subtitles.commandId = ++m_nextSubtitleCommand;
    sendCommand(command, m_subtitles.commandId);
}

void Player::finishSubtitleCommand(const QJsonObject &message)
{
    const SubtitleCommand command = std::exchange(m_subtitles.command, SubtitleCommand::None);
    m_subtitles.commandId = 0;
    if (command == SubtitleCommand::StartTime)
    {
        const QJsonValue time = message.value(u"data"_s);
        if (time.isDouble() && std::isfinite(time.toDouble()))
            m_subtitles.startTime = time.toDouble();
        else
            m_subtitles.startTime.reset();
        sendSubtitleCommand({u"get_property"_s, u"options/sid"_s}, SubtitleCommand::SourceSelection);
        return;
    }
    const QString commandError = message.value(u"error"_s).toString();
    if (!commandError.isEmpty() && (commandError != u"success"))
    {
        subtitleFailure(tr("The player could not load translated subtitles. Choose the original subtitle track."));
        return;
    }
    if (command == SubtitleCommand::SourceSelection)
    {
        updateSubtitleSelection(message.value(u"data"_s));
        sendSubtitleCommand({u"get_property"_s, u"track-list"_s}, SubtitleCommand::SourceTracks);
        return;
    }
    if (command == SubtitleCommand::SourceTracks)
    {
        initializeSubtitles(message.value(u"data"_s).toArray());
        return;
    }
    if (command == SubtitleCommand::Add)
    {
        sendSubtitleCommand({u"get_property"_s, u"track-list"_s}, SubtitleCommand::AddedTracks);
        return;
    }
    if (command == SubtitleCommand::Select)
    {
        sendSubtitleCommand({u"get_property"_s, u"track-list"_s}, SubtitleCommand::Verify);
        return;
    }

    const QJsonArray tracks = message.value(u"data"_s).toArray();
    const QUrl url(m_subtitles.loading.value(u"url"_s).toString());
    QJsonObject added;
    for (const QJsonValue &value : tracks)
    {
        const QJsonObject track = value.toObject();
        if ((track.value(u"type"_s).toString() == u"sub")
            && ((track.value(u"id"_s).toInt() == m_subtitles.loading.value(u"trackId"_s).toInt(-1))
                || (!url.isEmpty() && (subtitleUrl(track.value(u"external-filename"_s).toString()) == url))))
            added = track;
    }
    const int addedId = added.value(u"id"_s).toInt();
    if (addedId <= 0)
    {
        subtitleFailure(tr("The player could not find the translated subtitle track. Choose the original subtitle track."));
        return;
    }
    if (!m_subtitles.enabled || !m_subtitles.loaded || (m_subtitles.loadingGeneration != m_subtitles.generation))
    {
        if (!url.isEmpty())
            sendCommand({u"sub-remove"_s, addedId}, ++m_nextSubtitleCommand);
        m_subtitles.loading = {};
        applySubtitles();
        return;
    }
    if (command == SubtitleCommand::AddedTracks)
    {
        m_subtitles.loading.insert(u"trackId"_s, addedId);
        sendSubtitleCommand({u"set_property"_s, u"sid"_s, addedId}, SubtitleCommand::Select);
        return;
    }
    if (!added.value(u"selected"_s).toBool() || (added.value(u"main-selection"_s).toInt() != 0)
        || !isRussian(added.value(u"lang"_s).toString()))
    {
        subtitleFailure(tr("The player could not select translated subtitles. Choose the original subtitle track."));
        return;
    }
    if (url.isEmpty())
    {
        m_subtitles.loading = {};
        m_subtitles.enabled = false;
        return;
    }
    const bool firstTranslation = (m_subtitles.translatedId == 0);
    if ((m_subtitles.translatedId > 0) && (m_subtitles.translatedId != addedId))
        sendCommand({u"sub-remove"_s, m_subtitles.translatedId}, ++m_nextSubtitleCommand);
    m_subtitles.translatedId = addedId;
    m_subtitles.revision = m_subtitles.loading.value(u"revision"_s).toString();
    m_subtitles.loading = {};
    emit subtitleTranslationChanged({});
    if (firstTranslation)
        sendCommand({u"show-text"_s, tr("Russian subtitles ready"), 3000});
    applySubtitles();
}

void Player::subtitleFailure(const QString &message)
{
    if (!m_subtitles.enabled)
        return;

    m_subtitles.enabled = false;
    cancelSubtitleRequest();
    m_subtitles.queued = {};
    const QString explanation = message.isEmpty()
        ? tr("Subtitle translation is unavailable. Sign in to Codex or choose an AI provider in Reading settings.") : message;
    emit subtitleTranslationChanged(explanation);
    sendCommand({u"show-text"_s, explanation, 7000});
}

void Player::savePosition()
{
    if (m_current.positionKey.isEmpty() || (m_position <= 0))
        return;

    SettingsStorage *settings = SettingsStorage::instance();
    if ((m_duration > 0) && (m_position >= m_duration - 10))
        settings->removeValue(m_current.positionKey);
    else
        settings->storeValue(m_current.positionKey, m_position);
}

void Player::setPlaying(const bool playing)
{
    if (m_playing == playing)
        return;

    m_playing = playing;
    emit playbackChanged(playing);
}
