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
#include <QJsonObject>
#include <QMap>
#include <QObject>
#include <QPointer>
#include <QThreadPool>
#include <QTimer>

#include "repairanalysis.h"

namespace BitTorrent
{
    class RepairFileGuard;
    class StagingOperation;
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
        void analyzeStaged(const QStringList &sourceRoots, const QMap<int, QString> &mappings = {});
        void prepareStaged();
        void recoverStaged();
        void commitStaged();
        void rollbackStaged();
        QJsonObject stagingStatus() const;

    signals:
        void analyzed(const RepairAnalysis &analysis);
        void failed(const QString &error);
        void recheckStarted();
        void recheckFinished();
        void stagingChanged(const QJsonObject &status);
        void committedVerified();

    private:
        enum class State
        {
            Idle,
            Draining,
            Analyzing,
            Ready,
            Applying,
            Rechecking,
            PersistingPlan,
            PreparingStaging,
            SwitchingStaging,
            DownloadingStaging,
            DrainingStaging,
            VerifyingStaging,
            ReadyToCommit,
            CommittingStaging,
            RollingBackStaging,
            SwitchingDestination,
            PersistingDestination,
            Finished
        };

        QString conflictingTorrent() const;
        void snapshotOtherFiles();
        void analyzeDrainedData();
        void runWorker(std::function<void ()> work);
        void releaseOwnership();
        void fail(const QString &error);
        void verifyStaging();
        void prepareStagingData();
        void publishStaging();

        QPointer<TorrentImpl> m_torrent;
        std::shared_ptr<lt::torrent_info> m_target;
        lt::file_storage m_files;
        QString m_savePath;
        QMap<QString, QString> m_otherFiles;
        QMap<QString, QString> m_unresolvedDirectories;
        std::shared_ptr<RepairFileGuard> m_guard;
        std::unique_ptr<StagingOperation> m_staging;
        QStringList m_sourceRoots;
        QMap<int, QString> m_sourceMappings;
        QSet<int> m_selectedFiles;
        bool m_staged = false;
        bool m_recovering = false;
        bool m_rollingBack = false;
        QFutureWatcher<bool> m_drainWatcher;
        QFutureWatcher<bool> m_persistenceWatcher;
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
