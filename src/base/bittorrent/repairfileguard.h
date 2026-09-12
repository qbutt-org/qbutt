/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <atomic>
#include <memory>
#include <vector>

#include <libtorrent/fwd.hpp>

#include <QByteArray>
#include <QMap>
#include <QSet>
#include <QString>

namespace BitTorrent
{
    // Resolve aliases in the existing prefix even when the target is missing.
    QString repairPathIdentity(const QString &path);

    // Owns the handles used to validate and modify a repair data set. Missing
    // files are left for libtorrent; files outside the mapping are never opened.
    class RepairFileGuard
    {
    public:
        // renameChildren permits modification through retained directory handles
        // during commit. Directory replacement is still denied; callers must
        // use handle-relative renames, never traverse these paths again.
        static std::shared_ptr<RepairFileGuard> open(const lt::file_storage &files
            , const QString &savePath, bool writable, QString &error, const std::atomic_bool *cancelled = nullptr
            , bool renameChildren = false);
        ~RepairFileGuard();

        QByteArray identity() const;
        QSet<int> existingFiles() const;
        bool truncateOversized(QString &error, const std::atomic_bool *cancelled = nullptr);
        // Permit engine file I/O while retaining directory identities until recheck completes.
        void releaseFiles();
        void *directoryHandle(const QString &path) const;

    private:
        RepairFileGuard() = default;

        struct File
        {
            void *handle = nullptr;
            int nativeIndex = -1;
            QString path;
            qint64 expectedSize = 0;
            qint64 actualSize = 0;
        };

        QMap<QString, void *> m_directoryHandles;
        std::vector<File> m_files;
        QByteArray m_identity;
        bool m_writable = false;
    };
}
