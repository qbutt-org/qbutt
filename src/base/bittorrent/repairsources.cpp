/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "repairsources.h"

#include <libtorrent/file_storage.hpp>

#include <QCoreApplication>
#include <QDir>
#include <QDirIterator>
#include <QFileInfo>
#include <QSet>

#include "repairfileguard.h"

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
        error = QCoreApplication::translate("RepairSources", "Select at most 32 source directories per analysis.");
        return {};
    }
    for (const QString &root : roots)
    {
        const QFileInfo directory(root);
        if (!directory.isDir() || directory.isSymbolicLink() || directory.isJunction())
        {
            error = QCoreApplication::translate("RepairSources", "Select ordinary source directories without links or junctions.");
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
                    error = QCoreApplication::translate("RepairSources", "Source indexing cancelled.");
                    return {};
                }
                const QString path = QDir::cleanPath(QDir::fromNativeSeparators(iterator.next()));
                const QFileInfo info = iterator.fileInfo();
                if (++inspected > 100000)
                {
                    error = QCoreApplication::translate("RepairSources", "The selected source directories contain more than 100000 entries. Choose smaller directories.");
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
            error = QCoreApplication::translate("RepairSources", "An explicit source mapping is invalid.");
            return {};
        }
        const QFileInfo source(path);
        if (!source.isFile() || source.isSymbolicLink() || source.isJunction())
        {
            error = QCoreApplication::translate("RepairSources", "An explicit source must be an ordinary existing file: %1").arg(path);
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
