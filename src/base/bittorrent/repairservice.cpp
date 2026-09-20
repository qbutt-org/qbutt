/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "repairservice.h"

#include <exception>

#include <QDir>
#include <QFileInfo>
#include <QPromise>
#include <QSet>

#include "base/path.h"
#include "common.h"
#include "repairfileguard.h"
#include "repairplan.h"
#include "sessionimpl.h"
#include "stagingoperation.h"
#include "torrentimpl.h"

using namespace BitTorrent;

RepairService::RepairService(Torrent *torrent, QObject *parent)
    : QObject {parent}
    , m_torrent {qobject_cast<TorrentImpl *>(torrent)}
{
    m_worker.setMaxThreadCount(1);
    m_drainTimeout.setSingleShot(true);
    m_drainTimeout.setInterval(30000);
    connect(&m_drainTimeout, &QTimer::timeout, this, [this]
    {
        fail(m_staged ? tr("Staged repair timed out waiting for disk I/O or saved torrent location. The recovery journal is retained.")
            : tr("The torrent did not release disk I/O within 30 seconds. No repair changes were made."));
    });
    connect(&m_persistenceWatcher, &QFutureWatcher<bool>::finished, this, [this]
    {
        if ((m_state != State::PersistingDestination) && (m_state != State::PersistingPlan))
            return;
        m_drainTimeout.stop();
        if (m_persistenceWatcher.isCanceled() || !m_persistenceWatcher.result())
        {
            fail(tr("The final torrent location could not be saved. The recovery journal is retained."));
            return;
        }
        if (m_state == State::PersistingPlan)
        {
            prepareStagingData();
            return;
        }
        if (!m_staging->finish(m_error))
        {
            fail(m_error);
            return;
        }
        m_state = State::Finished;
        releaseOwnership();
        if (!m_rollingBack && m_torrent)
            static_cast<SessionImpl *>(m_torrent->session())->handleTorrentFinished(m_torrent);
        publishStaging();
        if (!m_rollingBack)
            emit committedVerified();
    });
    connect(&m_drainWatcher, &QFutureWatcher<bool>::finished, this, [this]
    {
        if ((m_state != State::Draining) && (m_state != State::DrainingStaging))
            return;
        if (m_drainWatcher.isCanceled() || !m_drainWatcher.result())
        {
            fail(tr("The torrent storage was unavailable while draining disk I/O."));
            return;
        }
        if (m_state == State::DrainingStaging)
        {
            m_drainTimeout.stop();
            if (m_rollingBack)
            {
                m_state = State::RollingBackStaging;
                runWorker([this] { m_staging->rollback(m_error, &m_cancelled); });
            }
            else
            {
                verifyStaging();
            }
        }
        else
        {
            analyzeDrainedData();
        }
    });
    if (m_torrent)
    {
        connect(m_torrent->session(), &Session::torrentFinishedChecking, this, [this](Torrent *torrent)
        {
            if ((m_state != State::Rechecking) || (torrent != m_torrent))
                return;
            if (m_staged)
            {
                m_state = State::DownloadingStaging;
                m_torrent->startStagedDownload();
                publishStaging();
                return;
            }
            m_state = State::Finished;
            releaseOwnership();
            emit recheckFinished();
        });
        connect(m_torrent->session(), &Session::torrentsUpdated, this, [this]
        {
            if ((m_state == State::Rechecking) && m_torrent && m_torrent->hasError())
                fail(tr("The native recheck failed: %1").arg(m_torrent->error()));
            if ((m_state == State::DownloadingStaging) && m_torrent && (m_torrent->progress() == 1.0))
            {
                m_torrent->stop();
                m_state = State::DrainingStaging;
                m_drainTimeout.start();
                m_drainWatcher.setFuture(static_cast<SessionImpl *>(m_torrent->session())->drainTorrentDisk(m_torrent));
            }
        });
        connect(m_torrent, &TorrentImpl::repairStorageChanged, this, [this](const QString &error)
        {
            if (!error.isEmpty())
            {
                fail(error);
                return;
            }
            if (m_state == State::SwitchingStaging)
            {
                m_state = State::Rechecking;
                m_torrent->startRepairRecheck();
                emit recheckStarted();
            }
            else if (m_state == State::SwitchingDestination)
            {
                m_state = State::PersistingDestination;
                m_drainTimeout.start();
                m_persistenceWatcher.setFuture(static_cast<SessionImpl *>(m_torrent->session())->persistStoppedTorrent(
                    m_torrent, Path(m_staging->destination())));
            }
        });
    }

    connect(&m_watcher, &QFutureWatcher<void>::finished, this, [this]
    {
        if (!m_error.isEmpty())
        {
            fail(m_error);
            return;
        }
        if (m_state == State::Analyzing)
        {
            m_state = State::Ready;
            emit analyzed(m_analysis);
            if (m_staged)
                publishStaging();
        }
        else if (m_state == State::PreparingStaging)
        {
            m_state = State::SwitchingStaging;
            m_torrent->switchRepairStorage(Path(m_staging->payloadPath()));
            publishStaging();
        }
        else if (m_state == State::VerifyingStaging)
        {
            m_state = State::ReadyToCommit;
            publishStaging();
        }
        else if ((m_state == State::CommittingStaging) || (m_state == State::RollingBackStaging))
        {
            m_state = State::SwitchingDestination;
            m_torrent->switchRepairStorage(Path(m_staging->destination())
                , m_rollingBack || m_recovering || (m_selectedFiles.size() != m_torrent->filesCount()));
        }
        else if (m_state == State::Applying)
        {
            // Exclusive repair changes are complete. The native recheck uses
            // ordinary engine file ownership; existing parent directories and
            // the application's torrent reservation stay held until it finishes.
            m_guard->releaseFiles();
            m_state = State::Rechecking;
            if (m_torrent)
            {
                m_torrent->startRepairRecheck();
                emit recheckStarted();
            }
        }
    });
}

RepairService::~RepairService()
{
    m_cancelled.store(true, std::memory_order_relaxed);
    if (m_watcher.isRunning())
        m_watcher.waitForFinished();
    releaseOwnership();
}

void RepairService::analyze()
{
    if (m_state != State::Idle)
        return;
    if (!m_torrent || !m_torrent->hasMetadata())
    {
        fail(tr("Select a torrent with its target metadata available."));
        return;
    }
    if (!m_recovering && QFileInfo::exists(StagingOperation::journalPath(m_torrent->id().toString())))
    {
        fail(tr("Recover the pending staged operation before starting a new analysis."));
        return;
    }
    if (m_staged && m_torrent->isAutoTMMEnabled())
    {
        fail(tr("Staged update currently requires manual torrent management."));
        return;
    }
    auto *session = static_cast<SessionImpl *>(m_torrent->session());
    if (m_staged && (!m_torrent->downloadPath().isEmpty() || session->isAppendExtensionEnabled()))
    {
        fail(tr("Staged update currently requires a single save directory and the incomplete-file extension disabled."));
        return;
    }
    if (m_staged)
    {
        for (const Path &path : m_torrent->filePaths())
        {
            if (path.hasExtension(QB_EXT))
            {
                fail(tr("Staged update requires target names without an incomplete-file extension."));
                return;
            }
        }
    }
    if (const auto result = m_torrent->beginRepair(m_recovering); !result)
    {
        fail(result.error());
        return;
    }

    m_ownsTorrent = true;
    m_state = State::Draining;
    m_drainTimeout.start();
    m_drainWatcher.setFuture(session->drainTorrentDisk(m_torrent));
}

void RepairService::snapshotOtherFiles()
{
    m_otherFiles.clear();
    m_unresolvedDirectories.clear();
    for (const Torrent *other : m_torrent->session()->torrents())
    {
        if (other == m_torrent)
            continue;
        if (!other->hasMetadata())
        {
            m_unresolvedDirectories.insert(other->savePath().toString(), other->name());
            if (!other->downloadPath().isEmpty())
                m_unresolvedDirectories.insert(other->downloadPath().toString(), other->name());
            continue;
        }
        const QDir directory(other->actualStorageLocation().toString());
        for (const Path &relative : other->actualFilePaths())
            m_otherFiles.insert(directory.filePath(relative.toString()), other->name());
    }
}

QString RepairService::conflictingTorrent() const
{
    const QString canonicalRoot = repairPathIdentity(m_savePath);
    if (canonicalRoot.isEmpty())
        return tr("Cannot resolve the data directory identity.");
    for (auto it = m_unresolvedDirectories.cbegin(); it != m_unresolvedDirectories.cend(); ++it)
    {
        const QString directory = repairPathIdentity(it.key());
        if (directory.isEmpty() || (directory == canonicalRoot)
            || directory.startsWith(canonicalRoot + u'/') || canonicalRoot.startsWith(directory + u'/'))
        {
            return tr("Another torrent is still resolving its data layout in this directory: %1").arg(it.value());
        }
    }
    QSet<QString> ownedPaths;
    QSet<QString> ownedDirectories;
    for (const lt::file_index_t index : m_files.file_range())
    {
        if (m_files.pad_file_at(index))
            continue;
        const QString path = repairPathIdentity(QDir(m_savePath).filePath(
            QDir::fromNativeSeparators(QString::fromStdString(m_files.file_path(index)))));
        if (path.isEmpty())
            return tr("Cannot resolve a target file identity.");
        ownedPaths.insert(path);
        for (QString parent = path.section(u'/', 0, -2); !parent.isEmpty(); parent = parent.section(u'/', 0, -2))
            ownedDirectories.insert(parent);
    }
    for (auto it = m_otherFiles.cbegin(); it != m_otherFiles.cend(); ++it)
    {
        if (m_cancelled.load(std::memory_order_relaxed))
            return tr("Analysis cancelled.");
        const QString otherPath = repairPathIdentity(it.key());
        if (otherPath.isEmpty())
            return tr("Cannot resolve another torrent's file identity: %1").arg(it.value());
        bool overlaps = ownedPaths.contains(otherPath) || ownedDirectories.contains(otherPath);
        for (QString parent = otherPath.section(u'/', 0, -2); !parent.isEmpty() && !overlaps; parent = parent.section(u'/', 0, -2))
        {
            overlaps = ownedPaths.contains(parent);
        }
        if (overlaps)
            return tr("Another torrent owns an overlapping data path: %1").arg(it.value());
    }
    return {};
}

void RepairService::analyzeDrainedData()
{
    if (m_state != State::Draining)
        return;
    m_drainTimeout.stop();
    // Capture the engine's physical layout only after ownership prevents changes
    // to priorities, names and storage location, and pending disk I/O has drained.
    m_savePath = m_torrent->actualStorageLocation().toString();
    m_target = m_torrent->info().nativeInfo();
    m_files = m_target->files();
    const QList<DownloadPriority> priorities = m_torrent->filePriorities();
    const QList<lt::file_index_t> indexes = m_torrent->info().nativeIndexes();
    for (int i = 0; i < m_torrent->filesCount(); ++i)
    {
        m_files.rename_file(indexes.at(i), m_torrent->actualFilePath(i).toString().toStdString());
        if (priorities.at(i) != DownloadPriority::Ignored)
            m_selectedFiles.insert(int(indexes.at(i)));
    }
    if (m_selectedFiles.isEmpty())
    {
        fail(tr("Select at least one target file before repairing its data."));
        return;
    }
    snapshotOtherFiles();
    m_state = State::Analyzing;
    runWorker([this]
    {
        if (m_recovering)
        {
            m_staging = StagingOperation::load(StagingOperation::journalPath(m_torrent->id().toString())
                , m_torrent->id().toString(), m_files, m_error);
            if (m_staging)
                m_savePath = m_staging->destination();
            return;
        }
        m_error = conflictingTorrent();
        if (!m_error.isEmpty())
            return;
        m_guard = RepairFileGuard::open(m_files, m_savePath, false, m_error, &m_cancelled);
        if (!m_guard)
            return;
        if (m_staged)
        {
            const QMap<int, QString> sources = findRepairSources(
                m_files, m_savePath, m_sourceRoots, m_sourceMappings, m_error, &m_cancelled);
            if (!m_error.isEmpty())
                return;
            m_staging = StagingOperation::plan(StagingOperation::journalPath(m_torrent->id().toString())
                , m_torrent->id().toString(), *m_target, m_files, m_savePath, sources, m_selectedFiles, m_error, &m_cancelled);
            if (m_staging)
                m_analysis = m_staging->analysis();
            return;
        }
        const QSet<int> readableFiles = m_guard->existingFiles();
        m_analysis = analyzeRepairData(*m_target, m_files, m_savePath, &m_cancelled, &readableFiles);
        for (RepairFileAnalysis &file : m_analysis.files)
            file.selected = m_selectedFiles.contains(file.nativeIndex);
        m_error = m_analysis.error;
    });
}

void RepairService::runWorker(std::function<void ()> work)
{
    // Only the worker accesses the result members until the future completes.
    // Destruction cancels and waits before releasing the torrent or handles.
    QPromise<void> promise;
    promise.start();
    m_watcher.setFuture(promise.future());
    m_worker.start([this, work = std::move(work), promise = std::move(promise)]() mutable
    {
        try
        {
            work();
        }
        catch (const std::exception &error)
        {
            m_error = tr("Repair operation failed: %1").arg(QString::fromLocal8Bit(error.what()));
        }
        promise.finish();
    });
}

void RepairService::apply()
{
    if (m_state != State::Ready)
        return;
    if (!m_torrent || !m_torrent->isStopped() || !m_ownsTorrent)
    {
        fail(tr("Torrent ownership changed. Analyze the data again."));
        return;
    }
    snapshotOtherFiles();

    m_state = State::Applying;
    runWorker([this]
    {
        m_error = conflictingTorrent();
        if (!m_error.isEmpty())
            return;
        const QByteArray analyzedIdentity = m_guard->identity();
        m_guard.reset();
        m_guard = RepairFileGuard::open(m_files, m_savePath, true, m_error, &m_cancelled, false, &m_selectedFiles);
        if (!m_guard)
            return;
        if (m_guard->identity() != analyzedIdentity)
        {
            m_error = tr("The data changed after analysis. No repair changes were made; analyze it again.");
            return;
        }
        if (!m_guard->createMissingEmpty(m_error, &m_cancelled))
            return;
        m_guard->truncateOversized(m_error, &m_cancelled);
    });
}

void RepairService::releaseOwnership()
{
    m_drainTimeout.stop();
    m_persistenceWatcher.cancel();
    if (m_ownsTorrent && m_torrent && ((m_state == State::Rechecking) || m_staged))
    {
        m_torrent->stop();
        auto *session = static_cast<SessionImpl *>(m_torrent->session());
        session->drainTorrentDisk(m_torrent).waitForFinished();
    }
    m_guard.reset();
    if (m_ownsTorrent && m_torrent)
        m_torrent->endRepair();
    m_ownsTorrent = false;
}

void RepairService::analyzeStaged(const QStringList &sourceRoots, const QMap<int, QString> &mappings)
{
    if (m_state != State::Idle)
        return;
    m_staged = true;
    m_sourceRoots = sourceRoots;
    m_sourceMappings = mappings;
    analyze();
}

void RepairService::recoverStaged()
{
    if (m_state != State::Idle)
        return;
    m_staged = true;
    m_recovering = true;
    analyze();
}

void RepairService::prepareStaged()
{
    if (!m_staged || !m_staging || (m_state != State::Ready))
        return;
    if (m_recovering)
    {
        if (m_staging->state() == u"downloading")
        {
            m_guard = RepairFileGuard::open(m_files, m_staging->payloadPath(), false, m_error, &m_cancelled);
            if (!m_guard || (m_guard->existingFiles().size() != m_torrent->filesCount()))
            {
                fail(m_error.isEmpty() ? tr("Staged files are missing. Roll back and prepare a new independent staging directory.") : m_error);
                return;
            }
            m_guard->releaseFiles();
            m_state = State::SwitchingStaging;
            m_torrent->switchRepairStorage(Path(m_staging->payloadPath()));
            publishStaging();
        }
        else
        {
            fail(tr("Use commit recovery or rollback for this interrupted operation."));
        }
        return;
    }
    m_state = State::PersistingPlan;
    publishStaging();
    m_drainTimeout.start();
    m_persistenceWatcher.setFuture(static_cast<SessionImpl *>(m_torrent->session())->persistStoppedTorrent(
        m_torrent, Path(m_savePath)));
}

void RepairService::prepareStagingData()
{
    m_state = State::PreparingStaging;
    runWorker([this]
    {
        if (!m_staging->prepare(m_error, &m_cancelled))
            return;
        m_guard.reset();
        m_guard = RepairFileGuard::open(m_files, m_staging->payloadPath(), false, m_error, &m_cancelled);
        if (m_guard)
            m_guard->releaseFiles();
    });
}

void RepairService::verifyStaging()
{
    m_state = State::VerifyingStaging;
    runWorker([this] { m_staging->verify(*m_target, m_error, &m_cancelled); });
}

void RepairService::commitStaged()
{
    if (!m_staging || ((m_state != State::ReadyToCommit) && !(m_recovering && (m_state == State::Ready))))
        return;
    m_guard.reset();
    m_state = State::CommittingStaging;
    publishStaging();
    snapshotOtherFiles();
    runWorker([this]
    {
        m_error = conflictingTorrent();
        if (m_error.isEmpty())
            m_staging->commit(*m_target, m_error, &m_cancelled);
    });
}

void RepairService::rollbackStaged()
{
    if (!m_staging || !m_ownsTorrent || m_watcher.isRunning()
        || ((m_state != State::Ready) && (m_state != State::ReadyToCommit) && (m_state != State::DownloadingStaging)))
        return;
    m_rollingBack = true;
    m_guard.reset();
    m_torrent->stop();
    m_state = State::DrainingStaging;
    publishStaging();
    m_drainTimeout.start();
    m_drainWatcher.setFuture(static_cast<SessionImpl *>(m_torrent->session())->drainTorrentDisk(m_torrent));
}

QJsonObject RepairService::stagingStatus() const
{
    if (!m_staging)
        return {};
    QJsonObject status = m_staging->status();
    const QString state = m_staging->state();
    status.insert(QStringLiteral("finalized"), (m_state == State::Finished) && m_error.isEmpty());
    status.insert(QStringLiteral("can_prepare"), (m_state == State::Ready) && ((state == u"planned") || (state == u"downloading")));
    status.insert(QStringLiteral("can_commit"), (m_state == State::ReadyToCommit) || ((m_state == State::Ready) && m_recovering
        && ((state == u"ready_to_commit") || (state == u"committing") || (state == u"committed"))));
    status.insert(QStringLiteral("can_rollback"), m_ownsTorrent && !m_watcher.isRunning() && (state != u"planned")
        && ((m_state == State::Ready) || (m_state == State::ReadyToCommit) || (m_state == State::DownloadingStaging)));
    return status;
}

void RepairService::publishStaging()
{
    emit stagingChanged(stagingStatus());
}

void RepairService::fail(const QString &error)
{
    m_error = error;
    releaseOwnership();
    m_state = State::Finished;
    emit failed(m_error);
}
