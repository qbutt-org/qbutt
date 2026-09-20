"""One-request, high-port HTTP source check for an owned qbutt-net SOCKS path."""
import ipaddress
import json
import secrets
import socket
import sys


def exact(sock, length):
    result = bytearray()
    while len(result) < length:
        chunk = sock.recv(length - len(result))
        if not chunk:
            raise ConnectionError("Source check socket closed")
        result.extend(chunk)
    return bytes(result)


def server():
    listener = socket.socket()
    listener.settimeout(35)
    for _ in range(32):
        port = 49152 + secrets.randbelow(16384)
        try:
            listener.bind(("0.0.0.0", port))
            break
        except OSError:
            continue
    else:
        raise RuntimeError("No owned high port for source check")
    listener.listen(1)
    print(json.dumps({"ready": True, "port": port}), flush=True)
    with listener:
        with listener.accept()[0] as connection:
            connection.settimeout(15)
            source = connection.getpeername()
            print(json.dumps({"accepted": True, "source": f"{source[0]}:{source[1]}"}), flush=True)
            request = bytearray()
            while b"\r\n\r\n" not in request and len(request) < 4096:
                chunk = connection.recv(4096 - len(request))
                if not chunk:
                    break
                request.extend(chunk)
            if not request.startswith(b"GET /source HTTP/1.1\r\n"):
                raise ValueError("Unexpected source-check request")
            body = json.dumps({"source": f"{source[0]}:{source[1]}"}).encode()
            connection.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: "
                               + str(len(body)).encode() + b"\r\nConnection: close\r\n\r\n" + body)
            print(json.dumps({"served": True, "source": f"{source[0]}:{source[1]}"}), flush=True)


def client(proxy_port, username, password, target_ip, target_port):
    target = ipaddress.IPv4Address(target_ip)
    username, password = username.encode(), password.encode()
    if not 1 <= len(username) <= 255 or not 1 <= len(password) <= 255:
        raise ValueError("Invalid path credentials")
    with socket.create_connection(("127.0.0.1", int(proxy_port)), timeout=20) as connection:
        connection.settimeout(25)
        connection.sendall(b"\x05\x01\x02")
        if exact(connection, 2) != b"\x05\x02":
            raise ConnectionError("Selected path rejected SOCKS authentication")
        connection.sendall(b"\x01" + bytes([len(username)]) + username
                           + bytes([len(password)]) + password)
        if exact(connection, 2) != b"\x01\x00":
            raise ConnectionError("Selected path rejected credentials")
        connection.sendall(b"\x05\x01\x00\x01" + target.packed + int(target_port).to_bytes(2, "big"))
        reply = exact(connection, 4)
        if reply[:3] != b"\x05\x00\x00":
            raise ConnectionError("Selected node could not reach observer source check")
        address_length = {1: 4, 4: 16}.get(reply[3])
        if address_length is None:
            address_length = exact(connection, 1)[0]
        exact(connection, address_length + 2)
        connection.sendall(b"GET /source HTTP/1.1\r\nHost: observer\r\nConnection: close\r\n\r\n")
        response = bytearray()
        while len(response) < 4096:
            chunk = connection.recv(4096 - len(response))
            if not chunk:
                break
            response.extend(chunk)
    if not response.startswith(b"HTTP/1.1 200 OK\r\n"):
        raise ConnectionError(f"Observer source check did not answer: {len(response)} bytes")
    print(response.split(b"\r\n\r\n", 1)[1].decode(), flush=True)


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["server"]:
            server()
        elif sys.argv[1:] == ["client"]:
            request = json.loads(sys.stdin.readline())
            if set(request) != {"port", "username", "password", "targetIP", "targetPort"}:
                raise ValueError("Invalid source-check request")
            client(request["port"], request["username"], request["password"],
                   request["targetIP"], request["targetPort"])
        else:
            raise ValueError("Invalid source-check arguments")
    except (ConnectionError, OSError, RuntimeError, ValueError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
