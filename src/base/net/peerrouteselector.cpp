/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "peerrouteselector.h"

#include <algorithm>
#include <cmath>
#include <limits>
#include <mutex>
#include <utility>

#include <boost/asio/error.hpp>

#include <libtorrent/error_code.hpp>

namespace
{
    constexpr std::size_t MAX_PEER_HISTORY = 16384;
    constexpr std::size_t MAX_DIAGNOSTIC_EVENTS = 512;
    constexpr std::size_t MAX_DIAGNOSTIC_ROUTES = 256;
    constexpr auto HISTORY_TTL = std::chrono::minutes {15};

    bool isNetworkFailure(const libtorrent::error_code &ec, const libtorrent::operation_t operation)
    {
        // Idle/choke/request timeouts, protocol rejection, cancellation, disk
        // errors and normal EOF are not evidence that a network route failed.
        // EOF while establishing a connection means this attempt never reached
        // the peer (including a proxy closing its handshake without a reply).
        using namespace boost::asio;
        return ((operation == libtorrent::operation_t::connect) && (ec == error::eof))
            || ((operation == libtorrent::operation_t::connect) && (ec == libtorrent::errors::timed_out))
            || (ec == error::connection_refused) || (ec == error::connection_reset)
            || (ec == error::connection_aborted) || (ec == error::network_down)
            || (ec == error::network_reset) || (ec == error::network_unreachable)
            || (ec == error::host_unreachable) || (ec == error::timed_out)
            || (ec == error::broken_pipe);
    }

    bool isAttemptTimeout(const libtorrent::error_code &ec, const libtorrent::operation_t operation)
    {
        return (ec == boost::asio::error::timed_out)
            || ((operation == libtorrent::operation_t::connect) && (ec == libtorrent::errors::timed_out))
            || (ec == libtorrent::errors::timed_out_no_handshake);
    }
}

void Net::PeerRouteSelector::DiagnosticHistory::configure(const std::vector<libtorrent::peer_route> &routes)
{
    const Clock::time_point now = Clock::now();
    std::scoped_lock lock {m_mutex};
    for (auto &entry : m_routes)
    {
        RetainedRoute &route = entry.second;
        if (route.current)
            route.touched = now;
        route.current = false;
    }
    for (const libtorrent::peer_route &route : routes)
    {
        RetainedRoute &retained = m_routes[{route.context.path_id, route.context.generation}];
        retained.current = true;
        retained.touched = now;
        retained.value.pathId = route.context.path_id;
        retained.value.generation = route.context.generation;
        retained.value.type = (route.type == libtorrent::peer_route::type_t::native) ? RouteType::Native
            : ((route.type == libtorrent::peer_route::type_t::socks5) ? RouteType::Relay : RouteType::Blocked);
    }
    prune(now);
}

void Net::PeerRouteSelector::DiagnosticHistory::retire()
{
    const Clock::time_point now = Clock::now();
    std::scoped_lock lock {m_mutex};
    for (auto &entry : m_routes)
    {
        RetainedRoute &route = entry.second;
        if (route.current)
        {
            route.current = false;
            route.touched = now;
        }
    }
    prune(now);
}

void Net::PeerRouteSelector::DiagnosticHistory::selected(
    const libtorrent::peer_route_context &route, const Decision decision)
{
    const Clock::time_point now = Clock::now();
    std::scoped_lock lock {m_mutex};
    if ((decision == Decision::BlockedNoRoute) || (decision == Decision::BlockedCooldown))
        ++m_blockedSelections;
    else if (const auto retained = m_routes.find({route.path_id, route.generation}); retained != m_routes.end())
    {
        ++retained->second.value.attempts;
        retained->second.touched = now;
    }
    record(now, route, decision);
}

void Net::PeerRouteSelector::DiagnosticHistory::observe(
    const libtorrent::peer_route_observation &observation)
{
    const Clock::time_point now = Clock::now();
    std::scoped_lock lock {m_mutex};
    const auto retained = m_routes.find({observation.route.path_id, observation.route.generation});
    if (retained == m_routes.end())
        return;

    RetainedRoute &route = retained->second;
    route.touched = now;
    route.value.payloadDownload += observation.payload_download;
    route.value.payloadUpload += observation.payload_upload;
    route.value.verifiedDownload += observation.verified_download;
    route.value.demandMilliseconds += observation.demand_duration_ms;
    route.value.chokedMilliseconds += observation.choked_duration_ms;
    if (observation.event == libtorrent::peer_route_observation::event_t::connected)
        ++route.value.connected;
    else if (observation.event == libtorrent::peer_route_observation::event_t::closed)
    {
        const bool networkFailure = isNetworkFailure(observation.error, observation.operation);
        const bool establishmentFailure = observation.error
            && (observation.error != boost::asio::error::operation_aborted)
            && ((observation.operation == libtorrent::operation_t::connect)
                || (observation.operation == libtorrent::operation_t::sock_bind)
                || (observation.operation == libtorrent::operation_t::get_interface));
        const bool handshakeTimeout = observation.error == libtorrent::errors::timed_out_no_handshake;
        const bool timeout = isAttemptTimeout(observation.error, observation.operation);
        ++route.value.closed;
        if (networkFailure || establishmentFailure || handshakeTimeout)
        {
            ++route.value.connectionFailures;
            route.value.timeouts += timeout;
        }
    }
}

Net::PeerRouteSelector::Diagnostics Net::PeerRouteSelector::DiagnosticHistory::snapshot() const
{
    const Clock::time_point now = Clock::now();
    std::scoped_lock lock {m_mutex};
    Diagnostics result;
    result.routes.reserve(m_routes.size());
    for (const auto &entry : m_routes)
    {
        const RetainedRoute &route = entry.second;
        if (route.current || ((now - route.touched) <= HISTORY_TTL))
            result.routes.push_back(route.value);
    }
    result.events.reserve(m_events.size());
    for (const EventRecord &event : m_events)
    {
        if ((now - event.timestamp) > HISTORY_TTL)
            continue;
        result.events.push_back({std::chrono::duration_cast<std::chrono::milliseconds>(now - event.timestamp).count(),
            event.pathId, event.generation, event.decision});
    }
    result.blockedSelections = m_blockedSelections;
    result.eventsTruncated = m_lastDroppedEvent && ((now - *m_lastDroppedEvent) <= HISTORY_TTL);
    return result;
}

void Net::PeerRouteSelector::DiagnosticHistory::record(const Clock::time_point now,
    const libtorrent::peer_route_context &route, const Decision decision)
{
    if (m_events.size() >= MAX_DIAGNOSTIC_EVENTS)
    {
        m_lastDroppedEvent = m_events.front().timestamp;
        m_events.pop_front();
    }
    m_events.push_back({now, route.path_id, route.generation, decision});
}

void Net::PeerRouteSelector::DiagnosticHistory::prune(const Clock::time_point now)
{
    while (!m_events.empty() && ((now - m_events.front().timestamp) > HISTORY_TTL))
        m_events.pop_front();
    if (m_lastDroppedEvent && ((now - *m_lastDroppedEvent) > HISTORY_TTL))
        m_lastDroppedEvent.reset();
    std::erase_if(m_routes, [now](const auto &entry)
    {
        return !entry.second.current && ((now - entry.second.touched) > HISTORY_TTL);
    });
    while (m_routes.size() > MAX_DIAGNOSTIC_ROUTES)
    {
        auto oldest = m_routes.end();
        for (auto candidate = m_routes.begin(); candidate != m_routes.end(); ++candidate)
        {
            if (!candidate->second.current && ((oldest == m_routes.end())
                || (candidate->second.touched < oldest->second.touched)))
                oldest = candidate;
        }
        if (oldest == m_routes.end())
            break;
        m_routes.erase(oldest);
    }
}

Net::PeerRouteSelector::PeerRouteSelector(std::vector<libtorrent::peer_route> routes, const bool mixed,
    std::shared_ptr<DiagnosticHistory> diagnostics)
    : m_routes {std::move(routes)}
    , m_mixed {mixed}
    , m_diagnosticHistory {diagnostics ? std::move(diagnostics) : std::make_shared<DiagnosticHistory>()}
{
    const Clock::time_point now = Clock::now();
    for (const libtorrent::peer_route &route : m_routes)
        m_history[{route.context.path_id, route.context.generation}].updated = now;
    m_diagnosticHistory->configure(m_routes);
    m_nextMaintenance = now + std::chrono::minutes {1};
}

libtorrent::peer_route Net::PeerRouteSelector::select(const libtorrent::peer_route_request &request)
{
    libtorrent::peer_route blocked;
    blocked.type = libtorrent::peer_route::type_t::blocked;
    if (m_routes.empty())
    {
        m_diagnosticHistory->selected({}, Decision::BlockedNoRoute);
        return blocked;
    }

    const auto eligible = [&request](const libtorrent::peer_route &route)
    {
        return ((route.type == libtorrent::peer_route::type_t::socks5)
                || (route.type == libtorrent::peer_route::type_t::native))
            && ((route.type != libtorrent::peer_route::type_t::native)
                || (route.local_endpoint.address().is_v6() == request.peer.address().is_v6()));
    };

    const Clock::time_point now = Clock::now();
    maintain(now);
    // Metadata-less magnets are not known public torrents. Their discovery and
    // first peers keep the same pinned identity until metadata says otherwise.
    if (!m_mixed || request.private_torrent || !request.has_metadata)
    {
        if (!eligible(m_routes.front()))
        {
            m_diagnosticHistory->selected({}, Decision::BlockedNoRoute);
            return blocked;
        }
        const RouteKey key {m_routes.front().context.path_id, m_routes.front().context.generation};
        ++m_history.at(key).attempts;
        m_diagnosticHistory->selected(m_routes.front().context, Decision::Pinned);
        return m_routes.front();
    }
    const PeerKey key {request.info_hashes, request.peer};
    auto peer = m_peers.find(key);
    if (peer == m_peers.end())
    {
        if (m_peers.size() >= MAX_PEER_HISTORY)
        {
            const auto oldest = std::min_element(m_peers.begin(), m_peers.end(),
                [](const auto &left, const auto &right) { return left.second.touched < right.second.touched; });
            m_peers.erase(oldest);
        }
        peer = m_peers.try_emplace(key).first;
    }
    peer->second.touched = now;

    const libtorrent::peer_route *selected = nullptr;
    const libtorrent::peer_route *leastTried = nullptr;
    double bestScore = -std::numeric_limits<double>::infinity();
    unsigned int fewestFailures = std::numeric_limits<unsigned int>::max();
    for (const libtorrent::peer_route &route : m_routes)
    {
        if (!eligible(route))
            continue;
        const RouteKey routeKey {route.context.path_id, route.context.generation};
        const auto failure = peer->second.failures.find(routeKey);
        if ((failure != peer->second.failures.end()) && (failure->second.retryAfter > now))
            continue;
        const unsigned int failures = (failure == peer->second.failures.end()) ? 0 : failure->second.count;

        RouteHistory &history = m_history.at(routeKey);
        // Verified bytes and unchoked demand occupancy share a decaying window.
        // Zero-demand and choked peers receive no negative throughput reward.
        history.decay(now);
        double usefulReward = 0;
        if ((history.verifiedBytes > 0) && (history.demandMilliseconds > 0))
        {
            const double bytesPerSecond = history.verifiedBytes * 1000 / history.demandMilliseconds;
            // Age sparse samples smoothly; a single valid tick must not lose
            // its entire reward as soon as it decays below one second.
            usefulReward = std::min(1.0, std::log2(1 + bytesPerSecond / 16384) / 8)
                * std::min(1.0, history.demandMilliseconds / 1000);
        }
        const double score = 1 - 0.75 * history.failurePressure + 0.25 * usefulReward;
        // After cooldown, a failed peer/route pair still loses to an unfailed
        // alternative. Otherwise the session's reconnect delay can outlast
        // cooldown and repeatedly select the same unreachable route.
        if (!selected || (failures < fewestFailures)
            || ((failures == fewestFailures) && ((score > bestScore)
                || ((score == bestScore) && (history.attempts < m_history.at(
                    {selected->context.path_id, selected->context.generation}).attempts)))))
        {
            selected = &route;
            bestScore = score;
            fewestFailures = failures;
        }
        if (!leastTried || (history.attempts < m_history.at(
            {leastTried->context.path_id, leastTried->context.generation}).attempts))
            leastTried = &route;
    }
    if (!selected)
    {
        m_diagnosticHistory->selected({}, Decision::BlockedCooldown);
        return blocked;
    }

    // Cover a new catalog once before throughput rewards can make an already
    // productive route dominate every new peer. Later, one in ten admitted
    // public dials explores. All attempts still share libtorrent's limits.
    ++m_attempts;
    const RouteKey leastTriedKey {leastTried->context.path_id, leastTried->context.generation};
    Decision decision = Decision::BestScore;
    if ((m_history.at(leastTriedKey).attempts == 0) || ((m_attempts % 10) == 0))
    {
        selected = leastTried;
        decision = Decision::Exploration;
    }
    RouteHistory &selectedHistory = m_history.at({selected->context.path_id, selected->context.generation});
    ++selectedHistory.attempts;
    m_diagnosticHistory->selected(selected->context, decision);
    return *selected;
}

void Net::PeerRouteSelector::observe(const libtorrent::peer_route_observation &observation)
{
    const RouteKey routeKey {observation.route.path_id, observation.route.generation};
    const Clock::time_point now = Clock::now();
    m_diagnosticHistory->observe(observation);
    const bool closed = observation.event == libtorrent::peer_route_observation::event_t::closed;
    const bool networkFailure = closed && isNetworkFailure(observation.error, observation.operation);
    // Establishment errors also include platform-specific address/interface
    // failures. They disqualify this peer/route attempt without implying
    // that other peers cannot use the route. Revocation is not a failure.
    const bool establishmentFailure = closed && observation.error
        && (observation.error != boost::asio::error::operation_aborted)
        && ((observation.operation == libtorrent::operation_t::connect)
            || (observation.operation == libtorrent::operation_t::sock_bind)
            || (observation.operation == libtorrent::operation_t::get_interface));
    const bool handshakeTimeout = closed && (observation.error == libtorrent::errors::timed_out_no_handshake);
    const auto route = m_history.find(routeKey);
    if (route == m_history.end())
        return;
    RouteHistory &history = route->second;
    history.decay(now);
    history.verifiedBytes += observation.verified_download;
    history.demandMilliseconds += observation.demand_duration_ms;

    const auto peer = m_peers.find({observation.info_hashes, observation.peer});
    // TCP establishment may only acknowledge a local relay. Clear peer-local
    // failures once the BitTorrent connection produces post-handshake evidence.
    if ((observation.event == libtorrent::peer_route_observation::event_t::activity)
        && ((observation.payload_download > 0) || (observation.payload_upload > 0)
            || (observation.demand_duration_ms > 0) || (observation.choked_duration_ms > 0)))
    {
        history.failurePressure *= 0.8;
        if (peer != m_peers.end())
            peer->second.failures.erase(routeKey);
    }
    else if (closed)
    {
        // A relay may acknowledge TCP before its remote handshake completes.
        // EOF then warrants trying this peer elsewhere, but is not evidence
        // that the whole route is bad or that a choked peer had low goodput.
        if (!networkFailure && !establishmentFailure && !handshakeTimeout
            && (observation.error != boost::asio::error::eof))
            return;
        if (networkFailure)
            history.failurePressure = 0.8 * history.failurePressure + 0.2;
        if (peer != m_peers.end())
        {
            Failure &failure = peer->second.failures[routeKey];
            failure.count = std::min(4U, failure.count + 1);
            failure.retryAfter = now + std::chrono::seconds {15 * (1 << failure.count)};
            peer->second.touched = now;
        }
    }
}

void Net::PeerRouteSelector::maintain(const Clock::time_point now)
{
    if (now < m_nextMaintenance)
        return;
    std::erase_if(m_peers, [now](const auto &peer) { return (now - peer.second.touched) > HISTORY_TTL; });
    m_nextMaintenance = now + std::chrono::minutes {1};
}

void Net::PeerRouteSelector::RouteHistory::decay(const Clock::time_point now)
{
    const double seconds = std::chrono::duration<double>(now - updated).count();
    const double factor = std::exp(-seconds / 120.0);
    verifiedBytes *= factor;
    demandMilliseconds *= factor;
    failurePressure *= factor;
    updated = now;
}
