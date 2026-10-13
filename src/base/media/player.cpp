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
#include <QRegularExpression>
#include <QUuid>

#include "base/bittorrent/infohash.h"
#include "base/bittorrent/torrent.h"
#include "base/global.h"
#include "base/path.h"
#include "base/settingsstorage.h"
#include "runtime.h"
#include "streamserver.h"

using namespace Media;

namespace
{
    const QRegularExpression HTTP_TOKEN_PATTERN {u"^[!#$%&'*+.^_`|~0-9A-Za-z-]+$"_s};
    constexpr int AUDIO_TRACKS_REQUEST = 1;
    constexpr int AUDIO_SELECT_REQUEST = 2;
    constexpr int AUDIO_VERIFY_REQUEST = 3;
}

QPointer<Player> Player::m_instance;

Player::Player(QObject *parent)
    : QObject(parent)
{
    connect(StreamServer::instance(), &StreamServer::error, this, &Player::error);
    connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, this, &Player::stop);
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

void Player::startPending()
{
    if (!m_pending.url.isValid() || (m_process.state() != QProcess::NotRunning) || (m_runtimeRequest != 0))
        return;

    m_runtimeRequest = Runtime::instance()->request(this, [this](const Runtime::Result &result)
    {
        m_runtimeRequest = 0;
        if (!m_pending.url.isValid())
            return;

        if (result.error != Runtime::Error::None)
        {
            failPreparation(Runtime::errorMessage(result.error, Runtime::Purpose::Playback));
            return;
        }

        emit preparationChanged({});
        if (m_pending.url.isValid() && (m_process.state() == QProcess::NotRunning) && (m_runtimeRequest == 0))
            startPrepared(result.info.executable);
    });
    if (m_runtimeRequest != 0)
        emit preparationChanged(tr("Preparing player…"));
}

void Player::cancelPreparation()
{
    if (m_runtimeRequest == 0)
        return;

    Runtime::instance()->cancel(std::exchange(m_runtimeRequest, 0));
}

void Player::failPreparation(const QString &message)
{
    Playback pending = std::exchange(m_pending, {});
    cancelPreparation();
    StreamServer::instance()->closeStream(pending.url);
    emit preparationChanged({});
    finishLaunch(pending);
    emit error(message);
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
    if (network && options.proxyUrl.isEmpty())
    {
        const auto localResource = [&url](const QUrl &source)
        {
            return (source.scheme() == u"http") && (source.host() == u"127.0.0.1")
                && (source.port() > 0) && (source.port() == url.port())
                && source.userInfo().isEmpty() && !source.hasQuery() && !source.hasFragment()
                && source.path().startsWith(u"/media/");
        };
        if (!localResource(url)
            || (!options.audioUrl.isEmpty() && !localResource(options.audioUrl))
            || std::any_of(options.subtitles.cbegin(), options.subtitles.cend(),
                [&localResource](const QUrl &subtitle) { return !localResource(subtitle); }))
        {
            emit error(tr("The video source did not provide a protected connection. Try opening it again."));
            return false;
        }
    }
    else if (network && ((options.proxyUrl.scheme() != u"http") || (options.proxyUrl.host() != u"127.0.0.1")
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
    const auto positionKey = [](const QByteArray &identity)
    {
        return u"Media/PlaybackPosition/"_s
            + QString::fromLatin1(QCryptographicHash::hash(identity, QCryptographicHash::Sha256).toHex());
    };
    const QByteArray positionIdentity = options.identity.isEmpty() ? url.toEncoded() : options.identity.toUtf8();
    const QString key = positionKey(positionIdentity);
    SettingsStorage *settings = SettingsStorage::instance();
    if ((options.startPosition < 0) && !options.legacyIdentity.isEmpty() && !settings->hasKey(key))
    {
        const QString legacyKey = positionKey(options.legacyIdentity.toUtf8());
        const double position = settings->loadValue<double>(legacyKey);
        if (std::isfinite(position) && (position > 0))
        {
            settings->storeValue(key, position);
            settings->removeValue(legacyKey);
        }
    }

    Playback pending = std::exchange(m_pending, {url, title, options, key, PlaybackState::Pending});
    StreamServer::instance()->closeStream(pending.url);
    finishLaunch(pending);
    if (m_process.state() == QProcess::NotRunning)
        startPending();
    else
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
    Playback pending = std::exchange(m_pending, {});
    cancelPreparation();
    StreamServer::instance()->closeStream(pending.url);
    stopProcess();
    finishLaunch(pending);
    if (m_runtimeRequest == 0)
        emit preparationChanged({});
}

void Player::startPrepared(const QString &executable)
{
    if (!QFileInfo(executable).isFile())
    {
        failPreparation(tr("The player is missing. Try opening the video again."));
        return;
    }

    m_current = std::exchange(m_pending, {});
    m_position = 0;
    m_duration = 0;
    m_stopping = false;
    if (!prepareCookies())
    {
        stopProcess();
        return;
    }
    m_pipeName = u"qbutt-mpv-"_s + QUuid::createUuid().toString(QUuid::WithoutBraces);
#ifdef Q_OS_WIN
    const QString ipcPath = u"\\\\.\\pipe\\"_s + m_pipeName;
#else
    const QString ipcPath = QDir::tempPath() + u'/' + m_pipeName;
    m_pipeName = ipcPath;
#endif
    QStringList arguments {
        u"--input-ipc-server="_s + ipcPath,
        u"--idle=yes"_s,
        u"--save-position-on-quit=no"_s,
        u"--resume-playback=no"_s,
        u"--cookies=no"_s,
        u"--http-proxy="_s + QString::fromUtf8(m_current.options.proxyUrl.toEncoded()),
        u"--terminal=no"_s
    };
    arguments.prepend(u"--external-downloads"_s);
    if (qEnvironmentVariable("QBUTT_LAB_PATHS") == u"1")
    {
        const QString profile = qEnvironmentVariable("QBUTT_LAB_QPLAY_PROFILE");
        if (!profile.isEmpty())
            arguments.append(u"--profile-dir="_s + profile);
    }
    if ((m_current.options.contentKind == u"anime") || (m_current.options.contentKind == u"movie"))
        arguments.append(u"--script-opts-append=video_profile-override="_s + m_current.options.contentKind);

    m_process.start(executable, arguments);
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
    if (m_stopping)
        return;

    savePosition();
    StreamServer::instance()->closeStream(m_current.url);
    m_connectTimer.stop();
    m_saveTimer.stop();
    m_stopping = true;
    finishLaunch(m_current);
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
        finishPlayback();
    }
}

void Player::finishPlayback()
{
    savePosition();
    StreamServer::instance()->closeStream(m_current.url);
    m_connectTimer.stop();
    m_stopTimer.stop();
    m_saveTimer.stop();
    m_ipc.abort();
    m_messages.clear();
    m_cookies.reset();
    Playback current = std::exchange(m_current, {});
    m_initialAudioId = 0;
    m_stopping = false;
    setPlaying(false);
    finishLaunch(current);
    if (m_pending.url.isValid())
        startPending();
}

void Player::confirmPlayback()
{
    if (m_stopping || (m_current.state != PlaybackState::Pending))
        return;

    m_current.state = PlaybackState::Started;
    const QString identity = m_current.options.identity;
    const quint64 token = m_current.options.launchToken;
    setPlaying(true);
    if (!m_stopping && (m_current.state == PlaybackState::Started) && (m_current.options.launchToken == token))
        emit playbackStarted(identity, token);
}

void Player::enableDownload(const quint64 launchToken)
{
    if ((launchToken == 0) || (m_current.options.launchToken != launchToken)
        || (m_current.download != DownloadState::Unavailable)
        || (m_current.state != PlaybackState::Started) || m_stopping
        || ((m_current.url.scheme() != u"http") && (m_current.url.scheme() != u"https")))
        return;

    m_current.download = DownloadState::Ready;
    sendCommand({u"script-message"_s, u"qplay-download-target"_s, QString::number(launchToken),
        QString::fromUtf8(m_current.url.toEncoded())});
}

void Player::finishDownload(const quint64 launchToken, const bool queued)
{
    if ((launchToken == 0) || (m_current.options.launchToken != launchToken)
        || (m_current.download != DownloadState::Pending))
        return;

    m_current.download = queued ? DownloadState::Queued : DownloadState::Failed;
    sendCommand({u"script-message"_s, u"qplay-download-result"_s, QString::number(launchToken),
        queued ? u"queued"_s : u"failed"_s});
}

void Player::finishLaunch(Playback &playback)
{
    if (playback.state == PlaybackState::Finished)
        return;

    playback.state = PlaybackState::Finished;
    if (playback.options.launchToken != 0)
        emit playbackFinished(playback.options.launchToken);
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
            confirmPlayback();
        }
        else if (event == u"file-loaded")
        {
            if (m_initialAudioId == -1)
            {
                sendCommand({u"get_property"_s, u"track-list"_s}, AUDIO_TRACKS_REQUEST);
            }
            else
            {
                confirmPlayback();
            }
        }
        else if ((event == u"playback-restart") && (m_initialAudioId == 0))
        {
            setPlaying(true);
        }
        else if (event == u"client-message")
        {
            const QJsonArray arguments = message.value(u"args"_s).toArray();
            if ((arguments.size() == 3) && (arguments.at(0).toString() == u"qplay-download")
                && !m_stopping && (m_current.state == PlaybackState::Started)
                && (m_current.options.launchToken != 0)
                && (arguments.at(1).toString() == QString::number(m_current.options.launchToken))
                && (arguments.at(2).toString() == QString::fromUtf8(m_current.url.toEncoded())))
            {
                if (m_current.download == DownloadState::Queued)
                {
                    sendCommand({u"script-message"_s, u"qplay-download-result"_s,
                        QString::number(m_current.options.launchToken), u"queued"_s});
                }
                else if ((m_current.download == DownloadState::Ready) || (m_current.download == DownloadState::Failed))
                {
                    m_current.download = DownloadState::Pending;
                    emit downloadRequested(m_current.options.launchToken);
                }
            }
        }
        else if ((event == u"end-file") && (message.value(u"reason"_s).toString() == u"error"))
        {
            emit error(tr("The video could not be opened. Check that the source is still available."));
            stop();
        }
        else if (event == u"end-file")
        {
            savePosition();
            setPlaying(false);
            finishLaunch(m_current);
        }
        else if (event == u"property-change")
        {
            const QString name = message.value(u"name"_s).toString();
            const QJsonValue data = message.value(u"data"_s);
            if ((name == u"time-pos") && data.isDouble())
            {
                m_position = data.toDouble();
                emit positionChanged(m_position);
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
            else if ((name == u"eof-reached") && data.isBool())
            {
                if (data.toBool())
                {
                    savePosition();
                    setPlaying(false);
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
