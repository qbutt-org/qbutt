/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QList>
#include <QStringList>
#include <QVariantMap>

#include "base/3rdparty/expected.hpp"
#include "base/bittorrent/infohash.h"
#include "base/bittorrent/loadtorrentparams.h"
#include "base/path.h"

struct ProfileImportTorrent
{
    BitTorrent::TorrentID id;
    BitTorrent::LoadTorrentParams params;
    Path sourcePath;
    Path destinationPath;
    bool selected = true;
};

struct ProfileImportPreview
{
    QList<ProfileImportTorrent> torrents;
    QVariantMap settings;
    QStringList skippedSettings;
    QStringList protectedDirectories;
};

// Import uses native resume records. The manifest is only a recoverable file
// transaction, never a second owner of the running session's state.
class ProfileImport
{
public:
    static nonstd::expected<ProfileImportPreview, QString> preview(const Path &settingsFile
        , const Path &dataDirectory, const Path &sourceBase);
    static nonstd::expected<void, QString> prepare(ProfileImportPreview preview, bool ownershipConfirmed);

    // Called with the profile instance lock held, before settings or the native
    // session opens its files. An interrupted installation is rolled back.
    static nonstd::expected<void, QString> recover();
};
