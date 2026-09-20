/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <iterator>
#include <memory>
#include <stdexcept>
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
    void require(const bool value, const char *message)
    {
        if (!value)
            throw std::runtime_error(message);
    }

    std::string readFile(const fs::path &path)
    {
        std::ifstream input(path, std::ios::binary);
        require(input.good(), "Fixture file could not be opened");
        return {std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>()};
    }
}

int main(const int argc, char **argv) try
{
    require(argc == 7, "Expected root, seed port, proxy port, username, password and certificate directory");
    const fs::path root {argv[1]};
    require(!fs::exists(root), "Fixture payload root already exists");
    fs::create_directories(root / "seed");
    fs::create_directories(root / "download");
    const auto seedPort = static_cast<unsigned short>(std::stoi(argv[2]));
    const fs::path certificates {argv[6]};
    const std::string certificate = readFile(certificates / "peer.pem");
    const std::string key = readFile(certificates / "peer-key.pem");
    const std::string dh = readFile(certificates / "dh.pem");
    std::string payload(512 * 1024, '\0');
    for (std::size_t index = 0; index < payload.size(); ++index)
        payload[index] = static_cast<char>((index * 47 + index / 139) & 255);
    std::ofstream(root / "seed" / "ssl-utp.bin", std::ios::binary).write(payload.data(), payload.size());
    lt::file_storage files;
    files.add_file("ssl-utp.bin", payload.size());
    lt::create_torrent creator {files, 64 * 1024, lt::create_torrent::v1_only};
    // This torrent trusts only its generated CA, never the machine trust store.
    creator.set_root_cert(readFile(certificates / "ca.pem"));
    lt::error_code error;
    lt::set_piece_hashes(creator, (root / "seed").string(), error);
    require(!error, "Fixture hashing failed");
    std::vector<char> metadata;
    lt::bencode(std::back_inserter(metadata), creator.generate());
    auto info = std::make_shared<lt::torrent_info>(lt::span<char const>(metadata), error, lt::from_span);
    require(!error && !info->ssl_cert().empty(), "SSL torrent metadata is invalid");

    std::atomic<std::uint64_t> selectedGeneration {1};
    std::array<std::atomic<std::int64_t>, 2> verified {};
    std::atomic<bool> firstClosed {false};
    lt::settings_pack settings;
    settings.set_str(lt::settings_pack::listen_interfaces, "");
    settings.set_str(lt::settings_pack::dht_bootstrap_nodes, "");
    settings.set_bool(lt::settings_pack::enable_dht, false);
    settings.set_bool(lt::settings_pack::enable_lsd, false);
    settings.set_bool(lt::settings_pack::enable_upnp, false);
    settings.set_bool(lt::settings_pack::enable_natpmp, false);
    settings.set_bool(lt::settings_pack::enable_incoming_tcp, false);
    settings.set_bool(lt::settings_pack::enable_outgoing_tcp, false);
    settings.set_bool(lt::settings_pack::enable_incoming_utp, false);
    settings.set_bool(lt::settings_pack::enable_outgoing_utp, true);
    settings.set_int(lt::settings_pack::alert_mask, lt::alert_category::all);
    lt::session client {settings};
    settings.set_str(lt::settings_pack::listen_interfaces, "127.0.0.1:" + std::to_string(seedPort) + "s");
    settings.set_bool(lt::settings_pack::enable_incoming_utp, true);
    settings.set_int(lt::settings_pack::max_retry_port_bind, 0);
    settings.set_int(lt::settings_pack::upload_rate_limit, 64 * 1024);
    lt::session seed {settings};
    lt::ip_filter seedPeerClasses;
    seedPeerClasses.add_rule(lt::address_v4::any(), lt::address_v4::broadcast(),
        1U << static_cast<std::uint32_t>(lt::session::global_peer_class_id));
    seed.set_peer_class_filter(seedPeerClasses);

    lt::network_route route;
    route.binding.type = lt::route_descriptor::type_t::socks5;
    route.binding.context = {9, 1};
    route.binding.proxy_endpoint = {lt::address_v4::loopback(), static_cast<unsigned short>(std::stoi(argv[3]))};
    route.binding.username = argv[4];
    route.binding.password = argv[5];
    lt::udp_route udp;
    static_cast<lt::route_descriptor &>(udp.route) = route.binding;
    udp.ssl = true;
    udp.enable_utp = true;
    lt::torrent_route_policy policy;
    policy.mode = lt::torrent_route_policy::mode_t::managed;
    policy.routes = {route};
    policy.pinned = route.binding.context;
    require(!client.set_torrent_route_policy_selector([policy](const lt::torrent_route_request &) { return policy; }),
        "Initial SSL-uTP policy rejected");
    client.set_peer_route_selector([binding = route.binding, &selectedGeneration](const lt::peer_route_request &)
    {
        lt::peer_route selected;
        static_cast<lt::route_descriptor &>(selected) = binding;
        selected.context.generation = selectedGeneration;
        selected.transport = lt::peer_route::transport_t::utp;
        return selected;
    }, [&](const lt::peer_route_observation &observation)
    {
        if ((observation.route.path_id != 9) || (observation.route.generation < 1)
            || (observation.route.generation > 2))
            return;
        verified[observation.route.generation - 1] += observation.verified_download;
        if ((observation.route.generation == 1)
            && (observation.event == lt::peer_route_observation::event_t::closed))
            firstClosed = true;
    });

    std::array<bool, 2> ready {};
    bool retired = false;
    bool staleRejected = false;
    int sslUtpConnections = 0;
    std::string lastPeerError;
    const auto poll = [&]
    {
        std::vector<lt::alert *> alerts;
        client.pop_alerts(&alerts);
        for (const auto *alert : alerts)
        {
            if (const auto *state = lt::alert_cast<lt::udp_route_alert>(alert))
            {
                require(state->ssl && state->state != lt::udp_route_state::failed,
                    "SSL-uTP route failed or lost SSL mode");
                if ((state->route.generation >= 1) && (state->route.generation <= 2))
                    ready[state->route.generation - 1] |= state->state == lt::udp_route_state::ready;
                retired |= (state->route == lt::peer_route_context {9, 1})
                    && (state->state == lt::udp_route_state::retired);
            }
            if (const auto *peer = lt::alert_cast<lt::peer_connect_alert>(alert))
            {
                require(peer->socket_type == lt::socket_type_t::utp_ssl, "Peer connection was not SSL-uTP");
                ++sslUtpConnections;
            }
            if (const auto *rejected = lt::alert_cast<lt::peer_route_alert>(alert))
                staleRejected |= (rejected->route == lt::peer_route_context {9, 1})
                    && (rejected->error == boost::asio::error::access_denied);
            if (const auto *disconnected = lt::alert_cast<lt::peer_disconnected_alert>(alert))
                lastPeerError = disconnected->error.message();
            require(!lt::alert_cast<lt::hash_failed_alert>(alert), "SSL-uTP payload hash failed");
            if (const auto *failure = lt::alert_cast<lt::torrent_error_alert>(alert))
                throw std::runtime_error("SSL-uTP torrent error: " + failure->error.message());
        }
    };
    const auto wait = [&](const auto &predicate, const char *message)
    {
        const auto deadline = std::chrono::steady_clock::now() + 25s;
        while (std::chrono::steady_clock::now() < deadline)
        {
            poll();
            if (predicate())
                return;
            std::this_thread::sleep_for(10ms);
        }
        throw std::runtime_error(std::string(message) + "; last peer error: " + lastPeerError);
    };
    require(!client.set_udp_routes({udp}), "Initial SSL-uTP route rejected");
    wait([&] { return ready[0] && seed.ssl_listen_port() == seedPort; }, "SSL-uTP listeners did not become ready");
    lt::add_torrent_params add;
    add.ti = info;
    add.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    add.flags |= lt::torrent_flags::disable_dht | lt::torrent_flags::disable_lsd | lt::torrent_flags::disable_pex;
    add.save_path = (root / "seed").string();
    auto source = seed.add_torrent(add);
    source.set_ssl_certificate_buffer(certificate, key, dh);
    add.save_path = (root / "download").string();
    auto target = client.add_torrent(add);
    target.set_ssl_certificate_buffer(certificate, key, dh);
    wait([&] { return source.status().is_seeding && target.status().state == lt::torrent_status::downloading; },
        "SSL-uTP torrents did not finish initial hashing");
    const lt::tcp::endpoint peer {lt::make_address("127.0.0.10"), seedPort};
    const auto hasPeer = [&](const std::uint64_t generation)
    {
        std::vector<lt::peer_info> peers;
        target.get_peer_info(peers);
        for (const auto &connected : peers)
        {
            require(connected.ip == peer, "SOCKS relay became the peer identity");
            require(bool(connected.flags & lt::peer_info::ssl_socket)
                && bool(connected.flags & lt::peer_info::utp_socket), "Peer snapshot was not SSL-uTP");
            if (connected.route == lt::peer_route_context {9, generation})
                return true;
        }
        return false;
    };
    target.connect_peer(peer);
    wait([&] { return verified[0] >= 64 * 1024 && hasPeer(1); }, "Generation 1 did not verify SSL-uTP payload");
    require(!target.status().is_seeding, "SSL-uTP transfer completed before retirement");
    const lt::torrent_route_policy blocked {lt::torrent_route_policy::mode_t::managed, {}, {}};
    require(!client.set_torrent_route_policy_selector([blocked](const lt::torrent_route_request &) { return blocked; }),
        "SSL-uTP blocked policy rejected");
    client.invalidate_peer_route({9, 1});
    wait([&] { return retired && firstClosed && target.status().num_peers == 0; }, "Generation 1 remained connected after retirement");
    target.clear_peers();

    route.binding.context.generation = 2;
    udp.route.context.generation = 2;
    require(!client.set_udp_routes({udp}), "Generation 2 SSL-uTP route rejected");
    wait([&] { return ready[1]; }, "Generation 2 SSL-uTP route not ready");
    policy.routes = {route};
    policy.pinned = route.binding.context;
    require(!client.set_torrent_route_policy_selector([policy](const lt::torrent_route_request &) { return policy; }),
        "Generation 2 policy rejected");
    poll();
    staleRejected = false;
    target.connect_peer(peer); // The selector still returns the revoked generation.
    wait([&] { return staleRejected; }, "Revoked SSL-uTP generation was not rejected");
    target.clear_peers();
    selectedGeneration = 2;
    target.connect_peer(peer);
    wait([&] { return verified[1] > 0 && hasPeer(2); }, "Generation 2 did not verify SSL-uTP payload");
    wait([&] { return target.status().is_seeding; }, "SSL-uTP transfer did not complete");
    require(readFile(root / "download" / "ssl-utp.bin") == payload, "SSL-uTP payload differs from the generated source");
    require(verified[0] + verified[1] == static_cast<std::int64_t>(payload.size()), "SSL-uTP verified route accounting differs from exact payload");
    require(sslUtpConnections == 2, "Expected one SSL-uTP connection per admitted generation");
    std::cout << "{\"passed\":true,\"verifiedBytes\":" << payload.size()
        << ",\"generation1VerifiedBytes\":" << verified[0] << ",\"generation2VerifiedBytes\":" << verified[1]
        << ",\"socketType\":\"utp_ssl\",\"sslUtpConnections\":" << sslUtpConnections
        << ",\"retiredGeneration\":1,\"activeGeneration\":2,\"staleGenerationRejected\":true}\n";
    return 0;
}
catch (const std::exception &error)
{
    std::cerr << error.what() << '\n';
    return 1;
}
