/*
 * qbutt, a native Qt BitTorrent client.
 * Copyright (C) 2026 qbutt contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#pragma once

#include <atomic>

#include <libtorrent/fwd.hpp>

#include <QList>
#include <QMap>
#include <QSet>
#include <QString>
#include <QStringList>

namespace BitTorrent
{
    struct RepairFileAnalysis
    {
        int nativeIndex = -1;
        QString path;
        qint64 expectedSize = 0;
        qint64 actualSize = -1;
        qint64 verifiedBytes = 0;
        bool selected = true;
        QStringList problems;
    };

    struct RepairAnalysis
    {
        QList<RepairFileAnalysis> files;
        qint64 expectedBytes = 0;
        qint64 verifiedBytes = 0;
        int validPieces = 0;
        int unverifiedPieces = 0;
        bool wholeFileV2Verification = false;
        QString error;
    };

    // The caller must validate mappings and keep the data protected against
    // concurrent writes and path replacement throughout this read-only scan.
    // readableFiles freezes the native file indexes present in that snapshot;
    // absent files are not opened even if they appear during analysis.
    // readDescriptors optionally binds reads to caller-owned open files instead
    // of reopening names. Descriptors remain borrowed and must permit seeking.
    RepairAnalysis analyzeRepairData(const lt::torrent_info &target
        , const lt::file_storage &mappedFiles, const QString &savePath
        , const std::atomic_bool *cancelled = nullptr, const QSet<int> *readableFiles = nullptr
        , const QMap<int, int> *readDescriptors = nullptr);
}
