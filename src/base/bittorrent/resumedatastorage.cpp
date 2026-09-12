/*
 * Bittorrent Client using Qt and libtorrent.
 * Copyright (C) 2015-2022  Vladimir Golovnev <glassez@yandex.ru>
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License
 * as published by the Free Software Foundation; either version 2
 * of the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA  02110-1301, USA.
 *
 * In addition, as a special exception, the copyright holders give permission to
 * link this program with the OpenSSL project's "OpenSSL" library (or with
 * modified versions of it that use the same license as the "OpenSSL" library),
 * and distribute the linked executables. You must obey the GNU General Public
 * License in all respects for all of the code used other than "OpenSSL".  If you
 * modify file(s), you may extend this exception to your version of the file(s),
 * but you are not obligated to do so. If you do not wish to do so, delete this
 * exception statement from your version.
 */

#include "resumedatastorage.h"

#include <algorithm>
#include <cstdint>
#include <memory>
#include <utility>
#include <vector>

#ifdef Q_OS_WIN
#include <fcntl.h>
#include <io.h>
#include <windows.h>
#endif

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QList>
#include <QMetaObject>
#include <QMutexLocker>
#include <QScopeGuard>
#include <QTemporaryDir>
#include <QThread>

#include "base/global.h"
#include "bencoderesumedatastorage.h"
#include "dbresumedatastorage.h"

const int TORRENTIDLIST_TYPEID = qRegisterMetaType<QList<BitTorrent::TorrentID>>();

BitTorrent::ResumeDataStorage::ResumeDataStorage(const Path &path, QObject *parent)
    : QObject(parent)
    , m_path {path}
{
}

BitTorrent::ExternalResumeDataResult BitTorrent::ResumeDataStorage::readExternal(
    const Path &sourceDataDirectory, const Path &sourceProfileBase)
{
    if (!sourceDataDirectory.isAbsolute() || !QFileInfo(sourceDataDirectory.data()).isDir()
        || (!sourceProfileBase.isEmpty()
            && (!sourceProfileBase.isAbsolute() || !QFileInfo(sourceProfileBase.data()).isDir())))
    {
        return nonstd::make_unexpected(tr("Select an existing external data directory and, when needed, its absolute profile base."));
    }
    const Path dbPath = sourceDataDirectory / Path(u"torrents.db"_s);
    if (!dbPath.exists())
    {
        const Path backupPath = sourceDataDirectory / Path(u"BT_backup"_s);
        if (!QFileInfo(backupPath.data()).isDir())
            return nonstd::make_unexpected(tr("The selected data directory contains neither torrents.db nor BT_backup."));
        return BencodeResumeDataStorage::readExternal(backupPath, sourceProfileBase);
    }

#ifndef Q_OS_WIN
    return nonstd::make_unexpected(tr("A write-excluding external SQLite snapshot is currently supported only on Windows."));
#else
    // A read-only SQLite connection can still create or change its WAL index.
    // Freeze source DB/WAL files using Windows sharing rules and let SQLite read
    // only a disposable copy. Existing source writers make this operation fail.
    QTemporaryDir snapshot {QDir::tempPath() + u"/qbutt-profile-read-XXXXXX"};
    if (!snapshot.isValid())
        return nonstd::make_unexpected(tr("Cannot create a temporary directory for the external profile snapshot."));
    std::vector<std::unique_ptr<QFile>> sourceFiles;
    qint64 totalSize = 0;
    for (const QString &suffix : {QString(), u"-wal"_s, u"-journal"_s})
    {
        const Path sourcePath = dbPath + suffix;
        if (!suffix.isEmpty() && !sourcePath.exists())
            continue;
        QString nativePath = QDir::toNativeSeparators(sourcePath.data());
        if (!nativePath.startsWith(u"\\\\?\\"))
        {
            if (nativePath.startsWith(u"\\\\"))
                nativePath = u"\\\\?\\UNC\\" + nativePath.sliced(2);
            else
                nativePath = u"\\\\?\\" + nativePath;
        }
        const HANDLE handle = CreateFileW(reinterpret_cast<LPCWSTR>(nativePath.utf16()), GENERIC_READ
            , FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        if (handle == INVALID_HANDLE_VALUE)
            return nonstd::make_unexpected(tr("Cannot freeze the external resume database (Windows error %1). Close the source client and try again.")
                .arg(GetLastError()));
        auto closeHandle = qScopeGuard([handle] { CloseHandle(handle); });
        BY_HANDLE_FILE_INFORMATION attributes {};
        if (!GetFileInformationByHandle(handle, &attributes)
            || (attributes.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)))
        {
            return nonstd::make_unexpected(tr("An external database file is not a regular file."));
        }
        const int descriptor = _open_osfhandle(reinterpret_cast<intptr_t>(handle), _O_RDONLY | _O_BINARY);
        if (descriptor < 0)
            return nonstd::make_unexpected(tr("Cannot read the external resume database snapshot."));
        closeHandle.dismiss();
        auto closeDescriptor = qScopeGuard([descriptor] { _close(descriptor); });
        auto file = std::make_unique<QFile>();
        if (!file->open(descriptor, QIODevice::ReadOnly, QFileDevice::AutoCloseHandle))
            return nonstd::make_unexpected(tr("Cannot read the external resume database snapshot: %1").arg(file->errorString()));
        closeDescriptor.dismiss();
        const qint64 size = file->size();
        if ((size < 0) || (size > ExternalTotalSizeLimit - totalSize))
            return nonstd::make_unexpected(tr("The external database snapshot exceeds the import size limit."));
        if ((suffix == u"-journal") && (size != 0))
            return nonstd::make_unexpected(tr("The external database has an unfinished rollback journal. Recover it in the source client before import."));
        totalSize += size;
        QFile target {snapshot.filePath(u"torrents.db"_s + suffix)};
        if (!target.open(QIODevice::WriteOnly | QIODevice::NewOnly))
            return nonstd::make_unexpected(tr("Cannot write the temporary resume database snapshot: %1").arg(target.errorString()));
        qint64 copied = 0;
        while (copied < size)
        {
            const QByteArray bytes = file->read(std::min(qint64(1024 * 1024), size - copied));
            if (bytes.isEmpty() || (target.write(bytes) != bytes.size()))
                return nonstd::make_unexpected(tr("Cannot finish copying the external resume database snapshot."));
            copied += bytes.size();
        }
        if (!target.flush())
            return nonstd::make_unexpected(tr("Cannot flush the temporary resume database snapshot: %1").arg(target.errorString()));
        sourceFiles.push_back(std::move(file));
    }
    return DBResumeDataStorage::readExternalSnapshot(Path(snapshot.filePath(u"torrents.db"_s)), sourceProfileBase);
#endif
}

Path BitTorrent::ResumeDataStorage::path() const
{
    return m_path;
}

void BitTorrent::ResumeDataStorage::loadAll() const
{
    m_loadedResumeData.reserve(1024);

    auto *loadingThread = QThread::create([this]()
    {
        doLoadAll();
    });
    loadingThread->setObjectName("ResumeDataStorage::loadAll loadingThread");
    connect(loadingThread, &QThread::finished, loadingThread, &QObject::deleteLater);
    loadingThread->start();
}

QList<BitTorrent::LoadedResumeData> BitTorrent::ResumeDataStorage::fetchLoadedResumeData() const
{
    const QMutexLocker locker {&m_loadedResumeDataMutex};

    const QList<BitTorrent::LoadedResumeData> loadedResumeData = m_loadedResumeData;
    m_loadedResumeData.clear();

    return loadedResumeData;
}

void BitTorrent::ResumeDataStorage::onResumeDataLoaded(const TorrentID &torrentID, LoadResumeDataResult loadResumeDataResult) const
{
    const QMutexLocker locker {&m_loadedResumeDataMutex};
    m_loadedResumeData.append({.torrentID = torrentID, .result = std::move(loadResumeDataResult)});
}
