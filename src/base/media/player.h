/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <memory>

#include <QElapsedTimer>
#include <QJsonArray>
#include <QList>
#include <QLocalSocket>
#include <QMap>
#include <QObject>
#include <QPointer>
#include <QProcess>
#include <QTemporaryFile>
#include <QTimer>
#include <QUrl>

namespace BitTorrent
{
    class Torrent;
}

namespace Media
{
    struct AudioTrack
    {
        QString title;
        QString type;
        QString language;
        QString codec;
        int channels = 0;
    };

    struct PlaybackOptions
    {
        QMap<QString, QString> headers;
        QString contentKind;
        QString identity;
        quint64 launchToken = 0;
        QString legacyIdentity;
        QUrl proxyUrl;
        QUrl audioUrl;
        AudioTrack audioTrack;
        QList<QUrl> subtitles;
        double startPosition = -1;
    };

    class Player final : public QObject
    {
        Q_OBJECT
        Q_DISABLE_COPY_MOVE(Player)

    public:
        static void initInstance(QObject *parent = nullptr);
        static void freeInstance();
        static Player *instance();
        static bool isMediaFile(const QString &fileName);
        static QString torrentIdentity(const BitTorrent::Torrent *torrent, int fileIndex);

        bool playUrl(const QUrl &url, const QString &title = {}, const PlaybackOptions &options = {});
        bool playTorrent(BitTorrent::Torrent *torrent, int fileIndex, const QString &contentKind = {});
        void stop();
        void enableDownload(quint64 launchToken);
        void finishDownload(quint64 launchToken, bool queued);

    signals:
        void error(const QString &message);
        void preparationChanged(const QString &message);
        void playbackChanged(bool playing);
        void playbackStarted(const QString &identity, quint64 launchToken);
        void playbackFinished(quint64 launchToken);
        void positionChanged(double seconds);
        void durationChanged(double seconds);
        void tracksChanged(const QJsonArray &tracks);
        void downloadRequested(quint64 launchToken);

    private:
        enum class PlaybackState
        {
            Pending,
            Started,
            Finished
        };

        enum class DownloadState
        {
            Unavailable,
            Ready,
            Pending,
            Queued,
            Failed
        };

        struct Playback
        {
            QUrl url;
            QString title;
            PlaybackOptions options;
            QString positionKey;
            PlaybackState state = PlaybackState::Finished;
            DownloadState download = DownloadState::Unavailable;
        };

        explicit Player(QObject *parent);
        ~Player() override;

        bool startPlayback(const QUrl &url, const QString &title, const PlaybackOptions &options);
        void startPending();
        void startPrepared(const QString &executable);
        void cancelPreparation();
        void failPreparation(const QString &message);
        bool prepareCookies();
        void stopProcess();
        void finishPlayback();
        void confirmPlayback();
        void finishLaunch(Playback &playback);
        void connectIpc();
        void loadMedia();
        void readMessages();
        void sendCommand(const QJsonArray &command, int requestId = 0);
        void savePosition();
        void setPlaying(bool playing);
        std::unique_ptr<QTemporaryFile> m_cookies;
        QProcess m_process;
        QLocalSocket m_ipc;
        QTimer m_connectTimer;
        QTimer m_stopTimer;
        QTimer m_saveTimer;
        QElapsedTimer m_startupTime;
        QByteArray m_messages;
        Playback m_current;
        Playback m_pending;
        QString m_pipeName;
        quint64 m_runtimeRequest = 0;
        double m_position = 0;
        double m_duration = 0;
        int m_initialAudioId = 0;
        bool m_playing = false;
        bool m_stopping = false;

        static QPointer<Player> m_instance;
    };
}
