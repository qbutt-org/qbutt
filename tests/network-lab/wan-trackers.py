"""Bounded HTTP/UDP trackers for one generated torrent on an independent host."""
import ipaddress
import json
import os
import secrets
import selectors
import socket
import struct
import sys
import time
import urllib.parse


def emit(value):
    print(json.dumps(value, separators=(",", ":")), flush=True)


config = json.loads(sys.stdin.readline(4097))
assert set(config) == {"address", "infoHash", "token", "selfPeer"}
address = str(ipaddress.IPv4Address(config["address"]))
info_hash = bytes.fromhex(config["infoHash"])
assert len(info_hash) == 20 and len(config["token"]) == 32
assert len(config["selfPeer"]) == 2 and 49152 <= config["selfPeer"][1] <= 65535
peer_bytes = ipaddress.IPv4Address(config["selfPeer"][0]).packed + struct.pack("!H", config["selfPeer"][1])
selector = selectors.DefaultSelector()
sockets = []
ports = {}
clients = {}
transactions = {}
observations = []
control = bytearray()
packets = 0


def close_client(connection):
    selector.unregister(connection)
    clients.pop(connection, None)
    connection.close()


try:
    for role in ("http", "udp"):
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM if role == "http" else socket.SOCK_DGRAM)
        sockets.append(listener)
        for _ in range(64):
            port = 49152 + secrets.randbelow(16384)
            if port in ports.values():
                continue
            try:
                listener.bind(("0.0.0.0", port))
                break
            except OSError:
                continue
        else:
            raise RuntimeError("No unused high tracker port")
        if role == "http":
            listener.listen(8)
        listener.setblocking(False)
        ports[role] = port
        selector.register(listener, selectors.EVENT_READ, role)
    selector.register(sys.stdin, selectors.EVENT_READ, "control")
    emit({"ready": True, "ports": ports})
    deadline = time.monotonic() + 220
    running = True
    while running and time.monotonic() < deadline:
        for connection, state in list(clients.items()):
            if time.monotonic() > state["deadline"]:
                close_client(connection)
        transactions = {key: value for key, value in transactions.items() if value[1] > time.monotonic()}
        for key, _ in selector.select(0.25):
            if key.data == "control":
                chunk = os.read(sys.stdin.fileno(), 4096)
                if not chunk:
                    running = False
                    break
                control.extend(chunk)
                assert len(control) <= 4096
                while b"\n" in control:
                    line, _, control = control.partition(b"\n")
                    command = json.loads(line)
                    if command == {"command": "clear-peers"}:
                        peer_bytes = b""
                        emit({"cleared": True})
                    else:
                        assert command == {"command": "snapshot"}
                        emit({"observations": observations, "packets": packets})
            elif key.data == "http":
                connection, source = key.fileobj.accept()
                if len(clients) >= 8:
                    connection.close()
                    continue
                connection.setblocking(False)
                clients[connection] = {"input": bytearray(), "source": source, "deadline": time.monotonic() + 5}
                selector.register(connection, selectors.EVENT_READ, "request")
            elif key.data == "request":
                connection = key.fileobj
                state = clients[connection]
                try:
                    chunk = connection.recv(8193 - len(state["input"]))
                except ConnectionError:
                    close_client(connection)
                    continue
                state["input"].extend(chunk)
                if not chunk or len(state["input"]) > 8192:
                    close_client(connection)
                    continue
                if b"\r\n\r\n" not in state["input"]:
                    continue
                lines = bytes(state["input"]).split(b"\r\n")
                try:
                    method, target, version = lines[0].decode("ascii").split(" ")
                    url = urllib.parse.urlsplit(target)
                    query = urllib.parse.parse_qs(url.query, encoding="latin-1", errors="strict")
                    headers = [line.split(b":", 1)[1].strip().decode("ascii") for line in lines[1:] if line.lower().startswith(b"host:")]
                except (UnicodeError, ValueError):
                    close_client(connection)
                    continue
                valid = (method == "GET" and version == "HTTP/1.1" and url.path == "/announce"
                         and query.get("token") == [config["token"]] and len(observations) < 128)
                if valid:
                    assert headers == [f"{address}:{ports['http']}"]
                    assert query["info_hash"][0].encode("latin-1") == info_hash
                    peer = query["peer_id"][0].encode("latin-1")
                    assert len(peer) == 20
                    observations.append({"protocol": "http",
                                         "source": f"{state['source'][0]}:{state['source'][1]}",
                                         "port": int(query["port"][0]), "ip": query.get("ip", [None])[0],
                                         "ipv4": query.get("ipv4", [None])[0], "ipv6": query.get("ipv6", [None])[0],
                                         "peerId": peer.hex(), "key": query["key"][0],
                                         "event": query.get("event", [None])[0]})
                body = (b"d8:intervali3600e8:completei0e10:incompletei0e5:peers"
                        + str(len(peer_bytes)).encode() + b":" + peer_bytes + b"e") if valid else b"not found"
                response = (b"HTTP/1.1 200 OK" if valid else b"HTTP/1.1 404 Not Found")
                connection.settimeout(1)
                try:
                    connection.sendall(response + b"\r\nConnection: close\r\nContent-Length: " + str(len(body)).encode()
                                       + b"\r\n\r\n" + body)
                except (ConnectionError, TimeoutError):
                    pass
                finally:
                    close_client(connection)
            else:
                packet, source = key.fileobj.recvfrom(513)
                packets += 1
                if packets > 256 or not 16 <= len(packet) <= 512 or len(observations) >= 128:
                    continue
                action, transaction = struct.unpack_from("!II", packet, 8)
                transaction_key = (key.data, source)
                if action == 0 and packet[:8] == bytes.fromhex("0000041727101980") and len(transactions) < 32:
                    cookie = secrets.token_bytes(8)
                    transactions[transaction_key] = (cookie, time.monotonic() + 60)
                    key.fileobj.sendto(struct.pack("!II", 0, transaction) + cookie, source)
                elif action == 1 and len(packet) >= 98 and transaction_key in transactions:
                    if packet[:8] != transactions[transaction_key][0] or packet[16:36] != info_hash:
                        continue
                    event, override, tracker_key, _, port = struct.unpack_from("!IIIiH", packet, 80)
                    observations.append({"protocol": "udp",
                                         "source": f"{source[0]}:{source[1]}", "port": port,
                                         "ip": str(ipaddress.IPv4Address(override)), "peerId": packet[36:56].hex(),
                                         "key": tracker_key, "event": event})
                    key.fileobj.sendto(struct.pack("!IIIII", 1, transaction, 3600, 0, 0) + peer_bytes, source)
    if time.monotonic() >= deadline:
        raise TimeoutError("Tracker fixture control deadline expired")
finally:
    for connection in list(clients):
        close_client(connection)
    for listener in sockets:
        listener.close()
    selector.close()
