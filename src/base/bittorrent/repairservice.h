/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <atomic>
#include <functional>
#include <memory>

#include <libtorrent/file_storage.hpp>

#include <QFutureWatcher>
#include <QMap>
#include <QObject>
#include <QPointer>
#include <QThreadPool>
#include <QTimer>

#include "repairanalysis.h"

namespace BitTorrent
{
    class RepairFileGuard;
    class Torrent;
    class TorrentImpl;

    class RepairService final : public QObject
    {
        Q_OBJECT

    public:
        explicit RepairService(Torrent *torrent, QObject *parent = nullptr);
        ~RepairService() override;

        void analyze();
        void apply();

    signals:
        void analyzed(const RepairAnalysis &analysis);
        void failed(const QString &error);
        void recheckStarted();
        void recheckFinished();

    private:
        enum class State
        {
            Idle,
            Draining,
            Analyzing,
            Ready,
            Applying,
            Rechecking,
            Finished
        };

        QString conflictingTorrent() const;
        void snapshotOtherFiles();
        void analyzeDrainedData();
        void runWorker(std::function<void ()> work);
        void releaseOwnership();
        void fail(const QString &error);

        QPointer<TorrentImpl> m_torrent;
        std::shared_ptr<lt::torrent_info> m_target;
        lt::file_storage m_files;
        QString m_savePath;
        QMap<QString, QString> m_otherFiles;
        QMap<QString, QString> m_unresolvedDirectories;
        std::shared_ptr<RepairFileGuard> m_guard;
        QFutureWatcher<bool> m_drainWatcher;
        QFutureWatcher<void> m_watcher;
        QThreadPool m_worker;
        RepairAnalysis m_analysis;
        QString m_error;
        QTimer m_drainTimeout;
        std::atomic_bool m_cancelled = false;
        State m_state = State::Idle;
        bool m_ownsTorrent = false;
    };
}
