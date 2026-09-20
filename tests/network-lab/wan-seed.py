"""Bounded, controlled TCP BitTorrent peer; not a production seed or benchmark."""

import asyncio
import base64
import ipaddress
import json
import math
import os
import secrets
import struct
import sys
import threading
import time


def emit(value):
    print(json.dumps(value, separators=(",", ":")), flush=True)


def bounded_integer(value, minimum, maximum, name):
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError("Invalid " + name)
    return value


async def main():
    stdin_pending = bytearray()

    def read_line(limit):
        # Raw reads avoid a daemon thread holding stdin's buffered lock at exit.
        searched = 0
        while True:
            newline = stdin_pending.find(b"\n", searched)
            if newline >= 0:
                if newline + 1 > limit:
                    raise ValueError("Input line exceeds limit")
                line = bytes(stdin_pending[:newline + 1])
                del stdin_pending[:newline + 1]
                return line
            if len(stdin_pending) >= limit:
                raise ValueError("Input line exceeds limit")
            searched = len(stdin_pending)
            chunk = os.read(sys.stdin.fileno(), min(65536, limit - searched))
            if not chunk:
                line = bytes(stdin_pending)
                stdin_pending.clear()
                return line
            stdin_pending.extend(chunk)

    # Read one bounded JSON line, never an unbounded subscription or file path.
    line = read_line(45 * 1024 * 1024)
    config = json.loads(line)
    info_hash = bytes.fromhex(config["infoHash"])
    if len(info_hash) != 20 or len(config["infoHash"]) != 40:
        raise ValueError("Invalid infoHash")
    payload = base64.b64decode(config["payload"], validate=True)
    if not 1 <= len(payload) <= 32 * 1024 * 1024:
        raise ValueError("Payload must be between 1 byte and 32 MiB")
    piece_length = bounded_integer(config["pieceLength"], 65536, 65536, "pieceLength")
    count = bounded_integer(config["count"], 1, 4, "count")
    rate = bounded_integer(config["rate"], 1024, 1024 * 1024, "rate")
    duration = bounded_integer(config["duration"], 1, 240, "duration")
    target = config.get("connectTarget")
    if target is not None:
        if (count != 1 or not isinstance(target, dict) or set(target) != {"host", "port"}
                or type(target["host"]) is not str):
            raise ValueError("Outbound peer requires one numeric target and one piece owner")
        target = (str(ipaddress.IPv4Address(target["host"])),
                  bounded_integer(target["port"], 49152, 65535, "target port"))
    proxy = config.get("connectProxy")
    if proxy is not None:
        if (target is None or not isinstance(proxy, dict)
                or set(proxy) != {"port", "username", "password"}):
            raise ValueError("Outbound SOCKS proxy requires a numeric target and loopback endpoint")
        proxy = {**proxy, "port": bounded_integer(proxy["port"], 1, 65535, "SOCKS port")}
        for field in ("username", "password"):
            if not isinstance(proxy[field], str) or not 1 <= len(proxy[field].encode("utf8")) <= 255:
                raise ValueError("Invalid local SOCKS credentials")
    piece_count = math.ceil(len(payload) / piece_length)
    if piece_count < count:
        raise ValueError("Each side must own at least one piece")
    del config, line

    started = asyncio.Event()
    stop = asyncio.Event()
    locks = [asyncio.Lock() for _ in range(count)]
    next_send = [0.0] * count
    sent = [0] * count
    side_limits = [4 * sum(min(piece_length, len(payload) - index * piece_length)
                          for index in range(side, piece_count, count)) for side in range(count)]
    connections = []
    errors = []
    clients = set()
    writers = set()
    servers = []
    commands = asyncio.Queue(maxsize=16)
    loop = asyncio.get_running_loop()

    def enqueue(command):
        if commands.full():
            errors.append("Control queue limit exceeded")
            stop.set()
        else:
            commands.put_nowait(command)

    def read_commands():
        try:
            for _ in range(16):
                command_line = read_line(4096)
                if not command_line:
                    loop.call_soon_threadsafe(stop.set)
                    return
                command = json.loads(command_line)
                loop.call_soon_threadsafe(enqueue, command)
                if isinstance(command, dict) and command.get("command") == "stop":
                    return
            raise ValueError("Control command limit exceeded")
        except (ValueError, OSError):
            loop.call_soon_threadsafe(enqueue, {"command": "invalid"})

    async def peer(reader, writer, side, initiator=False):
        task = asyncio.current_task()
        clients.add(task)
        writers.add(writer)
        record = None
        try:
            if len(connections) >= 32:
                return
            local = writer.get_extra_info("sockname")
            record = {"side": side, "remoteIP": writer.get_extra_info("peername")[0],
                      "localEndpoint": f"{local[0]}:{local[1]}",
                      "peerId": None, "requestedPieces": [], "payloadBytes": 0,
                      "startMonotonic": time.monotonic(), "endMonotonic": None,
                      "firstPayloadMonotonic": None, "lastPayloadMonotonic": None}
            connections.append(record)
            peer_id = b"-QBWA01-" + secrets.token_hex(6).encode("ascii")
            if initiator:
                writer.write(b"\x13BitTorrent protocol" + bytes(8) + info_hash + peer_id)
                await asyncio.wait_for(writer.drain(), 10)
            handshake = await asyncio.wait_for(reader.readexactly(68), 10)
            if handshake[:20] != b"\x13BitTorrent protocol" or handshake[28:48] != info_hash:
                raise ValueError("Invalid handshake or infohash")
            record["peerId"] = handshake[48:68].hex()
            if not initiator:
                writer.write(b"\x13BitTorrent protocol" + bytes(8) + info_hash + peer_id)
            bitfield = bytearray(math.ceil(piece_count / 8))
            for index in range(side, piece_count, count):
                bitfield[index // 8] |= 0x80 >> (index % 8)
            writer.write(struct.pack("!IB", len(bitfield) + 1, 5) + bitfield)
            await writer.drain()
            if initiator and proxy is not None:
                emit({"peerHandshake": True})
            await started.wait()
            writer.write(struct.pack("!IB", 1, 1))  # Unchoke only after control start.
            await writer.drain()
            requested = set()
            for _ in range(32768):
                try:
                    header = await asyncio.wait_for(reader.readexactly(4), 30)
                except asyncio.TimeoutError:
                    record["idleTimeout"] = True
                    return
                length = struct.unpack("!I", header)[0]
                if length == 0:
                    continue
                if length > 16393:
                    raise ValueError("Peer frame exceeds limit")
                frame = await asyncio.wait_for(reader.readexactly(length), 10)
                message = frame[0]
                if message in (0, 1, 2, 3) and length == 1:
                    continue
                if message == 4 and length == 5:
                    continue
                if message == 5 and length == len(bitfield) + 1:
                    continue
                if message == 8 and length == 13:  # Completed sequential sends need no queue cancellation.
                    continue
                if message != 6 or length != 13:
                    raise ValueError("Unsupported or malformed peer frame")
                index, offset, block_length = struct.unpack("!III", frame[1:])
                if (index >= piece_count or index % count != side or block_length == 0
                        or block_length > 16384 or offset + block_length > piece_length
                        or index * piece_length + offset + block_length > len(payload)):
                    raise ValueError("Request outside advertised pieces")
                requested.add(index)
                record["requestedPieces"] = sorted(requested)
                async with locks[side]:
                    if sent[side] + block_length > side_limits[side]:
                        raise ValueError("Side payload limit exceeded")
                    await asyncio.sleep(max(0, next_send[side] - time.monotonic()))
                    start = index * piece_length + offset
                    writer.write(struct.pack("!IBII", block_length + 9, 7, index, offset)
                                 + payload[start:start + block_length])
                    await asyncio.wait_for(writer.drain(), 10)
                    now = time.monotonic()
                    next_send[side] = now + block_length / rate
                    sent[side] += block_length
                    record["payloadBytes"] += block_length
                    if record["firstPayloadMonotonic"] is None:
                        record["firstPayloadMonotonic"] = now
                    record["lastPayloadMonotonic"] = now
            raise ValueError("Peer frame count limit exceeded")
        except (asyncio.IncompleteReadError, ConnectionError):
            pass
        except (ValueError, asyncio.TimeoutError, OSError) as error:
            errors.append({"side": side, "error": str(error) or type(error).__name__})
        finally:
            if record is not None:
                record["endMonotonic"] = time.monotonic()
            writer.close()
            writers.discard(writer)
            clients.discard(task)

    async def control():
        nonlocal rate
        while True:
            command = await commands.get()
            if not isinstance(command, dict):
                raise ValueError("Invalid control object")
            if command.get("command") == "stop":
                stop.set()
                return
            if command.get("command") != "start" or started.is_set():
                raise ValueError("Invalid or repeated control command")
            rate = bounded_integer(command.get("rate", rate), 1024, 1024 * 1024, "rate")
            started.set()
            emit({"started": True})

    async def connect_outbound():
        if proxy is None:
            return await asyncio.open_connection(*target)
        reader, writer = await asyncio.open_connection("127.0.0.1", proxy["port"])
        try:
            writer.write(b"\x05\x01\x02")
            await writer.drain()
            if await reader.readexactly(2) != b"\x05\x02":
                raise ValueError("Local SOCKS proxy rejected authentication")
            username = proxy["username"].encode("utf8")
            password = proxy["password"].encode("utf8")
            writer.write(b"\x01" + bytes([len(username)]) + username
                         + bytes([len(password)]) + password)
            await writer.drain()
            if await reader.readexactly(2) != b"\x01\x00":
                raise ValueError("Local SOCKS proxy rejected credentials")
            writer.write(b"\x05\x01\x00\x01" + ipaddress.IPv4Address(target[0]).packed
                         + struct.pack("!H", target[1]))
            await writer.drain()
            reply = await reader.readexactly(4)
            if reply[:3] != b"\x05\x00\x00" or reply[3] not in (1, 3, 4):
                raise ValueError("Local SOCKS proxy could not reach the public lease")
            length = {1: 4, 4: 16}.get(reply[3])
            if length is None:
                length = (await reader.readexactly(1))[0]
                if not 1 <= length <= 255:
                    raise ValueError("Invalid SOCKS bind address")
            await reader.readexactly(length + 2)
            return reader, writer
        except (Exception, asyncio.CancelledError):
            writer.close()
            await writer.wait_closed()
            raise

    control_task = None
    stop_task = None
    outbound_task = None
    try:
        if target is None:
            for side in range(count):
                server = await asyncio.start_server(
                    lambda reader, writer, side=side: peer(reader, writer, side),
                    "0.0.0.0", 0, limit=32768, backlog=8)
                servers.append(server)
        ports = [server.sockets[0].getsockname()[1] for server in servers]
        if ports and min(ports) <= 1024:
            raise ValueError("Unexpected privileged listener")
        emit({"ready": True, "ports": ports, "pieceCount": piece_count})
        threading.Thread(target=read_commands, daemon=True).start()
        if target is not None:
            async def outbound():
                try:
                    reader, writer = await asyncio.wait_for(connect_outbound(), 20)
                    if proxy is not None:
                        emit({"connected": True})
                    await peer(reader, writer, 0, True)
                    if sent[0] < len(payload) and not stop.is_set():
                        errors.append({"side": 0, "error": "Outbound peer closed before full payload"})
                        stop.set()
                except (OSError, asyncio.TimeoutError, asyncio.IncompleteReadError, ValueError) as error:
                    errors.append({"side": 0, "error": str(error) or type(error).__name__})
                    stop.set()
            outbound_task = asyncio.create_task(outbound())
        control_task = asyncio.create_task(control())
        stop_task = asyncio.create_task(stop.wait())
        done, _ = await asyncio.wait([control_task, stop_task], timeout=duration,
                                     return_when=asyncio.FIRST_COMPLETED)
        if not done:
            errors.append("Watchdog expired")
        if control_task in done:
            control_task.result()
    except (ValueError, OSError) as error:
        errors.append(str(error))
    finally:
        stop.set()
        for server in servers:
            server.close()
        for writer in tuple(writers):
            writer.close()
        for task in tuple(clients):
            task.cancel()
        for task in (control_task, stop_task, outbound_task):
            if task is not None:
                task.cancel()
        await asyncio.gather(*tuple(clients), *(task for task in (control_task, stop_task, outbound_task)
                                               if task is not None), return_exceptions=True)
        await asyncio.gather(*(server.wait_closed() for server in servers))
        emit({"stopped": True, "connections": connections, "errors": errors})
    return 1 if errors else 0


if __name__ == "__main__":
    try:
        sys.exit(asyncio.run(main()))
    except (ValueError, KeyError, TypeError) as error:
        emit({"stopped": True, "connections": [], "errors": [str(error)]})
        sys.exit(1)
