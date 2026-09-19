/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <atomic>

#include <libtorrent/fwd.hpp>

#include <QList>
#include <QMap>
#include <QString>
#include <QStringList>

namespace BitTorrent
{
    struct RepairPlanFile
    {
        int nativeIndex = -1;
        QString targetPath;
        QString sourcePath;
        qint64 expectedBytes = 0;
        qint64 candidateBytes = 0;
        qint64 verifiedBytes = 0;
        QStringList problems;
    };

    struct RepairPlan
    {
        QList<RepairPlanFile> files;
        QMap<int, QString> mappings;
        qint64 candidateBytes = 0;
        qint64 verifiedBytes = 0;
        qint64 requiredNetworkBytes = 0;
        qint64 temporaryStorageBytes = 0;
        qint64 availableStorageBytes = 0;
        int changedFiles = 0;
        int oversizedFiles = 0;
        QString error;
    };

    QMap<int, QString> findRepairSources(const lt::file_storage &files, const QString &destination
        , const QStringList &roots, const QMap<int, QString> &explicitMappings, QString &error
        , const std::atomic_bool *cancelled = nullptr);

    RepairPlan planRepairData(const lt::torrent_info &target, const lt::file_storage &files
        , const QString &destination, const QStringList &roots, const QMap<int, QString> &explicitMappings
        , const std::atomic_bool *cancelled = nullptr);
}
