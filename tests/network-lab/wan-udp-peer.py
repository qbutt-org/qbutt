"""One checked uTP seed and bounded inbound DHT probes through an owned SOCKS path."""
import ipaddress
import json
import os
import pathlib
import queue
import secrets
import socket
import struct
import sys
import threading
import time

import libtorrent as lt


def emit(value):
    print(json.dumps(value, separators=(",", ":")), flush=True)


def line():
    result = bytearray()
    while len(result) <= 16384:
        value = os.read(sys.stdin.fileno(), 1)
        if not value or value == b"\n":
            return bytes(result)
        result.extend(value)
    raise ValueError("Control line exceeds limit")


def exact(connection, length):
    result = bytearray()
    while len(result) < length:
        value = connection.recv(length - len(result))
        if not value:
            raise ConnectionError("SOCKS control closed")
        result.extend(value)
    return bytes(result)


def probe_dht(proxy, target, info_hash):
    # Keep TCP association alive until both replies. This source has no direct
    # public UDP socket: only the selected authenticated loopback relay is used.
    with socket.create_connection(("127.0.0.1", proxy["port"]), timeout=15) as control:
        control.sendall(b"\x05\x01\x02")
        if exact(control, 2) != b"\x05\x02":
            raise RuntimeError("DHT SOCKS authentication negotiation rejected")
        user, password = proxy["username"].encode(), proxy["password"].encode()
        control.sendall(b"\x01" + bytes([len(user)]) + user + bytes([len(password)]) + password)
        if exact(control, 2) != b"\x01\x00":
            raise RuntimeError("DHT SOCKS authentication rejected")
        control.sendall(b"\x05\x03\x00\x01" + bytes(6))
        if exact(control, 4) != b"\x05\x00\x00\x01":
            raise RuntimeError("DHT SOCKS UDP association rejected")
        relay = (socket.inet_ntoa(exact(control, 4)), int.from_bytes(exact(control, 2), "big"))
        if not ipaddress.IPv4Address(relay[0]).is_loopback or not relay[1]:
            raise RuntimeError("DHT relay is not the owned loopback endpoint")
        header = b"\x00\x00\x00\x01" + socket.inet_aton(target[0]) + struct.pack("!H", target[1])
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.bind(("127.0.0.1", 0))
            udp.connect(relay)
            udp.settimeout(3)
            node = secrets.token_bytes(20)
            results = {}
            for name in ("ping", "get_peers"):
                transaction = secrets.token_bytes(4)
                args = {b"id": node}
                if name == "get_peers":
                    args[b"info_hash"] = info_hash
                packet = header + lt.bencode({b"a": args, b"q": name.encode(), b"ro": 1,
                                             b"t": transaction, b"y": b"q"})
                deadline = time.monotonic() + 12
                while time.monotonic() < deadline:
                    udp.send(packet)
                    try:
                        received = udp.recv(4107)
                    except socket.timeout:
                        continue
                    if not 10 < len(received) <= 4106 or received[:10] != header:
                        raise RuntimeError("DHT reply did not preserve the public lease endpoint")
                    response = lt.bdecode(received[10:])
                    if response.get(b"t") != transaction:
                        continue
                    result = response.get(b"r", {})
                    if response.get(b"y") != b"r" or len(result.get(b"id", b"")) != 20:
                        raise RuntimeError("DHT response is not a successful node reply")
                    results[name] = {"id": result[b"id"].hex(), "source": f"{target[0]}:{target[1]}",
                                     "token": bool(result.get(b"token"))}
                    break
                else:
                    raise RuntimeError(f"Inbound WAN DHT {name} timed out")
            if results["ping"]["id"] != results["get_peers"]["id"] or not results["get_peers"]["token"]:
                raise RuntimeError("DHT node changed identity or omitted get_peers token")
            return results


def main():
    if lt.__version__ != "2.0.14.0":
        raise RuntimeError("Unexpected pinned libtorrent fixture binding")
    config = json.loads(line())
    proxy = config["connectProxy"]
    if type(proxy["port"]) is not int or not 1 <= proxy["port"] <= 65535:
        raise ValueError("Invalid loopback SOCKS port")
    for field in ("username", "password"):
        if not isinstance(proxy[field], str) or not 1 <= len(proxy[field].encode()) <= 255:
            raise ValueError("Invalid loopback SOCKS credentials")
    target = config["connectTarget"]
    target = (str(ipaddress.IPv4Address(target["host"])), target["port"])
    if type(target[1]) is not int or not 49152 <= target[1] <= 65535:
        raise ValueError("Target must use an owned high UDP port")
    for attempt in range(32):
        port = 49152 + secrets.randbelow(16384)
        with socket.socket() as tcp, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            try:
                tcp.bind(("127.0.0.1", port))
                udp.bind(("127.0.0.1", port))
                break
            except OSError:
                if attempt == 31:
                    raise
    session = lt.session({
        "listen_interfaces": f"127.0.0.1:{port}", "outgoing_interfaces": "127.0.0.1",
        "enable_incoming_tcp": False, "enable_outgoing_tcp": False,
        "enable_incoming_utp": False, "enable_outgoing_utp": True,
        "enable_dht": False, "enable_lsd": False, "enable_upnp": False, "enable_natpmp": False,
        "dht_bootstrap_nodes": "", "connections_limit": 2, "upload_rate_limit": 131072,
        "ignore_limits_on_local_network": False, "proxy_type": int(lt.proxy_type_t.socks5_pw),
        "proxy_hostname": "127.0.0.1", "proxy_port": proxy["port"],
        "proxy_username": proxy["username"], "proxy_password": proxy["password"],
        "proxy_peer_connections": True, "proxy_hostnames": True,
        "alert_mask": lt.alert.category_t.error_notification | lt.alert.category_t.peer_notification,
    })
    ip_filter = lt.ip_filter()
    ip_filter.add_rule("0.0.0.0", "255.255.255.255", 1)
    ip_filter.add_rule("::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", 1)
    ip_filter.add_rule(target[0], target[0], 0)
    session.set_ip_filter(ip_filter)
    params = lt.add_torrent_params()
    params.ti = lt.torrent_info(str(pathlib.Path(config["torrent"])))
    if params.ti.trackers() or params.ti.num_files() != 1 or params.ti.total_size() != 524288:
        raise ValueError("Expected the generated trackerless 512 KiB fixture")
    if str(params.ti.info_hashes().v1) != config["infoHash"]:
        raise ValueError("Generated torrent infohash differs")
    params.save_path = str(pathlib.Path(config["savePath"]))
    params.flags &= ~lt.torrent_flags.auto_managed
    params.flags &= ~lt.torrent_flags.paused
    handle = session.add_torrent(params)
    deadline = time.monotonic() + 30
    while not handle.status().is_seeding:
        for alert in session.pop_alerts():
            if isinstance(alert, (lt.torrent_error_alert, lt.listen_failed_alert)):
                raise RuntimeError(alert.message())
        if time.monotonic() >= deadline:
            raise RuntimeError("WAN UDP source did not verify its payload")
        time.sleep(.05)
    emit({"ready": True, "verifiedPayloadBytes": handle.status().total_done, "protocol": "utp"})
    commands = queue.Queue(maxsize=4)

    def control():
        try:
            while True:
                text = line()
                command = json.loads(text) if text else {"command": "stop"}
                commands.put_nowait(command)
                if command.get("command") == "stop":
                    return
        except Exception:
            commands.put({"command": "invalid"})

    threading.Thread(target=control, daemon=True).start()
    started = False
    probed = False
    endpoints = set()
    peer_errors = []
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        for peer in handle.get_peer_info():
            if not peer.flags & lt.peer_info.utp_socket or not peer.flags & lt.peer_info.local_connection:
                raise RuntimeError("WAN source used a non-outgoing-uTP peer")
            if peer.ip != target:
                raise RuntimeError("WAN source connected outside the owned lease")
            endpoints.add(f"{peer.ip[0]}:{peer.ip[1]}")
        for alert in session.pop_alerts():
            if isinstance(alert, (lt.torrent_error_alert, lt.listen_failed_alert)):
                raise RuntimeError(alert.message())
            if isinstance(alert, (lt.peer_error_alert, lt.peer_disconnected_alert)) and len(peer_errors) < 8:
                peer_errors.append(alert.message())
        try:
            command = commands.get(timeout=.05).get("command")
        except queue.Empty:
            continue
        if command == "stop":
            status = handle.status()
            emit({"stopped": True, "uploadPayloadBytes": status.total_payload_upload,
                  "downloadPayloadBytes": status.total_payload_download, "remoteEndpoints": sorted(endpoints),
                  "protocol": "utp", "peerErrors": peer_errors})
            return
        if command == "dht" and not started and not probed:
            emit({"dht": probe_dht(proxy, target, bytes.fromhex(config["infoHash"]))})
            probed = True
        elif command == "start" and not started:
            started = True
            handle.connect_peer(target)
            emit({"started": True})
        else:
            raise ValueError("Invalid or repeated WAN UDP command")
    raise RuntimeError("WAN UDP source watchdog expired")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        emit({"error": str(error)})
        sys.exit(1)
