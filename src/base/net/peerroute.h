/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QString>

namespace Net
{
    // Process-local payload endpoint. Credentials never enter diagnostics or settings.
    struct PeerRouteEndpoint
    {
        enum class Type
        {
            Blocked,
            Socks5,
            Native
        };

        Type type = Type::Blocked;
        quint64 pathId = 0;
        quint64 generation = 0;
        quint16 port = 0;
        QString username;
        QString password;
        QString localAddress;
        quint32 interfaceIndex = 0;
    };
}
