import assert from "node:assert/strict";
import { createConnection, createServer, type Server, type Socket } from "node:net";
import { networkInterfaces } from "node:os";
import { Transform, type TransformCallback } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";

/** One downstream byte budget shared by the controlled Native and SOCKS peers. */
export function createTcpBottleneck(bytesPerSecond: number) {
    assert(Number.isInteger(bytesPerSecond) && bytesPerSecond >= 32 * 1024 && bytesPerSecond <= 512 * 1024);
    const localAddresses = new Set(Object.values(networkInterfaces()).flatMap(entries =>
        (entries ?? []).filter(entry => entry.family === "IPv4").map(entry => entry.address)));
    localAddresses.add("127.0.0.1");
    const servers: Server[] = [];
    const sockets = new Set<Socket>();
    const listeners: { acceptedConnections: number; downstreamStreamBytes: number }[] = [];
    const burstMilliseconds = 100;
    const stats = { acceptedConnections: 0, downstreamStreamBytes: 0,
        burstAllowanceBytes: Math.ceil(bytesPerSecond * burstMilliseconds / 1000), listeners };
    let nextSend = performance.now();
    let closed = false;

    async function listen(address: string, targetHost: string, targetPort: number): Promise<number> {
        assert(!closed && localAddresses.has(address) && localAddresses.has(targetHost)
            && Number.isInteger(targetPort) && targetPort > 0 && targetPort <= 65535,
        "The bottleneck may only forward to an exact endpoint on this fixture host");
        const listenerStats = { acceptedConnections: 0, downstreamStreamBytes: 0 };
        listeners.push(listenerStats);
        const server = createServer({ allowHalfOpen: true, highWaterMark: 16 * 1024 }, client => {
            if (closed || client.remoteAddress !== address || sockets.size >= 16) {
                client.destroy();
                return;
            }
            stats.acceptedConnections++;
            listenerStats.acceptedConnections++;
            const upstream = createConnection({ host: targetHost, port: targetPort,
                localAddress: address, allowHalfOpen: true, highWaterMark: 16 * 1024 });
            const controller = new AbortController();
            // Each stream has at most one transform in flight. Reservations are
            // global and limited to 1 KiB, so additional paths cannot multiply
            // the budget or reserve an unbounded future queue.
            const shaped = new Transform({ highWaterMark: 16 * 1024,
                transform(chunk: Buffer, _encoding: BufferEncoding, callback: TransformCallback) {
                    (async () => {
                        for (let offset = 0; offset < chunk.length; offset += 1024) {
                            const part = chunk.subarray(offset, Math.min(offset + 1024, chunk.length));
                            const now = performance.now();
                            // Up to 100 ms of idle credit also absorbs timer
                            // granularity; one stream must not lose capacity
                            // merely because it has fewer concurrent timers.
                            nextSend = Math.max(now - burstMilliseconds, nextSend) + 1000 * part.length / bytesPerSecond;
                            const wait = Math.ceil(nextSend - now);
                            if (wait > 0)
                                await delay(wait, undefined, { signal: controller.signal });
                            controller.signal.throwIfAborted();
                            stats.downstreamStreamBytes += part.length;
                            listenerStats.downstreamStreamBytes += part.length;
                            this.push(part);
                        }
                    })().then(() => callback(), error => callback(error));
                },
            });
            const close = () => {
                controller.abort();
                shaped.destroy();
                upstream.destroy();
                client.destroy();
                sockets.delete(upstream);
                sockets.delete(client);
            };
            sockets.add(client);
            sockets.add(upstream);
            client.on("error", close);
            upstream.on("error", close);
            shaped.on("error", close);
            client.on("close", close);
            upstream.on("close", close);
            client.pipe(upstream);
            upstream.pipe(shaped).pipe(client);
        });
        servers.push(server);
        await new Promise<void>((resolve, reject) => {
            server.once("error", reject);
            server.listen(0, address, () => {
                server.removeListener("error", reject);
                resolve();
            });
        });
        const endpoint = server.address();
        assert(endpoint && typeof endpoint !== "string");
        return endpoint.port;
    }

    async function close() {
        closed = true;
        for (const socket of sockets) socket.destroy();
        await Promise.all(servers.filter(server => server.listening).map(server =>
            new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()))));
    }

    return { listen, close, stats };
}
