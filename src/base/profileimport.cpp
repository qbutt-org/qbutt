/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "profileimport.h"

#include <atomic>
#include <memory>
#include <set>

#include <libtorrent/file_storage.hpp>
#include <libtorrent/torrent_info.hpp>

#include <QCryptographicHash>
#include <QDir>
#include <QDirIterator>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLockFile>
#include <QRegularExpression>
#include <QSaveFile>
#include <QSemaphore>
#include <QSettings>
#include <QTemporaryDir>

#include "base/bittorrent/bencoderesumedatastorage.h"
#include "base/bittorrent/common.h"
#include "base/bittorrent/dbresumedatastorage.h"
#include "base/bittorrent/repairfileguard.h"
#include "base/bittorrent/resumedatastorage.h"
#include "base/bittorrent/torrentimpl.h"
#include "base/exceptions.h"
#include "base/global.h"
#include "base/profile.h"

namespace
{
    using Result = nonstd::expected<void, QString>;
    using namespace BitTorrent;

    const QStringList SAFE_SETTINGS
    {
        u"Preferences/General/Locale"_s,
        u"BitTorrent/Session/GlobalDLSpeedLimit"_s,
        u"BitTorrent/Session/GlobalUPSpeedLimit"_s,
        u"BitTorrent/Session/AlternativeGlobalDLSpeedLimit"_s,
        u"BitTorrent/Session/AlternativeGlobalUPSpeedLimit"_s,
        u"BitTorrent/Session/MaxConnections"_s,
        u"BitTorrent/Session/MaxConnectionsPerTorrent"_s,
        u"BitTorrent/Session/MaxUploads"_s,
        u"BitTorrent/Session/MaxUploadsPerTorrent"_s,
        u"BitTorrent/Session/QueueingSystemEnabled"_s,
        u"BitTorrent/Session/MaxActiveDownloads"_s,
        u"BitTorrent/Session/MaxActiveUploads"_s,
        u"BitTorrent/Session/MaxActiveTorrents"_s
    };

    // Only these application-owned objects participate in rollback. Payload
    // paths and arbitrary paths from an imported manifest are never deleted.
    const QStringList DATA_OBJECTS {u"BT_backup"_s, u"torrents.db"_s, u"torrents.db-wal"_s, u"torrents.db-shm"_s};

    QString transactionDirectory()
    {
        return QDir(specialFolderLocation(SpecialFolder::Data).data()).filePath(u"profile-import"_s);
    }

    QString settingsPath(const bool pending = false)
    {
        return Profile::instance()->applicationSettings(pending ? u"qbutt_new"_s : u"qbutt"_s)->fileName();
    }

    Result writeBytes(const QString &path, const QByteArray &bytes)
    {
        if (!QDir().mkpath(QFileInfo(path).absolutePath()))
            return nonstd::make_unexpected(u"Cannot create the profile migration directory."_s);
        QSaveFile file(path);
        if (!file.open(QIODevice::WriteOnly) || (file.write(bytes) != bytes.size()) || !file.commit())
            return nonstd::make_unexpected(u"Cannot persist the profile migration. Original data has been retained."_s);
        return {};
    }

    Result writeManifest(const QJsonObject &manifest)
    {
        return writeBytes(QDir(transactionDirectory()).filePath(u"transaction.json"_s)
            , QJsonDocument(manifest).toJson(QJsonDocument::Compact));
    }

    nonstd::expected<QJsonObject, QString> inventory(const QString &directory)
    {
        QJsonObject entries;
        QDirIterator iterator(directory, QDir::AllEntries | QDir::NoDotAndDotDot | QDir::Hidden | QDir::System
            , QDirIterator::Subdirectories);
        qint64 totalSize = 0;
        while (iterator.hasNext())
        {
            iterator.next();
            const QFileInfo info = iterator.fileInfo();
            if (info.isSymLink() || info.isJunction() || (entries.size() >= 30000))
                return nonstd::make_unexpected(u"The migration backup has an unsafe or excessive file tree."_s);
            const QString relative = QDir(directory).relativeFilePath(info.absoluteFilePath());
            if (info.isDir())
            {
                entries.insert(relative, u"directory"_s);
                continue;
            }
            QFile file(info.absoluteFilePath());
            QCryptographicHash hash(QCryptographicHash::Sha256);
            if (!info.isFile() || (info.size() < 0) || (info.size() > 1024LL * 1024 * 1024 - totalSize)
                || !file.open(QIODevice::ReadOnly) || !hash.addData(&file) || (file.error() != QFile::NoError))
            {
                return nonstd::make_unexpected(u"The migration backup cannot be verified completely."_s);
            }
            totalSize += info.size();
            entries.insert(relative, QString::fromLatin1(hash.result().toHex()));
        }
        return entries;
    }

    // Never follow links while copying or retiring application metadata. The
    // recursion is bounded by the profile format, not by an external path list.
    Result copyObject(const QString &source, const QString &destination, const int depth = 0)
    {
        const QFileInfo info(source);
        if (info.isSymLink() || info.isJunction() || (depth > 8))
            return nonstd::make_unexpected(u"Profile metadata contains an unsafe linked or nested path."_s);
        if (!info.exists())
            return {};
        if (info.isDir())
        {
            if (!QDir().mkpath(destination))
                return nonstd::make_unexpected(u"Cannot create the migration backup."_s);
            const auto entries = QDir(source).entryList(QDir::AllEntries | QDir::NoDotAndDotDot | QDir::Hidden | QDir::System);
            if (entries.size() > 30000)
                return nonstd::make_unexpected(u"The profile exceeds the migration file limit."_s);
            for (const QString &entry : entries)
            {
                const auto copied = copyObject(QDir(source).filePath(entry), QDir(destination).filePath(entry), depth + 1);
                if (!copied)
                    return copied;
            }
            return {};
        }
        QFile input(source);
        if (!info.isFile() || (info.size() > 512 * 1024 * 1024) || !input.open(QIODevice::ReadOnly))
            return nonstd::make_unexpected(u"Cannot read profile metadata for migration."_s);
        const QByteArray bytes = input.readAll();
        if ((input.error() != QFile::NoError) || (bytes.size() != info.size()))
            return nonstd::make_unexpected(u"Profile metadata changed while making its backup."_s);
        return writeBytes(destination, bytes);
    }

    Result removeObject(const QString &path, const int depth = 0)
    {
        const QFileInfo info(path);
        if (info.isSymLink() || info.isJunction() || (depth > 8))
            return nonstd::make_unexpected(u"Migration recovery refuses a linked metadata path."_s);
        if (!info.exists())
            return {};
        if (info.isDir())
        {
            for (const QString &entry : QDir(path).entryList(QDir::AllEntries | QDir::NoDotAndDotDot | QDir::Hidden | QDir::System))
            {
                const auto removed = removeObject(QDir(path).filePath(entry), depth + 1);
                if (!removed)
                    return removed;
            }
            if (QDir().rmdir(path))
                return {};
        }
        else if (QFile::remove(path))
            return {};
        return nonstd::make_unexpected(u"Cannot replace profile metadata. Close other clients and retry startup."_s);
    }

    Result copyData(const QString &source, const QString &destination)
    {
        for (const QString &name : DATA_OBJECTS)
        {
            const auto copied = copyObject(QDir(source).filePath(name), QDir(destination).filePath(name));
            if (!copied)
                return copied;
        }
        return {};
    }

    Result replaceData(const QString &source, const QString &destination)
    {
        for (const QString &name : DATA_OBJECTS)
        {
            const QString target = QDir(destination).filePath(name);
            const auto removed = removeObject(target);
            if (!removed)
                return removed;
            const auto copied = copyObject(QDir(source).filePath(name), target);
            if (!copied)
                return copied;
        }
        return {};
    }

    Result storeRecords(const QString &dataDirectory, const QList<LoadedResumeData> &records, const bool sqlite)
    {
        for (const auto &record : records)
        {
            if (!record.result)
                return nonstd::make_unexpected(u"Cannot import a damaged resume record."_s);
        }
        QSemaphore completed;
        std::atomic_bool success = true;
        std::unique_ptr<ResumeDataStorage> storage;
        if (sqlite)
            storage = std::make_unique<DBResumeDataStorage>(Path(QDir(dataDirectory).filePath(u"torrents.db"_s)));
        else
            storage = std::make_unique<BencodeResumeDataStorage>(Path(QDir(dataDirectory).filePath(u"BT_backup"_s)));

        QObject::connect(storage.get(), &ResumeDataStorage::stored, storage.get()
            , [&completed, &success](const quint64, const bool saved)
            {
                if (!saved)
                    success = false;
                completed.release();
            }, Qt::DirectConnection);
        quint64 revision = 0;
        for (const auto &record : records)
        {
            storage->store(record.torrentID, *record.result, ++revision);
        }
        const bool received = completed.tryAcquire(records.size(), 60000);
        // Destruction drains the native worker before callback state expires.
        storage.reset();
        if (!received || !success)
            return nonstd::make_unexpected(u"The native resume store did not confirm the migration write."_s);
        return {};
    }

    Result verifyRecords(const ExternalResumeDataResult &loaded, const QList<LoadedResumeData> &expected)
    {
        if (!loaded || (loaded->size() != expected.size()))
            return nonstd::make_unexpected(u"Native resume record count changed during migration."_s);
        QHash<TorrentID, const LoadTorrentParams *> records;
        for (const auto &record : *loaded)
        {
            if (!record.result || records.contains(record.torrentID))
                return nonstd::make_unexpected(u"A migrated resume record is unreadable or duplicated."_s);
            records.insert(record.torrentID, &*record.result);
        }
        for (const auto &record : expected)
        {
            const auto *actual = records.value(record.torrentID);
            if (!record.result || !actual)
                return nonstd::make_unexpected(u"A migrated torrent identity was lost."_s);
            const auto &wanted = *record.result;
            if ((actual->savePath != wanted.savePath) || (actual->downloadPath != wanted.downloadPath)
                || (actual->stopped != wanted.stopped) || (actual->useAutoTMM != wanted.useAutoTMM)
                || (actual->completionPolicyPreview != wanted.completionPolicyPreview)
                || (Path(actual->ltAddTorrentParams.save_path) != Path(wanted.ltAddTorrentParams.save_path))
                || (actual->ltAddTorrentParams.renamed_files != wanted.ltAddTorrentParams.renamed_files)
                || !actual->ltAddTorrentParams.ti || !wanted.ltAddTorrentParams.ti
                || (actual->ltAddTorrentParams.ti->info_hashes() != wanted.ltAddTorrentParams.ti->info_hashes()))
            {
                return nonstd::make_unexpected(u"A migrated native resume record differs from the approved mapping or policy."_s);
            }
        }
        return {};
    }

    nonstd::expected<lt::file_storage, QString> mappedFiles(const LoadTorrentParams &params)
    {
        if (!params.ltAddTorrentParams.ti)
            return nonstd::make_unexpected(u"Complete torrent metadata is required for import."_s);
        if (params.ltAddTorrentParams.ti->num_files() > 16384)
            return nonstd::make_unexpected(u"A torrent exceeds the profile import limit of 16384 file mappings."_s);
        lt::file_storage files = params.ltAddTorrentParams.ti->files();
        for (const auto &[index, name] : params.ltAddTorrentParams.renamed_files)
        {
            if ((index < lt::file_index_t(0)) || (index >= files.end_file()) || files.pad_file_at(index))
                return nonstd::make_unexpected(u"The resume record contains an invalid mapped-file index."_s);
            files.rename_file(index, name);
        }
        return files;
    }

    // Reserve names that the native session may use after checking the files or
    // changing its incomplete/unwanted naming preferences, including absent files.
    Result reserveTargets(const LoadTorrentParams &params, std::set<QString> &targets
        , const QStringList &protectedDirectories, const bool imported)
    {
        const auto files = mappedFiles(params);
        if (!files)
            return nonstd::make_unexpected(files.error());
        QStringList protectedIdentities;
        for (const QString &directory : protectedDirectories)
        {
            const QString identity = repairPathIdentity(directory);
            if (identity.isEmpty())
                return nonstd::make_unexpected(u"A protected profile directory cannot be resolved safely."_s);
            protectedIdentities.append(identity);
        }
        const QDir saveDirectory(QString::fromStdString(params.ltAddTorrentParams.save_path));
        for (const auto index : files->file_range())
        {
            if (files->pad_file_at(index))
                continue;
            if (targets.size() > 65536)
                return nonstd::make_unexpected(u"The profile import exceeds the total file reservation limit."_s);
            const Path actualPath(files->file_path(index));
            const QString actualIdentity = repairPathIdentity(saveDirectory.filePath(actualPath.data()));
            if (actualIdentity.isEmpty())
                return nonstd::make_unexpected(u"A payload mapping cannot be resolved safely."_s);
            const Path userPath = params.ltAddTorrentParams.renamed_files.contains(index)
                ? TorrentImpl::userFilePath(actualPath) : actualPath;
            const Path unwantedPath = userPath.parentPath() / Path(UNWANTED_FOLDER_NAME) / Path(userPath.filename());
            QStringList identities;
            for (const Path &alias : {actualPath, userPath, userPath + QB_EXT, unwantedPath, unwantedPath + QB_EXT})
            {
                const QString path = saveDirectory.filePath(alias.data());
                const QString identity = repairPathIdentity(path);
                if (identity.isEmpty())
                    return nonstd::make_unexpected(u"A native payload filename cannot be resolved safely."_s);
                const QFileInfo info(path);
                if (imported && (info.isSymLink() || info.isJunction()
                    || ((identity != actualIdentity) && info.exists())))
                {
                    return nonstd::make_unexpected(u"A native payload filename already belongs to another file: %1"_s.arg(path));
                }
                for (const QString &protectedIdentity : protectedIdentities)
                {
                    if ((identity == protectedIdentity) || identity.startsWith(protectedIdentity + u'/')
                        || protectedIdentity.startsWith(identity + u'/'))
                    {
                        return nonstd::make_unexpected(u"A payload mapping overlaps protected profile metadata: %1"_s.arg(path));
                    }
                }
                if (!identities.contains(identity))
                    identities.append(identity);
            }
            for (const QString &identity : identities)
            {
                // Check parents explicitly: a lexical predecessor can be a
                // sibling such as "file-" between "file" and "file/child".
                for (QString parent = identity; !parent.isEmpty(); parent = parent.left(parent.lastIndexOf(u'/')))
                {
                    if (targets.contains(parent))
                        return nonstd::make_unexpected(u"Selected torrents overlap another data set. Choose independent payload mappings."_s);
                    if (!parent.contains(u'/'))
                        break;
                }
                const QString prefix = identity + u'/';
                const auto descendant = targets.lower_bound(prefix);
                if ((descendant != targets.end()) && descendant->startsWith(prefix))
                    return nonstd::make_unexpected(u"Selected torrents overlap another data set. Choose independent payload mappings."_s);
                // Only this file's exact duplicates were removed above. A
                // later file cannot acquire any actual or derived name.
                targets.insert(identity);
            }
        }
        return {};
    }
}

nonstd::expected<ProfileImportPreview, QString> ProfileImport::preview(const Path &settingsFile
    , const Path &dataDirectory, const Path &sourceBase)
try
{
    if (!settingsFile.isAbsolute() || !dataDirectory.isAbsolute()
        || (!sourceBase.isEmpty() && !sourceBase.isAbsolute()))
        return nonstd::make_unexpected(u"Select absolute source paths."_s);
    if (repairPathIdentity(dataDirectory.data()) == repairPathIdentity(specialFolderLocation(SpecialFolder::Data).data()))
        return nonstd::make_unexpected(u"Choose a separate source profile."_s);
    QFile input(settingsFile.data());
    if (!input.open(QIODevice::ReadOnly) || (input.size() > 8 * 1024 * 1024))
        return nonstd::make_unexpected(u"Cannot read the source settings file."_s);
    const QByteArray settingsBytes = input.readAll();
    QTemporaryDir snapshot(QDir::tempPath() + u"/qbutt-profile-preview-XXXXXX"_s);
    if (!snapshot.isValid())
        return nonstd::make_unexpected(u"Cannot create a read-only settings snapshot."_s);
    const QString snapshotPath = QDir(snapshot.path()).filePath(u"settings.ini"_s);
    const auto saved = writeBytes(snapshotPath, settingsBytes);
    if (!saved)
        return nonstd::make_unexpected(saved.error());
    QSettings source(snapshotPath, QSettings::IniFormat);
    source.setFallbacksEnabled(false);
    ProfileImportPreview result;
    result.protectedDirectories = {dataDirectory.data(), settingsFile.parentPath().data()};
    const QStringList sourceKeys = source.allKeys();
    if (sourceKeys.size() > 10000)
        return nonstd::make_unexpected(u"The source exceeds the preview limit of 10000 settings."_s);
    for (const QString &key : sourceKeys)
    {
        if (SAFE_SETTINGS.contains(key))
            result.settings.insert(key, source.value(key));
        else
            result.skippedSettings.append(key);
    }
    if (source.status() != QSettings::NoError)
        return nonstd::make_unexpected(u"The source settings file is malformed."_s);

    const auto records = ResumeDataStorage::readExternal(dataDirectory, sourceBase);
    if (!records)
        return nonstd::make_unexpected(records.error());
    for (const auto &record : *records)
    {
        if (!record.result || !record.result->ltAddTorrentParams.ti)
            return nonstd::make_unexpected(u"The source contains a damaged or metadata-incomplete torrent. Resolve it before import."_s);
        const Path sourcePath(QString::fromStdString(record.result->ltAddTorrentParams.save_path));
        result.torrents.append({record.torrentID, *record.result, sourcePath, sourcePath, true});
    }
    if (!input.seek(0) || (input.readAll() != settingsBytes))
        return nonstd::make_unexpected(u"Source settings changed during preview. Close the source client and retry."_s);
    return result;
}
catch (const RuntimeError &error)
{
    return nonstd::make_unexpected(error.message());
}
catch (const std::exception &)
{
    return nonstd::make_unexpected(u"Cannot decode the source profile safely."_s);
}

Result ProfileImport::prepare(ProfileImportPreview preview, const bool ownershipConfirmed)
try
{
    if (!ownershipConfirmed)
        return nonstd::make_unexpected(u"Confirm that the source client has stopped managing the selected payloads."_s);
    const QDir transaction(transactionDirectory());
    if (transaction.exists())
        return nonstd::make_unexpected(u"A profile import is already pending. Restart qbutt to finish or recover it."_s);
    const QString data = specialFolderLocation(SpecialFolder::Data).data();
    QLockFile lock(QDir(data).filePath(u"profile-import.lock"_s));
    if (!lock.tryLock())
        return nonstd::make_unexpected(u"Another profile import is being prepared."_s);

    QList<LoadedResumeData> records;
    std::set<QString> targets;
    QStringList protectedDirectories = preview.protectedDirectories;
    for (const auto folder : {SpecialFolder::Config, SpecialFolder::Data, SpecialFolder::Cache})
        protectedDirectories.append(specialFolderLocation(folder).data());
    QList<std::shared_ptr<RepairFileGuard>> ownership;
    for (auto &torrent : preview.torrents)
    {
        if (!torrent.selected)
            continue;
        auto &params = torrent.params;
        auto &native = params.ltAddTorrentParams;
        if (!torrent.destinationPath.isAbsolute() || !native.ti)
            return nonstd::make_unexpected(u"Every selected torrent requires an explicit absolute payload mapping."_s);
        native.save_path = torrent.destinationPath.toString().toStdString();
        const auto reserved = reserveTargets(params, targets, protectedDirectories, true);
        if (!reserved)
            return reserved;
        const auto files = mappedFiles(params);
        if (!files)
            return nonstd::make_unexpected(files.error());
        QString error;
        auto guard = RepairFileGuard::open(*files, torrent.destinationPath.data(), true, error);
        if (!guard)
            return nonstd::make_unexpected(u"Cannot admit one writer for an imported payload: %1"_s.arg(error));
        ownership.append(std::move(guard));
        params.savePath = torrent.destinationPath;
        params.downloadPath = {};
        params.useAutoTMM = false;
        params.stopped = true;
        params.hasFinishedStatus = false;
        params.completionPolicyPreview = true;
        params.ratioLimit = NO_RATIO_LIMIT;
        params.seedingTimeLimit = NO_SEEDING_TIME_LIMIT;
        params.inactiveSeedingTimeLimit = NO_SEEDING_TIME_LIMIT;
        params.shareLimitAction = ShareLimitAction::Stop;
        params.sslParameters = {};
        native.flags = (lt::add_torrent_params {}.flags | lt::torrent_flags::paused) & ~lt::torrent_flags::auto_managed;
        if (native.ti->is_i2p())
            native.flags |= lt::torrent_flags::i2p_torrent;
        native.have_pieces.clear();
        native.verified_pieces.clear();
        native.unfinished_pieces.clear();
        native.peers.clear();
        native.banned_peers.clear();
        records.append({torrent.id, params});
    }
    if (records.isEmpty())
        return nonstd::make_unexpected(u"Select at least one torrent to import."_s);

    QTemporaryDir staging(QDir(data).filePath(u"profile-import-preparing-XXXXXX"_s));
    if (!staging.isValid())
        return nonstd::make_unexpected(u"Cannot create the native import staging directory."_s);
    const QString imports = QDir(staging.path()).filePath(u"imports"_s);
    const auto stored = storeRecords(imports, records, false);
    if (!stored)
        return stored;
    const auto verified = verifyRecords(ResumeDataStorage::readExternal(Path(imports), Profile::instance()->basePath()), records);
    if (!verified)
        return verified;
    QSettings settings(QDir(staging.path()).filePath(u"settings.ini"_s), QSettings::IniFormat);
    for (auto it = preview.settings.cbegin(); it != preview.settings.cend(); ++it)
    {
        if (SAFE_SETTINGS.contains(it.key()))
            settings.setValue(it.key(), it.value());
    }
    settings.sync();
    if (settings.status() != QSettings::NoError)
        return nonstd::make_unexpected(u"Cannot stage imported settings."_s);
    const auto manifest = writeBytes(QDir(staging.path()).filePath(u"transaction.json"_s)
        , QJsonDocument(QJsonObject {{u"version"_s, 1}, {u"state"_s, u"prepared"_s}
            , {u"protectedDirectories"_s, QJsonArray::fromStringList(preview.protectedDirectories)}}).toJson(QJsonDocument::Compact));
    if (!manifest)
        return manifest;
    if (!QDir().rename(staging.path(), transaction.path()))
        return nonstd::make_unexpected(u"Cannot publish the prepared profile import."_s);
    staging.setAutoRemove(false);
    return {};
}
catch (const RuntimeError &error)
{
    return nonstd::make_unexpected(error.message());
}
catch (const std::exception &)
{
    return nonstd::make_unexpected(u"Cannot prepare the native profile import."_s);
}

Result ProfileImport::recover()
{
    const QDir transaction(transactionDirectory());
    if (!transaction.exists())
        return {};
    QLockFile lock(QDir(specialFolderLocation(SpecialFolder::Data).data()).filePath(u"profile-import.lock"_s));
    if (!lock.tryLock())
        return nonstd::make_unexpected(u"Profile import is still being prepared by another process."_s);
    QFile manifestFile(transaction.filePath(u"transaction.json"_s));
    if (!manifestFile.open(QIODevice::ReadOnly) || (manifestFile.size() > 16384))
        return nonstd::make_unexpected(u"The profile import manifest cannot be read. Existing settings have not been opened."_s);
    const auto document = QJsonDocument::fromJson(manifestFile.readAll());
    manifestFile.close();
    QJsonObject manifest = document.object();
    if (manifest.value(u"version"_s).toInt() != 1)
        return nonstd::make_unexpected(u"The profile import manifest has an unsupported format."_s);
    const QString state = manifest.value(u"state"_s).toString();
    const QString data = specialFolderLocation(SpecialFolder::Data).data();

    const auto retire = [&]() -> Result
    {
        const QString backup = manifest.value(u"backup"_s).toString();
        if (!backup.isEmpty())
        {
            const QFileInfo info(backup);
            if (!info.fileName().startsWith(u"qbutt-profile-migration-"_s)
                || (repairPathIdentity(info.absolutePath()) != repairPathIdentity(QDir::tempPath())))
            {
                return nonstd::make_unexpected(u"Migration retirement refuses an invalid backup location."_s);
            }
            const auto removed = removeObject(backup);
            if (!removed)
                return removed;
        }
        return removeObject(transaction.path());
    };

    const auto restore = [&]() -> Result
    {
        const QString backup = manifest.value(u"backup"_s).toString();
        const QFileInfo backupInfo(backup);
        if (backupInfo.isSymLink() || backupInfo.isJunction() || !backupInfo.isDir()
            || !backupInfo.fileName().startsWith(u"qbutt-profile-migration-"_s)
            || (repairPathIdentity(backupInfo.absolutePath()) != repairPathIdentity(QDir::tempPath())))
        {
            return nonstd::make_unexpected(u"The migration backup is unavailable. Startup is held to preserve recoverable state."_s);
        }
        QFile index(transaction.filePath(u"backup.json"_s));
        if (!index.open(QIODevice::ReadOnly) || (index.size() > 16 * 1024 * 1024))
            return nonstd::make_unexpected(u"The migration backup inventory is unavailable. Startup remains held."_s);
        const QByteArray bytes = index.readAll();
        if (QString::fromLatin1(QCryptographicHash::hash(bytes, QCryptographicHash::Sha256).toHex())
            != manifest.value(u"backupDigest"_s).toString())
        {
            return nonstd::make_unexpected(u"The migration backup inventory is damaged. No recovery files were removed."_s);
        }
        const auto checked = inventory(backup);
        const auto expected = QJsonDocument::fromJson(bytes);
        if (!checked || !expected.isObject() || (*checked != expected.object()))
            return nonstd::make_unexpected(u"The migration backup is incomplete or changed. No recovery files were removed."_s);
        auto result = replaceData(QDir(backup).filePath(u"data"_s), data);
        if (!result)
            return result;
        for (const bool pending : {false, true})
        {
            const QString target = settingsPath(pending);
            result = removeObject(target);
            if (!result)
                return result;
            result = copyObject(QDir(backup).filePath(pending ? u"settings-new.ini"_s : u"settings.ini"_s), target);
            if (!result)
                return result;
        }
        // A completed rollback is durable before retirement. Repeating the
        // same replacement after an interrupted recovery is safe.
        manifest.insert(u"state"_s, u"rolled-back"_s);
        return writeManifest(manifest);
    };

    if (state == u"installing")
    {
        const auto restored = restore();
        if (!restored)
            return restored;
        // Keep the prepared import and backup for an explicit retry; do not
        // silently reapply a transaction which previously failed.
        return nonstd::make_unexpected(u"An interrupted profile import was rolled back. Original qbutt settings are restored. Start qbutt again to continue."_s);
    }
    if ((state == u"committed") || (state == u"rolled-back"))
        return retire();
    if (state != u"prepared")
        return nonstd::make_unexpected(u"The profile import state is invalid; startup is held."_s);

    const auto install = [&]() -> Result
    {
        const QDir staging(QDir(data).filePath(u"staging"_s));
        const QRegularExpression activeJournal(u"^[a-fA-F0-9]{40}\\.json$"_s);
        for (const QString &name : staging.entryList({u"*.json"_s}, QDir::Files))
        {
            if (activeJournal.match(name).hasMatch())
                return nonstd::make_unexpected(u"Finish staging recovery before importing another profile."_s);
        }

        const auto imports = ResumeDataStorage::readExternal(Path(transaction.filePath(u"imports"_s)), Profile::instance()->basePath());
        if (!imports || imports->isEmpty())
            return nonstd::make_unexpected(u"The prepared native import cannot be read. Existing qbutt settings are unchanged."_s);

        QList<LoadedResumeData> current;
        if (QFileInfo::exists(QDir(data).filePath(u"torrents.db"_s))
            || QFileInfo::exists(QDir(data).filePath(u"BT_backup"_s)))
        {
            const auto loaded = ResumeDataStorage::readExternal(Path(data), Profile::instance()->basePath());
            if (!loaded)
                return nonstd::make_unexpected(u"Cannot inspect existing qbutt jobs before import: %1"_s.arg(loaded.error()));
            current = *loaded;
        }
        std::set<QString> targets;
        QStringList protectedDirectories;
        for (const auto &directory : manifest.value(u"protectedDirectories"_s).toArray())
            protectedDirectories.append(directory.toString());
        for (const auto folder : {SpecialFolder::Config, SpecialFolder::Data, SpecialFolder::Cache})
            protectedDirectories.append(specialFolderLocation(folder).data());
        QList<InfoHash> hashes;
        for (const auto &record : current)
        {
            if (!record.result)
                return nonstd::make_unexpected(u"An existing qbutt resume record needs recovery before import."_s);
            hashes.append(InfoHash(record.result->ltAddTorrentParams.info_hashes));
            const auto reserved = reserveTargets(*record.result, targets, protectedDirectories, false);
            if (!reserved)
                return reserved;
        }
        QList<std::shared_ptr<RepairFileGuard>> ownership;
        for (const auto &record : *imports)
        {
            if (!record.result || !record.result->stopped || record.result->useAutoTMM)
                return nonstd::make_unexpected(u"Prepared imports must remain stopped with explicit payload mappings."_s);
            const auto &params = *record.result;
            const auto hash = params.ltAddTorrentParams.info_hashes;
            for (const InfoHash &existing : hashes)
            {
                const auto other = static_cast<lt::info_hash_t>(existing);
                if ((hash.has_v1() && other.has_v1() && (hash.v1 == other.v1))
                    || (hash.has_v2() && other.has_v2() && (hash.v2 == other.v2)))
                {
                    return nonstd::make_unexpected(u"An imported torrent already exists in qbutt. No trackers, files or settings were merged."_s);
                }
            }
            hashes.append(InfoHash(hash));
            const auto reserved = reserveTargets(params, targets, protectedDirectories, true);
            if (!reserved)
                return reserved;
            const auto files = mappedFiles(params);
            if (!files)
                return nonstd::make_unexpected(files.error());
            QString error;
            auto guard = RepairFileGuard::open(*files
                , QString::fromStdString(params.ltAddTorrentParams.save_path), true, error);
            if (!guard)
                return nonstd::make_unexpected(u"Imported payload ownership changed before startup: %1"_s.arg(error));
            ownership.append(std::move(guard));
        }

        QTemporaryDir backup(QDir::tempPath() + u"/qbutt-profile-migration-XXXXXX"_s);
        if (!backup.isValid())
            return nonstd::make_unexpected(u"Cannot create the required temporary migration backup."_s);
        auto result = copyData(data, QDir(backup.path()).filePath(u"data"_s));
        if (!result)
            return result;
        for (const bool pending : {false, true})
        {
            result = copyObject(settingsPath(pending)
                , QDir(backup.path()).filePath(pending ? u"settings-new.ini"_s : u"settings.ini"_s));
            if (!result)
                return result;
        }

        const auto backupFiles = inventory(backup.path());
        if (!backupFiles)
            return nonstd::make_unexpected(backupFiles.error());
        const QByteArray backupIndex = QJsonDocument(*backupFiles).toJson(QJsonDocument::Compact);
        result = writeBytes(transaction.filePath(u"backup.json"_s), backupIndex);
        if (!result)
            return result;

        QTemporaryDir merged(transaction.filePath(u"merged-XXXXXX"_s));
        if (!merged.isValid())
            return nonstd::make_unexpected(u"Cannot create the merged native resume store."_s);
        result = copyData(QDir(backup.path()).filePath(u"data"_s), merged.path());
        if (!result)
            return result;
        result = storeRecords(merged.path(), *imports, QFileInfo::exists(QDir(data).filePath(u"torrents.db"_s)));
        if (!result)
            return result;
        const auto checked = verifyRecords(ResumeDataStorage::readExternal(Path(merged.path()), Profile::instance()->basePath()), current + *imports);
        if (!checked)
            return checked;

        const QString mergedSettings = QDir(merged.path()).filePath(u"settings.ini"_s);
        QString previousSettings = QDir(backup.path()).filePath(u"settings.ini"_s);
        QSettings pending(QDir(backup.path()).filePath(u"settings-new.ini"_s), QSettings::IniFormat);
        pending.setFallbacksEnabled(false);
        // Match SettingsStorage's recovery rule: an interrupted file containing
        // only INI headers or whitespace does not replace the current settings.
        const bool hasPendingSettings = !pending.allKeys().isEmpty();
        if (pending.status() != QSettings::NoError)
            return nonstd::make_unexpected(u"The pending qbutt settings need recovery before import."_s);
        if (hasPendingSettings)
            previousSettings = pending.fileName();
        result = copyObject(previousSettings, mergedSettings);
        if (!result)
            return result;
        {
            QSettings settings(mergedSettings, QSettings::IniFormat);
            settings.setFallbacksEnabled(false);
            QSettings incoming(transaction.filePath(u"settings.ini"_s), QSettings::IniFormat);
            incoming.setFallbacksEnabled(false);
            for (const QString &key : SAFE_SETTINGS)
            {
                if (incoming.contains(key))
                    settings.setValue(key, incoming.value(key));
            }
            settings.sync();
            if ((settings.status() != QSettings::NoError) || (incoming.status() != QSettings::NoError))
                return nonstd::make_unexpected(u"Merged settings failed validation before installation."_s);
        }

        manifest.insert(u"backup"_s, backup.path());
        manifest.insert(u"backupDigest"_s, QString::fromLatin1(QCryptographicHash::hash(backupIndex, QCryptographicHash::Sha256).toHex()));
        manifest.insert(u"state"_s, u"installing"_s);
        result = writeManifest(manifest);
        if (!result)
            return result;
        backup.setAutoRemove(false);
        result = replaceData(merged.path(), data);
        if (result)
            result = copyObject(mergedSettings, settingsPath());
        if (result)
            result = removeObject(settingsPath(true));
        if (!result)
        {
            const auto restored = restore();
            return restored ? result : restored;
        }
        manifest.insert(u"state"_s, u"committed"_s);
        result = writeManifest(manifest);
        if (!result)
        {
            const auto restored = restore();
            return restored ? result : restored;
        }
        return retire();
    };
    Result installed;
    try
    {
        installed = install();
    }
    catch (const RuntimeError &error)
    {
        installed = nonstd::make_unexpected(error.message());
    }
    catch (const std::exception &)
    {
        installed = nonstd::make_unexpected(u"Profile migration failed before completion."_s);
    }
    if (!installed && (manifest.value(u"state"_s).toString() == u"installing"))
    {
        const auto restored = restore();
        if (!restored)
            return restored;
    }
    if (!installed && (manifest.value(u"state"_s).toString() == u"prepared"))
    {
        manifest.insert(u"state"_s, u"rolled-back"_s);
        const auto rejected = writeManifest(manifest);
        if (!rejected)
            return rejected;
        return nonstd::make_unexpected(installed.error() + u" Import was cancelled before installation; start qbutt again to continue."_s);
    }
    return installed;
}
