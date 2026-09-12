"""Download generated legal data from one explicit loopback seed for integration."""

import json
import pathlib
import sys
import time

import libtorrent as lt

session = lt.session({
    "listen_interfaces": "127.0.0.1:0",
    "outgoing_interfaces": "127.0.0.1",
    "enable_dht": False,
    "enable_lsd": False,
    "enable_upnp": False,
    "enable_natpmp": False,
    "enable_outgoing_utp": False,
    "enable_incoming_utp": False,
})
destination = pathlib.Path(sys.argv[2])
destination.mkdir()
params = lt.add_torrent_params()
params.ti = lt.torrent_info(sys.argv[1])
params.save_path = str(destination)
params.flags &= ~(lt.torrent_flags.paused | lt.torrent_flags.auto_managed)
handle = session.add_torrent(params)
handle.connect_peer(("127.0.0.1", int(sys.argv[3])))
deadline = time.monotonic() + 60
while time.monotonic() < deadline:
    status = handle.status()
    if status.is_finished:
        handle.pause()
        print(json.dumps({"finished": True, "verified_bytes": status.total_wanted_done}))
        break
    time.sleep(0.05)
else:
    raise RuntimeError("Explicit loopback download did not finish")
