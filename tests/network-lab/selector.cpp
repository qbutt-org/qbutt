/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include <array>
#include <atomic>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>

#include <libtorrent/add_torrent_params.hpp>
#include <libtorrent/alert_types.hpp>
#include <libtorrent/bencode.hpp>
#include <libtorrent/create_torrent.hpp>
#include <libtorrent/file_storage.hpp>
#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/torrent_status.hpp>

#include "../../src/base/net/peerrouteselector.h"

namespace lt = libtorrent;
namespace fs = std::filesystem;
using namespace std::chrono_literals;

namespace
{
    unsigned int readInt(const char *data)
    {
        unsigned int result = 0;
        for (int index = 0; index < 4; ++index)
            result = (result << 8) | static_cast<unsigned char>(data[index]);
        return result;
    }

    void writeInt(char *data, const unsigned int value)
    {
        for (int index = 0; index < 4; ++index)
            data[index] = static_cast<char>(value >> (24 - index * 8));
    }

    bool exchange(lt::tcp::socket &socket, char *data, const std::size_t length,
        const bool write, const std::stop_token stop)
    {
        std::size_t offset = 0;
        const auto deadline = std::chrono::steady_clock::now() + 20s;
        while ((offset < length) && !stop.stop_requested() && (std::chrono::steady_clock::now() < deadline))
        {
            lt::error_code error;
            const auto buffer = boost::asio::buffer(data + offset, length - offset);
            offset += write ? socket.write_some(buffer, error) : socket.read_some(buffer, error);
            if (error && (error != boost::asio::error::would_block) && (error != boost::asio::error::try_again))
                return false;
            if (error)
                std::this_thread::sleep_for(2ms);
        }
        return offset == length;
    }

    // Minimal real BitTorrent peer for generated legal fixtures. The first
    // connection stays choked, serves one accepted block, then sends TCP RST.
    // The next connection supplies the rest; a separate run corrupts all data.
    class FixturePeer
    {
    public:
        FixturePeer(const std::vector<char> &payload, const int pieceSize, const bool corrupt, const bool failover)
            : m_acceptor {m_io, {lt::address_v4::loopback(), 0}}
            , m_thread {[this, &payload, pieceSize, corrupt, failover](const std::stop_token stop)
                { run(payload, pieceSize, corrupt, failover, stop); }}
        {
        }

        unsigned short port() const
        {
            return m_acceptor.local_endpoint().port();
        }

        std::atomic<int> connections = 0;
        std::atomic<int> firstBlockBytes = 0;

    private:
        void run(const std::vector<char> &payload, const int pieceSize, const bool corrupt,
            const bool failover, const std::stop_token stop)
        {
            m_acceptor.non_blocking(true);
            while (!stop.stop_requested())
            {
                lt::tcp::socket socket {m_io};
                lt::error_code error;
                m_acceptor.accept(socket, error);
                if (error)
                {
                    std::this_thread::sleep_for(2ms);
                    continue;
                }
                socket.non_blocking(true);
                const int connection = ++connections;
                std::array<char, 68> handshake;
                if (!exchange(socket, handshake.data(), handshake.size(), false, stop))
                    continue;
                std::fill(handshake.begin() + 20, handshake.begin() + 28, 0);
                std::fill(handshake.begin() + 48, handshake.end(), 'q');
                handshake.back() = static_cast<char>('0' + connection);
                if (!exchange(socket, handshake.data(), handshake.size(), true, stop))
                    continue;
                const std::size_t pieces = (payload.size() + pieceSize - 1) / pieceSize;
                std::vector<char> bitfield(5 + (pieces + 7) / 8);
                writeInt(bitfield.data(), static_cast<unsigned int>(bitfield.size() - 4));
                bitfield[4] = 5;
                for (std::size_t index = 0; index < pieces; ++index)
                    bitfield[5 + index / 8] |= static_cast<char>(0x80 >> (index % 8));
                if (!exchange(socket, bitfield.data(), bitfield.size(), true, stop))
                    continue;
                if (failover && (connection == 1))
                {
                    const auto unchokeAt = std::chrono::steady_clock::now() + 2200ms;
                    while (!stop.stop_requested() && (std::chrono::steady_clock::now() < unchokeAt))
                        std::this_thread::sleep_for(10ms);
                }
                std::array<char, 5> unchoke {0, 0, 0, 1, 1};
                if (!exchange(socket, unchoke.data(), unchoke.size(), true, stop))
                    continue;
                while (!stop.stop_requested())
                {
                    std::array<char, 4> prefix;
                    if (!exchange(socket, prefix.data(), prefix.size(), false, stop))
                        break;
                    const unsigned int size = readInt(prefix.data());
                    if (size == 0)
                        continue;
                    if (size > 65536)
                        break;
                    std::vector<char> message(size);
                    if (!exchange(socket, message.data(), message.size(), false, stop))
                        break;
                    if ((message.front() != 6) || (size != 13))
                        continue;
                    const unsigned int piece = readInt(message.data() + 1);
                    const unsigned int offset = readInt(message.data() + 5);
                    const unsigned int length = readInt(message.data() + 9);
                    const std::size_t payloadOffset = piece * pieceSize + offset;
                    if ((length > 16384) || (offset + length > pieceSize) || (payloadOffset + length > payload.size()))
                        break;
                    std::vector<char> response(13 + length);
                    writeInt(response.data(), 9 + length);
                    response[4] = 7;
                    writeInt(response.data() + 5, piece);
                    writeInt(response.data() + 9, offset);
                    for (unsigned int index = 0; index < length; ++index)
                        response[13 + index] = payload[payloadOffset + index] ^ (corrupt ? 0x5a : 0);
                    if (!exchange(socket, response.data(), response.size(), true, stop))
                        break;
                    if (failover && (connection == 2))
                        std::this_thread::sleep_for(50ms);
                    if (failover && (connection == 1))
                    {
                        firstBlockBytes = static_cast<int>(length);
                        std::this_thread::sleep_for(150ms);
                        socket.set_option(lt::tcp::socket::linger {true, 0});
                        socket.close(error);
                        break;
                    }
                }
            }
        }

        lt::io_context m_io;
        lt::tcp::acceptor m_acceptor;
        std::jthread m_thread;
    };
}

int main(const int argc, char **argv)
{
    if ((argc != 3) || ((std::string(argv[2]) != "provenance") && (std::string(argv[2]) != "corrupt")
        && (std::string(argv[2]) != "unchecked") && (std::string(argv[2]) != "v2-late")
        && (std::string(argv[2]) != "padding")))
        return 2;
    const bool unchecked = std::string(argv[2]) == "unchecked";
    const bool corrupt = (std::string(argv[2]) == "corrupt") || unchecked;
    const bool v2Late = std::string(argv[2]) == "v2-late";
    const bool padding = std::string(argv[2]) == "padding";
    const bool failover = std::string(argv[2]) == "provenance";
    const fs::path root {argv[1]};
    if (fs::exists(root))
        return 3;
    fs::create_directories(root / "seed");
    fs::create_directories(root / "download");
    std::vector<char> payload(512 * 1024);
    const std::size_t fileBytes = padding ? 509000 : payload.size();
    for (std::size_t index = 0; index < fileBytes; ++index)
        payload[index] = static_cast<char>((index * 31 + index / 127) & 255);
    const fs::path payloadPath = padding ? fs::path {"fixture/payload.bin"} : fs::path {"payload.bin"};
    fs::create_directories((root / "seed" / payloadPath).parent_path());
    std::ofstream(root / "seed" / payloadPath, std::ios::binary).write(payload.data(), fileBytes);
    lt::file_storage files;
    files.add_file(payloadPath.generic_string(), fileBytes);
    if (padding)
        files.add_file("fixture/.pad/15288", payload.size() - fileBytes, lt::file_storage::flag_pad_file);
    const int pieceSize = v2Late ? 64 * 1024 : static_cast<int>(payload.size());
    lt::create_torrent creator {files, pieceSize, v2Late ? lt::create_torrent::v2_only : lt::create_torrent::v1_only};
    lt::error_code error;
    lt::set_piece_hashes(creator, (root / "seed").string(), error);
    if (error)
        return 4;
    std::vector<char> metadata;
    lt::entry metadataEntry = creator.generate();
    if (v2Late)
        metadataEntry.dict().erase("piece layers");
    lt::bencode(std::back_inserter(metadata), metadataEntry);
    auto info = std::make_shared<lt::torrent_info>(lt::span<char const>(metadata), error, lt::from_span);
    if (error)
    {
        std::cerr << error.message() << '\n';
        return 5;
    }

    FixturePeer peer {payload, pieceSize, corrupt, failover};
    lt::settings_pack settings;
    for (const auto setting : {lt::settings_pack::enable_dht, lt::settings_pack::enable_lsd,
        lt::settings_pack::enable_upnp, lt::settings_pack::enable_natpmp,
        lt::settings_pack::enable_outgoing_utp, lt::settings_pack::enable_incoming_utp,
        lt::settings_pack::enable_incoming_tcp})
        settings.set_bool(setting, false);
    settings.set_str(lt::settings_pack::listen_interfaces, "");
    settings.set_bool(lt::settings_pack::disable_hash_checks, unchecked);
    settings.set_int(lt::settings_pack::alert_mask, lt::alert_category::all);
    settings.set_int(lt::settings_pack::min_reconnect_time, 1);
    settings.set_int(lt::settings_pack::out_enc_policy, lt::settings_pack::pe_disabled);
    settings.set_int(lt::settings_pack::in_enc_policy, lt::settings_pack::pe_disabled);
    std::vector<lt::peer_route> routes;
    for (unsigned int index = 0; index < 3; ++index)
    {
        lt::peer_route route;
        route.type = lt::peer_route::type_t::native;
        route.context = {index + 1, 7};
        route.local_endpoint = {lt::address_v4::loopback(), 0};
        routes.push_back(std::move(route));
    }
    auto selector = std::make_shared<Net::PeerRouteSelector>(std::move(routes), true);
    std::array<std::atomic<std::int64_t>, 4> verified {};
    std::atomic<std::int64_t> chokedMilliseconds = 0;
    std::atomic<int> firstRoute = 0;
    std::atomic<int> secondRoute = 0;
    std::atomic<int> attempts = 0;
    std::atomic<int> firstPathAttempts = 0;
    std::atomic<int> thirdPathAttempts = 0;
    std::atomic<int> unknownMetadataPath = 0;
    std::atomic<int> privatePath = 0;
    std::atomic<int> failures = 0;
    lt::session client {settings};
    client.set_peer_route_selector([&](const lt::peer_route_request &request)
    {
        const lt::peer_route route = selector->select(request);
        const int attempt = ++attempts;
        if (route.context.path_id == 1)
            ++firstPathAttempts;
        else if (route.context.path_id == 3)
            ++thirdPathAttempts;
        if (!request.has_metadata)
            unknownMetadataPath = static_cast<int>(route.context.path_id);
        if (request.private_torrent)
            privatePath = static_cast<int>(route.context.path_id);
        if (attempt == 1)
            firstRoute = static_cast<int>(route.context.path_id);
        else if (attempt == 2)
            secondRoute = static_cast<int>(route.context.path_id);
        return route;
    }, [&](const lt::peer_route_observation &observation)
    {
        selector->observe(observation);
        if (observation.route.path_id < verified.size())
            verified[observation.route.path_id] += observation.verified_download;
        chokedMilliseconds += observation.choked_duration_ms;
        if ((observation.event == lt::peer_route_observation::event_t::closed)
            && (observation.error == boost::asio::error::connection_reset))
            ++failures;
    });
    lt::add_torrent_params add;
    add.ti = info;
    add.save_path = (root / "download").string();
    add.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    const lt::torrent_handle target = client.add_torrent(add);
    const lt::tcp::endpoint endpoint {lt::address_v4::loopback(), peer.port()};
    target.connect_peer(endpoint);
    bool hashFailed = false;
    const auto deadline = std::chrono::steady_clock::now() + 40s;
    while (std::chrono::steady_clock::now() < deadline)
    {
        std::vector<lt::alert *> alerts;
        client.pop_alerts(&alerts);
        for (const lt::alert *alert : alerts)
            hashFailed |= lt::alert_cast<lt::hash_failed_alert>(alert) != nullptr;
        if (target.status().is_seeding || (corrupt && hashFailed))
            break;
        if (failover && (failures > 0))
            target.connect_peer(endpoint);
        std::this_thread::sleep_for(50ms);
    }
    const std::int64_t credited = verified[1] + verified[2] + verified[3];
    if (corrupt)
    {
        client.set_peer_route_selector({});
        if ((unchecked ? !target.status().is_seeding : !hashFailed)
            || (credited != 0) || (target.status().total_payload_download == 0))
        {
            std::cerr << "seed=" << target.status().is_seeding << " hashFailed=" << hashFailed
                << " credited=" << credited << " payload=" << target.status().total_payload_download << '\n';
            return 6;
        }
        std::cout << "{\"passed\":true,\"corruptPayloadVerifiedBytes\":0,\"hashChecksDisabled\":"
            << (unchecked ? "true" : "false") << "}\n";
        return 0;
    }
    std::ifstream downloaded(root / "download" / payloadPath, std::ios::binary);
    const std::vector<char> actual((std::istreambuf_iterator<char>(downloaded)), {});
    if (v2Late || padding)
    {
        client.set_peer_route_selector({});
        if (!target.status().is_seeding || (actual.size() != fileBytes)
            || !std::equal(actual.begin(), actual.end(), payload.begin())
            || (credited != static_cast<std::int64_t>(fileBytes)))
        {
            std::cerr << "seed=" << target.status().is_seeding << " actual=" << actual.size()
                << " credited=" << credited << " expected=" << fileBytes << '\n';
            return 11;
        }
        std::cout << "{\"passed\":true,\"verifiedBytes\":" << credited
            << ",\"lateV2Hashes\":" << (v2Late ? "true" : "false")
            << ",\"paddingExcluded\":" << (padding ? "true" : "false") << "}\n";
        return 0;
    }
    if (!target.status().is_seeding || (actual != payload) || (failures == 0)
        || (firstRoute != 1) || (secondRoute != 2) || (peer.connections != 2)
        || (chokedMilliseconds < 1000) || (verified[1] != peer.firstBlockBytes)
        || (credited != static_cast<std::int64_t>(payload.size())))
    {
        std::cerr << "seed=" << target.status().is_seeding << " bytes=" << actual.size()
            << " failed=" << failures << " routes=" << firstRoute << ',' << secondRoute
            << " connections=" << peer.connections << " chokeMs=" << chokedMilliseconds
            << " verified=" << verified[1] << ',' << verified[2] << '\n';
        return 7;
    }

    // New infohashes make real new dials through the same selector. The measured
    // productive route beats an untried failure-free route except for the
    // admitted 10% exploration budget; failure avoidance alone cannot pass.
    const std::int64_t originalCredit = verified[1];
    const std::int64_t retryCredit = verified[2];
    for (int index = 0; index < 20; ++index)
    {
        lt::entry entry = creator.generate();
        entry["info"]["name"] = "payload-" + std::to_string(index) + ".bin";
        metadata.clear();
        lt::bencode(std::back_inserter(metadata), entry);
        add.ti = std::make_shared<lt::torrent_info>(lt::span<char const>(metadata), error, lt::from_span);
        const lt::torrent_handle next = client.add_torrent(add);
        next.connect_peer(endpoint);
        const auto transferDeadline = std::chrono::steady_clock::now() + 8s;
        while (!next.status().is_seeding && (std::chrono::steady_clock::now() < transferDeadline))
            std::this_thread::sleep_for(20ms);
        if (!next.status().is_seeding)
            return 8;
        client.remove_torrent(next);
    }
    const int explored = firstPathAttempts + thirdPathAttempts - 1;
    const int publicAttempts = attempts;
    if ((publicAttempts != 22) || (explored != publicAttempts / 10))
    {
        std::cerr << "publicAttempts=" << publicAttempts << " explored=" << explored << '\n';
        return 9;
    }

    lt::entry privateEntry = creator.generate();
    privateEntry["info"]["name"] = "private.bin";
    privateEntry["info"]["private"] = 1;
    metadata.clear();
    lt::bencode(std::back_inserter(metadata), privateEntry);
    add.ti = std::make_shared<lt::torrent_info>(lt::span<char const>(metadata), error, lt::from_span);
    const lt::torrent_handle privateTorrent = client.add_torrent(add);
    privateTorrent.connect_peer(endpoint);
    const auto privateDeadline = std::chrono::steady_clock::now() + 8s;
    while ((privatePath == 0) && (std::chrono::steady_clock::now() < privateDeadline))
        std::this_thread::sleep_for(20ms);
    client.remove_torrent(privateTorrent);

    lt::add_torrent_params magnet;
    magnet.info_hashes = lt::info_hash_t {lt::sha1_hash("01234567890123456789")};
    magnet.save_path = (root / "download").string();
    magnet.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    const lt::torrent_handle unknown = client.add_torrent(magnet);
    unknown.connect_peer(endpoint);
    const auto metadataDeadline = std::chrono::steady_clock::now() + 8s;
    while ((unknownMetadataPath == 0) && (std::chrono::steady_clock::now() < metadataDeadline))
        std::this_thread::sleep_for(20ms);
    client.set_peer_route_selector({});
    if ((privatePath != 1) || (unknownMetadataPath != 1))
        return 10;

    std::cout << "{\"passed\":true,\"verifiedBytes\":" << credited
        << ",\"originalRouteVerifiedBytes\":" << originalCredit
        << ",\"retryRouteVerifiedBytes\":" << retryCredit
        << ",\"chokedMilliseconds\":" << chokedMilliseconds
        << ",\"publicAttempts\":" << publicAttempts << ",\"explorationAttempts\":" << explored
        << ",\"privatePinned\":true,\"unknownMetadataPinned\":true"
        << ",\"networkFailureRetry\":true,\"immutableBlockOrigin\":true}\n";
}
