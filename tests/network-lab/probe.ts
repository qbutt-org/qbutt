import { randomBytes } from "node:crypto";
import { once } from "node:events";
import { createConnection, createServer, type Socket } from "node:net";
import { startProxy } from "./proxy";

function check(condition: boolean, message: string): asserts condition {
    if (!condition)
        throw new Error(message);
}

function readBytes(socket: Socket, length: number): Promise<Buffer> {
    return new Promise((resolve, reject) => {
        const cleanup = () => {
            clearTimeout(timer);
            socket.removeListener("readable", readable);
            socket.removeListener("error", failed);
            socket.removeListener("end", ended);
            socket.removeListener("close", ended);
        };
        const failed = (error: Error) => { cleanup(); reject(error); };
        const ended = () => failed(new Error("Stream ended before the expected bytes arrived"));
        const readable = () => {
            const bytes = socket.read(length) as Buffer | null;
            if (bytes) {
                cleanup();
                resolve(bytes);
            }
        };
        const timer = setTimeout(() => failed(new Error("Timed out reading SOCKS stream")), 3000);
        socket.on("readable", readable);
        socket.on("error", failed);
        socket.on("end", ended);
        socket.on("close", ended);
        readable();
    });
}

// Real loopback TCP integration: no external peer, DNS or production configuration.
const echoClients = new Set<Socket>();
const echo = createServer(socket => {
    echoClients.add(socket);
    socket.on("error", () => socket.destroy());
    socket.on("close", () => echoClients.delete(socket));
    socket.pipe(socket);
});
echo.listen(0, "127.0.0.1");
await once(echo, "listening");
const echoAddress = echo.address();
check(echoAddress !== null && typeof echoAddress !== "string", "Echo listener has no TCP address");
const username = randomBytes(16).toString("hex");
const password = randomBytes(24).toString("hex");
const syntheticHost = "127.77.0.2";
const proxy = await startProxy({
    username,
    password,
    targets: [
        { host: syntheticHost, port: echoAddress.port, connectHost: "127.0.0.1" },
        { host: "peer.qbutt.invalid", port: echoAddress.port, connectHost: "127.0.0.1" },
        { host: "2001:db8::2", port: echoAddress.port, connectHost: "127.0.0.1" },
    ],
    handshakeTimeoutMs: 1000,
});
const clients = new Set<Socket>();

async function connect(): Promise<Socket> {
    const socket = createConnection({ host: proxy.host, port: proxy.port });
    clients.add(socket);
    socket.on("error", () => {});
    socket.on("close", () => clients.delete(socket));
    await once(socket, "connect", { signal: AbortSignal.timeout(3000) });
    return socket;
}

async function authenticate(socket: Socket, suppliedPassword = password): Promise<Buffer> {
    // Fragmented greeting exercises stream parsing across TCP writes.
    socket.write(Buffer.from([5]));
    socket.write(Buffer.from([1, 2]));
    check((await readBytes(socket, 2)).equals(Buffer.from([5, 2])), "Proxy did not require username/password");
    socket.write(Buffer.concat([
        Buffer.from([1, Buffer.byteLength(username)]), Buffer.from(username),
        Buffer.from([Buffer.byteLength(suppliedPassword)]), Buffer.from(suppliedPassword),
    ]));
    return readBytes(socket, 2);
}

function request(host: string, port: number): Buffer {
    const message = Buffer.from([5, 1, 0, 1, ...host.split(".").map(Number), 0, 0]);
    message.writeUInt16BE(port, 8);
    return message;
}

try {
    const direct = createConnection({ host: syntheticHost, port: echoAddress.port });
    direct.setTimeout(1000, () => direct.destroy(new Error("Direct route timed out")));
    const directConnected = await new Promise<boolean>(resolve => {
        direct.once("connect", () => { direct.destroy(); resolve(true); });
        direct.once("error", () => resolve(false));
    });
    check(!directConnected, "Synthetic peer unexpectedly accepts a direct connection");

    const noAuth = await connect();
    noAuth.write(Buffer.from([5, 1, 0]));
    check((await readBytes(noAuth, 2)).equals(Buffer.from([5, 255])), "Unauthenticated method was accepted");
    noAuth.destroy();

    const wrong = await connect();
    check((await authenticate(wrong, randomBytes(24).toString("hex"))).equals(Buffer.from([1, 1])), "Wrong credentials were accepted");
    wrong.destroy();

    const denied = await connect();
    check((await authenticate(denied)).equals(Buffer.from([1, 0])), "Valid credentials failed");
    denied.write(request("127.0.0.1", echoAddress.port));
    check((await readBytes(denied, 10))[1] === 2, "Target outside the allowlist was accepted");
    denied.destroy();

    const stalled = await connect();
    const stalledClosed = once(stalled, "close", { signal: AbortSignal.timeout(3000) });
    stalled.write(Buffer.from([5]));
    await stalledClosed;

    const halfClosePayload = Buffer.from("half-close echo!!");
    for (const address of [
        Buffer.concat([Buffer.from([3, 18]), Buffer.from("peer.qbutt.invalid")]),
        Buffer.from([4, 0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]),
    ]) {
        const connection = await connect();
        check((await authenticate(connection)).equals(Buffer.from([1, 0])), "Valid credentials failed");
        const port = Buffer.alloc(2);
        port.writeUInt16BE(echoAddress.port);
        connection.write(Buffer.concat([Buffer.from([5, 1, 0]), address, port]));
        check((await readBytes(connection, 10))[1] === 0, "Domain or IPv6 mapping failed");
        const response = readBytes(connection, halfClosePayload.length);
        connection.end(halfClosePayload);
        check((await response).equals(halfClosePayload), "Half-close lost the response");
        connection.destroy();
    }

    const allowed = await connect();
    check((await authenticate(allowed)).equals(Buffer.from([1, 0])), "Valid credentials failed");
    const earlyPayload = randomBytes(257);
    allowed.write(Buffer.concat([request(syntheticHost, echoAddress.port), earlyPayload]));
    const connected = await readBytes(allowed, 10);
    check(connected[1] === 0 && connected.readUInt16BE(8) > 0, "Mapped synthetic target or bound port failed");
    check((await readBytes(allowed, earlyPayload.length)).equals(earlyPayload), "Pipelined payload was changed");
    const payload = randomBytes(512 * 1024);
    allowed.write(payload);
    check((await readBytes(allowed, payload.length)).equals(payload), "Echo payload was changed");
    const streamBytes = payload.length + earlyPayload.length + 2 * halfClosePayload.length;
    check(proxy.stats.uploadStreamBytes === streamBytes, "Incorrect upload stream count");
    check(proxy.stats.downloadStreamBytes === streamBytes, "Incorrect download stream count");
    check(proxy.stats.deniedConnections === 3, "Incorrect denied connection count");

    const broke = once(allowed, "close", { signal: AbortSignal.timeout(3000) });
    await proxy.close();
    await broke;
    check(proxy.stats.activeConnections === 0, "Proxy close retained active clients");
    console.log(JSON.stringify({
        status: "passed",
        scenarios: ["direct target unavailable", "authentication required", "wrong credentials", "target denied", "handshake timeout", "domain and IPv6 mappings", "half-close", "mapped TCP transfer", "pipelined payload", "relay death"],
        stats: proxy.stats,
        byteMeaning: "Observed SOCKS payload stream bytes; not verified torrent bytes or wire bytes",
    }, null, 2));
}
finally {
    for (const client of clients)
        client.destroy();
    await proxy.close();
    for (const client of echoClients)
        client.destroy();
    await new Promise<void>((resolve, reject) => echo.close(error => error ? reject(error) : resolve()));
}
