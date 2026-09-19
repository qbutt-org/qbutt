/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QString>

namespace Net
{
    enum class RoutePolicy
    {
        Pinned,
        Mixed,
        TunnelsOnly
    };

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
        QString publicAddress;
        quint16 publicPort = 0;
        quint32 interfaceIndex = 0;
        bool supportsIPv4 = true;
        bool supportsIPv6 = true;
        bool supportsUdp = false;
        bool publicTcp = false;
        bool publicUdp = false;
    };

    // A public endpoint is usable only while this descriptor is registered in
    // libtorrent. Gateway authentication material never crosses this boundary.
    struct TrustedInboundRoute
    {
        quint64 pathId = 0;
        quint64 generation = 0;
        QString publicAddress;
        quint16 publicPort = 0;
        QString relayAddress;
        quint16 relayPort = 0;
    };
}
