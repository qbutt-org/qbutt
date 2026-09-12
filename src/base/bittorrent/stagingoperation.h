/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <atomic>
#include <memory>

#include <libtorrent/file_storage.hpp>

#include <QJsonObject>
#include <QList>
#include <QMap>
#include <QSet>
#include <QStringList>

#include "repairanalysis.h"

namespace BitTorrent
{
    class RepairFileGuard;

    // One durable operation owns the staging payload and backups. Target file
    // paths are relative to the caller's target torrent file mapping.
    // The caller must stop and drain its sole libtorrent writer before prepare,
    // verify, commit or rollback. Downloading is allowed only in payloadPath().
    class StagingOperation
    {
    public:
        static QString journalPath(const QString &torrentId);
        static QString pendingDestination(const QString &torrentId);
        static std::unique_ptr<StagingOperation> plan(const QString &journalPath, const QString &torrentId
            , const lt::torrent_info &target, const lt::file_storage &files, const QString &destination
            , const QMap<int, QString> &sources, const QSet<int> &selected, QString &error, const std::atomic_bool *cancelled = nullptr);
        static std::unique_ptr<StagingOperation> load(const QString &journalPath, const QString &torrentId
            , const lt::file_storage &files, QString &error);

        bool prepare(QString &error, const std::atomic_bool *cancelled = nullptr);
        bool verify(const lt::torrent_info &target, QString &error, const std::atomic_bool *cancelled = nullptr);
        bool commit(const lt::torrent_info &target, QString &error, const std::atomic_bool *cancelled = nullptr);
        bool rollback(QString &error, const std::atomic_bool *cancelled = nullptr);
        bool finish(QString &error);

        QString destination() const;
        QString payloadPath() const;
        QString state() const;
        QJsonObject status() const;
        const RepairAnalysis &analysis() const;

    private:
        StagingOperation() = default;
        bool save(QString &error);
        bool transact(bool rollback, QString &error, const std::atomic_bool *cancelled, const lt::torrent_info *target = nullptr);

        QString m_journalPath;
        lt::file_storage m_files;
        QJsonObject m_journal;
        RepairAnalysis m_analysis;
        QList<std::shared_ptr<RepairFileGuard>> m_sourceGuards;
    };
}
