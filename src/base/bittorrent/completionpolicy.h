/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QFutureWatcher>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QSet>
#include <QTimer>

#include "infohash.h"

namespace BitTorrent
{
    class SessionImpl;
    class TorrentImpl;

    // Rules observe the native torrent model. Only the outstanding disk/write
    // barrier and durable action receipts live here; this is not torrent state.
    class CompletionPolicy final : public QObject
    {
        Q_OBJECT

    public:
        explicit CompletionPolicy(SessionImpl *session);
        ~CompletionPolicy() override;

        static QString validateConfiguration(const QJsonObject &configuration);
        QJsonObject configuration() const;
        QString configure(const QJsonObject &configuration);
        QJsonArray preview(const QJsonObject &configuration = {}) const;
        QJsonArray journal() const;
        QString acknowledgePreview(const TorrentID &id);
        void enqueue(TorrentImpl *torrent, bool nativeEvent = true);
        bool hasPendingEvents() const;
        bool isBusy() const;
        bool hasError() const;

    signals:
        void changed();
        void wantedFilesCommitted(TorrentImpl *torrent);
        void notified(const QString &title, const QString &message);
        void allWantedFilesCommitted();

    private:
        QJsonArray evaluate(const TorrentImpl *torrent, const QJsonObject &configuration) const;
        void process();
        void finish(bool success);
        QString saveJournal(const QJsonArray &entries);

        SessionImpl *m_session;
        QJsonObject m_configuration;
        QJsonArray m_journal;
        QString m_error;
        QSet<QString> m_executed;
        QHash<TorrentID, bool> m_pending;
        QPointer<TorrentImpl> m_current;
        QFutureWatcher<bool> m_watcher;
        QTimer m_timer;
        bool m_persisting = false;
        bool m_acknowledging = false;
        bool m_nativeEvent = false;
        bool m_notifySession = false;
    };
}
