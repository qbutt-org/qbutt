/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <chrono>
#include <map>
#include <vector>

#include <libtorrent/peer_route.hpp>

namespace Net
{
    // The selector and its observer run only on libtorrent's network thread.
    // The catalog contains one current transport per edge. Selection changes
    // future dials only; libtorrent retains peer deduplication and session limits.
    class PeerRouteSelector
    {
    public:
        PeerRouteSelector(std::vector<libtorrent::peer_route> routes, bool mixed);

        libtorrent::peer_route select(const libtorrent::peer_route_request &request);
        void observe(const libtorrent::peer_route_observation &observation);

    private:
        using Clock = std::chrono::steady_clock;
        using PeerKey = std::pair<libtorrent::info_hash_t, libtorrent::tcp::endpoint>;
        using RouteKey = std::pair<std::uint64_t, std::uint64_t>;

        struct Failure
        {
            Clock::time_point retryAfter;
            unsigned int count = 0;
        };

        struct PeerHistory
        {
            Clock::time_point touched;
            std::map<RouteKey, Failure> failures;
        };

        struct RouteHistory
        {
            void decay(Clock::time_point now);

            Clock::time_point updated;
            double verifiedBytes = 0;
            double demandMilliseconds = 0;
            double failurePressure = 0;
            std::uint64_t attempts = 0;
        };

        void maintain(Clock::time_point now);

        const std::vector<libtorrent::peer_route> m_routes;
        const bool m_mixed;
        std::map<PeerKey, PeerHistory> m_peers;
        std::map<RouteKey, RouteHistory> m_history;
        Clock::time_point m_nextMaintenance;
        std::uint64_t m_attempts = 0;
    };
}
