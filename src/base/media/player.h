/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <memory>
#include <optional>

#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonObject>
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

    signals:
        void error(const QString &message);
        void preparationChanged(const QString &message);
        void playbackChanged(bool playing);
        void playbackStarted(const QString &identity);
        void positionChanged(double seconds);
        void durationChanged(double seconds);
        void tracksChanged(const QJsonArray &tracks);
        void subtitleTranslationChanged(const QString &message);
        void downloadRequested(const QUrl &url, const QString &title, const Media::PlaybackOptions &options);

    private:
        struct Playback
        {
            QUrl url;
            QString title;
            PlaybackOptions options;
            QString positionKey;
        };

        enum class SubtitleCommand
        {
            None,
            StartTime,
            SourceSelection,
            SourceTracks,
            Add,
            AddedTracks,
            Select,
            Verify
        };

        struct SubtitleTranslation
        {
            QJsonObject source;
            QJsonObject loading;
            QJsonObject queued;
            QString revision;
            qint64 request = 0;
            qint64 requestedBucket = -1;
            double windowStart = 0;
            double windowEnd = 0;
            double delay = 0;
            std::optional<double> startTime;
            int translatedId = 0;
            int commandId = 0;
            SubtitleCommand command = SubtitleCommand::None;
            quint64 generation = 0;
            quint64 loadingGeneration = 0;
            bool loaded = false;
            bool enabled = true;
            bool seeking = false;
            bool announced = false;
        };

        explicit Player(QObject *parent);
        ~Player() override;

        bool startPlayback(const QUrl &url, const QString &title, const PlaybackOptions &options);
        bool startPending();
        bool startPrepared(const QJsonObject &runtime);
        bool prepareCookies();
        void stopProcess();
        void finishPlayback();
        void connectIpc();
        void loadMedia();
        void readMessages();
        void sendCommand(const QJsonArray &command, int requestId = 0);
        void savePosition();
        void setPlaying(bool playing);
        void cancelSubtitleRequest();
        void updateSubtitleSelection(const QJsonValue &selection);
        void initializeSubtitles(const QJsonArray &tracks);
        void requestSubtitles();
        void receiveSubtitles(const QString &event, const QJsonObject &data);
        void applySubtitles();
        void sendSubtitleCommand(const QJsonArray &command, SubtitleCommand stage);
        void finishSubtitleCommand(const QJsonObject &message);
        void subtitleFailure(const QString &message);

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
        double m_position = 0;
        double m_duration = 0;
        int m_initialAudioId = 0;
        bool m_playing = false;
        bool m_stopping = false;
        qint64 m_preparationRequest = 0;
        SubtitleTranslation m_subtitles;
        int m_nextSubtitleCommand = 10;

        static QPointer<Player> m_instance;
    };
}
