"""A checked, loopback-only, TCP controlled seeder; stdin EOF shuts it down."""

import json
import pathlib
import socket
import sys
import time

import libtorrent as lt

if lt.__version__ != "2.0.14.0":
    raise RuntimeError("Install tests/fixtures/requirements.txt with --require-hashes")

torrent = pathlib.Path(sys.argv[1])
save_path = pathlib.Path(sys.argv[2])
# libtorrent opens UDP on the TCP listen port even with uTP and DHT disabled.
# Windows may exclude a port for only one protocol; choose a port both can bind
# instead of treating a disabled-transport bind failure as a healthy seed.
for attempt in range(32):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as tcp, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
        tcp.bind(("127.0.0.1", 0))
        port = tcp.getsockname()[1]
        try:
            udp.bind(("127.0.0.1", port))
        except OSError:
            if attempt == 31:
                raise
            continue
        break
session = lt.session({
    "listen_interfaces": f"127.0.0.1:{port}",
    "outgoing_interfaces": "127.0.0.1",
    "enable_dht": False,
    "enable_lsd": False,
    "enable_upnp": False,
    "enable_natpmp": False,
    "enable_incoming_utp": False,
    "enable_outgoing_utp": False,
    "enable_incoming_tcp": True,
    "enable_outgoing_tcp": False,
    "dht_bootstrap_nodes": "",
    "upload_rate_limit": 256 * 1024,
    "ignore_limits_on_local_network": False,
    "connections_limit": 10,
})
params = lt.add_torrent_params()
params.ti = lt.torrent_info(str(torrent))
params.save_path = str(save_path)
params.flags &= ~lt.torrent_flags.auto_managed
params.flags &= ~lt.torrent_flags.paused
handle = session.add_torrent(params)
deadline = time.monotonic() + 30
while not handle.status().is_seeding or session.listen_port() == 0:
    if time.monotonic() > deadline:
        raise RuntimeError("Controlled seed did not verify its payload")
    for alert in session.pop_alerts():
        if isinstance(alert, (lt.torrent_error_alert, lt.listen_failed_alert)):
            raise RuntimeError(alert.message())
    time.sleep(0.05)
print(json.dumps({"ready": True, "host": "127.0.0.1", "port": session.listen_port(), "libtorrent": lt.__version__}), flush=True)
sys.stdin.readline()
status = handle.status()
print(json.dumps({"uploadPayloadBytes": status.total_payload_upload, "downloadPayloadBytes": status.total_payload_download}), flush=True)
del handle
del session
