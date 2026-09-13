/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "repairfileguard.h"

#include <limits>

#ifdef Q_OS_WIN
#include <windows.h>
#include <winternl.h>
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
        error = modifying ? QStringLiteral("Repair cancelled. Earlier in-place changes may already have been applied.")
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

    HANDLE createOwnedObject(const HANDLE parent, const QString &component, const QString &path
        , const ACCESS_MASK access, const ULONG shareAccess, const ULONG options, QString &error)
    {
        static const auto createFile = reinterpret_cast<decltype(&NtCreateFile)>(
            GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "NtCreateFile"));
        static const auto statusToError = reinterpret_cast<decltype(&RtlNtStatusToDosError)>(
            GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "RtlNtStatusToDosError"));
        if (!createFile || !statusToError)
        {
            error = QStringLiteral("The handle-relative creation API is unavailable.");
            return INVALID_HANDLE_VALUE;
        }
        const qsizetype bytes = component.size() * qsizetype(sizeof(wchar_t));
        if (bytes > std::numeric_limits<USHORT>::max())
        {
            error = QStringLiteral("The target path component is too long: %1").arg(path);
            return INVALID_HANDLE_VALUE;
        }
        UNICODE_STRING name {};
        name.Length = static_cast<USHORT>(bytes);
        name.MaximumLength = name.Length;
        name.Buffer = const_cast<PWSTR>(reinterpret_cast<LPCWSTR>(component.utf16()));
        OBJECT_ATTRIBUTES attributes {};
        InitializeObjectAttributes(&attributes, &name, OBJ_CASE_INSENSITIVE, parent, nullptr);
        IO_STATUS_BLOCK result {};
        HANDLE object = INVALID_HANDLE_VALUE;
        const NTSTATUS status = createFile(&object, access, &attributes, &result, nullptr
            , FILE_ATTRIBUTE_NORMAL, shareAccess, FILE_CREATE, options, nullptr, 0);
        if (!NT_SUCCESS(status))
        {
            error = QStringLiteral("Cannot atomically create the missing repair target %1 "
                "(Windows error %2, NT status 0x%3).")
                .arg(path).arg(statusToError(status)).arg(quint32(status), 8, 16, QLatin1Char('0'));
            return INVALID_HANDLE_VALUE;
        }
        if ((object == INVALID_HANDLE_VALUE) || (result.Information != FILE_CREATED))
        {
            if (object != INVALID_HANDLE_VALUE)
                CloseHandle(object);
            error = QStringLiteral("The handle-relative create did not exclusively create repair target %1.").arg(path);
            return INVALID_HANDLE_VALUE;
        }
        return object;
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

QList<std::shared_ptr<BitTorrent::RepairFileGuard>> BitTorrent::RepairFileGuard::openSources(const lt::file_storage &targetFiles
    , const QMap<int, QString> &sources, QString &error, const std::atomic_bool *cancelled)
{
    error.clear();
    QMap<QString, lt::file_storage> sourceVolumes;
    QSet<QString> sourcePaths;
    for (auto it = sources.cbegin(); it != sources.cend(); ++it)
    {
        const QString path = QDir::fromNativeSeparators(it.value());
        if ((it.key() < 0) || (it.key() >= targetFiles.num_files()) || (path.size() < 4)
            || (path.mid(1, 2) != u":/") || (QDir::cleanPath(path) != path))
        {
            error = QStringLiteral("A source mapping is not a normalized local file path.");
            return {};
        }
        if (!sourcePaths.contains(path.toCaseFolded()))
        {
            sourceVolumes[path.left(3)].add_file(
                path.mid(3).toStdString(), targetFiles.file_size(lt::file_index_t(it.key())));
            sourcePaths.insert(path.toCaseFolded());
        }
    }

    QList<std::shared_ptr<RepairFileGuard>> guards;
    for (auto it = sourceVolumes.cbegin(); it != sourceVolumes.cend(); ++it)
    {
        auto guard = open(it.value(), it.key(), false, error, cancelled);
        if (!guard)
            return {};
        if (guard->existingFiles().size() != it.value().num_files())
        {
            error = QStringLiteral("A mapped source changed before it could be owned. Analyze the data again.");
            return {};
        }
        guards.append(std::move(guard));
    }
    return guards;
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
    guard->m_root = root;

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
    QMap<QString, QByteArray> directoryIdentities;
    QSet<QByteArray> directoryIds;
    const auto lockDirectories = [&](const QString &path) -> DirectoryState
    {
        QString current = root.left(3);
        const QStringList components = path.mid(3).split(u'/', Qt::SkipEmptyParts);
        bool missing = false;
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
            if (missing || missingDirectories.contains(key))
            {
                missing = true;
                missingDirectories.insert(key);
                directoryIdentities.insert(key, QByteArray {});
                continue;
            }
            if ((guard->m_directoryHandles.size() + guard->m_files.size()) >= MaximumHandles)
            {
                error = QStringLiteral("Repair exceeds the limit of %1 open file and directory handles.").arg(MaximumHandles);
                return DirectoryState::Invalid;
            }

            const QString native = windowsPath(current);
            // Attribute-only handles do not enforce share exclusions. Directory
            // read access prevents rename/reparse conversion without blocking child I/O.
            HANDLE handle = CreateFileW(reinterpret_cast<LPCWSTR>(native.utf16())
                , FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES
                , FILE_SHARE_READ | (renameChildren ? FILE_SHARE_WRITE : 0), nullptr, OPEN_EXISTING
                , FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
            if (handle == INVALID_HANDLE_VALUE)
            {
                const DWORD code = GetLastError();
                if ((code == ERROR_FILE_NOT_FOUND) || (code == ERROR_PATH_NOT_FOUND))
                {
                    missing = true;
                    missingDirectories.insert(key);
                    directoryIdentities.insert(key, QByteArray {});
                    continue;
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
            QByteArray directoryIdentity;
            QDataStream(&directoryIdentity, QIODevice::WriteOnly) << quint32(info.dwVolumeSerialNumber)
                << quint32(info.nFileIndexHigh) << quint32(info.nFileIndexLow);
            if (directoryIds.contains(directoryIdentity))
            {
                error = QStringLiteral("Multiple target directory paths refer to the same directory: %1").arg(current);
                return DirectoryState::Invalid;
            }
            directoryIds.insert(directoryIdentity);
            directoryIdentities.insert(key, directoryIdentity);
            guard->m_directoryHandles.insert(key, handle);
            closeHandle.dismiss();
        }
        return missing ? DirectoryState::Missing : DirectoryState::Locked;
    };

    const DirectoryState rootState = lockDirectories(root);
    if (rootState == DirectoryState::Invalid)
        return {};
    if (rootState == DirectoryState::Missing)
    {
        error = QStringLiteral("The repair data directory must already exist.");
        return {};
    }

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
            if (files.file_size(index) == 0)
                guard->m_missingEmptyFiles.push_back({int(index), path});
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
                if (files.file_size(index) == 0)
                    guard->m_missingEmptyFiles.push_back({int(index), path});
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
    identity << directoryIdentities;
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

bool BitTorrent::RepairFileGuard::createMissingEmpty(QString &error, const std::atomic_bool *cancelled)
{
    error.clear();
    if (!m_writable)
    {
        error = QStringLiteral("Read-only analysis cannot create files.");
        return false;
    }
#ifdef Q_OS_WIN
    for (const MissingEmptyFile &file : m_missingEmptyFiles)
    {
        if (checkCancellation(error, cancelled, true))
            return false;
        QString current = m_root;
        const QString relativeParent = QDir(m_root).relativeFilePath(QFileInfo(file.path).path());
        const QStringList components = (relativeParent == u".")
            ? QStringList {} : relativeParent.split(u'/', Qt::SkipEmptyParts);
        for (const QString &component : components)
        {
            const HANDLE parent = static_cast<HANDLE>(m_directoryHandles.value(current.toCaseFolded()));
            if (!parent)
            {
                error = QStringLiteral("Repair lost ownership of target directory %1.").arg(current);
                return false;
            }
            current = QDir(current).filePath(component);
            const QString key = current.toCaseFolded();
            if (m_directoryHandles.contains(key))
                continue;
            if ((m_directoryHandles.size() + m_files.size()) >= MaximumHandles)
            {
                error = QStringLiteral("Repair exceeds the limit of %1 open file and directory handles. Earlier empty directories may already have been created.")
                    .arg(MaximumHandles);
                return false;
            }
            const HANDLE directory = createOwnedObject(parent, component, current
                , FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES, FILE_SHARE_READ
                , FILE_DIRECTORY_FILE, error);
            if (directory == INVALID_HANDLE_VALUE)
            {
                error += QStringLiteral(" Earlier target directories may already have been created.");
                return false;
            }
            auto closeDirectory = qScopeGuard([directory] { CloseHandle(directory); });
            BY_HANDLE_FILE_INFORMATION directoryInfo {};
            if (!GetFileInformationByHandle(directory, &directoryInfo)
                || !(directoryInfo.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY)
                || (directoryInfo.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT))
            {
                error = QStringLiteral("The newly created target directory is not an exclusive regular directory: %1").arg(current);
                return false;
            }
            m_directoryHandles.insert(key, directory);
            closeDirectory.dismiss();
        }
        if ((m_directoryHandles.size() + m_files.size()) >= MaximumHandles)
        {
            error = QStringLiteral("Repair exceeds the limit of %1 open file and directory handles. "
                "Earlier empty directories or targets may already have been created.").arg(MaximumHandles);
            return false;
        }
        const HANDLE parent = static_cast<HANDLE>(m_directoryHandles.value(current.toCaseFolded()));
        if (!parent)
        {
            error = QStringLiteral("Repair lost ownership of target directory %1.").arg(current);
            return false;
        }
        const HANDLE handle = createOwnedObject(parent, QFileInfo(file.path).fileName(), file.path
            , GENERIC_READ | GENERIC_WRITE, 0, FILE_NON_DIRECTORY_FILE, error);
        if (handle == INVALID_HANDLE_VALUE)
        {
            error += QStringLiteral(" Earlier empty targets may already have been created.");
            return false;
        }
        auto closeHandle = qScopeGuard([handle] { CloseHandle(handle); });
        BY_HANDLE_FILE_INFORMATION info {};
        if (!GetFileInformationByHandle(handle, &info)
            || (info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY))
            || (info.nNumberOfLinks != 1) || (info.nFileSizeHigh != 0) || (info.nFileSizeLow != 0))
        {
            error = QStringLiteral("The newly created empty target is not an exclusive regular file: %1").arg(file.path);
            return false;
        }
        m_files.push_back({handle, file.nativeIndex, file.path, 0, 0});
        closeHandle.dismiss();
    }
    m_missingEmptyFiles.clear();
    return true;
#else
    Q_UNUSED(cancelled)
    error = QStringLiteral("Managed repair currently requires Windows file ownership checks.");
    return false;
#endif
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
            const DWORD code = GetLastError();
            error = QStringLiteral("In-place repair stopped at %1 (Windows error %2). "
                "Earlier in-place changes may already have been applied.")
                .arg(file.path).arg(code);
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
