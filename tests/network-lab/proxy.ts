import { timingSafeEqual } from "node:crypto";
import { createConnection, createServer, isIP, type Socket } from "node:net";

export interface ProxyTarget {
    host: string;
    port: number;
    // Synthetic peer addresses can map to a controlled loopback endpoint.
    connectHost?: string;
    connectPort?: number;
}

export interface ProxyOptions {
    username: string;
    password: string;
    targets: ProxyTarget[];
    handshakeTimeoutMs?: number;
    maxConnections?: number;
}

export interface ProxyStats {
    acceptedConnections: number;
    authenticatedConnections: number;
    deniedConnections: number;
    activeConnections: number;
    // Payload bytes observed by the relay, not verified torrent or wire bytes.
    uploadStreamBytes: number;
    downloadStreamBytes: number;
}

function normalizedHost(host: string): string {
    return isIP(host) === 6
        ? new URL(`http://[${host}]/`).hostname.slice(1, -1)
        : host.toLowerCase();
}

function isLoopback(host: string): boolean {
    return (isIP(host) === 4 && host.startsWith("127.")) || host === "::1";
}

function validPort(port: number): boolean {
    return Number.isInteger(port) && port > 0 && port <= 65535;
}

/** Controlled TCP-only integration relay. Never resolves DNS or routes arbitrary targets. */
export async function startProxy(options: ProxyOptions) {
    const username = Buffer.from(options.username);
    const password = Buffer.from(options.password);
    if (!username.length || username.length > 255 || !password.length || password.length > 255)
        throw new Error("SOCKS credentials must each contain 1–255 UTF-8 bytes");

    const timeoutMs = options.handshakeTimeoutMs ?? 5000;
    const maxConnections = options.maxConnections ?? 32;
    if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 30000)
        throw new Error("Handshake timeout must be between 1 and 30000 ms");
    if (!Number.isInteger(maxConnections) || maxConnections < 1 || maxConnections > 256)
        throw new Error("Connection limit must be between 1 and 256");

    const targets = new Map<string, { host: string; port: number }>();
    for (const target of options.targets) {
        const host = normalizedHost(target.host);
        const connectHost = normalizedHost(target.connectHost ?? host);
        const connectPort = target.connectPort ?? target.port;
        if (!host || host.includes("\0") || !validPort(target.port)
            || !isLoopback(connectHost) || !validPort(connectPort))
            throw new Error("Targets must name an exact host/port and a numeric loopback destination");
        const key = `${host}\0${target.port}`;
        if (targets.has(key))
            throw new Error("Duplicate SOCKS target");
        targets.set(key, { host: connectHost, port: connectPort });
    }

    const stats: ProxyStats = {
        acceptedConnections: 0,
        authenticatedConnections: 0,
        deniedConnections: 0,
        activeConnections: 0,
        uploadStreamBytes: 0,
        downloadStreamBytes: 0,
    };
    const sockets = new Set<Socket>();
    const reply = (code: number, socket?: Socket) => {
        const ipv6 = socket?.localFamily === "IPv6";
        const response = Buffer.alloc(ipv6 ? 22 : 10);
        response.set([5, code, 0, ipv6 ? 4 : 1]);
        if (socket) {
            if (ipv6)
                response[19] = 1; // The only allowed IPv6 destination is ::1.
            else
                response.set(socket.localAddress!.split(".").map(Number), 4);
            response.writeUInt16BE(socket.localPort!, response.length - 2);
        }
        return response;
    };
    const server = createServer({ allowHalfOpen: true, highWaterMark: 16 * 1024 }, client => {
        stats.acceptedConnections++;
        if (stats.activeConnections >= maxConnections) {
            stats.deniedConnections++;
            client.destroy();
            return;
        }
        stats.activeConnections++;
        sockets.add(client);
        let peer: Socket | undefined;
        let phase: "greeting" | "auth" | "request" | "connect" | "relay" | "closed" = "greeting";
        let buffered = Buffer.alloc(0);
        const timer = setTimeout(() => client.destroy(), timeoutMs);

        client.on("error", () => client.destroy());
        client.on("close", () => {
            clearTimeout(timer);
            stats.activeConnections--;
            sockets.delete(client);
            peer?.destroy();
        });
        // A client that never completes the handshake must not retain a connection slot.
        client.on("end", () => {
            if (phase !== "relay")
                client.destroy();
        });

        const reject = (response: Buffer) => {
            stats.deniedConnections++;
            phase = "closed";
            client.pause();
            client.end(response, () => client.destroy());
        };

        const onHandshake = (chunk: Buffer) => {
            // Bounds pipelined handshake + early payload, before normal pipe backpressure applies.
            if (buffered.length + chunk.length > 64 * 1024) {
                client.destroy();
                return;
            }
            buffered = Buffer.concat([buffered, chunk]);
            while (true) {
                if (phase === "greeting") {
                    if (buffered.length < 2)
                        return;
                    const length = 2 + buffered[1]!;
                    if (buffered.length < length)
                        return;
                    if (buffered[0] !== 5 || !buffered.subarray(2, length).includes(2)) {
                        reject(Buffer.from([5, 255]));
                        return;
                    }
                    buffered = buffered.subarray(length);
                    client.write(Buffer.from([5, 2]));
                    phase = "auth";
                }
                else if (phase === "auth") {
                    if (buffered.length < 2)
                        return;
                    const usernameLength = buffered[1]!;
                    if (buffered.length < 3 + usernameLength)
                        return;
                    const passwordLength = buffered[2 + usernameLength]!;
                    const length = 3 + usernameLength + passwordLength;
                    if (buffered.length < length)
                        return;
                    if (buffered[0] !== 1 || usernameLength !== username.length || passwordLength !== password.length
                        || !timingSafeEqual(buffered.subarray(2, 2 + usernameLength), username)
                        || !timingSafeEqual(buffered.subarray(3 + usernameLength, length), password)) {
                        reject(Buffer.from([1, 1]));
                        return;
                    }
                    stats.authenticatedConnections++;
                    buffered = buffered.subarray(length);
                    client.write(Buffer.from([1, 0]));
                    phase = "request";
                }
                else if (phase === "request") {
                    if (buffered.length < 5)
                        return;
                    if (buffered[0] !== 5 || buffered[1] !== 1 || buffered[2] !== 0) {
                        reject(reply(7));
                        return;
                    }
                    const addressType = buffered[3]!;
                    let addressLength: number;
                    let offset = 4;
                    if (addressType === 1)
                        addressLength = 4;
                    else if (addressType === 4)
                        addressLength = 16;
                    else if (addressType === 3) {
                        offset = 5;
                        addressLength = buffered[4]!;
                    }
                    else {
                        reject(reply(8));
                        return;
                    }
                    const length = offset + addressLength + 2;
                    if (buffered.length < length)
                        return;
                    const address = buffered.subarray(offset, offset + addressLength);
                    let host: string;
                    if (addressType === 1)
                        host = Array.from(address).join(".");
                    else if (addressType === 4)
                        host = Array.from({ length: 8 }, (_, i) => address.readUInt16BE(i * 2).toString(16)).join(":");
                    else
                        host = address.toString("utf8");
                    const port = buffered.readUInt16BE(offset + addressLength);
                    const target = targets.get(`${normalizedHost(host)}\0${port}`);
                    if (!target) {
                        reject(reply(2));
                        return;
                    }

                    phase = "connect";
                    client.pause();
                    client.removeListener("data", onHandshake);
                    const earlyPayload = buffered.subarray(length);
                    buffered = Buffer.alloc(0);
                    peer = createConnection({ ...target, allowHalfOpen: true });
                    const upstream = peer;
                    sockets.add(upstream);
                    upstream.on("error", () => {
                        if (phase === "connect") {
                            phase = "closed";
                            client.end(reply(5), () => client.destroy());
                        }
                        else
                            client.destroy();
                    });
                    upstream.on("close", () => {
                        sockets.delete(upstream);
                    });
                    upstream.once("connect", () => {
                        if (client.destroyed) {
                            upstream.destroy();
                            return;
                        }
                        clearTimeout(timer);
                        phase = "relay";
                        client.write(reply(0, upstream));
                        upstream.on("data", data => { stats.downloadStreamBytes += data.length; });
                        client.on("data", data => { stats.uploadStreamBytes += data.length; });
                        upstream.pipe(client);
                        if (earlyPayload.length) {
                            stats.uploadStreamBytes += earlyPayload.length;
                            upstream.write(earlyPayload, () => client.pipe(upstream));
                        }
                        else
                            client.pipe(upstream);
                    });
                    return;
                }
                else
                    return;
            }
        };
        client.on("data", onHandshake);
    });

    await new Promise<void>((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", () => {
            server.removeListener("error", reject);
            resolve();
        });
    });
    const address = server.address();
    if (!address || typeof address === "string")
        throw new Error("SOCKS listener did not receive a TCP address");

    let closePromise: Promise<void> | undefined;
    return {
        host: "127.0.0.1",
        port: address.port,
        stats,
        close(): Promise<void> {
            if (!closePromise) {
                const socketsClosed = Array.from(sockets, socket => new Promise<void>(resolve => {
                    socket.once("close", resolve);
                }));
                const listenerClosed = new Promise<void>((resolve, reject) => {
                    server.close(error => error ? reject(error) : resolve());
                });
                for (const socket of sockets)
                    socket.destroy();
                closePromise = Promise.all([listenerClosed, ...socketsClosed]).then(() => {});
            }
            return closePromise;
        },
    };
}
