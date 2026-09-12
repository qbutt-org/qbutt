/*
 * Bittorrent Client using Qt and libtorrent.
 * Copyright (C) 2021-2025  Vladimir Golovnev <glassez@yandex.ru>
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

#include "dbresumedatastorage.h"

#include <exception>
#include <memory>
#include <queue>
#include <utility>

#include <libtorrent/bdecode.hpp>
#include <libtorrent/bencode.hpp>
#include <libtorrent/entry.hpp>
#include <libtorrent/read_resume_data.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/write_resume_data.hpp>

#include <QByteArray>
#include <QDebug>
#include <QList>
#include <QMutex>
#include <QMutexLocker>
#include <QScopeGuard>
#include <QSet>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QSqlRecord>
#include <QThread>
#include <QUuid>
#include <QWaitCondition>

#include "base/exceptions.h"
#include "base/global.h"
#include "base/logger.h"
#include "base/path.h"
#include "base/preferences.h"
#include "base/profile.h"
#include "base/utils/fs.h"
#include "base/utils/sslkey.h"
#include "base/utils/string.h"
#include "infohash.h"
#include "loadtorrentparams.h"

namespace
{
    const QString DB_CONNECTION_NAME = u"ResumeDataStorage"_s;

    const int DB_VERSION = 9;

    const QString DB_TABLE_META = u"meta"_s;
    const QString DB_TABLE_TORRENTS = u"torrents"_s;

    const QString META_VERSION = u"version"_s;

    using namespace BitTorrent;

    class Job
    {
    public:
        virtual ~Job() = default;
        virtual bool perform(QSqlDatabase db) = 0;

        quint64 revision = 0;
    };

    class StoreJob final : public Job
    {
    public:
        StoreJob(const TorrentID &torrentID, LoadTorrentParams resumeData);
        bool perform(QSqlDatabase db) override;

    private:
        const TorrentID m_torrentID;
        const LoadTorrentParams m_resumeData;
    };

    class RemoveJob final : public Job
    {
    public:
        explicit RemoveJob(const TorrentID &torrentID);
        bool perform(QSqlDatabase db) override;

    private:
        const TorrentID m_torrentID;
    };

    class StoreQueueJob final : public Job
    {
    public:
        explicit StoreQueueJob(const QList<TorrentID> &queue);
        bool perform(QSqlDatabase db) override;

    private:
        const QList<TorrentID> m_queue;
    };

    struct Column
    {
        QString name;
        QString placeholder;
    };

    Column makeColumn(const QString &columnName)
    {
        return {.name = columnName, .placeholder = (u':' + columnName)};
    }

    const Column DB_COLUMN_ID = makeColumn(u"id"_s);
    const Column DB_COLUMN_TORRENT_ID = makeColumn(u"torrent_id"_s);
    const Column DB_COLUMN_QUEUE_POSITION = makeColumn(u"queue_position"_s);
    const Column DB_COLUMN_NAME = makeColumn(u"name"_s);
    const Column DB_COLUMN_CATEGORY = makeColumn(u"category"_s);
    const Column DB_COLUMN_TAGS = makeColumn(u"tags"_s);
    const Column DB_COLUMN_COMMENT = makeColumn(u"comment"_s);
    const Column DB_COLUMN_TARGET_SAVE_PATH = makeColumn(u"target_save_path"_s);
    const Column DB_COLUMN_DOWNLOAD_PATH = makeColumn(u"download_path"_s);
    const Column DB_COLUMN_CONTENT_LAYOUT = makeColumn(u"content_layout"_s);
    const Column DB_COLUMN_RATIO_LIMIT = makeColumn(u"ratio_limit"_s);
    const Column DB_COLUMN_SEEDING_TIME_LIMIT = makeColumn(u"seeding_time_limit"_s);
    const Column DB_COLUMN_INACTIVE_SEEDING_TIME_LIMIT = makeColumn(u"inactive_seeding_time_limit"_s);
    const Column DB_COLUMN_SHARE_LIMIT_ACTION = makeColumn(u"share_limit_action"_s);
    const Column DB_COLUMN_HAS_OUTER_PIECES_PRIORITY = makeColumn(u"has_outer_pieces_priority"_s);
    const Column DB_COLUMN_HAS_SEED_STATUS = makeColumn(u"has_seed_status"_s);
    const Column DB_COLUMN_OPERATING_MODE = makeColumn(u"operating_mode"_s);
    const Column DB_COLUMN_STOPPED = makeColumn(u"stopped"_s);
    const Column DB_COLUMN_STOP_CONDITION = makeColumn(u"stop_condition"_s);
    const Column DB_COLUMN_SSL_CERTIFICATE = makeColumn(u"ssl_certificate"_s);
    const Column DB_COLUMN_SSL_PRIVATE_KEY = makeColumn(u"ssl_private_key"_s);
    const Column DB_COLUMN_SSL_DH_PARAMS = makeColumn(u"ssl_dh_params"_s);
    const Column DB_COLUMN_RESUMEDATA = makeColumn(u"libtorrent_resume_data"_s);
    const Column DB_COLUMN_METADATA = makeColumn(u"metadata"_s);
    const Column DB_COLUMN_VALUE = makeColumn(u"value"_s);

    template <typename LTStr>
    QString fromLTString(const LTStr &str)
    {
        return QString::fromUtf8(str.data(), static_cast<qsizetype>(str.size()));
    }

    QString quoted(const QString &name)
    {
        const QChar quote = u'`';
        return (quote + name + quote);
    }

    QString makeCreateTableStatement(const QString &tableName, const QStringList &items)
    {
        return u"CREATE TABLE %1 (%2)"_s.arg(quoted(tableName), items.join(u','));
    }

    std::pair<QString, QString> joinColumns(const QList<Column> &columns)
    {
        qsizetype namesSize = columns.size();
        qsizetype valuesSize = columns.size();
        for (const Column &column : columns)
        {
            namesSize += column.name.size() + 2;
            valuesSize += column.placeholder.size();
        }

        QString names;
        names.reserve(namesSize);
        QString values;
        values.reserve(valuesSize);
        for (const Column &column : columns)
        {
            names.append(quoted(column.name) + u',');
            values.append(column.placeholder + u',');
        }
        names.chop(1);
        values.chop(1);

        return std::make_pair(names, values);
    }

    QString makeInsertStatement(const QString &tableName, const QList<Column> &columns)
    {
        const auto [names, values] = joinColumns(columns);
        return u"INSERT INTO %1 (%2) VALUES (%3)"_s
                .arg(quoted(tableName), names, values);
    }

    QString makeUpdateStatement(const QString &tableName, const QList<Column> &columns)
    {
        const auto [names, values] = joinColumns(columns);
        return u"UPDATE %1 SET (%2) = (%3)"_s
                .arg(quoted(tableName), names, values);
    }

    QString makeOnConflictUpdateStatement(const Column &constraint, const QList<Column> &columns)
    {
        const auto [names, values] = joinColumns(columns);
        return u" ON CONFLICT (%1) DO UPDATE SET (%2) = (%3)"_s
                .arg(quoted(constraint.name), names, values);
    }

    QString makeColumnDefinition(const Column &column, const QString &definition)
    {
        return u"%1 %2"_s.arg(quoted(column.name), definition);
    }
}

namespace BitTorrent
{
    class DBResumeDataStorage::Worker final : public QThread
    {
        Q_DISABLE_COPY_MOVE(Worker)

    public:
        Worker(const Path &dbPath, QReadWriteLock &dbLock, DBResumeDataStorage *storage);

        void run() override;
        void requestInterruption();

        void store(const TorrentID &id, LoadTorrentParams resumeData, quint64 revision);
        void remove(const TorrentID &id);
        void storeQueue(const QList<TorrentID> &queue);

    private:
        void addJob(std::unique_ptr<Job> job);

        const QString m_connectionName = u"ResumeDataStorageWorker"_s;
        const Path m_path;
        QReadWriteLock &m_dbLock;
        DBResumeDataStorage *const m_storage;

        std::queue<std::unique_ptr<Job>> m_jobs;
        QMutex m_jobsMutex;
        QWaitCondition m_waitCondition;
    };
}

BitTorrent::DBResumeDataStorage::DBResumeDataStorage(const Path &dbPath, QObject *parent)
    : ResumeDataStorage(dbPath, parent)
{
    const bool needCreateDB = !dbPath.exists();

    auto db = QSqlDatabase::addDatabase(u"QSQLITE"_s, DB_CONNECTION_NAME);
    db.setDatabaseName(dbPath.data());
    if (!db.open())
        throw RuntimeError(db.lastError().text());

    if (needCreateDB)
    {
        createDB();
    }
    else
    {
        const int dbVersion = (!db.record(DB_TABLE_TORRENTS).contains(DB_COLUMN_DOWNLOAD_PATH.name) ? 1 : currentDBVersion());
        if (dbVersion < DB_VERSION)
            updateDB(dbVersion);
    }

    m_asyncWorker = new Worker(dbPath, m_dbLock, this);
    m_asyncWorker->start();
}

BitTorrent::DBResumeDataStorage::~DBResumeDataStorage()
{
    m_asyncWorker->requestInterruption();
    m_asyncWorker->wait();
    QSqlDatabase::removeDatabase(DB_CONNECTION_NAME);
}

BitTorrent::ExternalResumeDataResult BitTorrent::DBResumeDataStorage::readExternalSnapshot(
    const Path &dbPath, const Path &sourceProfileBase)
{
    const QString connectionName = u"ExternalResumeData-"_s + QUuid::createUuid().toString(QUuid::WithoutBraces);
    const auto removeConnection = qScopeGuard([&connectionName] { QSqlDatabase::removeDatabase(connectionName); });
    auto db = QSqlDatabase::addDatabase(u"QSQLITE"_s, connectionName);
    db.setDatabaseName(dbPath.data());
    db.setConnectOptions(u"QSQLITE_OPEN_READONLY;QSQLITE_BUSY_TIMEOUT=1000"_s);
    if (!db.open())
        return nonstd::make_unexpected(tr("Cannot open the external resume database snapshot: %1").arg(db.lastError().text()));
    if (!db.transaction())
        return nonstd::make_unexpected(tr("Cannot read a consistent external resume database snapshot: %1").arg(db.lastError().text()));

    QSqlQuery query {db};
    if (!query.exec(u"PRAGMA quick_check(1)"_s) || !query.next() || (query.value(0).toString() != u"ok"))
        return nonstd::make_unexpected(tr("The external resume database failed its integrity check."));
    if (!query.exec(u"SELECT value FROM meta WHERE name = 'version'"_s) || !query.next())
        return nonstd::make_unexpected(tr("The external resume database has an unsupported schema: no version is available."));
    bool versionValid = false;
    const int version = query.value(0).toInt(&versionValid);
    if (!versionValid || (version != DB_VERSION))
        return nonstd::make_unexpected(tr("The external resume database schema is unsupported. Expected version %1; found %2.")
            .arg(DB_VERSION).arg(version));

    const QSqlRecord columns = db.record(DB_TABLE_TORRENTS);
    const QList<Column> requiredColumns = {
        DB_COLUMN_TORRENT_ID, DB_COLUMN_QUEUE_POSITION, DB_COLUMN_NAME, DB_COLUMN_CATEGORY, DB_COLUMN_TAGS,
        DB_COLUMN_COMMENT, DB_COLUMN_TARGET_SAVE_PATH, DB_COLUMN_DOWNLOAD_PATH, DB_COLUMN_CONTENT_LAYOUT,
        DB_COLUMN_RATIO_LIMIT, DB_COLUMN_SEEDING_TIME_LIMIT, DB_COLUMN_INACTIVE_SEEDING_TIME_LIMIT,
        DB_COLUMN_SHARE_LIMIT_ACTION, DB_COLUMN_HAS_OUTER_PIECES_PRIORITY, DB_COLUMN_HAS_SEED_STATUS,
        DB_COLUMN_OPERATING_MODE, DB_COLUMN_STOPPED, DB_COLUMN_STOP_CONDITION, DB_COLUMN_SSL_CERTIFICATE,
        DB_COLUMN_SSL_PRIVATE_KEY, DB_COLUMN_SSL_DH_PARAMS, DB_COLUMN_RESUMEDATA, DB_COLUMN_METADATA
    };
    for (const Column &column : requiredColumns)
    {
        if (!columns.contains(column.name))
            return nonstd::make_unexpected(tr("The external resume database schema is missing column %1.").arg(column.name));
    }

    if (!query.exec(u"SELECT * FROM torrents ORDER BY queue_position LIMIT %1"_s.arg(ExternalTorrentCountLimit + 1)))
        return nonstd::make_unexpected(tr("Cannot read the external resume database: %1").arg(query.lastError().text()));
    const auto resolvePath = [&sourceProfileBase](const Path &storedPath)
    {
        if (storedPath.isRelative() && !storedPath.isEmpty() && sourceProfileBase.isEmpty())
            throw RuntimeError(tr("Select the source profile base to resolve relative resume paths."));
        return (storedPath.isEmpty() || storedPath.isAbsolute()) ? storedPath : sourceProfileBase / storedPath;
    };
    QList<LoadedResumeData> result;
    QSet<TorrentID> identities;
    qint64 totalSize = 0;
    while (query.next())
    {
        if (result.size() >= ExternalTorrentCountLimit)
            return nonstd::make_unexpected(tr("The external profile exceeds the limit of %1 torrents.").arg(ExternalTorrentCountLimit));
        const TorrentID torrentID = TorrentID::fromString(query.value(DB_COLUMN_TORRENT_ID.name).toString());
        if (!torrentID.isValid() || identities.contains(torrentID))
            return nonstd::make_unexpected(tr("The external resume database contains an invalid or duplicate torrent ID."));
        identities.insert(torrentID);
        const qsizetype resumeSize = query.value(DB_COLUMN_RESUMEDATA.name).toByteArray().size();
        const qsizetype metadataSize = query.value(DB_COLUMN_METADATA.name).toByteArray().size();
        if ((resumeSize > ExternalFileSizeLimit) || (metadataSize > ExternalFileSizeLimit)
            || ((resumeSize + metadataSize) > ExternalTotalSizeLimit - totalSize))
        {
            return nonstd::make_unexpected(tr("The external resume data exceeds the import size limit."));
        }
        totalSize += resumeSize + metadataSize;

        LoadResumeDataResult parsed;
        try
        {
            parsed = parseQueryResultRow(query, resolvePath, ExternalDecodeDepthLimit, ExternalDecodeTokenLimit);
        }
        catch (const RuntimeError &error)
        {
            parsed = nonstd::make_unexpected(error.message());
        }
        if (parsed)
        {
            const auto &params = parsed->ltAddTorrentParams;
            const InfoHash infoHash {params.ti ? params.ti->info_hashes() : params.info_hashes};
            const bool mismatchingMetadata = params.ti
                && ((params.info_hashes.has_v1() && (params.info_hashes.v1 != params.ti->info_hashes().v1))
                    || (params.info_hashes.has_v2() && (params.info_hashes.v2 != params.ti->info_hashes().v2)));
            if (mismatchingMetadata || ((torrentID != TorrentID::fromInfoHash(infoHash))
                && (!infoHash.v1().isValid() || (torrentID != TorrentID::fromSHA1Hash(infoHash.v1())))))
            {
                parsed = nonstd::make_unexpected(tr("The database torrent ID or metadata does not match its resume info-hash."));
            }
        }
        result.append({.torrentID = torrentID, .result = std::move(parsed)});
    }
    if (query.lastError().isValid())
        return nonstd::make_unexpected(tr("Cannot finish reading the external resume database: %1").arg(query.lastError().text()));
    query.finish();
    if (!db.commit())
        return nonstd::make_unexpected(tr("Cannot finish the external resume database snapshot: %1").arg(db.lastError().text()));
    return result;
}

QList<BitTorrent::TorrentID> BitTorrent::DBResumeDataStorage::registeredTorrents() const
{
    const auto selectTorrentIDStatement = u"SELECT %1 FROM %2 ORDER BY %3;"_s
            .arg(quoted(DB_COLUMN_TORRENT_ID.name), quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_QUEUE_POSITION.name));

    auto db = QSqlDatabase::database(DB_CONNECTION_NAME);
    QSqlQuery query {db};

    if (!query.exec(selectTorrentIDStatement))
        throw RuntimeError(query.lastError().text());

    QList<TorrentID> registeredTorrents;
    registeredTorrents.reserve(query.size());
    while (query.next())
        registeredTorrents.append(BitTorrent::TorrentID::fromString(query.value(0).toString()));

    return registeredTorrents;
}

BitTorrent::LoadResumeDataResult BitTorrent::DBResumeDataStorage::load(const TorrentID &id) const
{
    const QString selectTorrentStatement = u"SELECT * FROM %1 WHERE %2 = %3;"_s
        .arg(quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_TORRENT_ID.name), DB_COLUMN_TORRENT_ID.placeholder);

    auto db = QSqlDatabase::database(DB_CONNECTION_NAME);
    QSqlQuery query {db};
    try
    {
        if (!query.prepare(selectTorrentStatement))
            throw RuntimeError(query.lastError().text());

        query.bindValue(DB_COLUMN_TORRENT_ID.placeholder, id.toString());
        if (!query.exec())
            throw RuntimeError(query.lastError().text());

        if (!query.next())
            throw RuntimeError(tr("Not found."));
    }
    catch (const RuntimeError &err)
    {
        return nonstd::make_unexpected(tr("Couldn't load resume data of torrent '%1'. Error: %2")
            .arg(id.toString(), err.message()));
    }

    return parseQueryResultRow(query);
}

void BitTorrent::DBResumeDataStorage::store(const TorrentID &id, LoadTorrentParams resumeData, const quint64 revision) const
{
    m_asyncWorker->store(id, std::move(resumeData), revision);
}

void BitTorrent::DBResumeDataStorage::remove(const BitTorrent::TorrentID &id) const
{
    m_asyncWorker->remove(id);
}

void BitTorrent::DBResumeDataStorage::storeQueue(const QList<TorrentID> &queue) const
{
    m_asyncWorker->storeQueue(queue);
}

void BitTorrent::DBResumeDataStorage::doLoadAll() const
{
    const QString connectionName = u"ResumeDataStorageLoadAll"_s;

    {
        auto db = QSqlDatabase::addDatabase(u"QSQLITE"_s, connectionName);
        db.setDatabaseName(path().data());
        if (!db.open())
            throw RuntimeError(db.lastError().text());

        QSqlQuery query {db};

        const auto selectTorrentIDStatement = u"SELECT %1 FROM %2 ORDER BY %3;"_s
                .arg(quoted(DB_COLUMN_TORRENT_ID.name), quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_QUEUE_POSITION.name));

        const QReadLocker locker {&m_dbLock};

        if (!query.exec(selectTorrentIDStatement))
            throw RuntimeError(query.lastError().text());

        QList<TorrentID> registeredTorrents;
        registeredTorrents.reserve(query.size());
        while (query.next())
            registeredTorrents.append(TorrentID::fromString(query.value(0).toString()));

        emit const_cast<DBResumeDataStorage *>(this)->loadStarted(registeredTorrents);

        const auto selectStatement = u"SELECT * FROM %1 ORDER BY %2;"_s.arg(quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_QUEUE_POSITION.name));
        if (!query.exec(selectStatement))
            throw RuntimeError(query.lastError().text());

        while (query.next())
        {
            const auto torrentID = TorrentID::fromString(query.value(DB_COLUMN_TORRENT_ID.name).toString());
            onResumeDataLoaded(torrentID, parseQueryResultRow(query));
        }
    }

    emit const_cast<DBResumeDataStorage *>(this)->loadFinished();

    QSqlDatabase::removeDatabase(connectionName);
}

int BitTorrent::DBResumeDataStorage::currentDBVersion() const
{
    const auto selectDBVersionStatement = u"SELECT %1 FROM %2 WHERE %3 = %4;"_s
            .arg(quoted(DB_COLUMN_VALUE.name), quoted(DB_TABLE_META), quoted(DB_COLUMN_NAME.name), DB_COLUMN_NAME.placeholder);

    auto db = QSqlDatabase::database(DB_CONNECTION_NAME);
    QSqlQuery query {db};

    if (!query.prepare(selectDBVersionStatement))
        throw RuntimeError(query.lastError().text());

    query.bindValue(DB_COLUMN_NAME.placeholder, META_VERSION);

    const QReadLocker locker {&m_dbLock};

    if (!query.exec())
        throw RuntimeError(query.lastError().text());

    if (!query.next())
        throw RuntimeError(tr("Database is corrupted."));

    bool ok;
    const int dbVersion = query.value(0).toInt(&ok);
    if (!ok)
        throw RuntimeError(tr("Database is corrupted."));

    return dbVersion;
}

void BitTorrent::DBResumeDataStorage::createDB() const
{
    try
    {
        enableWALMode();
    }
    catch (const RuntimeError &err)
    {
        LogMsg(tr("Couldn't enable Write-Ahead Logging (WAL) journaling mode. Error: %1.")
               .arg(err.message()), Log::WARNING);
    }

    auto db = QSqlDatabase::database(DB_CONNECTION_NAME);

    if (!db.transaction())
        throw RuntimeError(db.lastError().text());

    QSqlQuery query {db};

    try
    {
        const QStringList tableMetaItems = {
            makeColumnDefinition(DB_COLUMN_ID, u"INTEGER PRIMARY KEY"_s),
            makeColumnDefinition(DB_COLUMN_NAME, u"TEXT NOT NULL UNIQUE"_s),
            makeColumnDefinition(DB_COLUMN_VALUE, u"BLOB"_s)
        };
        const QString createTableMetaQuery = makeCreateTableStatement(DB_TABLE_META, tableMetaItems);
        if (!query.exec(createTableMetaQuery))
            throw RuntimeError(query.lastError().text());

        const QString insertMetaVersionQuery = makeInsertStatement(DB_TABLE_META, {DB_COLUMN_NAME, DB_COLUMN_VALUE});
        if (!query.prepare(insertMetaVersionQuery))
            throw RuntimeError(query.lastError().text());

        query.bindValue(DB_COLUMN_NAME.placeholder, META_VERSION);
        query.bindValue(DB_COLUMN_VALUE.placeholder, DB_VERSION);

        if (!query.exec())
            throw RuntimeError(query.lastError().text());

        const QStringList tableTorrentsItems = {
            makeColumnDefinition(DB_COLUMN_ID, u"INTEGER PRIMARY KEY"_s),
            makeColumnDefinition(DB_COLUMN_TORRENT_ID, u"BLOB NOT NULL UNIQUE"_s),
            makeColumnDefinition(DB_COLUMN_QUEUE_POSITION, u"INTEGER NOT NULL DEFAULT -1"_s),
            makeColumnDefinition(DB_COLUMN_NAME, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_CATEGORY, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_TAGS, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_COMMENT, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_TARGET_SAVE_PATH, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_DOWNLOAD_PATH, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_CONTENT_LAYOUT, u"TEXT NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_RATIO_LIMIT, u"INTEGER NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_SEEDING_TIME_LIMIT, u"INTEGER NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_INACTIVE_SEEDING_TIME_LIMIT, u"INTEGER NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_SHARE_LIMIT_ACTION, u"TEXT NOT NULL DEFAULT `Default`"_s),
            makeColumnDefinition(DB_COLUMN_HAS_OUTER_PIECES_PRIORITY, u"INTEGER NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_HAS_SEED_STATUS, u"INTEGER NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_OPERATING_MODE, u"TEXT NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_STOPPED, u"INTEGER NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_STOP_CONDITION, u"TEXT NOT NULL DEFAULT `None`"_s),
            makeColumnDefinition(DB_COLUMN_SSL_CERTIFICATE, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_SSL_PRIVATE_KEY, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_SSL_DH_PARAMS, u"TEXT"_s),
            makeColumnDefinition(DB_COLUMN_RESUMEDATA, u"BLOB NOT NULL"_s),
            makeColumnDefinition(DB_COLUMN_METADATA, u"BLOB"_s)
        };
        const QString createTableTorrentsQuery = makeCreateTableStatement(DB_TABLE_TORRENTS, tableTorrentsItems);
        if (!query.exec(createTableTorrentsQuery))
            throw RuntimeError(query.lastError().text());

        const QString torrentsQueuePositionIndexName = u"%1_%2_INDEX"_s.arg(DB_TABLE_TORRENTS, DB_COLUMN_QUEUE_POSITION.name);
        const QString createTorrentsQueuePositionIndexQuery = u"CREATE INDEX %1 ON %2 (%3)"_s
                .arg(quoted(torrentsQueuePositionIndexName), quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_QUEUE_POSITION.name));
        if (!query.exec(createTorrentsQueuePositionIndexQuery))
            throw RuntimeError(query.lastError().text());

        if (!db.commit())
            throw RuntimeError(db.lastError().text());
    }
    catch (const RuntimeError &)
    {
        db.rollback();
        throw;
    }
}

void BitTorrent::DBResumeDataStorage::updateDB(const int fromVersion) const
{
    Q_ASSERT(fromVersion > 0);
    Q_ASSERT(fromVersion != DB_VERSION);

    auto db = QSqlDatabase::database(DB_CONNECTION_NAME);

    const QWriteLocker locker {&m_dbLock};

    if (!db.transaction())
        throw RuntimeError(db.lastError().text());

    QSqlQuery query {db};

    try
    {
        const auto addColumn = [&query](const QString &table, const Column &column, const QString &definition)
        {
            const auto testQuery = u"SELECT COUNT(%1) FROM %2;"_s.arg(quoted(column.name), quoted(table));
            if (query.exec(testQuery))
                return;

            const auto alterTableQuery = u"ALTER TABLE %1 ADD %2"_s.arg(quoted(table), makeColumnDefinition(column, definition));
            if (!query.exec(alterTableQuery))
                throw RuntimeError(query.lastError().text());
        };

        if (fromVersion <= 1)
            addColumn(DB_TABLE_TORRENTS, DB_COLUMN_DOWNLOAD_PATH, u"TEXT"_s);

        if (fromVersion <= 2)
            addColumn(DB_TABLE_TORRENTS, DB_COLUMN_STOP_CONDITION, u"TEXT NOT NULL DEFAULT `None`"_s);

        if (fromVersion <= 3)
        {
            const QString torrentsQueuePositionIndexName = u"%1_%2_INDEX"_s.arg(DB_TABLE_TORRENTS, DB_COLUMN_QUEUE_POSITION.name);
            const QString createTorrentsQueuePositionIndexQuery = u"CREATE INDEX IF NOT EXISTS %1 ON %2 (%3)"_s
                    .arg(quoted(torrentsQueuePositionIndexName), quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_QUEUE_POSITION.name));
            if (!query.exec(createTorrentsQueuePositionIndexQuery))
                throw RuntimeError(query.lastError().text());
        }

        if (fromVersion <= 4)
            addColumn(DB_TABLE_TORRENTS, DB_COLUMN_INACTIVE_SEEDING_TIME_LIMIT, u"INTEGER NOT NULL DEFAULT -2"_s);

        if (fromVersion <= 5)
        {
            addColumn(DB_TABLE_TORRENTS, DB_COLUMN_SSL_CERTIFICATE, u"TEXT"_s);
            addColumn(DB_TABLE_TORRENTS, DB_COLUMN_SSL_PRIVATE_KEY, u"TEXT"_s);
            addColumn(DB_TABLE_TORRENTS, DB_COLUMN_SSL_DH_PARAMS, u"TEXT"_s);
        }

        if (fromVersion <= 6)
            addColumn(DB_TABLE_TORRENTS, DB_COLUMN_SHARE_LIMIT_ACTION, u"TEXT NOT NULL DEFAULT `Default`"_s);

        if (fromVersion == 7)
        {
            const QString TEMP_COLUMN_NAME = DB_COLUMN_SHARE_LIMIT_ACTION.name + u"_temp";

            auto queryStr = u"ALTER TABLE %1 ADD %2 %3"_s
                    .arg(quoted(DB_TABLE_TORRENTS), TEMP_COLUMN_NAME, u"TEXT NOT NULL DEFAULT `Default`");
            if (!query.exec(queryStr))
                throw RuntimeError(query.lastError().text());

            queryStr = u"UPDATE %1 SET %2 = %3"_s
                    .arg(quoted(DB_TABLE_TORRENTS), quoted(TEMP_COLUMN_NAME), quoted(DB_COLUMN_SHARE_LIMIT_ACTION.name));
            if (!query.exec(queryStr))
                throw RuntimeError(query.lastError().text());

            queryStr = u"ALTER TABLE %1 DROP %2"_s.arg(quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_SHARE_LIMIT_ACTION.name));
            if (!query.exec(queryStr))
                throw RuntimeError(query.lastError().text());

            queryStr = u"ALTER TABLE %1 RENAME %2 TO %3"_s
                    .arg(quoted(DB_TABLE_TORRENTS), quoted(TEMP_COLUMN_NAME), quoted(DB_COLUMN_SHARE_LIMIT_ACTION.name));
            if (!query.exec(queryStr))
                throw RuntimeError(query.lastError().text());
        }

        if (fromVersion <= 8)
            addColumn(DB_TABLE_TORRENTS, DB_COLUMN_COMMENT, u"TEXT"_s);

        const QString updateMetaVersionQuery = makeUpdateStatement(DB_TABLE_META, {DB_COLUMN_NAME, DB_COLUMN_VALUE});
        if (!query.prepare(updateMetaVersionQuery))
            throw RuntimeError(query.lastError().text());

        query.bindValue(DB_COLUMN_NAME.placeholder, META_VERSION);
        query.bindValue(DB_COLUMN_VALUE.placeholder, DB_VERSION);

        if (!query.exec())
            throw RuntimeError(query.lastError().text());

        if (!db.commit())
            throw RuntimeError(db.lastError().text());
    }
    catch (const RuntimeError &)
    {
        db.rollback();
        throw;
    }
}

void BitTorrent::DBResumeDataStorage::enableWALMode() const
{
    auto db = QSqlDatabase::database(DB_CONNECTION_NAME);
    QSqlQuery query {db};

    if (!query.exec(u"PRAGMA journal_mode = WAL;"_s))
        throw RuntimeError(query.lastError().text());

    if (!query.next())
        throw RuntimeError(tr("Couldn't obtain query result."));

    const QString result = query.value(0).toString();
    if (result.compare(u"WAL"_s, Qt::CaseInsensitive) != 0)
        throw RuntimeError(tr("WAL mode is probably unsupported due to filesystem limitations."));
}

LoadResumeDataResult DBResumeDataStorage::parseQueryResultRow(const QSqlQuery &query) const
{
    const auto *pref = Preferences::instance();
    return parseQueryResultRow(query
        , [](const Path &path) { return Profile::instance()->fromPortablePath(path); }
        , pref->getBdecodeDepthLimit(), pref->getBdecodeTokenLimit());
}

LoadResumeDataResult DBResumeDataStorage::parseQueryResultRow(const QSqlQuery &query
    , const ResumeDataPathResolver &resolvePath, const int depthLimit, const int tokenLimit)
{
    LoadTorrentParams resumeData;
    resumeData.name = query.value(DB_COLUMN_NAME.name).toString();
    resumeData.category = query.value(DB_COLUMN_CATEGORY.name).toString();
    resumeData.comment = query.value(DB_COLUMN_COMMENT.name).toString();
    const QString tagsData = query.value(DB_COLUMN_TAGS.name).toString();
    if (!tagsData.isEmpty())
    {
        const QStringList tagList = tagsData.split(u',');
        resumeData.tags.insert(tagList.cbegin(), tagList.cend());
    }
    resumeData.hasFinishedStatus = query.value(DB_COLUMN_HAS_SEED_STATUS.name).toBool();
    resumeData.firstLastPiecePriority = query.value(DB_COLUMN_HAS_OUTER_PIECES_PRIORITY.name).toBool();
    resumeData.ratioLimit = query.value(DB_COLUMN_RATIO_LIMIT.name).toInt() / 1000.0;
    resumeData.seedingTimeLimit = query.value(DB_COLUMN_SEEDING_TIME_LIMIT.name).toInt();
    resumeData.inactiveSeedingTimeLimit = query.value(DB_COLUMN_INACTIVE_SEEDING_TIME_LIMIT.name).toInt();
    resumeData.shareLimitAction = Utils::String::toEnum<ShareLimitAction>(
        query.value(DB_COLUMN_SHARE_LIMIT_ACTION.name).toString(), ShareLimitAction::Default);
    resumeData.contentLayout = Utils::String::toEnum<TorrentContentLayout>(
        query.value(DB_COLUMN_CONTENT_LAYOUT.name).toString(), TorrentContentLayout::Original);
    resumeData.operatingMode = Utils::String::toEnum<TorrentOperatingMode>(
        query.value(DB_COLUMN_OPERATING_MODE.name).toString(), TorrentOperatingMode::AutoManaged);
    resumeData.stopped = query.value(DB_COLUMN_STOPPED.name).toBool();
    resumeData.stopCondition = Utils::String::toEnum(
        query.value(DB_COLUMN_STOP_CONDITION.name).toString(), Torrent::StopCondition::None);
    resumeData.sslParameters =
        {
            .certificate = QSslCertificate(query.value(DB_COLUMN_SSL_CERTIFICATE.name).toByteArray()),
            .privateKey = Utils::SSLKey::load(query.value(DB_COLUMN_SSL_PRIVATE_KEY.name).toByteArray()),
            .dhParams = query.value(DB_COLUMN_SSL_DH_PARAMS.name).toByteArray()
        };

    resumeData.savePath = resolvePath(
        Path(query.value(DB_COLUMN_TARGET_SAVE_PATH.name).toString()));
    resumeData.useAutoTMM = resumeData.savePath.isEmpty();
    if (!resumeData.useAutoTMM)
    {
        resumeData.downloadPath = resolvePath(
            Path(query.value(DB_COLUMN_DOWNLOAD_PATH.name).toString()));
    }

    const QByteArray bencodedResumeData = query.value(DB_COLUMN_RESUMEDATA.name).toByteArray();
    lt::error_code ec;
    const lt::bdecode_node resumeDataRoot = lt::bdecode(bencodedResumeData, ec, nullptr, depthLimit, tokenLimit);
    if (ec)
        return nonstd::make_unexpected(tr("Cannot parse resume data: %1").arg(QString::fromStdString(ec.message())));

    lt::add_torrent_params &p = resumeData.ltAddTorrentParams;

    resumeData.completionPolicyPreview = resumeDataRoot.dict_find_int_value("qbutt-completion-policy-preview");

    p = lt::read_resume_data(resumeDataRoot, ec);
    if (ec)
        return nonstd::make_unexpected(tr("Cannot parse resume data: %1").arg(QString::fromStdString(ec.message())));

    if (const QByteArray bencodedMetadata = query.value(DB_COLUMN_METADATA.name).toByteArray()
            ; !bencodedMetadata.isEmpty())
    {
        const lt::bdecode_node torrentInfoRoot = lt::bdecode(bencodedMetadata, ec
                , nullptr, depthLimit, tokenLimit);
        if (ec)
            return nonstd::make_unexpected(tr("Cannot parse torrent info: %1").arg(QString::fromStdString(ec.message())));

        p.ti = std::make_shared<lt::torrent_info>(torrentInfoRoot, ec);
        if (ec)
            return nonstd::make_unexpected(tr("Cannot parse torrent info: %1").arg(QString::fromStdString(ec.message())));
    }

    p.save_path = resolvePath(Path(fromLTString(p.save_path)))
            .toString().toStdString();
    if (p.save_path.empty())
        return nonstd::make_unexpected(tr("Corrupted resume data: %1").arg(tr("save_path is invalid")));

    if (p.flags & lt::torrent_flags::stop_when_ready)
    {
        p.flags &= ~lt::torrent_flags::stop_when_ready;
        resumeData.stopCondition = Torrent::StopCondition::FilesChecked;
    }

    return resumeData;
}

BitTorrent::DBResumeDataStorage::Worker::Worker(const Path &dbPath, QReadWriteLock &dbLock, DBResumeDataStorage *storage)
    : QThread(storage)
    , m_path {dbPath}
    , m_dbLock {dbLock}
    , m_storage {storage}
{
}

void BitTorrent::DBResumeDataStorage::Worker::run()
{
    {
        auto db = QSqlDatabase::addDatabase(u"QSQLITE"_s, m_connectionName);
        db.setDatabaseName(m_path.data());
        if (!db.open())
            LogMsg(tr("Couldn't open resume data database. Error: %1").arg(db.lastError().text()), Log::CRITICAL);

        while (true)
        {
            std::queue<std::unique_ptr<Job>> batch;
            {
                QMutexLocker jobsLocker {&m_jobsMutex};
                while (m_jobs.empty() && !isInterruptionRequested())
                    m_waitCondition.wait(&m_jobsMutex);
                if (m_jobs.empty())
                    break;
                batch.swap(m_jobs);
            }

            QList<QPair<quint64, bool>> receipts;
            bool committed = false;
            {
                const QWriteLocker dbLocker {&m_dbLock};
                const bool transacting = db.isOpen() && db.transaction();
                if (!transacting)
                    LogMsg(tr("Save resume data transaction failed. Error: %1").arg(db.lastError().text()), Log::CRITICAL);

                while (!batch.empty())
                {
                    std::unique_ptr<Job> job = std::move(batch.front());
                    batch.pop();
                    bool success = false;
                    if (transacting)
                    {
                        try
                        {
                            success = job->perform(db);
                        }
                        catch (const std::exception &error)
                        {
                            LogMsg(tr("Couldn't store resume data. Error: %1")
                                    .arg(QString::fromLocal8Bit(error.what())), Log::CRITICAL);
                        }
                    }
                    if (job->revision != 0)
                        receipts.append({job->revision, success});
                }

                if (transacting)
                {
                    committed = db.commit();
                    if (!committed)
                    {
                        LogMsg(tr("Commit resume data transaction failed. Error: %1").arg(db.lastError().text()), Log::CRITICAL);
                        db.rollback();
                    }
                }
            }
            for (const auto &[revision, success] : asConst(receipts))
                emit m_storage->stored(revision, success && committed);
        }

        db.close();
    }

    QSqlDatabase::removeDatabase(m_connectionName);
}

void DBResumeDataStorage::Worker::requestInterruption()
{
    const QMutexLocker jobsLocker {&m_jobsMutex};
    QThread::requestInterruption();
    m_waitCondition.wakeAll();
}

void BitTorrent::DBResumeDataStorage::Worker::store(const TorrentID &id, LoadTorrentParams resumeData, const quint64 revision)
{
    auto job = std::make_unique<StoreJob>(id, std::move(resumeData));
    job->revision = revision;
    addJob(std::move(job));
}

void BitTorrent::DBResumeDataStorage::Worker::remove(const TorrentID &id)
{
    addJob(std::make_unique<RemoveJob>(id));
}

void BitTorrent::DBResumeDataStorage::Worker::storeQueue(const QList<TorrentID> &queue)
{
    addJob(std::make_unique<StoreQueueJob>(queue));
}

void BitTorrent::DBResumeDataStorage::Worker::addJob(std::unique_ptr<Job> job)
{
    m_jobsMutex.lock();
    m_jobs.push(std::move(job));
    m_jobsMutex.unlock();

    m_waitCondition.wakeAll();
}

namespace
{
    using namespace BitTorrent;

StoreJob::StoreJob(const TorrentID &torrentID, LoadTorrentParams resumeData)
        : m_torrentID {torrentID}
        , m_resumeData {std::move(resumeData)}
    {
    }

    bool StoreJob::perform(QSqlDatabase db)
    {
        // We need to adjust native libtorrent resume data
        lt::add_torrent_params p = m_resumeData.ltAddTorrentParams;
        p.save_path = Profile::instance()->toPortablePath(Path(p.save_path))
                .toString().toStdString();
        if (m_resumeData.stopped)
        {
            p.flags |= lt::torrent_flags::paused;
            p.flags &= ~lt::torrent_flags::auto_managed;
        }
        else
        {
            // Torrent can be actually "running" but temporarily "paused" to perform some
            // service jobs behind the scenes so we need to restore it as "running"
            if (m_resumeData.operatingMode == BitTorrent::TorrentOperatingMode::AutoManaged)
            {
                p.flags |= lt::torrent_flags::auto_managed;
            }
            else
            {
                p.flags &= ~lt::torrent_flags::paused;
                p.flags &= ~lt::torrent_flags::auto_managed;
            }
        }

        QList<Column> columns {
            DB_COLUMN_TORRENT_ID,
            DB_COLUMN_NAME,
            DB_COLUMN_CATEGORY,
            DB_COLUMN_TAGS,
            DB_COLUMN_COMMENT,
            DB_COLUMN_TARGET_SAVE_PATH,
            DB_COLUMN_DOWNLOAD_PATH,
            DB_COLUMN_CONTENT_LAYOUT,
            DB_COLUMN_RATIO_LIMIT,
            DB_COLUMN_SEEDING_TIME_LIMIT,
            DB_COLUMN_INACTIVE_SEEDING_TIME_LIMIT,
            DB_COLUMN_SHARE_LIMIT_ACTION,
            DB_COLUMN_HAS_OUTER_PIECES_PRIORITY,
            DB_COLUMN_HAS_SEED_STATUS,
            DB_COLUMN_OPERATING_MODE,
            DB_COLUMN_STOPPED,
            DB_COLUMN_STOP_CONDITION,
            DB_COLUMN_SSL_CERTIFICATE,
            DB_COLUMN_SSL_PRIVATE_KEY,
            DB_COLUMN_SSL_DH_PARAMS,
            DB_COLUMN_RESUMEDATA
        };

        lt::entry data = lt::write_resume_data(p);
        data["qbutt-completion-policy-preview"] = m_resumeData.completionPolicyPreview;

        // metadata is stored in separate column
        QByteArray bencodedMetadata;
        if (p.ti)
        {
            lt::entry::dictionary_type &dataDict = data.dict();
            lt::entry metadata {lt::entry::dictionary_t};
            lt::entry::dictionary_type &metadataDict = metadata.dict();
            metadataDict.insert(dataDict.extract("info"));
            metadataDict.insert(dataDict.extract("creation date"));
            metadataDict.insert(dataDict.extract("created by"));
            metadataDict.insert(dataDict.extract("comment"));

            try
            {
                bencodedMetadata.reserve(512 * 1024);
                lt::bencode(std::back_inserter(bencodedMetadata), metadata);
            }
            catch (const std::exception &err)
            {
                LogMsg(ResumeDataStorage::tr("Couldn't save torrent metadata. Error: %1.")
                        .arg(QString::fromLocal8Bit(err.what())), Log::CRITICAL);
                return false;
            }

            columns.append(DB_COLUMN_METADATA);
        }

        QByteArray bencodedResumeData;
        bencodedResumeData.reserve(256 * 1024);
        lt::bencode(std::back_inserter(bencodedResumeData), data);

        const QString insertTorrentStatement = makeInsertStatement(DB_TABLE_TORRENTS, columns)
                + makeOnConflictUpdateStatement(DB_COLUMN_TORRENT_ID, columns);
        QSqlQuery query {db};

        try
        {
            if (!query.prepare(insertTorrentStatement))
                throw RuntimeError(query.lastError().text());

            query.bindValue(DB_COLUMN_TORRENT_ID.placeholder, m_torrentID.toString());
            query.bindValue(DB_COLUMN_NAME.placeholder, m_resumeData.name);
            query.bindValue(DB_COLUMN_CATEGORY.placeholder, m_resumeData.category);
            query.bindValue(DB_COLUMN_TAGS.placeholder, (m_resumeData.tags.isEmpty()
                    ? QString() : Utils::String::joinIntoString(m_resumeData.tags, u","_s)));
            query.bindValue(DB_COLUMN_COMMENT.placeholder, m_resumeData.comment);
            query.bindValue(DB_COLUMN_CONTENT_LAYOUT.placeholder, Utils::String::fromEnum(m_resumeData.contentLayout));
            query.bindValue(DB_COLUMN_RATIO_LIMIT.placeholder, static_cast<int>(m_resumeData.ratioLimit * 1000));
            query.bindValue(DB_COLUMN_SEEDING_TIME_LIMIT.placeholder, m_resumeData.seedingTimeLimit);
            query.bindValue(DB_COLUMN_INACTIVE_SEEDING_TIME_LIMIT.placeholder, m_resumeData.inactiveSeedingTimeLimit);
            query.bindValue(DB_COLUMN_SHARE_LIMIT_ACTION.placeholder, Utils::String::fromEnum(m_resumeData.shareLimitAction));
            query.bindValue(DB_COLUMN_HAS_OUTER_PIECES_PRIORITY.placeholder, m_resumeData.firstLastPiecePriority);
            query.bindValue(DB_COLUMN_HAS_SEED_STATUS.placeholder, m_resumeData.hasFinishedStatus);
            query.bindValue(DB_COLUMN_OPERATING_MODE.placeholder, Utils::String::fromEnum(m_resumeData.operatingMode));
            query.bindValue(DB_COLUMN_STOPPED.placeholder, m_resumeData.stopped);
            query.bindValue(DB_COLUMN_STOP_CONDITION.placeholder, Utils::String::fromEnum(m_resumeData.stopCondition));
            query.bindValue(DB_COLUMN_SSL_CERTIFICATE.placeholder, QString::fromLatin1(m_resumeData.sslParameters.certificate.toPem()));
            query.bindValue(DB_COLUMN_SSL_PRIVATE_KEY.placeholder, QString::fromLatin1(m_resumeData.sslParameters.privateKey.toPem()));
            query.bindValue(DB_COLUMN_SSL_DH_PARAMS.placeholder, QString::fromLatin1(m_resumeData.sslParameters.dhParams));

            if (!m_resumeData.useAutoTMM)
            {
                query.bindValue(DB_COLUMN_TARGET_SAVE_PATH.placeholder, Profile::instance()->toPortablePath(m_resumeData.savePath).data());
                query.bindValue(DB_COLUMN_DOWNLOAD_PATH.placeholder, Profile::instance()->toPortablePath(m_resumeData.downloadPath).data());
            }

            query.bindValue(DB_COLUMN_RESUMEDATA.placeholder, bencodedResumeData);
            if (!bencodedMetadata.isEmpty())
                query.bindValue(DB_COLUMN_METADATA.placeholder, bencodedMetadata);

            if (!query.exec())
                throw RuntimeError(query.lastError().text());
        }
        catch (const RuntimeError &err)
        {
            LogMsg(ResumeDataStorage::tr("Couldn't store resume data for torrent '%1'. Error: %2")
                    .arg(m_torrentID.toString(), err.message()), Log::CRITICAL);
            return false;
        }
        return true;
    }

    RemoveJob::RemoveJob(const TorrentID &torrentID)
        : m_torrentID {torrentID}
    {
    }

    bool RemoveJob::perform(QSqlDatabase db)
    {
        const auto deleteTorrentStatement = u"DELETE FROM %1 WHERE %2 = %3;"_s
                .arg(quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_TORRENT_ID.name), DB_COLUMN_TORRENT_ID.placeholder);

        QSqlQuery query {db};
        try
        {
            if (!query.prepare(deleteTorrentStatement))
                throw RuntimeError(query.lastError().text());

            query.bindValue(DB_COLUMN_TORRENT_ID.placeholder, m_torrentID.toString());

            if (!query.exec())
                throw RuntimeError(query.lastError().text());
        }
        catch (const RuntimeError &err)
        {
            LogMsg(ResumeDataStorage::tr("Couldn't delete resume data of torrent '%1'. Error: %2")
                    .arg(m_torrentID.toString(), err.message()), Log::CRITICAL);
            return false;
        }
        return true;
    }

    StoreQueueJob::StoreQueueJob(const QList<TorrentID> &queue)
        : m_queue {queue}
    {
    }

    bool StoreQueueJob::perform(QSqlDatabase db)
    {
        const auto updateQueuePosStatement = u"UPDATE %1 SET %2 = %3 WHERE %4 = %5;"_s
                .arg(quoted(DB_TABLE_TORRENTS), quoted(DB_COLUMN_QUEUE_POSITION.name), DB_COLUMN_QUEUE_POSITION.placeholder
                        , quoted(DB_COLUMN_TORRENT_ID.name), DB_COLUMN_TORRENT_ID.placeholder);

        try
        {
            QSqlQuery query {db};

            if (!query.prepare(updateQueuePosStatement))
                throw RuntimeError(query.lastError().text());

            int pos = 0;
            for (const TorrentID &torrentID : m_queue)
            {
                query.bindValue(DB_COLUMN_TORRENT_ID.placeholder, torrentID.toString());
                query.bindValue(DB_COLUMN_QUEUE_POSITION.placeholder, pos++);
                if (!query.exec())
                    throw RuntimeError(query.lastError().text());
            }
        }
        catch (const RuntimeError &err)
        {
            LogMsg(ResumeDataStorage::tr("Couldn't store torrents queue positions. Error: %1")
                    .arg(err.message()), Log::CRITICAL);
            return false;
        }
        return true;
    }
}
