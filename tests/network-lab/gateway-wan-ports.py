"""Probe three independent high ports on the observer without changing its configuration."""

import json
import secrets
import socket
import sys


def available(port, protocol, ipv6):
    kind = socket.SOCK_STREAM if protocol == "tcp" else socket.SOCK_DGRAM
    with socket.socket(socket.AF_INET6 if ipv6 else socket.AF_INET, kind) as candidate:
        candidate.bind(("::" if ipv6 else "0.0.0.0", port))


ports = []
if sys.argv[1:] not in ([], ["--ipv6"]):
    raise ValueError("Only --ipv6 is supported")
for role in ("control", "datagrams", "listener"):
    for attempt in range(256):
        port = 49152 + secrets.randbelow(16384)
        if port in ports:
            continue
        try:
            ipv6 = role == "listener" and "--ipv6" in sys.argv
            available(port, "tcp", ipv6)
            available(port, "udp", ipv6)
        except OSError:
            continue
        ports.append(port)
        break
    else:
        raise RuntimeError("No unused high port for " + role)

print(json.dumps(dict(zip(("control", "datagrams", "listener"), ports))))
