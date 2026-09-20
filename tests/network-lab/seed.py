"""A checked fixture seed restricted to the selected local address."""

import ipaddress
import json
import pathlib
import secrets
import socket
import sys
import threading
import time

import libtorrent as lt

if lt.__version__ != "2.0.14.0":
    raise RuntimeError("Install tests/fixtures/requirements.txt with --require-hashes")

torrent = pathlib.Path(sys.argv[1])
save_path = pathlib.Path(sys.argv[2])
expected_pieces = json.loads(sys.argv[3]) if len(sys.argv) > 3 else None
listen_address = sys.argv[4] if len(sys.argv) > 4 else "127.0.0.1"
upload_rate = int(sys.argv[5]) if len(sys.argv) > 5 else 256 * 1024
neighbor = json.loads(sys.argv[6]) if len(sys.argv) > 6 else None
transport = sys.argv[7] if len(sys.argv) > 7 else "tcp"
allowed_peer_addresses = json.loads(sys.argv[8]) if len(sys.argv) > 8 else []
if (not isinstance(allowed_peer_addresses, list) or len(allowed_peer_addresses) > 16
        or any(type(item) is not str for item in allowed_peer_addresses)):
    raise RuntimeError("Expected at most 16 numeric fixture peer addresses")
allowed_peer_addresses = [ipaddress.IPv4Address(item) for item in allowed_peer_addresses]
if any(not item.is_private or item.is_unspecified or item.is_multicast for item in allowed_peer_addresses):
    raise RuntimeError("Fixture peer addresses must be explicit private IPv4 addresses")
if transport not in ("tcp", "utp"):
    raise RuntimeError("Unsupported fixture transport")
address = ipaddress.IPv4Address(listen_address)
if not address.is_private or address.is_unspecified or address.is_multicast:
    raise RuntimeError("Fixture listener requires an explicit private local IPv4 address")
if not 1024 <= upload_rate <= 1024 * 1024:
    raise RuntimeError("Fixture upload rate must be bounded between 1 KiB/s and 1 MiB/s")
if neighbor is not None:
    if (not isinstance(neighbor, dict) or set(neighbor) != {"host", "port"}
            or type(neighbor["host"]) is not str or type(neighbor["port"]) is not int):
        raise RuntimeError("Invalid fixture neighbor")
    neighbor_address = ipaddress.IPv4Address(neighbor["host"])
    if (not neighbor_address.is_loopback or neighbor_address == address
            or not 0 <= neighbor["port"] <= 65535):
        raise RuntimeError("Fixture neighbor must use a distinct loopback address")
# libtorrent opens UDP on the TCP listen port even with uTP and DHT disabled.
# Windows may exclude a port for only one protocol; choose a port both can bind
# instead of treating a disabled-transport bind failure as a healthy seed.
for attempt in range(32):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as tcp, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
        # Protocol-specific Windows exclusions can cover many adjacent ports.
        port = 49152 + secrets.randbelow(16384)
        try:
            udp.bind((listen_address, port))
            tcp.bind((listen_address, port))
        except OSError:
            if attempt == 31:
                raise
            continue
        break
session = lt.session({
    "listen_interfaces": f"{listen_address}:{port}",
    "outgoing_interfaces": listen_address,
    "enable_dht": False,
    "enable_lsd": False,
    "enable_upnp": False,
    "enable_natpmp": False,
    "enable_incoming_utp": transport == "utp" and not (neighbor and neighbor["port"]),
    "enable_outgoing_utp": transport == "utp" and bool(neighbor and neighbor["port"]),
    "enable_incoming_tcp": transport == "tcp",
    "enable_outgoing_tcp": transport == "tcp" and bool(neighbor and neighbor["port"]),
    "dht_bootstrap_nodes": "",
    "upload_rate_limit": upload_rate,
    "ignore_limits_on_local_network": False,
    "close_redundant_connections": False if neighbor is not None else True,
    "connections_limit": 10,
    "alert_mask": lt.alert.category_t.error_notification | lt.alert.category_t.peer_notification,
})
peer_filter = lt.ip_filter()
peer_filter.add_rule("0.0.0.0", "255.255.255.255", 1)
peer_filter.add_rule("::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", 1)
peer_filter.add_rule(listen_address, listen_address, 0)
for peer_address in allowed_peer_addresses:
    peer_filter.add_rule(str(peer_address), str(peer_address), 0)
if neighbor is not None:
    # The exact-target SOCKS relay connects from the default loopback address.
    peer_filter.add_rule("127.0.0.1", "127.0.0.1", 0)
    peer_filter.add_rule(str(neighbor_address), str(neighbor_address), 0)
session.set_ip_filter(peer_filter)
params = lt.add_torrent_params()
params.ti = lt.torrent_info(str(torrent))
params.save_path = str(save_path)
if expected_pieces is not None:
    if (not expected_pieces or sorted(set(expected_pieces)) != expected_pieces
            or expected_pieces[0] < 0 or expected_pieces[-1] >= params.ti.num_pieces()):
        raise RuntimeError("Expected pieces must be a nonempty sorted subset")
    # The partial peer must never acquire the other peer's missing data.
    # Its actual files still pass through the normal native hash check.
    params.piece_priorities = [0] * params.ti.num_pieces()
params.flags &= ~lt.torrent_flags.auto_managed
params.flags &= ~lt.torrent_flags.paused
handle = session.add_torrent(params)
deadline = time.monotonic() + 30
while True:
    status = handle.status()
    pieces = [index for index, have in enumerate(status.pieces) if have]
    checked = status.state not in (lt.torrent_status.checking_files,
                                    lt.torrent_status.checking_resume_data)
    ready = (status.is_seeding if expected_pieces is None
             else checked and pieces == expected_pieces)
    if ready and session.listen_port() != 0:
        break
    if time.monotonic() > deadline:
        raise RuntimeError("Controlled seed did not verify its payload")
    for alert in session.pop_alerts():
        if isinstance(alert, (lt.torrent_error_alert, lt.listen_failed_alert)):
            raise RuntimeError(alert.message())
    time.sleep(0.05)
if neighbor is not None and neighbor["port"]:
    handle.connect_peer((str(neighbor_address), neighbor["port"]))
    deadline = time.monotonic() + 20
    peer_errors = []
    while not any(peer.ip[0] == str(neighbor_address) and peer.progress > 0
                  for peer in handle.get_peer_info()):
        if time.monotonic() > deadline:
            peers = [(peer.ip[0], peer.progress, peer.client) for peer in handle.get_peer_info()]
            raise RuntimeError(f"Fixture peers did not establish their bootstrap link: {peers}; alerts: {peer_errors}")
        for alert in session.pop_alerts():
            if isinstance(alert, (lt.peer_error_alert, lt.peer_disconnected_alert)) and len(peer_errors) < 16:
                peer_errors.append(alert.message())
        time.sleep(0.05)
print(json.dumps({"ready": True, "host": listen_address, "port": session.listen_port(),
                  "libtorrent": lt.__version__, "pieces": pieces,
                  "allowedPeerAddresses": [str(item) for item in allowed_peer_addresses],
                  "verifiedPayloadBytes": status.total_done}), flush=True)
finished = threading.Event()
command_errors = []


def await_commands():
    try:
        for line in sys.stdin:
            command = json.loads(line)
            if (set(command) != {"controlId", "uploadRate"} or type(command["controlId"]) is not int
                    or command["controlId"] <= 0 or type(command["uploadRate"]) is not int
                    or not 1024 <= command["uploadRate"] <= 1024 * 1024):
                raise RuntimeError("Invalid seed control command")
            session.apply_settings({"upload_rate_limit": command["uploadRate"]})
            deadline = time.monotonic() + 2
            while session.get_settings()["upload_rate_limit"] != command["uploadRate"]:
                if time.monotonic() >= deadline:
                    raise RuntimeError("Upload rate was not applied before acknowledgement")
                time.sleep(0.005)
            print(json.dumps(command), flush=True)
    except Exception as error:
        command_errors.append(error)
    finally:
        finished.set()


threading.Thread(target=await_commands, daemon=True).start()
peer_addresses = set()
outgoing_peer_addresses = set()
while not finished.wait(0.05):
    peers = handle.get_peer_info()
    peer_addresses.update(peer.ip[0] for peer in peers)
    outgoing_peer_addresses.update(peer.ip[0] for peer in peers if peer.flags & lt.peer_info.local_connection)
if command_errors:
    raise RuntimeError("Seed control failed") from command_errors[0]
status = handle.status()
pieces = [index for index, have in enumerate(status.pieces) if have]
if expected_pieces is not None and (pieces != expected_pieces or status.total_payload_download != 0):
    raise RuntimeError("Partial peer acquired data outside its original checked subset")
print(json.dumps({"uploadPayloadBytes": status.total_payload_upload,
                  "downloadPayloadBytes": status.total_payload_download, "pieces": pieces,
                  "peerAddresses": sorted(peer_addresses),
                  "outgoingPeerAddresses": sorted(outgoing_peer_addresses)}), flush=True)
del handle
del session
