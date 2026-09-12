/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "repairservice.h"

#include <exception>

#include <QDir>
#include <QPromise>
#include <QSet>

#include "base/path.h"
#include "common.h"
#include "repairfileguard.h"
#include "sessionimpl.h"
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
        fail(tr("The torrent did not release disk I/O within 30 seconds. No repair changes were made."));
    });
    connect(&m_drainWatcher, &QFutureWatcher<bool>::finished, this, [this]
    {
        if (m_state != State::Draining)
            return;
        if (m_drainWatcher.isCanceled() || !m_drainWatcher.result())
        {
            fail(tr("The torrent storage was unavailable while draining disk I/O."));
            return;
        }
        analyzeDrainedData();
    });
    if (m_torrent)
    {
        connect(m_torrent->session(), &Session::torrentFinishedChecking, this, [this](Torrent *torrent)
        {
            if ((m_state != State::Rechecking) || (torrent != m_torrent))
                return;
            m_state = State::Finished;
            releaseOwnership();
            emit recheckFinished();
        });
        connect(m_torrent->session(), &Session::torrentsUpdated, this, [this]
        {
            if ((m_state == State::Rechecking) && m_torrent && m_torrent->hasError())
                fail(tr("The native recheck failed: %1").arg(m_torrent->error()));
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
    if (m_torrent->isAutoTMMEnabled() || !m_torrent->downloadPath().isEmpty())
    {
        fail(tr("This repair slice requires manual torrent management and a single save directory."));
        return;
    }
    auto *session = static_cast<SessionImpl *>(m_torrent->session());
    if (session->isAppendExtensionEnabled())
    {
        fail(tr("Disable the incomplete-file extension before using this repair slice."));
        return;
    }
    const QList<DownloadPriority> priorities = m_torrent->filePriorities();
    for (int i = 0; i < m_torrent->filesCount(); ++i)
    {
        if ((priorities.at(i) == DownloadPriority::Ignored)
            || (m_torrent->actualFilePath(i) != m_torrent->filePath(i))
            || m_torrent->filePath(i).hasExtension(QB_EXT))
        {
            fail(tr("This repair slice requires all files selected and no temporary filename or unwanted-folder mappings. Explicit torrent file renames are supported."));
            return;
        }
    }

    m_savePath = m_torrent->actualStorageLocation().toString();
    m_target = m_torrent->info().nativeInfo();
    m_files = m_target->files();
    const QList<lt::file_index_t> indexes = m_torrent->info().nativeIndexes();
    for (int i = 0; i < m_torrent->filesCount(); ++i)
        m_files.rename_file(indexes.at(i), m_torrent->actualFilePath(i).toString().toStdString());

    snapshotOtherFiles();
    if (const auto result = m_torrent->beginRepair(); !result)
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
    m_state = State::Analyzing;
    runWorker([this]
    {
        m_error = conflictingTorrent();
        if (!m_error.isEmpty())
            return;
        m_guard = RepairFileGuard::open(m_files, m_savePath, false, m_error, &m_cancelled);
        if (!m_guard)
            return;
        const QSet<int> readableFiles = m_guard->existingFiles();
        m_analysis = analyzeRepairData(*m_target, m_files, m_savePath, &m_cancelled, &readableFiles);
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
    for (const RepairFileAnalysis &file : m_analysis.files)
    {
        // libtorrent initialize_storage creates absent zero-length files even
        // during recheck. This slice does not safely reserve missing paths for
        // that write. Nonzero missing targets are read-only during the check.
        if ((file.expectedSize == 0) && (file.actualSize < 0))
        {
            fail(tr("This repair slice cannot apply while a zero-length target file is missing. "
                "Create it with the normal downloader, stop the torrent, and analyze again."));
            return;
        }
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
        m_guard = RepairFileGuard::open(m_files, m_savePath, true, m_error, &m_cancelled);
        if (!m_guard)
            return;
        if (m_guard->identity() != analyzedIdentity)
        {
            m_error = tr("The data changed after analysis. No repair changes were made; analyze it again.");
            return;
        }
        m_guard->truncateOversized(m_error, &m_cancelled);
    });
}

void RepairService::releaseOwnership()
{
    m_drainTimeout.stop();
    if (m_ownsTorrent && m_torrent && (m_state == State::Rechecking))
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

void RepairService::fail(const QString &error)
{
    releaseOwnership();
    m_state = State::Finished;
    emit failed(error);
}
