/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "repairplan.h"

#include <algorithm>

#include <libtorrent/file_storage.hpp>
#include <libtorrent/torrent_info.hpp>

#include <QDir>
#include <QDirIterator>
#include <QFileInfo>

#include "repairanalysis.h"
#include "repairfileguard.h"
#include "stagingoperation.h"

using namespace BitTorrent;

namespace
{
    bool isCancelled(const std::atomic_bool *cancelled)
    {
        return cancelled && cancelled->load(std::memory_order_relaxed);
    }
}

QMap<int, QString> BitTorrent::findRepairSources(const lt::file_storage &files, const QString &destination
    , const QStringList &roots, const QMap<int, QString> &explicitMappings, QString &error
    , const std::atomic_bool *cancelled)
{
    error.clear();
    QMap<qint64, QStringList> bySize;
    QSet<QString> indexed;
    int inspected = 0;
    if (roots.size() > 32)
    {
        error = QStringLiteral("Select at most 32 source directories per analysis.");
        return {};
    }
    for (const QString &root : roots)
    {
        const QFileInfo directory(root);
        if (!directory.isDir() || directory.isSymbolicLink() || directory.isJunction())
        {
            error = QStringLiteral("Select ordinary source directories without links or junctions.");
            return {};
        }
        auto rootGuard = RepairFileGuard::open(lt::file_storage {}, root, false, error, cancelled);
        if (!rootGuard)
            return {};
        QStringList directories {QDir::cleanPath(QDir::fromNativeSeparators(root))};
        while (!directories.isEmpty())
        {
            QDirIterator iterator(directories.takeLast(), QDir::Files | QDir::Dirs | QDir::NoDotAndDotDot);
            while (iterator.hasNext())
            {
                if (isCancelled(cancelled))
                {
                    error = QStringLiteral("Source indexing cancelled.");
                    return {};
                }
                const QString path = QDir::cleanPath(QDir::fromNativeSeparators(iterator.next()));
                const QFileInfo info = iterator.fileInfo();
                if (++inspected > 100000)
                {
                    error = QStringLiteral("The selected source roots exceed 100000 entries. Select narrower directories.");
                    return {};
                }
                const QString key = path.toCaseFolded();
                if (info.isSymbolicLink() || info.isJunction() || indexed.contains(key))
                    continue;
                if (info.isDir())
                {
                    directories.append(path);
                    continue;
                }
                indexed.insert(key);
                bySize[info.size()].append(path);
            }
        }
    }

    QMap<int, QString> mappings = explicitMappings;
    for (auto it = mappings.cbegin(); it != mappings.cend(); ++it)
    {
        const QString path = QDir::cleanPath(QDir::fromNativeSeparators(it.value()));
        if ((it.key() < 0) || (it.key() >= files.num_files()) || files.pad_file_at(lt::file_index_t(it.key()))
            || !QDir::isAbsolutePath(path) || (path != QDir::fromNativeSeparators(it.value())))
        {
            error = QStringLiteral("An explicit source mapping is invalid.");
            return {};
        }
        const QFileInfo source(path);
        if (!source.isFile() || source.isSymbolicLink() || source.isJunction())
        {
            error = QStringLiteral("An explicit source must be an ordinary existing file: %1").arg(path);
            return {};
        }
    }

    for (const lt::file_index_t index : files.file_range())
    {
        if (files.pad_file_at(index) || mappings.contains(int(index)))
            continue;
        const QString relative = QDir::fromNativeSeparators(QString::fromStdString(files.file_path(index)));
        QStringList exact;
        for (const QString &root : roots)
        {
            const QString path = QDir(root).filePath(relative);
            if (QFileInfo::exists(path) && !exact.contains(path))
                exact.append(path);
        }
        if (!exact.isEmpty())
        {
            mappings.insert(int(index), exact.first());
            continue;
        }
        const QStringList candidates = bySize.value(files.file_size(index));
        QStringList names;
        for (const QString &path : candidates)
        {
            if (QFileInfo(path).fileName().compare(QFileInfo(relative).fileName(), Qt::CaseInsensitive) == 0)
                names.append(path);
        }
        const QStringList &matches = names.isEmpty() ? candidates : names;
        if (matches.size() == 1)
            mappings.insert(int(index), matches.first());
        else if (QFileInfo::exists(QDir(destination).filePath(relative)))
            mappings.insert(int(index), QDir(destination).filePath(relative));
        // Ambiguous metadata never becomes a claimed match. The user may set
        // an explicit native-file mapping; otherwise the engine downloads it.
    }
    return mappings;
}

RepairPlan BitTorrent::planRepairData(const lt::torrent_info &target, const lt::file_storage &files
    , const QString &destination, const QStringList &roots, const QMap<int, QString> &explicitMappings
    , const QSet<int> &selected, const std::atomic_bool *cancelled)
{
    RepairPlan result;
    if (!target.is_valid())
    {
        result.error = QStringLiteral("Select valid torrent metadata.");
        return result;
    }
    if (selected.isEmpty() || std::any_of(selected.cbegin(), selected.cend(), [&files](const int index)
        { return (index < 0) || (index >= files.num_files()) || files.pad_file_at(lt::file_index_t(index)); }))
    {
        result.error = QStringLiteral("Select at least one valid target file for repair.");
        return result;
    }

    auto destinationGuard = RepairFileGuard::open(files, destination, false, result.error, cancelled);
    if (!destinationGuard)
        return result;
    result.mappings = findRepairSources(files, destination, roots, explicitMappings, result.error, cancelled);
    if (!result.error.isEmpty())
        return result;
    const auto sourceGuards = RepairFileGuard::openSources(files, result.mappings, result.error, cancelled);
    if (!result.error.isEmpty())
        return result;

    lt::file_storage sourceLayout = files;
    QSet<int> readable;
    for (auto it = result.mappings.cbegin(); it != result.mappings.cend(); ++it)
    {
        sourceLayout.rename_file(lt::file_index_t(it.key()), it.value().toStdString());
        if (QFileInfo::exists(it.value()))
            readable.insert(it.key());
    }
    // Keep the complete logical layout: a selected v1 boundary can only be
    // verified when its bytes in adjacent ignored files also match the hash.
    const RepairAnalysis analysis = analyzeRepairData(target, sourceLayout, destination, cancelled, &readable);
    if (!analysis.error.isEmpty())
    {
        result.error = analysis.error;
        return result;
    }

    const auto storage = StagingOperation::storageRequirement(files, destination, result.mappings);
    if (!storage)
    {
        result.error = storage.error();
        return result;
    }
    result.temporaryStorageBytes = storage->requiredBytes;
    result.availableStorageBytes = storage->availableBytes;

    qint64 targetBytes = 0;
    for (const RepairFileAnalysis &analysisFile : analysis.files)
    {
        RepairPlanFile file;
        file.nativeIndex = analysisFile.nativeIndex;
        file.targetPath = QDir::fromNativeSeparators(QString::fromStdString(files.file_path(
            lt::file_index_t(analysisFile.nativeIndex))));
        file.sourcePath = result.mappings.value(file.nativeIndex);
        file.expectedBytes = analysisFile.expectedSize;
        file.candidateBytes = (analysisFile.actualSize < 0) ? 0
            : std::min(analysisFile.actualSize, analysisFile.expectedSize);
        file.verifiedBytes = analysisFile.verifiedBytes;
        file.problems = analysisFile.problems;
        result.files.append(file);
        if (!selected.contains(file.nativeIndex))
            continue;
        targetBytes += file.expectedBytes;
        result.candidateBytes += file.candidateBytes;
        result.verifiedBytes += file.verifiedBytes;
        if ((analysisFile.actualSize != analysisFile.expectedSize) || (analysisFile.verifiedBytes != analysisFile.expectedSize))
            ++result.changedFiles;
        if (analysisFile.actualSize > analysisFile.expectedSize)
            ++result.oversizedFiles;
    }
    result.requiredNetworkBytes = std::max<qint64>(0, targetBytes - result.verifiedBytes);
    return result;
}
