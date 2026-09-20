import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { readFile, rm, writeFile } from "node:fs/promises";
import { createServer, type Socket } from "node:net";
import { dirname, join, resolve } from "node:path";
import { createLab, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface Route {
    pathId: string; generation: number; attempts: number; connected: number; closed: number;
    connectionFailures: number; timeouts: number; payloadDownload: number; verifiedDownload: number;
    demandMilliseconds: number; chokedMilliseconds: number;
}
interface Status {
    busy: boolean;
    paths: { pathId: string; generation: number; proxyName: string; open: boolean }[];
    peers: { pathId: string; generation: number; peer: string; port: number; payloadDownload: number }[];
    diagnostics: { routes: Route[] };
}

const lab = await createLab("idle-peers");
const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
const infoHash = Buffer.from(torrent.infoHashV1!, "hex");
const payload = Buffer.alloc(torrent.files.reduce((size, file) => Math.max(size, file.offset + file.size), 0));
for (const file of torrent.files) {
    assert(!file.pad, "Idle peer fixture requires a v1 payload without padding");
    (await readFile(join(lab.fixtures, "seed", file.path))).copy(payload, file.offset);
}
const errors: string[] = [];
const credentials = [0, 1].map(() => ({ username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") }));
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
const peers: Awaited<ReturnType<typeof startPeer>>[] = [];
let failure: unknown;

async function startPeer(hasPieces: boolean) {
    const sockets = new Set<Socket>();
    const counts = { connections: 0, interested: 0, requests: 0, payloadBytes: 0 };
    let choked = hasPieces;
    const unchoke = Buffer.from([0, 0, 0, 1, 1]);
    const server = createServer(socket => {
        sockets.add(socket);
        counts.connections++;
        let pending = Buffer.alloc(0);
        let handshaken = false;
        socket.on("error", error => errors.push(String(error)));
        socket.on("close", () => sockets.delete(socket));
        socket.on("data", chunk => {
            try {
                pending = Buffer.concat([pending, chunk]);
                assert(pending.length <= 128 * 1024, "Fixture peer input exceeded its bound");
                if (!handshaken) {
                    if (pending.length < 68) return;
                    assert.equal(pending[0], 19);
                    assert.equal(pending.subarray(1, 20).toString(), "BitTorrent protocol");
                    assert(pending.subarray(28, 48).equals(infoHash));
                    const reply = Buffer.from(pending.subarray(0, 68));
                    reply.fill(0, 20, 28);
                    randomBytes(20).copy(reply, 48);
                    const bitfield = Buffer.alloc(5 + Math.ceil(torrent.pieceCount / 8));
                    bitfield.writeUInt32BE(bitfield.length - 4);
                    bitfield[4] = 5;
                    if (hasPieces) for (let index = 0; index < torrent.pieceCount; index++)
                        bitfield[5 + (index >> 3)]! |= 0x80 >> (index % 8);
                    socket.write(Buffer.concat([reply, bitfield, ...(choked ? [] : [unchoke])]));
                    pending = pending.subarray(68);
                    handshaken = true;
                }
                while (pending.length >= 4) {
                    const length = pending.readUInt32BE(0);
                    assert(length <= 65536, "Fixture peer frame exceeded its bound");
                    if (pending.length < length + 4) return;
                    const message = pending.subarray(4, 4 + length);
                    pending = pending.subarray(4 + length);
                    if (length === 0) continue;
                    if (message[0] === 2) counts.interested++;
                    if (message[0] !== 6) continue;
                    counts.requests++;
                    assert(!choked && hasPieces, "The app requested data without available unchoked pieces");
                    assert.equal(length, 13);
                    const piece = message.readUInt32BE(1);
                    const offset = message.readUInt32BE(5);
                    const bytes = message.readUInt32BE(9);
                    const from = piece * torrent.pieceLength + offset;
                    assert(piece < torrent.pieceCount && bytes > 0 && bytes <= 16384
                        && offset + bytes <= torrent.pieceLength && from + bytes <= payload.length);
                    const response = Buffer.alloc(13 + bytes);
                    response.writeUInt32BE(9 + bytes);
                    response[4] = 7;
                    response.writeUInt32BE(piece, 5);
                    response.writeUInt32BE(offset, 9);
                    payload.copy(response, 13, from, from + bytes);
                    socket.write(response);
                    counts.payloadBytes += bytes;
                }
            }
            catch (error) { errors.push(String(error)); socket.destroy(); }
        });
    });
    await new Promise<void>((accept, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", accept);
    });
    const address = server.address();
    assert(address && typeof address !== "string");
    return { port: address.port, counts,
        release() { choked = false; for (const socket of sockets) socket.write(unchoke); },
        async close() {
            for (const socket of sockets) socket.destroy();
            await new Promise<void>(accept => server.close(() => accept()));
        },
    };
}

try {
    for (const side of [0, 1]) {
        const peer = await startPeer(side === 0);
        peers.push(peer);
        proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`,
            targets: [{ host: `127.0.0.${side + 2}`, port: peer.port, connectHost: "127.0.0.1" }] }));
    }
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
        name: `idle-${side}`, type: "socks5", server: proxy.host, port: proxy.port,
        ...credentials[side], udp: false,
    })) }));
    await lab.start();
    await lab.request("app/setPreferences", { json: JSON.stringify({ encryption: 2 }) });
    assert.equal((await lab.json<{ encryption: number }>("app/preferences")).encryption, 2,
        "The controlled peer implements plaintext BitTorrent only");
    const status = () => lab.json<Status>("qbuttPaths/status");
    for (const side of [0, 1]) {
        await lab.request("qbuttPaths/open", { configPath, proxyName: `idle-${side}`,
            interfaceName: "Loopback Pseudo-Interface 1" });
        await waitFor("idle fixture route opens", status,
            value => !value.busy && value.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const destination = join(lab.root, "download");
    const hash = await lab.add(torrent.name, destination);
    await lab.request("torrents/start", { hashes: hash });
    for (const side of [0, 1]) {
        await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.${side + 2}:${peers[side]!.port}` });
        await waitFor("idle fixture handshake", async () => {
            assert.deepEqual(errors, []);
            return { current: await status(), count: peers[side]!.counts.connections };
        },
            value => value.count === 1 && value.current.peers.some(peer => peer.peer === `127.0.0.${side + 2}`));
    }
    await waitFor("choked peer has wanted pieces", async () => peers[0]!.counts.interested, count => count > 0);
    const before = await status();
    assert.equal(new Set(before.peers.map(peer => peer.pathId)).size, 2, "Idle peers must use different routes");
    const started = performance.now();
    await Bun.sleep(12000);
    const held = await status();
    assert.deepEqual(errors, []);
    assert.equal(held.peers.length, 2);
    assert.equal((await lab.info(hash)).completed, 0);
    for (const side of [0, 1]) {
        const peer = held.peers.find(peer => peer.peer === `127.0.0.${side + 2}`)!;
        const route = held.diagnostics.routes.find(route => route.pathId === peer.pathId && route.generation === peer.generation)!;
        const prior = before.diagnostics.routes.find(previous => previous.pathId === route.pathId && previous.generation === route.generation)!;
        for (const key of ["attempts", "connected", "closed", "connectionFailures", "timeouts"] as const)
            assert.equal(route[key], prior[key], `Idle peer caused route ${key}`);
        assert.equal(route.demandMilliseconds, 0, "Choke/no demand counted as a payload demand interval");
        assert.equal(route.payloadDownload, 0);
        assert.equal(route.verifiedDownload, 0);
        if (side === 0) assert(route.chokedMilliseconds - prior.chokedMilliseconds >= 8000);
        else assert.equal(route.chokedMilliseconds, prior.chokedMilliseconds);
        assert.equal(peers[side]!.counts.connections, 1);
        assert.equal(peers[side]!.counts.requests, 0);
    }
    assert.equal(peers[1]!.counts.interested, 0, "The app wanted pieces from an empty peer");
    await lab.checkpoint({ check: "choked-and-no-demand-peers-stay-on-route", milliseconds: performance.now() - started,
        before: before.diagnostics.routes, held: held.diagnostics.routes, peers: peers.map(peer => ({ ...peer.counts })) });
    peers[0]!.release();
    await waitFor("same connection becomes useful after unchoke", () => lab.info(hash), info => info.progress === 1, 30000);
    await lab.request("torrents/stop", { hashes: hash });
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    assert.equal(peers[0]!.counts.connections, 1);
    assert.equal(peers[0]!.counts.payloadBytes, verifiedBytes);
    assert.equal(peers[1]!.counts.payloadBytes, 0);
    assert.deepEqual(errors, []);
    await lab.checkpoint({ check: "idle-peer-unchokes-without-reconnect", verifiedBytes, exactSizesAndHashes: true,
        scope: "Controlled TCP choke/no-demand hold and release, not timeout or long-term route stability" });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    const stopped = await Promise.allSettled([...proxies.map(proxy => proxy.close()), ...peers.map(peer => peer.close())]);
    for (const result of stopped) if (result.status === "rejected") failure ??= result.reason;
    if (!failure) for (const name of ["fixtures", "profile", "download", "nodes.json"]) {
        const target = resolve(lab.root, name);
        assert.equal(dirname(target), resolve(lab.root), "Cleanup escaped the owned idle fixture");
        try { await rm(target, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
