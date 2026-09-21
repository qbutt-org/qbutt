/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <chrono>
#include <cstdint>
#include <deque>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <vector>

#include <libtorrent/peer_route.hpp>

namespace Net
{
    // Selection and observation run on libtorrent's network thread. Diagnostics
    // take a bounded snapshot for the application thread under their own lock.
    // The catalog contains one current transport per edge; libtorrent retains
    // peer deduplication and session limits.
    class PeerRouteSelector
    {
    public:
        enum class Decision
        {
            Pinned,
            BestScore,
            Exploration,
            BlockedNoRoute,
            BlockedCooldown
        };

        enum class RouteType
        {
            Blocked,
            Relay,
            Native
        };

        struct RouteDiagnostics
        {
            std::uint64_t pathId = 0;
            std::uint64_t generation = 0;
            RouteType type = RouteType::Blocked;
            std::uint64_t attempts = 0;
            std::uint64_t connected = 0;
            std::uint64_t closed = 0;
            std::uint64_t connectionFailures = 0;
            std::uint64_t timeouts = 0;
            std::int64_t payloadDownload = 0;
            std::int64_t payloadUpload = 0;
            // Libtorrent credits the winning downloaded blocks once when their
            // piece passes hashing, excluding pad bytes and redundant copies.
            // This is transfer goodput, not the size of existing verified data.
            std::int64_t verifiedDownload = 0;
            std::int64_t demandMilliseconds = 0;
            std::int64_t chokedMilliseconds = 0;
        };

        struct DiagnosticEvent
        {
            std::int64_t ageMilliseconds = 0;
            std::uint64_t pathId = 0;
            std::uint64_t generation = 0;
            Decision decision = Decision::Pinned;
        };

        struct Diagnostics
        {
            std::vector<RouteDiagnostics> routes;
            std::vector<DiagnosticEvent> events;
            std::uint64_t blockedSelections = 0;
            bool eventsTruncated = false;
        };

        class DiagnosticHistory
        {
        public:
            void configure(const std::vector<libtorrent::peer_route> &routes);
            void retire();
            void selected(const libtorrent::peer_route_context &route, Decision decision);
            void observe(const libtorrent::peer_route_observation &observation);
            Diagnostics snapshot() const;

        private:
            using Clock = std::chrono::steady_clock;
            using RouteKey = std::pair<std::uint64_t, std::uint64_t>;

            struct EventRecord
            {
                Clock::time_point timestamp;
                std::uint64_t pathId = 0;
                std::uint64_t generation = 0;
                Decision decision = Decision::Pinned;
            };

            struct RetainedRoute
            {
                RouteDiagnostics value;
                Clock::time_point touched;
                bool current = false;
            };

            void record(Clock::time_point now, const libtorrent::peer_route_context &route, Decision decision);
            void prune(Clock::time_point now);

            std::map<RouteKey, RetainedRoute> m_routes;
            std::deque<EventRecord> m_events;
            std::optional<Clock::time_point> m_lastDroppedEvent;
            std::uint64_t m_blockedSelections = 0;
            mutable std::mutex m_mutex;
        };

        PeerRouteSelector(std::vector<libtorrent::peer_route> routes, bool mixed,
            std::shared_ptr<DiagnosticHistory> diagnostics = {},
            const std::shared_ptr<PeerRouteSelector> &previous = {});

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
            double recentAssignments = 0;
        };

        struct History
        {
            std::map<PeerKey, PeerHistory> peers;
            std::map<RouteKey, RouteHistory> routes;
            Clock::time_point nextMaintenance;
            std::uint64_t attempts = 0;
        };

        void maintain(Clock::time_point now);

        const std::vector<libtorrent::peer_route> m_routes;
        const bool m_mixed;
        const std::shared_ptr<History> m_history;
        const std::shared_ptr<DiagnosticHistory> m_diagnosticHistory;
        bool m_catalogApplied = false;
    };
}
