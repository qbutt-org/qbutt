/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "repairfileguard.h"

#ifdef Q_OS_WIN
#include <windows.h>
#endif

#include <libtorrent/file_storage.hpp>

#include <QDataStream>
#include <QDir>
#include <QFileInfo>
#include <QScopeGuard>
#include <QSet>

namespace
{
    bool checkCancellation(QString &error, const std::atomic_bool *cancelled, const bool modifying = false)
    {
        if (!cancelled || !cancelled->load(std::memory_order_relaxed))
            return false;
        error = modifying ? QStringLiteral("Repair cancelled. Earlier truncations may already be applied.")
            : QStringLiteral("Repair cancelled.");
        return true;
    }

#ifdef Q_OS_WIN
    constexpr std::size_t MaximumHandles = 32768;

    enum class DirectoryState
    {
        Locked,
        Missing,
        Invalid
    };

    bool isSafeComponent(const QString &component)
    {
        if (component.isEmpty() || component.endsWith(u'.') || component.endsWith(u' '))
            return false;
        for (const QChar character : component)
        {
            if (!character.isPrint() || QStringView {u"<>:\"/\\|?*"}.contains(character))
                return false;
        }

        const QString stem = component.section(u'.', 0, 0).toUpper();
        return (stem != u"CON") && (stem != u"PRN") && (stem != u"AUX") && (stem != u"NUL")
            && (stem != u"CONIN$") && (stem != u"CONOUT$")
            && !((stem.size() == 4) && (stem.startsWith(u"COM") || stem.startsWith(u"LPT"))
                && (((stem.back() >= u'1') && (stem.back() <= u'9'))
                    || QStringView {u"\u00b9\u00b2\u00b3"}.contains(stem.back())));
    }

    QString windowsPath(const QString &path)
    {
        return u"\\\\?\\" + QDir::toNativeSeparators(path);
    }

    QString fileError(const QString &path, const DWORD code)
    {
        return QStringLiteral("Cannot exclusively access %1 (Windows error %2). Close programs using this data.")
            .arg(path).arg(code);
    }
#endif
}

QString BitTorrent::repairPathIdentity(const QString &path)
{
    QString current = QDir::cleanPath(QDir::fromNativeSeparators(path));
    if (!QDir::isAbsolutePath(current))
        return {};
    QString suffix;
    while (!current.isEmpty())
    {
        const QFileInfo info(current);
#ifdef Q_OS_WIN
        QString native = QDir::toNativeSeparators(current);
        if (!native.startsWith(u"\\\\?\\"))
        {
            if (native.startsWith(u"\\\\"))
                native = u"\\\\?\\UNC\\" + native.sliced(2);
            else
                native = u"\\\\?\\" + native;
        }
        const HANDLE handle = CreateFileW(reinterpret_cast<LPCWSTR>(native.utf16()), FILE_READ_ATTRIBUTES
            , FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING
            , FILE_FLAG_BACKUP_SEMANTICS, nullptr);
        QString canonical;
        if (handle != INVALID_HANDLE_VALUE)
        {
            const auto closeHandle = qScopeGuard([handle] { CloseHandle(handle); });
            BY_HANDLE_FILE_INFORMATION attributes {};
            if (!GetFileInformationByHandle(handle, &attributes)
                || (!suffix.isEmpty() && !(attributes.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)))
                return {};
            const DWORD required = GetFinalPathNameByHandleW(handle, nullptr, 0, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
            if (required == 0)
                return {};
            canonical.resize(required);
            const DWORD length = GetFinalPathNameByHandleW(handle, reinterpret_cast<LPWSTR>(canonical.data())
                , required, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
            if ((length == 0) || (length >= required))
                return {};
            canonical.resize(length);
            if (canonical.startsWith(u"\\\\?\\UNC\\"))
                canonical = u"//" + canonical.sliced(8);
            else if (canonical.startsWith(u"\\\\?\\"))
                canonical = canonical.sliced(4);
            canonical = QDir::fromNativeSeparators(canonical);
        }
        else
        {
            const DWORD code = GetLastError();
            if ((code != ERROR_FILE_NOT_FOUND) && (code != ERROR_PATH_NOT_FOUND))
                return {};
            // A dangling reparse point is an unresolved existing object, not a
            // missing lexical suffix that can safely be appended to its parent.
            if (GetFileAttributesW(reinterpret_cast<LPCWSTR>(native.utf16())) != INVALID_FILE_ATTRIBUTES)
                return {};
        }
#else
        const QString canonical = info.canonicalFilePath();
        if (canonical.isEmpty() && info.exists())
            return {};
#endif
        if (!canonical.isEmpty())
        {
            QString identity = suffix.isEmpty() ? canonical : QDir(canonical).filePath(suffix);
            if (identity.endsWith(u'/'))
                identity.chop(1);
            return identity.toCaseFolded();
        }
        suffix = suffix.isEmpty() ? info.fileName() : info.fileName() + u'/' + suffix;
        const QString parent = info.path();
        if (parent == current)
            break;
        current = parent;
    }
    return {};
}

std::shared_ptr<BitTorrent::RepairFileGuard> BitTorrent::RepairFileGuard::open(
    const lt::file_storage &files, const QString &savePath, const bool writable, QString &error
    , const std::atomic_bool *cancelled, const bool renameChildren)
{
#ifndef Q_OS_WIN
    Q_UNUSED(files)
    Q_UNUSED(savePath)
    Q_UNUSED(writable)
    Q_UNUSED(cancelled)
    Q_UNUSED(renameChildren)
    error = QStringLiteral("Managed repair currently requires Windows file ownership checks.");
    return {};
#else
    error.clear();
    if (checkCancellation(error, cancelled))
        return {};
    auto guard = std::shared_ptr<RepairFileGuard>(new RepairFileGuard);
    guard->m_writable = writable;
    const QString root = QDir::fromNativeSeparators(savePath);
    if ((root.size() < 3) || !root.at(0).isLetter() || (root.mid(1, 2) != u":/")
        || (QDir::cleanPath(root) != root))
    {
        error = QStringLiteral("Repair requires an absolute, normalized local drive path.");
        return {};
    }

    if (GetDriveTypeW(reinterpret_cast<LPCWSTR>(root.left(3).utf16())) != DRIVE_FIXED)
    {
        error = QStringLiteral("Repair currently requires a local fixed drive.");
        return {};
    }

    QSet<QString> targets;
    QStringList relativePaths;
    for (const lt::file_index_t index : files.file_range())
    {
        if (checkCancellation(error, cancelled))
            return {};
        if (files.pad_file_at(index))
            continue;

        const QString relative = QDir::fromNativeSeparators(QString::fromStdString(files.file_path(index)));
        const QString key = relative.toCaseFolded();
        if (QDir::isAbsolutePath(relative) || (files.file_flags(index) & lt::file_storage::flag_symlink)
            || targets.contains(key))
        {
            error = QStringLiteral("Unsafe, linked or colliding target mapping: %1").arg(relative);
            return {};
        }
        for (const QString &component : relative.split(u'/'))
        {
            if (!isSafeComponent(component))
            {
                error = QStringLiteral("Unsafe target path component: %1").arg(relative);
                return {};
            }
        }
        targets.insert(key);
        relativePaths.append(relative);
    }

    for (const QString &relative : relativePaths)
    {
        if (checkCancellation(error, cancelled))
            return {};
        QString parent = relative.section(u'/', 0, -2);
        while (!parent.isEmpty())
        {
            if (targets.contains(parent.toCaseFolded()))
            {
                error = QStringLiteral("A mapped file is also used as a directory: %1").arg(parent);
                return {};
            }
            parent = parent.section(u'/', 0, -2);
        }
    }

    QSet<QString> missingDirectories;
    const auto lockDirectories = [&](const QString &path) -> DirectoryState
    {
        QString current = root.left(3);
        const QStringList components = path.mid(3).split(u'/', Qt::SkipEmptyParts);
        for (qsizetype i = -1; i < components.size(); ++i)
        {
            if (checkCancellation(error, cancelled))
                return DirectoryState::Invalid;
            if (i >= 0)
            {
                if (!isSafeComponent(components.at(i)))
                {
                    error = QStringLiteral("Unsafe directory component: %1").arg(path);
                    return DirectoryState::Invalid;
                }
                if (!current.endsWith(u'/'))
                    current += u'/';
                current += components.at(i);
            }
            const QString key = current.toCaseFolded();
            if (guard->m_directoryHandles.contains(key))
                continue;
            if (missingDirectories.contains(key))
                return DirectoryState::Missing;
            if ((guard->m_directoryHandles.size() + guard->m_files.size()) >= MaximumHandles)
            {
                error = QStringLiteral("Repair exceeds the limit of %1 open file and directory handles.").arg(MaximumHandles);
                return DirectoryState::Invalid;
            }

            const QString native = windowsPath(current);
            // Attribute-only handles do not enforce share exclusions. Directory
            // read access prevents rename/reparse conversion without blocking child I/O.
            HANDLE handle = CreateFileW(reinterpret_cast<LPCWSTR>(native.utf16()), FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES
                , FILE_SHARE_READ | (renameChildren ? FILE_SHARE_WRITE : 0), nullptr, OPEN_EXISTING
                , FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
            if (handle == INVALID_HANDLE_VALUE)
            {
                const DWORD code = GetLastError();
                if ((code == ERROR_FILE_NOT_FOUND) || (code == ERROR_PATH_NOT_FOUND))
                {
                    missingDirectories.insert(key);
                    return DirectoryState::Missing;
                }
                error = fileError(current, code);
                return DirectoryState::Invalid;
            }
            auto closeHandle = qScopeGuard([handle] { CloseHandle(handle); });
            BY_HANDLE_FILE_INFORMATION info {};
            if (!GetFileInformationByHandle(handle, &info)
                || !(info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
                || (info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT))
            {
                error = QStringLiteral("Repair refuses a reparse point or non-directory: %1").arg(current);
                return DirectoryState::Invalid;
            }
            guard->m_directoryHandles.insert(key, handle);
            closeHandle.dismiss();
        }
        return DirectoryState::Locked;
    };

    if (lockDirectories(root) == DirectoryState::Invalid)
        return {};

    QSet<QByteArray> fileIds;
    QDataStream identity(&guard->m_identity, QIODevice::WriteOnly);
    identity << root;
    for (const lt::file_index_t index : files.file_range())
    {
        if (checkCancellation(error, cancelled))
            return {};
        if (files.pad_file_at(index))
            continue;
        const QString relative = QDir::fromNativeSeparators(QString::fromStdString(files.file_path(index)));
        const QString path = QDir(root).filePath(relative);
        const DirectoryState directoryState = lockDirectories(path.section(u'/', 0, -2));
        if (directoryState == DirectoryState::Invalid)
            return {};
        identity << relative << qint64(files.file_size(index));
        if (directoryState == DirectoryState::Missing)
        {
            identity << false;
            continue;
        }
        if ((guard->m_directoryHandles.size() + guard->m_files.size()) >= MaximumHandles)
        {
            error = QStringLiteral("Repair exceeds the limit of %1 open file and directory handles.").arg(MaximumHandles);
            return {};
        }

        const QString native = windowsPath(path);
        HANDLE handle = CreateFileW(reinterpret_cast<LPCWSTR>(native.utf16())
            , writable ? (GENERIC_READ | GENERIC_WRITE) : GENERIC_READ
            , writable ? 0 : FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        if (handle == INVALID_HANDLE_VALUE)
        {
            const DWORD code = GetLastError();
            if ((code == ERROR_FILE_NOT_FOUND) || (code == ERROR_PATH_NOT_FOUND))
            {
                identity << false;
                continue;
            }
            error = fileError(path, code);
            return {};
        }

        auto closeHandle = qScopeGuard([handle] { CloseHandle(handle); });
        guard->m_files.push_back({handle, int(index), path, files.file_size(index), 0});
        closeHandle.dismiss();
        BY_HANDLE_FILE_INFORMATION info {};
        if (!GetFileInformationByHandle(handle, &info)
            || (info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY))
            || (info.nNumberOfLinks != 1))
        {
            error = QStringLiteral("Repair refuses a hardlink, reparse point or non-file: %1").arg(path);
            return {};
        }
        const qint64 size = (qint64(info.nFileSizeHigh) << 32) | info.nFileSizeLow;
        QByteArray fileId;
        QDataStream(&fileId, QIODevice::WriteOnly) << quint32(info.dwVolumeSerialNumber)
            << quint32(info.nFileIndexHigh) << quint32(info.nFileIndexLow);
        if (fileIds.contains(fileId))
        {
            error = QStringLiteral("Multiple target paths refer to the same file: %1").arg(path);
            return {};
        }
        fileIds.insert(fileId);
        guard->m_files.back().actualSize = size;
        identity << true << quint32(info.dwVolumeSerialNumber)
            << quint32(info.nFileIndexHigh) << quint32(info.nFileIndexLow) << size
            << quint32(info.ftLastWriteTime.dwHighDateTime) << quint32(info.ftLastWriteTime.dwLowDateTime);
    }
    return guard;
#endif
}

BitTorrent::RepairFileGuard::~RepairFileGuard()
{
    releaseFiles();
#ifdef Q_OS_WIN
    for (auto it = m_directoryHandles.cbegin(); it != m_directoryHandles.cend(); ++it)
        CloseHandle(it.value());
#endif
}

void BitTorrent::RepairFileGuard::releaseFiles()
{
#ifdef Q_OS_WIN
    for (const File &file : m_files)
        CloseHandle(file.handle);
#endif
    m_files.clear();
}

void *BitTorrent::RepairFileGuard::directoryHandle(const QString &path) const
{
    return m_directoryHandles.value(QDir::cleanPath(path).toCaseFolded());
}

QByteArray BitTorrent::RepairFileGuard::identity() const
{
    return m_identity;
}

QSet<int> BitTorrent::RepairFileGuard::existingFiles() const
{
    QSet<int> indexes;
    for (const File &file : m_files)
        indexes.insert(file.nativeIndex);
    return indexes;
}

bool BitTorrent::RepairFileGuard::truncateOversized(QString &error, const std::atomic_bool *cancelled)
{
    error.clear();
    if (!m_writable)
    {
        error = QStringLiteral("Read-only analysis cannot modify files.");
        return false;
    }
    if (checkCancellation(error, cancelled, true))
        return false;
#ifdef Q_OS_WIN
    for (const File &file : m_files)
    {
        if (checkCancellation(error, cancelled, true))
            return false;
        if (file.actualSize <= file.expectedSize)
            continue;
        LARGE_INTEGER end {};
        end.QuadPart = file.expectedSize;
        // libtorrent::truncate_files reopens paths with FILE_SHARE_WRITE. Keep
        // the exclusive preflight handle instead, so no writer or path swap can
        // intervene between validation and truncation.
        if (!SetFilePointerEx(file.handle, end, nullptr, FILE_BEGIN) || !SetEndOfFile(file.handle)
            || !FlushFileBuffers(file.handle))
        {
            error = QStringLiteral("In-place repair stopped at %1 (Windows error %2). Earlier truncations may already be applied.")
                .arg(file.path).arg(GetLastError());
            return false;
        }
        LARGE_INTEGER size {};
        if (!GetFileSizeEx(file.handle, &size) || (size.QuadPart != file.expectedSize))
        {
            error = QStringLiteral("Exact size verification failed after truncating %1.").arg(file.path);
            return false;
        }
    }
    return true;
#else
    error = QStringLiteral("Managed repair currently requires Windows file ownership checks.");
    return false;
#endif
}
