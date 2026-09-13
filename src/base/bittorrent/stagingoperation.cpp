/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "stagingoperation.h"

#include <algorithm>
#include <array>
#include <limits>
#include <utility>
#include <vector>

#ifdef Q_OS_WIN
#include <fcntl.h>
#include <io.h>
#include <windows.h>
#include <winternl.h>
#endif

#include <libtorrent/torrent_info.hpp>

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QStorageInfo>
#include <QUuid>

#include "base/path.h"
#include "base/profile.h"
#include "repairfileguard.h"

using namespace BitTorrent;

namespace
{
    constexpr qint64 JournalLimit = 16 * 1024 * 1024;

#if defined(QBUTT_STAGING_FAULTS) && defined(Q_OS_WIN)
    void crashCheckpoint(const QString &checkpoint)
    {
        if (qEnvironmentVariable("QBUTT_STAGING_FAULT") == checkpoint)
            TerminateProcess(GetCurrentProcess(), 197);
    }
#endif

    bool cancelled(const std::atomic_bool *flag, QString &error)
    {
        if (!flag || !flag->load(std::memory_order_relaxed))
            return false;
        error = QStringLiteral("Staging stopped. Original data is preserved; the journal can be recovered.");
        return true;
    }

#ifdef Q_OS_WIN
    QString nativePath(const QString &path)
    {
        return u"\\\\?\\" + QDir::toNativeSeparators(path);
    }

    struct FileHandle
    {
        HANDLE value = INVALID_HANDLE_VALUE;
        int descriptor = -1;

        FileHandle() = default;
        FileHandle(const FileHandle &) = delete;
        FileHandle &operator=(const FileHandle &) = delete;
        FileHandle(FileHandle &&other) noexcept
            : value {std::exchange(other.value, INVALID_HANDLE_VALUE)}
            , descriptor {std::exchange(other.descriptor, -1)}
        {
        }
        ~FileHandle()
        {
            if (descriptor >= 0)
                _close(descriptor);
            else if (value != INVALID_HANDLE_VALUE)
                CloseHandle(value);
        }
    };

    QJsonObject identity(HANDLE handle, QString &error)
    {
        BY_HANDLE_FILE_INFORMATION info {};
        if (!GetFileInformationByHandle(handle, &info)
            || (info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY))
            || (info.nNumberOfLinks != 1))
        {
            error = QStringLiteral("Staging refuses a hardlink, reparse point or non-file.");
            return {};
        }
        return {{QStringLiteral("volume"), QString::number(info.dwVolumeSerialNumber)}
            , {QStringLiteral("id"), QString::number((quint64(info.nFileIndexHigh) << 32) | info.nFileIndexLow)}
            , {QStringLiteral("size"), QString::number((quint64(info.nFileSizeHigh) << 32) | info.nFileSizeLow)}
            , {QStringLiteral("mtime"), QString::number((quint64(info.ftLastWriteTime.dwHighDateTime) << 32)
                | info.ftLastWriteTime.dwLowDateTime)}};
    }

    bool openFile(const QString &path, FileHandle &file, QJsonObject &attributes, QString &error, const bool rename = false)
    {
        const QString native = nativePath(path);
        file.value = CreateFileW(reinterpret_cast<LPCWSTR>(native.utf16()), GENERIC_READ | (rename ? DELETE : 0)
            , FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        if (file.value == INVALID_HANDLE_VALUE)
        {
            const DWORD code = GetLastError();
            if ((code == ERROR_FILE_NOT_FOUND) || (code == ERROR_PATH_NOT_FOUND))
            {
                attributes = {};
                return true;
            }
            error = QStringLiteral("Cannot own %1 (Windows error %2). Close other writers and try recovery again.")
                .arg(path).arg(code);
            return false;
        }
        attributes = identity(file.value, error);
        return error.isEmpty();
    }

    bool renameFile(HANDLE handle, HANDLE parent, const QString &path, QString &error)
    {
        const QString native = QFileInfo(path).fileName();
        const DWORD bytes = static_cast<DWORD>(native.size() * sizeof(wchar_t));
        std::vector<char> storage(sizeof(FILE_RENAME_INFO) + bytes);
        auto *rename = reinterpret_cast<FILE_RENAME_INFO *>(storage.data());
        rename->ReplaceIfExists = FALSE;
        rename->RootDirectory = parent;
        rename->FileNameLength = bytes;
        memcpy(rename->FileName, native.utf16(), bytes);
        using SetInformation = NTSTATUS (NTAPI *)(HANDLE, PIO_STATUS_BLOCK, PVOID, ULONG, FILE_INFORMATION_CLASS);
        const auto setInformation = reinterpret_cast<SetInformation>(GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "NtSetInformationFile"));
        if (!parent || !setInformation)
        {
            error = QStringLiteral("The owned parent directory or handle-relative rename API is unavailable.");
            return false;
        }
        IO_STATUS_BLOCK result {};
        const NTSTATUS status = setInformation(handle, &result, rename, static_cast<ULONG>(storage.size())
            , static_cast<FILE_INFORMATION_CLASS>(10)); // FileRenameInformation
        if (status < 0)
        {
            error = QStringLiteral("Recovery stopped before replacing %1 (NT status 0x%2). No unowned file was overwritten.")
                .arg(path).arg(quint32(status), 8, 16, QLatin1Char('0'));
            return false;
        }
        return true;
    }
#endif

    QString relativePath(const lt::file_storage &files, const lt::file_index_t index)
    {
        return QDir::fromNativeSeparators(QString::fromStdString(files.file_path(index)));
    }

    QString transactionRoot(const QJsonObject &journal)
    {
        return QDir(journal.value(QStringLiteral("destination")).toString())
            .filePath(u".qbutt-staging-" + journal.value(QStringLiteral("operation")).toString());
    }

    bool validIdentity(const QJsonValue &value)
    {
        if (!value.isObject())
            return false;
        const QJsonObject attributes = value.toObject();
        if (attributes.isEmpty())
            return true; // An explicitly absent original file.
        if (attributes.size() != 4)
            return false;
        for (const QString &key : {QStringLiteral("volume"), QStringLiteral("id")
            , QStringLiteral("size"), QStringLiteral("mtime")})
        {
            const QJsonValue field = attributes.value(key);
            bool parsed = false;
            const quint64 number = field.toString().toULongLong(&parsed);
            if (!field.isString() || !parsed || (QString::number(number) != field.toString())
                || ((key == u"volume") && (number > std::numeric_limits<quint32>::max()))
                || ((key == u"size") && (number > quint64(std::numeric_limits<qint64>::max()))))
                return false;
        }
        return true;
    }
}

nonstd::expected<StagingOperation::StorageRequirement, QString> StagingOperation::storageRequirement(
    const lt::file_storage &files, const QString &destination, const QMap<int, QString> &sources)
{
    const QStorageInfo storage(destination);
    if (!storage.isValid() || !storage.isReady())
        return nonstd::make_unexpected(QStringLiteral("Cannot inspect storage for the target destination."));
    const qint64 allocationUnit = std::max<qint64>(4096, storage.blockSize());
    qint64 required = 0;
    qint64 journalBytes = allocationUnit;
    const auto accountJournalString = [&journalBytes](const QString &value, const qint64 fixedBytes = 0)
    {
        const qint64 maximum = std::numeric_limits<qint64>::max();
        if (value.size() > ((maximum - fixedBytes) / 4))
            return false;
        const qint64 bytes = fixedBytes + (static_cast<qint64>(value.size()) * 4);
        if (journalBytes > (maximum - bytes))
            return false;
        journalBytes += bytes;
        return true;
    };
    if (!accountJournalString(destination, 4096))
        return nonstd::make_unexpected(QStringLiteral("The target exceeds supported storage accounting."));
    for (const lt::file_index_t index : files.file_range())
    {
        if (files.pad_file_at(index))
            continue;
        const qint64 size = files.file_size(index);
        const QString relative = relativePath(files, index);
        const qint64 overhead = (relative.count(u'/') + 3) * allocationUnit;
        if ((size < 0) || (required > (std::numeric_limits<qint64>::max() - overhead))
            || (size > (std::numeric_limits<qint64>::max() - required - overhead)))
        {
            return nonstd::make_unexpected(QStringLiteral("The target size is invalid or exceeds supported storage accounting."));
        }
        const qint64 allocation = ((size + allocationUnit - 1) / allocationUnit) * allocationUnit;
        required += allocation + ((relative.count(u'/') + 2) * allocationUnit);
        if (!accountJournalString(relative, 512) || !accountJournalString(sources.value(int(index))))
            return nonstd::make_unexpected(QStringLiteral("The target exceeds supported storage accounting."));
    }
    if (journalBytes > (std::numeric_limits<qint64>::max() - allocationUnit + 1))
        return nonstd::make_unexpected(QStringLiteral("The target exceeds supported storage accounting."));
    const qint64 roundedJournal = ((journalBytes + allocationUnit - 1) / allocationUnit) * allocationUnit;
    if (roundedJournal > (std::numeric_limits<qint64>::max() / 2))
        return nonstd::make_unexpected(QStringLiteral("The target exceeds supported storage accounting."));
    const qint64 journalAllocation = 2 * roundedJournal;
    if (required > (std::numeric_limits<qint64>::max() - journalAllocation))
        return nonstd::make_unexpected(QStringLiteral("The target exceeds supported storage accounting."));
    return StorageRequirement {required + journalAllocation, qint64(storage.bytesAvailable())};
}

QString StagingOperation::journalPath(const QString &torrentId)
{
    return (specialFolderLocation(SpecialFolder::Data) / Path(u"staging/" + torrentId + u".json")).toString();
}

QString StagingOperation::pendingDestination(const QString &torrentId)
{
    QFile file(journalPath(torrentId));
    if (!file.open(QIODevice::ReadOnly) || (file.size() > JournalLimit))
        return {};
    return QJsonDocument::fromJson(file.readAll()).object().value(QStringLiteral("destination")).toString();
}

std::unique_ptr<StagingOperation> StagingOperation::plan(const QString &journalPath, const QString &torrentId
    , const lt::torrent_info &target, const lt::file_storage &files, const QString &destination
    , const QMap<int, QString> &sources, const QSet<int> &selected, QString &error, const std::atomic_bool *cancelFlag)
{
    error.clear();
    if (selected.isEmpty() || std::any_of(selected.cbegin(), selected.cend(), [&files](const int index)
        { return (index < 0) || (index >= files.num_files()) || files.pad_file_at(lt::file_index_t(index)); }))
    {
        error = QStringLiteral("Select at least one valid target file for staging.");
        return {};
    }
    auto destinationGuard = RepairFileGuard::open(files, destination, false, error, cancelFlag);
    if (!destinationGuard)
        return {};
    if (QFileInfo::exists(journalPath))
    {
        error = QStringLiteral("Recover the existing staged operation before starting another one.");
        return {};
    }
    auto operation = std::unique_ptr<StagingOperation>(new StagingOperation);
    operation->m_journalPath = journalPath;
    operation->m_files = files;
    operation->m_journal = {{QStringLiteral("version"), 1}, {QStringLiteral("torrent"), torrentId}
        , {QStringLiteral("operation"), QUuid::createUuid().toString(QUuid::WithoutBraces)}
        , {QStringLiteral("destination"), destination}, {QStringLiteral("state"), QStringLiteral("planned")}};

    operation->m_sourceGuards = RepairFileGuard::openSources(files, sources, error, cancelFlag);
    if (!error.isEmpty())
        return {};

    QJsonArray entries;
    qint64 payloadBytes = 0;
    lt::file_storage sourceLayout = files;
    QSet<int> readable;
    for (const lt::file_index_t index : files.file_range())
    {
        if (files.pad_file_at(index))
            continue;
        if (cancelled(cancelFlag, error))
            return {};
        const QString relative = relativePath(files, index);
        const QString source = sources.value(int(index));
        const qint64 size = files.file_size(index);
        payloadBytes += size;
        QJsonObject original;
#ifdef Q_OS_WIN
        FileHandle handle;
        if (!openFile(QDir(destination).filePath(relative), handle, original, error))
            return {};
#endif
        entries.append(QJsonObject {{QStringLiteral("index"), int(index)}, {QStringLiteral("path"), relative}
            , {QStringLiteral("size"), QString::number(size)}, {QStringLiteral("source"), source}
            , {QStringLiteral("selected"), selected.contains(int(index))}
            , {QStringLiteral("original"), original}, {QStringLiteral("step"), QStringLiteral("untouched")}});
        if (!source.isEmpty())
        {
            sourceLayout.rename_file(index, source.toStdString());
            if (QFileInfo::exists(source))
                readable.insert(int(index));
        }
    }
    operation->m_analysis = analyzeRepairData(target, sourceLayout, destination, cancelFlag, &readable);
    if (!operation->m_analysis.error.isEmpty())
    {
        error = operation->m_analysis.error;
        return {};
    }
    operation->m_journal.insert(QStringLiteral("files"), entries);
    const auto storage = storageRequirement(files, destination, sources);
    if (!storage)
    {
        error = storage.error();
        return {};
    }
    operation->m_journal.insert(QStringLiteral("payload_bytes"), QString::number(payloadBytes));
    operation->m_journal.insert(QStringLiteral("required_bytes"), QString::number(storage->requiredBytes));
    operation->m_journal.insert(QStringLiteral("available_bytes"), QString::number(storage->availableBytes));
    // Planning is read-only, including the journal. Preparation creates it only
    // after explicit consent and before any operation-owned payload is written.
    return operation;
}

std::unique_ptr<StagingOperation> StagingOperation::load(const QString &journalPath, const QString &torrentId
    , const lt::file_storage &files, QString &error)
{
    error.clear();
    QFile input(journalPath);
    if (!input.open(QIODevice::ReadOnly) || (input.size() > JournalLimit))
    {
        error = QStringLiteral("Cannot read the bounded staging journal.");
        return {};
    }
    const QJsonObject journal = QJsonDocument::fromJson(input.readAll()).object();
    const QString id = journal.value(QStringLiteral("operation")).toString();
    const QString state = journal.value(QStringLiteral("state")).toString();
    const QStringList states {QStringLiteral("preparing"), QStringLiteral("downloading")
        , QStringLiteral("ready_to_commit"), QStringLiteral("committing"), QStringLiteral("committed")
        , QStringLiteral("rolling_back"), QStringLiteral("rolled_back")};
    if ((journal.value(QStringLiteral("version")) != QJsonValue(1))
        || (journal.value(QStringLiteral("torrent")).toString() != torrentId)
        || QUuid(id).isNull() || (QUuid(id).toString(QUuid::WithoutBraces) != id)
        || !states.contains(state))
    {
        error = QStringLiteral("The staging journal identity, version or state is invalid.");
        return {};
    }
    const QJsonArray entries = journal.value(QStringLiteral("files")).toArray();
    int expectedCount = 0;
    int selectedCount = 0;
    QSet<int> indexes;
    const bool requiresVerified = (state == u"ready_to_commit") || (state == u"committing") || (state == u"committed");
    const QStringList steps {QStringLiteral("untouched"), QStringLiteral("backed_up_pending"), QStringLiteral("backed_up")
        , QStringLiteral("installed_pending"), QStringLiteral("installed"), QStringLiteral("uninstalled_pending")
        , QStringLiteral("uninstalled"), QStringLiteral("restored_pending"), QStringLiteral("restored")};
    for (const lt::file_index_t index : files.file_range())
        expectedCount += !files.pad_file_at(index);
    for (const QJsonValue &entry : entries)
    {
        const QJsonObject file = entry.toObject();
        const int index = file.value(QStringLiteral("index")).toInt(-1);
        const QJsonValue selected = file.value(QStringLiteral("selected"));
        const QJsonValue verified = file.value(QStringLiteral("verified"));
        if ((index < 0) || (index >= files.num_files()) || indexes.contains(index)
            || (file.value(QStringLiteral("index")) != QJsonValue(index))
            || files.pad_file_at(lt::file_index_t(index))
            || (file.value(QStringLiteral("path")).toString() != relativePath(files, lt::file_index_t(index)))
            || (file.value(QStringLiteral("size")).toString() != QString::number(files.file_size(lt::file_index_t(index))))
            || !selected.isBool() || !validIdentity(file.value(QStringLiteral("original")))
            || !steps.contains(file.value(QStringLiteral("step")).toString())
            || (!selected.toBool() && (file.value(QStringLiteral("step")) != QJsonValue(QStringLiteral("untouched"))))
            || ((!verified.isUndefined() || (requiresVerified && selected.toBool()))
                && (!selected.toBool() || !validIdentity(verified) || verified.toObject().isEmpty()
                    || (verified.toObject().value(QStringLiteral("size")) != file.value(QStringLiteral("size"))))))
        {
            error = QStringLiteral("The journal does not match the target torrent file mapping.");
            return {};
        }
        indexes.insert(index);
        selectedCount += selected.toBool();
    }
    if ((indexes.size() != expectedCount) || (selectedCount == 0))
    {
        error = QStringLiteral("The journal omits target files or has no selected files.");
        return {};
    }
    auto operation = std::unique_ptr<StagingOperation>(new StagingOperation);
    operation->m_journalPath = journalPath;
    operation->m_journal = journal;
    operation->m_files = files;
    if (!RepairFileGuard::open(files, operation->destination(), false, error))
        return {};
    return operation;
}

bool StagingOperation::save(QString &error)
{
#ifndef Q_OS_WIN
    error = QStringLiteral("Durable staged repair currently requires Windows.");
    return false;
#else
    const QByteArray data = QJsonDocument(m_journal).toJson(QJsonDocument::Compact);
    if ((data.size() > JournalLimit) || !QDir().mkpath(QFileInfo(m_journalPath).absolutePath()))
    {
        error = QStringLiteral("Cannot allocate the bounded staging journal.");
        return false;
    }
    const QString temporary = m_journalPath + u".pending-" + QUuid::createUuid().toString(QUuid::WithoutBraces);
    QFile output(temporary);
    if (!output.open(QIODevice::WriteOnly | QIODevice::NewOnly)
        || (output.write(data) != data.size()) || !output.flush()
        || !FlushFileBuffers(reinterpret_cast<HANDLE>(_get_osfhandle(output.handle()))))
    {
        error = QStringLiteral("Cannot durably write the staging journal.");
        return false;
    }
    output.close();
    const QString from = nativePath(temporary);
    const QString to = nativePath(m_journalPath);
    if (!MoveFileExW(reinterpret_cast<LPCWSTR>(from.utf16()), reinterpret_cast<LPCWSTR>(to.utf16())
        , MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH))
    {
        error = QStringLiteral("Cannot publish the staging journal (Windows error %1).").arg(GetLastError());
        return false;
    }
#ifdef QBUTT_STAGING_FAULTS
    crashCheckpoint(state());
#endif
    return true;
#endif
}

bool StagingOperation::prepare(QString &error, const std::atomic_bool *cancelFlag)
{
    error.clear();
    if (state() != u"planned")
    {
        error = QStringLiteral("Only a new read-only plan may prepare staging. Recover an interrupted operation first.");
        return false;
    }
    const QStorageInfo storage(destination());
    const qint64 required = m_journal.value(QStringLiteral("required_bytes")).toString().toLongLong();
    if (!storage.isValid() || !storage.isReady() || (storage.bytesAvailable() < required))
    {
        error = QStringLiteral("Safe staging needs %1 additional bytes on the target volume. No in-place fallback is used.").arg(required);
        return false;
    }
    auto destinationGuard = RepairFileGuard::open(m_files, destination(), false, error, cancelFlag);
    if (!destinationGuard)
        return false;
    m_journal.insert(QStringLiteral("state"), QStringLiteral("preparing"));
    if (!save(error))
        return false;
    const QString root = transactionRoot(m_journal);
    if (!QDir().mkdir(root) || !QDir(root).mkdir(QStringLiteral("payload")) || !QDir(root).mkdir(QStringLiteral("backup")))
    {
        error = QStringLiteral("Cannot create a new, independent staging directory.");
        return false;
    }
    const QJsonArray entries = m_journal.value(QStringLiteral("files")).toArray();
    for (const QJsonValue &entry : entries)
    {
        const QJsonObject file = entry.toObject();
        const QString path = QDir(payloadPath()).filePath(file.value(QStringLiteral("path")).toString());
        if (!QDir().mkpath(QFileInfo(path).absolutePath()))
        {
            error = QStringLiteral("Cannot prepare target staging directories.");
            return false;
        }
    }
    auto stagingGuard = RepairFileGuard::open(m_files, payloadPath(), false, error, cancelFlag);
    if (!stagingGuard)
        return false;
    std::array<char, 64 * 1024> buffer {};
    for (const QJsonValue &entry : entries)
    {
        if (cancelled(cancelFlag, error))
            return false;
        const QJsonObject file = entry.toObject();
        const QString path = QDir(payloadPath()).filePath(file.value(QStringLiteral("path")).toString());
        const QString source = file.value(QStringLiteral("source")).toString();
        const qint64 size = file.value(QStringLiteral("size")).toString().toLongLong();
        QFile output(path);
        if (!output.open(QIODevice::WriteOnly | QIODevice::NewOnly))
        {
            error = QStringLiteral("Cannot exclusively create staging file %1.").arg(path);
            return false;
        }
        if (!source.isEmpty())
        {
            QFile input(source);
            if (!input.open(QIODevice::ReadOnly))
            {
                error = QStringLiteral("A selected source is no longer readable: %1.").arg(source);
                return false;
            }
            for (qint64 remaining = std::min(input.size(), size); remaining > 0;)
            {
                if (cancelled(cancelFlag, error))
                    return false;
                const qint64 count = std::min<qint64>(remaining, buffer.size());
                if ((input.read(buffer.data(), count) != count) || (output.write(buffer.data(), count) != count))
                {
                    error = QStringLiteral("Copying source data to independent staging failed.");
                    return false;
                }
                remaining -= count;
            }
        }
        if (!output.resize(size) || !output.flush())
        {
            error = QStringLiteral("Cannot allocate the exact target file size in staging.");
            return false;
        }
#ifdef Q_OS_WIN
        if (!FlushFileBuffers(reinterpret_cast<HANDLE>(_get_osfhandle(output.handle()))))
        {
            error = QStringLiteral("Cannot flush staging data to disk.");
            return false;
        }
#endif
    }
    m_sourceGuards.clear();
    m_journal.insert(QStringLiteral("state"), QStringLiteral("downloading"));
    return save(error);
}

bool StagingOperation::verify(const lt::torrent_info &target, QString &error, const std::atomic_bool *cancelFlag)
{
    error.clear();
    if ((state() != u"downloading") && (state() != u"ready_to_commit"))
    {
        error = QStringLiteral("This staged payload is not ready for verification.");
        return false;
    }
    auto guard = RepairFileGuard::open(m_files, payloadPath(), false, error, cancelFlag);
    if (!guard)
        return false;
    const QSet<int> readable = guard->existingFiles();
    m_analysis = analyzeRepairData(target, m_files, payloadPath(), cancelFlag, &readable);
    if (!m_analysis.error.isEmpty())
    {
        error = m_analysis.error;
        return false;
    }
    QJsonArray entries = m_journal.value(QStringLiteral("files")).toArray();
    QSet<int> selected;
    for (const QJsonValue &entry : entries)
    {
        const QJsonObject value = entry.toObject();
        if (value.value(QStringLiteral("selected")).toBool())
            selected.insert(value.value(QStringLiteral("index")).toInt());
    }
    for (const RepairFileAnalysis &file : m_analysis.files)
    {
        if (!selected.contains(file.nativeIndex))
            continue;
        if ((file.verifiedBytes != file.expectedSize) || (file.actualSize != file.expectedSize))
        {
            error = QStringLiteral("Staging must match every target hash and exact file size before commit: %1.").arg(file.path);
            return false;
        }
    }
    for (qsizetype i = 0; i < entries.size(); ++i)
    {
        QJsonObject entry = entries.at(i).toObject();
#ifdef Q_OS_WIN
        FileHandle file;
        QJsonObject attributes;
        if (!entry.value(QStringLiteral("selected")).toBool())
            continue;
        if (!openFile(QDir(payloadPath()).filePath(entry.value(QStringLiteral("path")).toString()), file, attributes, error))
            return false;
        entry.insert(QStringLiteral("verified"), attributes);
#endif
        entries[i] = entry;
    }
    m_journal.insert(QStringLiteral("files"), entries);
    m_journal.insert(QStringLiteral("state"), QStringLiteral("ready_to_commit"));
    return save(error);
}

bool StagingOperation::commit(const lt::torrent_info &target, QString &error, const std::atomic_bool *cancelFlag)
{
    return transact(false, error, cancelFlag, &target);
}

bool StagingOperation::rollback(QString &error, const std::atomic_bool *cancelFlag)
{
    return transact(true, error, cancelFlag);
}

bool StagingOperation::finish(QString &error)
{
    if ((state() != u"committed") && (state() != u"rolled_back"))
    {
        error = QStringLiteral("Only a completed commit or rollback may release its recovery reservation.");
        return false;
    }
    // Retire the reservation with a durable rename on the journal volume.
    // Retain the final manifest in the profile and backups at the destination;
    // no recursive cleanup touches unknown files or needs a cross-volume move.
#ifdef Q_OS_WIN
    const QString archive = nativePath(m_journalPath + u".finished-" + m_journal.value(QStringLiteral("operation")).toString());
    const QString active = nativePath(m_journalPath);
    if (!MoveFileExW(reinterpret_cast<LPCWSTR>(active.utf16()), reinterpret_cast<LPCWSTR>(archive.utf16()), MOVEFILE_WRITE_THROUGH))
    {
        error = QStringLiteral("Cannot archive the completed recovery journal.");
        return false;
    }
#else
    error = QStringLiteral("Durable journal retirement currently requires Windows.");
    return false;
#endif
#ifdef QBUTT_STAGING_FAULTS
    crashCheckpoint(QStringLiteral("journal_retired"));
#endif
    return true;
}

bool StagingOperation::transact(const bool rollback, QString &error, const std::atomic_bool *cancelFlag, const lt::torrent_info *target)
{
    error.clear();
#ifndef Q_OS_WIN
    error = QStringLiteral("Recoverable staged commit currently requires Windows.");
    return false;
#else
    if ((!rollback && (state() != u"ready_to_commit") && (state() != u"committing") && (state() != u"committed"))
        || (rollback && (state() == u"planned")))
    {
        error = QStringLiteral("The durable operation state does not permit this transition.");
        return false;
    }
    const QString backup = QDir(transactionRoot(m_journal)).filePath(QStringLiteral("backup"));
    QJsonArray entries = m_journal.value(QStringLiteral("files")).toArray();
    // Existing parents remain held while absent directories are prepared. Then
    // reopen the complete paths to reject aliases introduced during creation.
    auto targetDirectories = RepairFileGuard::open(m_files, destination(), false, error);
    if (!targetDirectories)
        return false;
    targetDirectories->releaseFiles();
    for (const QJsonValue &entry : entries)
    {
        const QString relative = entry.toObject().value(QStringLiteral("path")).toString();
        for (const QString &root : {destination(), payloadPath(), backup})
        {
            if (!QDir().mkpath(QFileInfo(QDir(root).filePath(relative)).absolutePath()))
            {
                error = QStringLiteral("Cannot prepare recoverable rename directories.");
                return false;
            }
        }
    }
    QList<std::shared_ptr<RepairFileGuard>> parents;
    QList<std::shared_ptr<RepairFileGuard>> checkingDirectories;
    for (const QString &root : {destination(), payloadPath(), backup})
    {
        auto checking = RepairFileGuard::open(m_files, root, false, error);
        if (!checking)
            return false;
        checking->releaseFiles();
        checkingDirectories.append(std::move(checking));
        // Rename may open the parent for child modification. Keep its identity
        // against deletion, then address the already opened parent plus one
        // basename through NtSetInformationFile. No reparse traversal occurs
        // through a newly opened full destination pathname.
        auto guard = RepairFileGuard::open(m_files, root, false, error, nullptr, true);
        if (!guard)
            return false;
        guard->releaseFiles();
        parents.append(std::move(guard));
    }
    targetDirectories.reset();

    struct OwnedFile
    {
        std::array<FileHandle, 3> handles;
        std::array<QJsonObject, 3> identities;
    };
    std::vector<OwnedFile> owned(entries.size());
    const QStringList roots {destination(), payloadPath(), backup};
    QSet<int> readable;
    QSet<int> selected;
    QMap<int, int> descriptors;
    // Acquire every existing target, stage and backup before changing any file.
    // Missing names are protected by non-replacing handle renames. An unexpected
    // identity stops recovery instead of deleting or replacing somebody's file.
    for (qsizetype i = 0; i < entries.size(); ++i)
    {
        if (cancelled(cancelFlag, error))
            return false;
        const QJsonObject entry = entries.at(i).toObject();
        const int index = entry.value(QStringLiteral("index")).toInt();
        if (!entry.value(QStringLiteral("selected")).toBool())
        {
            if (!rollback)
            {
                const QString path = QDir(payloadPath()).filePath(entry.value(QStringLiteral("path")).toString());
                if (!openFile(path, owned[i].handles[1], owned[i].identities[1], error))
                    return false;
                if (!owned[i].identities[1].isEmpty())
                    readable.insert(index);
            }
            continue;
        }
        selected.insert(index);
        const QString relative = entry.value(QStringLiteral("path")).toString();
        const QJsonObject original = entry.value(QStringLiteral("original")).toObject();
        const QJsonObject verified = entry.value(QStringLiteral("verified")).toObject();
        int originals = 0;
        int replacements = 0;
        for (int slot = 0; slot < 3; ++slot)
        {
            if (!openFile(QDir(roots.at(slot)).filePath(relative), owned[i].handles[slot], owned[i].identities[slot], error, true))
                return false;
            const QJsonObject actual = owned[i].identities[slot];
            if (actual.isEmpty())
                continue;
            if (!original.isEmpty() && (actual == original) && (slot != 1))
                ++originals;
            else if (!verified.isEmpty() && (actual == verified) && (slot != 2))
            {
                ++replacements;
                readable.insert(index);
            }
            else if (rollback && verified.isEmpty() && (slot == 1))
                continue; // Interrupted preparation/download never installs data.
            else
            {
                error = QStringLiteral("File identity changed at %1. Recovery preserved it and needs attention.").arg(QDir(roots.at(slot)).filePath(relative));
                return false;
            }
        }
        if ((!original.isEmpty() && (originals != 1)) || (original.isEmpty() && (originals != 0))
            || (!rollback && (replacements != 1)) || (replacements > 1))
        {
            error = QStringLiteral("The original or verified file is missing or duplicated. Recovery stopped without deleting data.");
            return false;
        }
    }
    if (!rollback)
    {
        for (qsizetype i = 0; i < entries.size(); ++i)
        {
            const QJsonObject entry = entries.at(i).toObject();
            const int index = entry.value(QStringLiteral("index")).toInt();
            if (!readable.contains(index))
                continue;
            const int slot = (entry.value(QStringLiteral("selected")).toBool()
                && (owned[i].identities[0] == entry.value(QStringLiteral("verified")).toObject())) ? 0 : 1;
            FileHandle &file = owned[i].handles[slot];
            file.descriptor = _open_osfhandle(reinterpret_cast<intptr_t>(file.value), _O_RDONLY | _O_BINARY);
            if (file.descriptor < 0)
            {
                error = QStringLiteral("Cannot bind verification to the owned file handle.");
                return false;
            }
            descriptors.insert(index, file.descriptor);
        }
        // Persisted identity is an ownership check, never proof of content.
        // Rehash the recovered layout while file handles deny writes and strict
        // parent handles deny reparse changes. Keep files held through renames.
        const RepairAnalysis verification = analyzeRepairData(*target, m_files, destination(), cancelFlag, &readable, &descriptors);
        if (!verification.error.isEmpty())
        {
            error = verification.error;
            return false;
        }
        for (const RepairFileAnalysis &file : verification.files)
        {
            if (!selected.contains(file.nativeIndex))
                continue;
            if ((file.verifiedBytes != file.expectedSize) || (file.actualSize != file.expectedSize))
            {
                error = QStringLiteral("Target hashes or exact sizes changed before commit. Recovery retained every file.");
                return false;
            }
        }
    }
    checkingDirectories.clear();
    m_journal.insert(QStringLiteral("state"), rollback ? QStringLiteral("rolling_back") : QStringLiteral("committing"));
    if (!save(error))
        return false;
    for (qsizetype offset = 0; offset < entries.size(); ++offset)
    {
        if (cancelled(cancelFlag, error))
            return false;
        const qsizetype i = rollback ? (entries.size() - 1 - offset) : offset;
        QJsonObject entry = entries.at(i).toObject();
        if (!entry.value(QStringLiteral("selected")).toBool())
            continue;
        const QString relative = entry.value(QStringLiteral("path")).toString();
        const QJsonObject original = entry.value(QStringLiteral("original")).toObject();
        const QJsonObject verified = entry.value(QStringLiteral("verified")).toObject();
        const auto move = [&](const int from, const int to, const QString &step) -> bool
        {
            entry.insert(QStringLiteral("step"), QString(step + u"_pending"));
            entries[i] = entry;
            m_journal.insert(QStringLiteral("files"), entries);
            const QString path = QDir(roots.at(to)).filePath(relative);
            if (!save(error))
                return false;
#ifdef QBUTT_STAGING_FAULTS
            const QString checkpoint = QStringLiteral("%1:%2:%3:").arg(state()).arg(i).arg(step);
            crashCheckpoint(checkpoint + u"before");
#endif
            if (!renameFile(owned[i].handles[from].value
                , parents.at(to)->directoryHandle(QFileInfo(path).absolutePath()), path, error))
                return false;
#ifdef QBUTT_STAGING_FAULTS
            crashCheckpoint(checkpoint + u"renamed");
#endif
            owned[i].identities[to] = std::exchange(owned[i].identities[from], {});
            std::swap(owned[i].handles[to].value, owned[i].handles[from].value);
            std::swap(owned[i].handles[to].descriptor, owned[i].handles[from].descriptor);
            entry.insert(QStringLiteral("step"), step);
            entries[i] = entry;
            m_journal.insert(QStringLiteral("files"), entries);
            if (!save(error))
                return false;
#ifdef QBUTT_STAGING_FAULTS
            crashCheckpoint(checkpoint + u"after");
#endif
            return true;
        };
        if (rollback)
        {
            if (!verified.isEmpty() && (owned[i].identities[0] == verified) && !move(0, 1, QStringLiteral("uninstalled")))
                return false;
            if (!original.isEmpty() && (owned[i].identities[2] == original) && !move(2, 0, QStringLiteral("restored")))
                return false;
        }
        else
        {
            if (!original.isEmpty() && (owned[i].identities[0] == original) && !move(0, 2, QStringLiteral("backed_up")))
                return false;
            if (owned[i].identities[1] == verified && !move(1, 0, QStringLiteral("installed")))
                return false;
        }
    }
    m_journal.insert(QStringLiteral("state"), rollback ? QStringLiteral("rolled_back") : QStringLiteral("committed"));
    return save(error);
#endif
}

QString StagingOperation::destination() const
{
    return m_journal.value(QStringLiteral("destination")).toString();
}

QString StagingOperation::payloadPath() const
{
    return QDir(transactionRoot(m_journal)).filePath(QStringLiteral("payload"));
}

QString StagingOperation::state() const
{
    return m_journal.value(QStringLiteral("state")).toString();
}

QJsonObject StagingOperation::status() const
{
    QJsonObject status = m_journal;
    status.insert(QStringLiteral("payload_path"), payloadPath());
    return status;
}

const RepairAnalysis &StagingOperation::analysis() const
{
    return m_analysis;
}
