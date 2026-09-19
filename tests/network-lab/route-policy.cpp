/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <thread>
#include <vector>

#include <libtorrent/add_torrent_params.hpp>
#include <libtorrent/alert_types.hpp>
#include <libtorrent/bencode.hpp>
#include <libtorrent/create_torrent.hpp>
#include <libtorrent/file_storage.hpp>
#include <libtorrent/ip_filter.hpp>
#include <libtorrent/peer_info.hpp>
#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/torrent_route_policy.hpp>
#include <libtorrent/udp_route.hpp>

namespace fs = std::filesystem;
namespace lt = libtorrent;
using namespace std::chrono_literals;

namespace
{
    std::shared_ptr<lt::torrent_info> makeTorrent(const fs::path &seedRoot, const std::string &name,
        const std::vector<char> &payload, const std::vector<std::string> &trackers,
        const std::string &urlSeed)
    {
        fs::create_directories(seedRoot);
        std::ofstream(seedRoot / name, std::ios::binary).write(payload.data(), payload.size());
        lt::file_storage files;
        files.add_file(name, payload.size());
        lt::create_torrent creator {files, 64 * 1024, lt::create_torrent::v1_only};
        for (const std::string &tracker : trackers)
            creator.add_tracker(tracker, 0);
        if (!urlSeed.empty())
            creator.add_url_seed(urlSeed);
        lt::error_code error;
        lt::set_piece_hashes(creator, seedRoot.string(), error);
        if (error)
            throw std::runtime_error(error.message());
        std::vector<char> metadata;
        lt::bencode(std::back_inserter(metadata), creator.generate());
        auto result = std::make_shared<lt::torrent_info>(lt::span<char const>(metadata), error, lt::from_span);
        if (error)
            throw std::runtime_error(error.message());
        return result;
    }

    lt::network_route nativeRoute(const std::uint64_t pathId, const std::uint64_t generation,
        const char *address, const lt::address &publicAddress, const unsigned short publicPort)
    {
        lt::network_route result;
        result.family = lt::route_family::ipv4;
        result.binding.type = lt::route_descriptor::type_t::native;
        result.binding.context = {pathId, generation};
        result.binding.local_endpoint = {lt::make_address(address), 0};
        result.public_endpoint = {publicAddress, publicPort};
        return result;
    }

    lt::network_route socksRoute(const unsigned short port, const std::string &username,
        const std::string &password)
    {
        lt::network_route result;
        result.family = lt::route_family::ipv4;
        result.binding.type = lt::route_descriptor::type_t::socks5;
        result.binding.context = {2, 1};
        result.binding.proxy_endpoint = {lt::address_v4::loopback(), port};
        result.binding.username = username;
        result.binding.password = password;
        return result;
    }

    lt::udp_route udpRoute(const lt::network_route &route, const lt::address &externalAddress,
        const unsigned short publicPort,
        const bool enableUtp = false, const bool enableDht = true, const bool enableTrackers = true)
    {
        lt::udp_route result;
        static_cast<lt::route_descriptor &>(result.route) = route.binding;
        result.family = route.family;
        result.enable_utp = enableUtp;
        result.enable_dht = enableDht;
        result.enable_trackers = enableTrackers;
        result.external_address = externalAddress;
        if (publicPort != 0)
            result.public_endpoint = {externalAddress, publicPort};
        return result;
    }

    bool waitForRoute(lt::session &session, const lt::peer_route_context context)
    {
        const auto deadline = std::chrono::steady_clock::now() + 10s;
        while (std::chrono::steady_clock::now() < deadline)
        {
            std::vector<lt::alert *> alerts;
            session.pop_alerts(&alerts);
            for (const lt::alert *alert : alerts)
            {
                const auto *route = lt::alert_cast<lt::udp_route_alert>(alert);
                if (!route || (route->route != context))
                    continue;
                if (route->state == lt::udp_route_state::ready)
                    return true;
                if (route->state == lt::udp_route_state::failed)
                {
                    std::cerr << route->message() << '\n';
                    return false;
                }
            }
            std::this_thread::sleep_for(10ms);
        }
        return false;
    }

    bool waitForFile(const fs::path &path, const std::chrono::seconds timeout)
    {
        const auto deadline = std::chrono::steady_clock::now() + timeout;
        while ((std::chrono::steady_clock::now() < deadline) && !fs::exists(path))
            std::this_thread::sleep_for(10ms);
        return fs::exists(path);
    }
}

int main(const int argc, char **argv) try
{
    if (argc != 11)
        return 2;
    const fs::path root {argv[1]};
    if (fs::exists(root))
        return 3;
    fs::create_directories(root / "download");
    const unsigned short httpTrackerPort = static_cast<unsigned short>(std::stoi(argv[2]));
    const unsigned short udpTrackerPort = static_cast<unsigned short>(std::stoi(argv[3]));
    const unsigned short dhtPort = static_cast<unsigned short>(std::stoi(argv[4]));
    const unsigned short webSeedPort = static_cast<unsigned short>(std::stoi(argv[5]));
    const unsigned short proxyPort = static_cast<unsigned short>(std::stoi(argv[6]));
    const std::string username {argv[7]};
    const std::string password {argv[8]};
    lt::error_code addressError;
    const lt::address externalAddress = lt::make_address(argv[9], addressError);
    if (addressError)
        return 4;
    const fs::path markers {argv[10]};

    std::vector<char> trackerPayload(64 * 1024, 0x31);
    std::vector<char> utpPayload(512 * 1024);
    for (std::size_t index = 0; index < utpPayload.size(); ++index)
        utpPayload[index] = static_cast<char>((index * 43 + index / 131) & 255);
    std::vector<char> webPayload(1024 * 1024);
    for (std::size_t index = 0; index < webPayload.size(); ++index)
        webPayload[index] = static_cast<char>((index * 29 + index / 97) & 255);
    const auto trackerInfo = makeTorrent(root / "tracker-seed", "tracker.bin", trackerPayload,
        {"http://127.0.0.1:" + std::to_string(httpTrackerPort) + "/announce",
            "udp://127.0.0.1:" + std::to_string(udpTrackerPort) + "/announce"}, {});
    const auto anonymousInfo = makeTorrent(root / "anonymous-seed", "anonymous.bin", trackerPayload,
        {"http://127.0.0.1:" + std::to_string(httpTrackerPort) + "/announce",
            "udp://127.0.0.1:" + std::to_string(udpTrackerPort) + "/announce"}, {});
    const auto defaultInfo = makeTorrent(root / "default-seed", "default.bin", trackerPayload,
        {"http://127.0.0.1:" + std::to_string(httpTrackerPort) + "/announce"}, {});
    const auto webInfo = makeTorrent(root / "web-seed", "payload.bin", webPayload,
        {"http://tracker.invalid:" + std::to_string(httpTrackerPort) + "/announce"},
        "http://webseed.invalid:" + std::to_string(webSeedPort) + "/");
    const auto utpInfo = makeTorrent(root / "utp-seed", "utp.bin", utpPayload, {}, {});
    const auto outgoingDhtInfo = makeTorrent(root / "outgoing-dht", "outgoing.bin", trackerPayload, {}, {});

    constexpr unsigned short routeAPublicPort = 41001;
    constexpr unsigned short routeBPublicPort = 41002;
    constexpr unsigned short utpPublicPort = 41003;
    constexpr unsigned short routeAUdpPublicPort = 42001;
    constexpr unsigned short routeBUdpPublicPort = 42002;
    const lt::network_route routeA = nativeRoute(1, 1, "127.0.0.2", externalAddress, routeAPublicPort);
    const lt::network_route routeB = nativeRoute(1, 2, "127.0.0.3", externalAddress, routeBPublicPort);
    const lt::network_route utpRoute = nativeRoute(3, 1, "127.0.0.6", externalAddress, utpPublicPort);
    const lt::network_route outgoingDhtRoute = nativeRoute(4, 1, "127.0.0.7", {}, 0);
    const lt::network_route webRoute = socksRoute(proxyPort, username, password);
    lt::torrent_route_policy policyA;
    policyA.mode = lt::torrent_route_policy::mode_t::managed;
    policyA.routes = {routeA};
    policyA.pinned = routeA.binding.context;
    lt::torrent_route_policy policyB = policyA;
    policyB.routes = {routeB};
    policyB.pinned = routeB.binding.context;
    lt::torrent_route_policy webPolicy;
    webPolicy.mode = lt::torrent_route_policy::mode_t::managed;
    webPolicy.routes = {webRoute};
    webPolicy.pinned = webRoute.binding.context;
    lt::torrent_route_policy utpPolicy;
    utpPolicy.mode = lt::torrent_route_policy::mode_t::managed;
    utpPolicy.routes = {utpRoute};
    utpPolicy.pinned = utpRoute.binding.context;
    lt::torrent_route_policy outgoingDhtPolicy;
    outgoingDhtPolicy.mode = lt::torrent_route_policy::mode_t::managed;
    outgoingDhtPolicy.routes = {outgoingDhtRoute};
    outgoingDhtPolicy.pinned = outgoingDhtRoute.binding.context;

    lt::settings_pack settings;
    settings.set_str(lt::settings_pack::listen_interfaces, "");
    settings.set_str(lt::settings_pack::dht_bootstrap_nodes, "");
    settings.set_str(lt::settings_pack::announce_ip, "192.0.2.123");
    settings.set_bool(lt::settings_pack::enable_dht, true);
    settings.set_bool(lt::settings_pack::enable_lsd, false);
    settings.set_bool(lt::settings_pack::enable_upnp, false);
    settings.set_bool(lt::settings_pack::enable_natpmp, false);
    settings.set_bool(lt::settings_pack::enable_incoming_tcp, false);
    settings.set_bool(lt::settings_pack::enable_incoming_utp, false);
    settings.set_bool(lt::settings_pack::enable_outgoing_utp, false);
    settings.set_bool(lt::settings_pack::announce_to_all_trackers, true);
    settings.set_bool(lt::settings_pack::announce_to_all_tiers, true);
    settings.set_int(lt::settings_pack::alert_mask, lt::alert_category::all);
    lt::session session {settings};
    const lt::udp_route udpA = udpRoute(routeA, externalAddress, routeAUdpPublicPort);
    const lt::udp_route udpB = udpRoute(routeB, externalAddress, routeBUdpPublicPort);
    const lt::udp_route outgoingDht = udpRoute(outgoingDhtRoute, externalAddress, 0);
    if (session.set_udp_routes({udpA}))
        return 5;
    if (!waitForRoute(session, routeA.binding.context))
        return 6;
    if (session.add_dht_route_node(routeA.binding.context, lt::route_family::ipv4,
        {lt::address_v4::loopback(), dhtPort}))
        return 7;
    if (session.set_udp_routes({udpA, outgoingDht})
        || !waitForRoute(session, outgoingDhtRoute.binding.context)
        || session.add_dht_route_node(outgoingDhtRoute.binding.context, lt::route_family::ipv4,
            {lt::address_v4::loopback(), dhtPort}))
        return 7;
    const auto selectA = [trackerHash = trackerInfo->info_hashes(),
        outgoingDhtHash = outgoingDhtInfo->info_hashes(), policyA, outgoingDhtPolicy, webPolicy]
        (const lt::torrent_route_request &request)
    {
        if (request.info_hashes == trackerHash)
            return policyA;
        return (request.info_hashes == outgoingDhtHash) ? outgoingDhtPolicy : webPolicy;
    };
    if (session.set_torrent_route_policy_selector(selectA))
        return 8;

    lt::add_torrent_params trackerAdd;
    trackerAdd.ti = trackerInfo;
    trackerAdd.save_path = (root / "download").string();
    trackerAdd.file_priorities = {lt::dont_download};
    trackerAdd.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    trackerAdd.flags |= lt::torrent_flags::disable_lsd | lt::torrent_flags::disable_pex;
    session.add_torrent(trackerAdd);
    lt::add_torrent_params outgoingDhtAdd;
    outgoingDhtAdd.ti = outgoingDhtInfo;
    outgoingDhtAdd.save_path = (root / "download").string();
    outgoingDhtAdd.file_priorities = {lt::dont_download};
    outgoingDhtAdd.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    outgoingDhtAdd.flags |= lt::torrent_flags::disable_lsd | lt::torrent_flags::disable_pex;
    session.add_torrent(outgoingDhtAdd);
    if (!waitForFile(markers / "http-a", 10s)
        || !waitForFile(markers / "udp-a", 10s)
        || !waitForFile(markers / "dht-a", 10s)
        || !waitForFile(markers / "dht-outgoing", 10s))
        return 9;

    if (session.set_udp_routes({udpA, udpB, outgoingDht}))
        return 10;
    if (!waitForRoute(session, routeB.binding.context))
        return 11;
    if (session.add_dht_route_node(routeB.binding.context, lt::route_family::ipv4,
        {lt::address_v4::loopback(), dhtPort}))
        return 12;
    const auto selectB = [trackerHash = trackerInfo->info_hashes(), anonymousHash = anonymousInfo->info_hashes(),
        utpHash = utpInfo->info_hashes(),
        outgoingDhtHash = outgoingDhtInfo->info_hashes(), policyB, webPolicy, utpPolicy, outgoingDhtPolicy]
        (const lt::torrent_route_request &request)
    {
        if (request.info_hashes == trackerHash || request.info_hashes == anonymousHash)
            return policyB;
        if (request.info_hashes == outgoingDhtHash)
            return outgoingDhtPolicy;
        return (request.info_hashes == utpHash) ? utpPolicy : webPolicy;
    };
    std::ofstream(markers / "phase-b").put('1');
    if (session.set_torrent_route_policy_selector(selectB))
        return 13;
    if (session.set_udp_routes({udpB, outgoingDht}))
        return 14;

    bool httpReply = false;
    bool udpReply = false;
    const auto networkDeadline = std::chrono::steady_clock::now() + 15s;
    while (std::chrono::steady_clock::now() < networkDeadline)
    {
        std::vector<lt::alert *> alerts;
        session.pop_alerts(&alerts);
        for (const lt::alert *alert : alerts)
        {
            if (const auto *reply = lt::alert_cast<lt::tracker_reply_alert>(alert))
            {
                const std::string url {reply->tracker_url()};
                httpReply |= url.starts_with("http://");
                udpReply |= url.starts_with("udp://");
            }
        }
        if (httpReply && udpReply && fs::exists(markers / "http-b")
            && fs::exists(markers / "udp-a") && fs::exists(markers / "udp-b")
            && fs::exists(markers / "dht-a") && fs::exists(markers / "dht-b")
            && fs::exists(markers / "dht-outgoing"))
        {
            break;
        }
        std::this_thread::sleep_for(10ms);
    }
    if (!httpReply || !udpReply || !fs::exists(markers / "http-b")
        || !fs::exists(markers / "udp-a") || !fs::exists(markers / "udp-b")
        || !fs::exists(markers / "dht-a") || !fs::exists(markers / "dht-b")
        || !fs::exists(markers / "dht-outgoing"))
    {
        return 15;
    }

    std::ofstream(markers / "phase-anonymous").put('1');
    lt::settings_pack anonymousSettings;
    anonymousSettings.set_bool(lt::settings_pack::anonymous_mode, true);
    session.apply_settings(anonymousSettings);
    lt::add_torrent_params anonymousAdd;
    anonymousAdd.ti = anonymousInfo;
    anonymousAdd.save_path = (root / "download").string();
    anonymousAdd.file_priorities = {lt::dont_download};
    anonymousAdd.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    anonymousAdd.flags |= lt::torrent_flags::disable_dht | lt::torrent_flags::disable_lsd
        | lt::torrent_flags::disable_pex;
    session.add_torrent(anonymousAdd);
    if (!waitForFile(markers / "http-anonymous", 10s)
        || !waitForFile(markers / "udp-anonymous", 10s))
        return 22;
    anonymousSettings.set_bool(lt::settings_pack::anonymous_mode, false);
    session.apply_settings(anonymousSettings);
    std::ofstream(markers / "phase-anonymous-end").put('1');

    lt::add_torrent_params webAdd;
    webAdd.ti = webInfo;
    webAdd.save_path = (root / "download").string();
    webAdd.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    webAdd.flags |= lt::torrent_flags::disable_dht | lt::torrent_flags::disable_lsd
        | lt::torrent_flags::disable_pex;
    const lt::torrent_handle webTorrent = session.add_torrent(webAdd);
    const auto transferDeadline = std::chrono::steady_clock::now() + 30s;
    while (!webTorrent.status().is_seeding && (std::chrono::steady_clock::now() < transferDeadline))
        std::this_thread::sleep_for(20ms);
    std::ifstream downloaded(root / "download" / "payload.bin", std::ios::binary);
    const std::vector<char> actual((std::istreambuf_iterator<char>(downloaded)), {});
    if (!webTorrent.status().is_seeding || (actual != webPayload)
        || !waitForFile(markers / "http-socks", 10s))
        return 16;

    lt::settings_pack defaultSettings;
    defaultSettings.set_str(lt::settings_pack::listen_interfaces, "127.0.0.4:0");
    defaultSettings.set_bool(lt::settings_pack::enable_dht, false);
    defaultSettings.set_bool(lt::settings_pack::enable_lsd, false);
    defaultSettings.set_bool(lt::settings_pack::enable_upnp, false);
    defaultSettings.set_bool(lt::settings_pack::enable_natpmp, false);
    lt::session defaultSession {defaultSettings};
    const auto defaultListenDeadline = std::chrono::steady_clock::now() + 10s;
    while ((defaultSession.listen_port() == 0)
        && (std::chrono::steady_clock::now() < defaultListenDeadline))
        std::this_thread::sleep_for(20ms);
    if (defaultSession.listen_port() == 0)
        return 17;
    lt::add_torrent_params defaultAdd;
    defaultAdd.ti = defaultInfo;
    defaultAdd.save_path = (root / "download").string();
    defaultAdd.file_priorities = {lt::dont_download};
    defaultAdd.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    defaultSession.add_torrent(defaultAdd);
    if (!waitForFile(markers / "http-default", 10s))
        return 17;

    lt::settings_pack seedSettings;
    seedSettings.set_str(lt::settings_pack::listen_interfaces, "127.0.0.5:0");
    seedSettings.set_str(lt::settings_pack::dht_bootstrap_nodes, "");
    seedSettings.set_bool(lt::settings_pack::enable_dht, false);
    seedSettings.set_bool(lt::settings_pack::enable_lsd, false);
    seedSettings.set_bool(lt::settings_pack::enable_upnp, false);
    seedSettings.set_bool(lt::settings_pack::enable_natpmp, false);
    seedSettings.set_bool(lt::settings_pack::enable_incoming_tcp, false);
    seedSettings.set_bool(lt::settings_pack::enable_incoming_utp, true);
    seedSettings.set_int(lt::settings_pack::upload_rate_limit, 128 * 1024);
    lt::session seedSession {seedSettings};
    lt::ip_filter seedPeerClasses;
    seedPeerClasses.add_rule(lt::address_v4::any(), lt::address_v4::broadcast(),
        1 << static_cast<std::uint32_t>(lt::session::global_peer_class_id));
    seedSession.set_peer_class_filter(seedPeerClasses);
    lt::add_torrent_params seedAdd;
    seedAdd.ti = utpInfo;
    seedAdd.save_path = (root / "utp-seed").string();
    seedAdd.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    seedAdd.flags |= lt::torrent_flags::disable_dht | lt::torrent_flags::disable_lsd
        | lt::torrent_flags::disable_pex;
    const lt::torrent_handle seedTorrent = seedSession.add_torrent(seedAdd);
    const auto seedDeadline = std::chrono::steady_clock::now() + 15s;
    while ((!seedTorrent.status().is_seeding || (seedSession.listen_port() == 0))
        && (std::chrono::steady_clock::now() < seedDeadline))
        std::this_thread::sleep_for(20ms);
    if (!seedTorrent.status().is_seeding || (seedSession.listen_port() == 0))
        return 18;

    const lt::udp_route managedUtp = udpRoute(utpRoute, externalAddress, utpPublicPort,
        true, false, false);
    if (session.set_udp_routes({udpB, outgoingDht, managedUtp}))
        return 19;
    if (!waitForRoute(session, utpRoute.binding.context))
        return 20;
    session.set_peer_route_selector([utpHash = utpInfo->info_hashes(), route = utpRoute.binding]
        (const lt::peer_route_request &request)
    {
        if (request.info_hashes != utpHash)
            return lt::peer_route {};
        lt::peer_route result;
        static_cast<lt::route_descriptor &>(result) = route;
        return result;
    });
    lt::settings_pack utpSettings;
    utpSettings.set_bool(lt::settings_pack::enable_outgoing_tcp, false);
    utpSettings.set_bool(lt::settings_pack::enable_outgoing_utp, true);
    session.apply_settings(utpSettings);
    lt::add_torrent_params utpAdd;
    utpAdd.ti = utpInfo;
    utpAdd.save_path = (root / "utp-download").string();
    utpAdd.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    utpAdd.flags |= lt::torrent_flags::disable_dht | lt::torrent_flags::disable_lsd
        | lt::torrent_flags::disable_pex;
    const lt::torrent_handle utpTorrent = session.add_torrent(utpAdd);
    utpTorrent.connect_peer({lt::make_address("127.0.0.5"), seedSession.listen_port()});
    bool utpObserved = false;
    bool utpSourceObserved = false;
    const auto utpDeadline = std::chrono::steady_clock::now() + 30s;
    while (!utpTorrent.status().is_seeding && (std::chrono::steady_clock::now() < utpDeadline))
    {
        std::vector<lt::peer_info> peers;
        utpTorrent.get_peer_info(peers);
        for (const lt::peer_info &peer : peers)
        {
            utpObserved |= static_cast<bool>(peer.flags & lt::peer_info::utp_socket);
            utpSourceObserved |= peer.local_endpoint.address() == lt::make_address("127.0.0.6");
        }
        std::this_thread::sleep_for(20ms);
    }
    std::ifstream utpDownloaded(root / "utp-download" / "utp.bin", std::ios::binary);
    const std::vector<char> actualUtp((std::istreambuf_iterator<char>(utpDownloaded)), {});
    if (!utpTorrent.status().is_seeding || !utpObserved || !utpSourceObserved || (actualUtp != utpPayload))
    {
        std::cerr << "uTP result: seeding=" << utpTorrent.status().is_seeding
            << " transport=" << utpObserved << " source=" << utpSourceObserved
            << " bytes=" << actualUtp.size() << '\n';
        return 21;
    }

    std::cout << "{\"passed\":true,\"httpTrackerTransition\":true"
        << ",\"udpTrackerTransition\":true,\"dhtTransition\":true"
        << ",\"webSeedVerifiedBytes\":" << actual.size()
        << ",\"webSeedThroughAuthenticatedSocks\":true"
        << ",\"httpTrackerThroughTcpOnlySocks\":true"
        << ",\"defaultHttpTrackerUnaffected\":true"
        << ",\"managedAutomaticUtp\":true"
        << ",\"utpVerifiedBytes\":" << actualUtp.size() << "}\n";
    return 0;
}
catch (const std::exception &error)
{
    std::cerr << error.what() << '\n';
    return 99;
}
