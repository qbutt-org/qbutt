/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <atomic>

#include <libtorrent/fwd.hpp>

#include <QMap>
#include <QString>
#include <QStringList>

namespace BitTorrent
{
    QMap<int, QString> findRepairSources(const lt::file_storage &files, const QString &destination
        , const QStringList &roots, const QMap<int, QString> &explicitMappings, QString &error
        , const std::atomic_bool *cancelled = nullptr);
}
