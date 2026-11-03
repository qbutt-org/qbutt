/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "repairfileguard.h"

#include <cerrno>
#include <csignal>

#include <fcntl.h>
#include <linux/magic.h>
#include <sys/stat.h>
#include <sys/vfs.h>
#include <unistd.h>

#include <libtorrent/file_storage.hpp>

#include <QCoreApplication>
#include <QDataStream>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QScopeGuard>

namespace
{
    constexpr qsizetype MaximumDescriptors = 32768;

    bool cancelled(const std::atomic_bool *flag, QString &error)
    {
        if (!flag || !flag->load(std::memory_order_relaxed))
            return false;
        error = QCoreApplication::translate("RepairFileGuard", "Repair cancelled.");
        return true;
    }

    QByteArray objectIdentity(const struct stat &attributes)
    {
        QByteArray result;
        QDataStream stream(&result, QIODevice::WriteOnly);
        stream << quint64(attributes.st_dev) << quint64(attributes.st_ino)
            << quint32(attributes.st_mode & S_IFMT);
        if (S_ISREG(attributes.st_mode))
        {
            stream << qint64(attributes.st_size) << quint64(attributes.st_nlink)
                << qint64(attributes.st_mtim.tv_sec) << qint64(attributes.st_mtim.tv_nsec)
                << qint64(attributes.st_ctim.tv_sec) << qint64(attributes.st_ctim.tv_nsec);
        }
        return result;
    }

    bool localFilesystem(const int descriptor, const QString &path, QString &error)
    {
        struct statfs filesystem {};
        if (fstatfs(descriptor, &filesystem) == 0)
        {
            switch (filesystem.f_type)
            {
            case EXT4_SUPER_MAGIC:
            case XFS_SUPER_MAGIC:
            case BTRFS_SUPER_MAGIC:
            case TMPFS_MAGIC:
                return true;
            }
        }
        error = QCoreApplication::translate("RepairFileGuard", "Read-only repair analysis requires a supported local Linux filesystem: %1.").arg(path);
        return false;
    }

    QString accessError(const QString &path, const int code)
    {
        return QCoreApplication::translate("RepairFileGuard", "Cannot safely read %1 (Linux error %2).").arg(path).arg(code);
    }

    bool safeComponent(const QString &component)
    {
        return !component.isEmpty() && (component != u".") && (component != u"..")
            && !component.contains(QChar::Null)
            && (QFile::decodeName(QFile::encodeName(component)) == component);
    }
}

std::shared_ptr<BitTorrent::RepairFileGuard> BitTorrent::RepairFileGuard::openLinux(
    const lt::file_storage &files, const QString &savePath, const bool writable, QString &error
    , const std::atomic_bool *cancelFlag, const bool renameChildren)
{
    error.clear();
    if (cancelled(cancelFlag, error))
        return {};
    if (writable || renameChildren)
    {
        // Leases can be forcibly broken and dirfds do not prevent rename.
        // These primitives cannot provide the mutation ownership contract.
        error = QCoreApplication::translate("RepairFileGuard", "Managed repair cannot obtain exclusive file and directory ownership on Linux. Read-only analysis is available.");
        return {};
    }
    if (!savePath.startsWith(u'/') || (QDir::cleanPath(savePath) != savePath) || savePath.contains(QChar::Null))
    {
        error = QCoreApplication::translate("RepairFileGuard", "Repair requires a clean absolute Linux path.");
        return {};
    }

    auto guard = std::shared_ptr<RepairFileGuard>(new RepairFileGuard);
    guard->m_root = savePath;
    QSet<QString> targets;
    QSet<QPair<quint64, quint64>> fileIds;
    for (const lt::file_index_t index : files.file_range())
    {
        if (files.pad_file_at(index))
            continue;
        const QString relative = QString::fromStdString(files.file_path(index));
        if ((relative.toStdString() != files.file_path(index)) || relative.startsWith(u'/')
            || (files.file_flags(index) & lt::file_storage::flag_symlink)
            || targets.contains(relative))
        {
            error = QCoreApplication::translate("RepairFileGuard", "Unsafe, linked or colliding target mapping: %1").arg(relative);
            return {};
        }
        for (const QString &component : relative.split(u'/'))
        {
            if (!safeComponent(component))
            {
                error = QCoreApplication::translate("RepairFileGuard", "Unsafe target path component: %1").arg(relative);
                return {};
            }
        }
        targets.insert(relative);
    }
    for (const QString &relative : targets)
    {
        for (QString parent = relative.section(u'/', 0, -2); !parent.isEmpty(); parent = parent.section(u'/', 0, -2))
        {
            if (targets.contains(parent))
            {
                error = QCoreApplication::translate("RepairFileGuard", "A mapped file is also used as a directory: %1").arg(parent);
                return {};
            }
        }
    }

    QSet<QByteArray> directoryIds;
    const auto openDirectories = [&](const QString &path) -> int
    {
        QString current = QStringLiteral("/");
        int parent = AT_FDCWD;
        const QStringList components = path.mid(1).split(u'/', Qt::SkipEmptyParts);
        for (qsizetype i = -1; i < components.size(); ++i)
        {
            if (cancelled(cancelFlag, error))
                return -1;
            const QString component = (i < 0) ? QStringLiteral("/") : components.at(i);
            if (!safeComponent(component))
            {
                error = QCoreApplication::translate("RepairFileGuard", "Unsafe directory component: %1").arg(path);
                return -1;
            }
            if (i >= 0)
                current = QDir(current).filePath(component);
            if (guard->m_directoryDescriptors.contains(current))
            {
                parent = guard->m_directoryDescriptors.value(current);
                continue;
            }
            if (guard->m_pathIdentities.contains(current))
                return -1;
            if ((guard->m_directoryDescriptors.size() + qsizetype(guard->m_files.size())) >= MaximumDescriptors)
            {
                error = QCoreApplication::translate("RepairFileGuard", "Repair exceeds the limit of %1 open file and directory descriptors.").arg(MaximumDescriptors);
                return -1;
            }
            const QByteArray name = QFile::encodeName(component);
            const int descriptor = openat(parent, name.constData(), O_PATH | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
            if (descriptor < 0)
            {
                if (errno == ENOENT)
                    guard->m_pathIdentities.insert(current, {});
                else
                    error = accessError(current, errno);
                return -1;
            }
            auto closeDirectory = qScopeGuard([descriptor] { close(descriptor); });
            struct stat attributes {};
            if ((fstat(descriptor, &attributes) != 0) || !S_ISDIR(attributes.st_mode))
            {
                error = accessError(current, errno);
                return -1;
            }
            const QByteArray identity = objectIdentity(attributes);
            if (directoryIds.contains(identity))
            {
                error = QCoreApplication::translate("RepairFileGuard", "Multiple target directory paths refer to the same directory: %1").arg(current);
                return -1;
            }
            directoryIds.insert(identity);
            guard->m_pathIdentities.insert(current, identity);
            guard->m_directoryDescriptors.insert(current, descriptor);
            closeDirectory.dismiss();
            parent = descriptor;
        }
        return parent;
    };

    const int root = openDirectories(savePath);
    if (root < 0)
    {
        if (error.isEmpty())
            error = QCoreApplication::translate("RepairFileGuard", "The repair data directory must already exist.");
        return {};
    }
    if (!localFilesystem(root, savePath, error))
        return {};

    for (const lt::file_index_t index : files.file_range())
    {
        if (cancelled(cancelFlag, error))
            return {};
        if (files.pad_file_at(index))
            continue;
        const QString path = QDir(savePath).filePath(QString::fromStdString(files.file_path(index)));
        const int parent = openDirectories(QFileInfo(path).absolutePath());
        if (parent < 0)
        {
            if (!error.isEmpty())
                return {};
            continue;
        }
        if ((guard->m_directoryDescriptors.size() + qsizetype(guard->m_files.size())) >= MaximumDescriptors)
        {
            error = QCoreApplication::translate("RepairFileGuard", "Repair exceeds the limit of %1 open file and directory descriptors.").arg(MaximumDescriptors);
            return {};
        }
        const QByteArray name = QFile::encodeName(QFileInfo(path).fileName());
        const int descriptor = openat(parent, name.constData(), O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC);
        if (descriptor < 0)
        {
            if (errno != ENOENT)
            {
                error = accessError(path, errno);
                return {};
            }
            guard->m_pathIdentities.insert(path, {});
            continue;
        }
        auto closeFile = qScopeGuard([descriptor] { close(descriptor); });
        struct stat attributes {};
        if ((fstat(descriptor, &attributes) != 0) || !S_ISREG(attributes.st_mode) || (attributes.st_nlink != 1))
        {
            error = QCoreApplication::translate("RepairFileGuard", "Repair requires an ordinary file without hard links or symbolic links: %1").arg(path);
            return {};
        }
        if (!localFilesystem(descriptor, path, error))
            return {};
        const QPair<quint64, quint64> fileId {attributes.st_dev, attributes.st_ino};
        if (fileIds.contains(fileId))
        {
            error = QCoreApplication::translate("RepairFileGuard", "Multiple target paths refer to the same file: %1").arg(path);
            return {};
        }
        fileIds.insert(fileId);
        // Poll F_GETLEASE rather than installing process-global signal handlers.
        // SIGURG has an ignored default disposition during acquisition; clearing
        // the recipient then leaves break detection entirely descriptor-local.
        if ((fcntl(descriptor, F_SETSIG, SIGURG) != 0)
            || (fcntl(descriptor, F_SETLEASE, F_RDLCK) != 0)
            || (fcntl(descriptor, F_SETOWN, 0) != 0))
        {
            error = QCoreApplication::translate("RepairFileGuard", "Cannot obtain a stable read lease for %1 (Linux error %2). Close writers or use a local filesystem that supports leases.").arg(path).arg(errno);
            return {};
        }
        // Read attributes after lease acquisition, excluding a writer that
        // closed between openat and F_SETLEASE from the accepted snapshot.
        if (fstat(descriptor, &attributes) != 0)
        {
            error = accessError(path, errno);
            return {};
        }
        if (attributes.st_nlink != 1)
        {
            error = QCoreApplication::translate("RepairFileGuard", "Repair requires an ordinary file without hard links or symbolic links: %1").arg(path);
            return {};
        }
        const QByteArray snapshot = objectIdentity(attributes);
        guard->m_pathIdentities.insert(path, snapshot);
        guard->m_files.push_back({descriptor, int(index), path, files.file_size(index), attributes.st_size, false});
        closeFile.dismiss();
    }
    if (!guard->validateLinuxSnapshot(error))
        return {};
    QDataStream identity(&guard->m_identity, QIODevice::WriteOnly);
    identity << savePath << guard->m_pathIdentities;
    return guard;
}

bool BitTorrent::RepairFileGuard::validateLinuxSnapshot(QString &error) const
{
    for (const File &file : m_files)
    {
        struct stat attributes {};
        if ((fcntl(file.descriptor, F_GETLEASE) != F_RDLCK) || (fstat(file.descriptor, &attributes) != 0)
            || (objectIdentity(attributes) != m_pathIdentities.value(file.path)))
        {
            error = QCoreApplication::translate("RepairFileGuard", "The file or its read lease changed during analysis: %1. Analyze the data again.").arg(file.path);
            return false;
        }
    }
    for (auto it = m_pathIdentities.cbegin(); it != m_pathIdentities.cend(); ++it)
    {
        struct stat attributes {};
        const QString path = it.key();
        const int result = (path == u"/") ? fstat(m_directoryDescriptors.value(path), &attributes)
            : fstatat(m_directoryDescriptors.value(QFileInfo(path).absolutePath(), -1)
                , QFile::encodeName(QFileInfo(path).fileName()).constData(), &attributes, AT_SYMLINK_NOFOLLOW);
        if ((result == 0) ? (objectIdentity(attributes) == it.value()) : ((errno == ENOENT) && it.value().isEmpty()))
            continue;
        error = QCoreApplication::translate("RepairFileGuard", "A target path changed during analysis: %1. Analyze the data again.").arg(path);
        return false;
    }
    return true;
}
