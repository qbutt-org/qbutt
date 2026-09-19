"""Probe three independent high ports on the observer without changing its configuration."""

import json
import secrets
import socket


def available(port, protocol):
    kind = socket.SOCK_STREAM if protocol == "tcp" else socket.SOCK_DGRAM
    with socket.socket(socket.AF_INET, kind) as candidate:
        candidate.bind(("0.0.0.0", port))


ports = []
for role in ("control", "datagrams", "listener"):
    for attempt in range(256):
        port = 49152 + secrets.randbelow(16384)
        if port in ports:
            continue
        try:
            available(port, "tcp")
            available(port, "udp")
        except OSError:
            continue
        ports.append(port)
        break
    else:
        raise RuntimeError("No unused high port for " + role)

print(json.dumps(dict(zip(("control", "datagrams", "listener"), ports))))
