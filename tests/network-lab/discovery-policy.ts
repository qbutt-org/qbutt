import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { createServer, type Socket } from "node:net";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { compact, decode, encode, type Value } from "./bencode";
import { startProxy } from "./proxy";

interface Status {
    busy: boolean;
    paths: { pathId: string; generation: number; edgeId: string; open: boolean }[];
    peers: { infoHash: string; pathId: string; generation: number; peer: string; port: number; payloadDownload: number }[];
}
interface Info { has_metadata: boolean; private: boolean | null }

const lab = await createLab("discovery-policy", { pex: true });
const loopback = Object.entries(networkInterfaces()).find(([, addresses]) =>
    addresses?.some(address => address.internal && address.family === "IPv4"))?.[0];
assert(loopback, "A loopback interface is required");
const fixtures = ["v1", "v1-64k"].map(name => lab.manifest.torrents.find(item => item.name === name)!);
const hashes = fixtures.map(torrent => torrent.infoHashV1!);
const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
const dht = [createSocket("udp4"), createSocket("udp4")];
const trackers: ReturnType<typeof Bun.serve>[] = [];
const sockets = new Set<Socket>();
const errors: string[] = [];
const dhtQueries: { side: number; query: string; hash: string; phase: string }[] = [];
const announces: { side: number; hash: string; phase: string }[] = [];
const privateHandshakes: string[][] = [];
let phase = "bootstrap";
let releaseMetadata!: () => void;
const metadataReleased = new Promise<void>(accept => { releaseMetadata = accept; });
let failure: unknown;

// This peer only inspects negotiation. Real libtorrent seeds serve all payload.
const probe = createServer(socket => {
    if (sockets.size >= 4) { socket.destroy(); return; }
    sockets.add(socket);
    socket.setTimeout(15000, () => socket.destroy());
    socket.on("error", () => socket.destroy());
    socket.on("close", () => sockets.delete(socket));
    let bytes = Buffer.alloc(0), handshake = false, frames = 0;
    socket.on("data", chunk => {
        try {
            assert(bytes.length + chunk.length <= 16384, "Private handshake probe buffer limit");
            bytes = Buffer.concat([bytes, chunk]);
            if (!handshake) {
                if (bytes.length < 68) return;
                assert(bytes[0] === 19 && bytes.subarray(1, 20).toString() === "BitTorrent protocol");
                assert(bytes.subarray(28, 48).toString("hex") === hashes[0]);
                assert(bytes[25]! & 0x10, "Client did not support extension negotiation");
                const reply = Buffer.from(bytes.subarray(0, 68));
                Buffer.concat([Buffer.from("-QP0001-"), randomBytes(12)]).copy(reply, 48);
                const extension = Buffer.concat([Buffer.from([20, 0]), encode({ m: { ut_pex: 1 } })]);
                const size = Buffer.alloc(4); size.writeUInt32BE(extension.length);
                socket.write(Buffer.concat([reply, size, extension]));
                bytes = bytes.subarray(68); handshake = true;
            }
            while (bytes.length >= 4) {
                const length = bytes.readUInt32BE();
                assert(length <= 4096 && frames <= 64, "Private handshake probe frame limit");
                if (bytes.length < length + 4) return;
                const frame = bytes.subarray(4, length + 4); bytes = bytes.subarray(length + 4); frames++;
                if (frame[0] === 20 && frame[1] === 0) {
                    const extensions = decode(frame.subarray(2)).m as Record<string, Value>;
                    assert(extensions && !extensions.ut_pex, "Private torrent advertised PEX despite its flag");
                    privateHandshakes.push(Object.keys(extensions));
                    socket.destroy(); return;
                }
            }
        }
        catch (error) { errors.push(String(error)); socket.destroy(); }
    });
});

try {
    for (const fixture of fixtures)
        seeds.push(await startSeed(lab.python, lab.fixtures, fixture.name, lab.root,
            { label: `policy-${fixture.name}`, uploadRate: 96 * 1024 }));
    await new Promise<void>(accept => probe.listen(0, "127.0.0.1", accept));
    const probeAddress = probe.address(); assert(probeAddress && typeof probeAddress !== "string");
    const probePort = probeAddress.port;
    let bootstrapPort = 0, trackerPort = 0;
    for (const side of [0, 1]) {
        const identity = randomBytes(20);
        dht[side]!.on("message", (bytes, remote) => {
            try {
                assert(dhtQueries.length < 2048);
                const message = decode(bytes);
                if (!Buffer.isBuffer(message.y) || message.y.toString() !== "q") return;
                assert(Buffer.isBuffer(message.t) && Buffer.isBuffer(message.q));
                const args = message.a as Record<string, Value>; assert(args && Buffer.isBuffer(args.id));
                const hash = Buffer.isBuffer(args.info_hash) ? args.info_hash.toString("hex") : "";
                const query = message.q.toString();
                dhtQueries.push({ side, query, hash, phase });
                assert(message.ro === 1 && query !== "announce_peer", "Outgoing-only DHT role changed");
                assert(hash !== hashes[0], "Known-private infohash escaped into DHT");
                if (hash === hashes[1]) {
                    assert(side === 0, "Unknown-metadata discovery left its pinned path");
                    assert(phase !== "metadata-private", "Private metadata was followed by a new DHT query");
                }
                const result: Record<string, Value> = { id: identity, nodes: Buffer.alloc(0) };
                if (query === "get_peers") result.token = Buffer.from("fixture");
                dht[side]!.send(encode({ r: result, t: message.t, y: Buffer.from("r") }), remote.port, remote.address);
            }
            catch (error) { errors.push(String(error)); }
        });
        await new Promise<void>(accept => dht[side]!.bind(0, "127.0.0.1", accept));
        if (side === 0) bootstrapPort = dht[side]!.address().port;
        trackers.push(Bun.serve({ hostname: "127.0.0.1", port: 0, async fetch(request) {
            try {
                assert(announces.length < 128);
                const encoded = /[?&]info_hash=([^&]*)/.exec(request.url)?.[1]; assert(encoded);
                const hash = Buffer.from(encoded.replace(/%([0-9a-f]{2})/gi, (_, byte) =>
                    String.fromCharCode(Number.parseInt(byte, 16))), "latin1").toString("hex");
                announces.push({ side, hash, phase });
                const index = hashes.indexOf(hash); assert(index >= 0 && side === 0, "Tracker announce left the pinned path");
                if (index === 1) await metadataReleased;
                return new Response(encode({ interval: 30, "min interval": 1, complete: 1, incomplete: 0,
                    peers: Buffer.concat([compact(`127.0.0.${index + 2}`, seeds[index]!.port),
                        ...(index === 0 ? [compact("127.0.0.4", probePort)] : [])]) }));
            }
            catch (error) { errors.push(String(error)); return new Response("Invalid controlled announce", { status: 400 }); }
        } }));
        if (side === 0) trackerPort = trackers[side]!.port!;
    }
    const credentials = [0, 1].map(() => ({ username: randomBytes(8).toString("hex"), password: randomBytes(16).toString("hex") }));
    for (const side of [0, 1]) proxies.push(await startProxy({ ...credentials[side]!, udp: true, targets: [
        { host: "127.0.0.10", port: bootstrapPort, connectHost: "127.0.0.1", connectPort: dht[side]!.address().port },
        { host: "127.0.0.11", port: trackerPort, connectHost: "127.0.0.1", connectPort: trackers[side]!.port! },
        ...seeds.map((seed, index) => ({ host: `127.0.0.${index + 2}`, port: seed.port, connectHost: seed.host })),
        { host: "127.0.0.4", port: probePort, connectHost: "127.0.0.1" },
    ] }));
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({ name: `policy-${side}`,
        type: "socks5", server: proxy.host, port: proxy.port, udp: true, ...credentials[side] })) }));
    await lab.start();
    const status = (hash?: string) => lab.json<Status>(`qbuttPaths/status${hash ? `?hash=${hash}` : ""}`);
    for (const side of [0, 1]) {
        await lab.request("qbuttPaths/open", { configPath, proxyName: `policy-${side}`, edgeId: `policy-${side}`, interfaceName: loopback });
        await waitFor("policy path ready", status, current => !current.busy && current.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const pinned = (await status()).paths.find(path => path.edgeId === "policy-0")!;
    assert(pinned?.open);
    await lab.request("app/setPreferences", { json: JSON.stringify({ dht: true, pex: true, lsd: false,
        dht_bootstrap_nodes: `127.0.0.10:${bootstrapPort}` }) });
    await waitFor("both DHT responders observed live bootstrap", async () => dhtQueries,
        queries => [0, 1].every(side => queries.some(query => query.side === side)));
    const preferences = await lab.json<{ dht: boolean; pex: boolean }>("app/preferences");
    assert(preferences.dht && preferences.pex, "Global discovery settings must be enabled");
    const trackerUrl = `http://127.0.0.11:${trackerPort}/announce`;
    for (const index of [0, 1]) {
        const hash = hashes[index]!;
        const destination = join(lab.root, `download-${index}`);
        phase = index === 0 ? "known-private" : "unknown-metadata";
        if (index === 0) {
            assert.equal(await lab.add(fixtures[index]!.name, destination), hash);
            await lab.request("torrents/addTrackers", { hash, urls: trackerUrl });
            await lab.request("torrents/start", { hashes: hash });
        }
        else {
            await mkdir(destination, { recursive: true });
            await lab.request("torrents/add", { urls: `magnet:?xt=urn:btih:${hash}&tr=${encodeURIComponent(trackerUrl)}`,
                savepath: destination, stopped: "false", autoTMM: "false", contentLayout: "Original" });
            await waitFor("unknown metadata stays on its pinned discovery path", async () => {
                assert.deepEqual(errors, []);
                const jobs = await lab.json<Info[]>(`torrents/info?hashes=${hash}`);
                return { job: jobs[0], queries: dhtQueries.filter(query => query.hash === hash) };
            }, current => !!current.job && !current.job.has_metadata && current.queries.some(query => query.side === 0)
                && announces.some(announce => announce.hash === hash && announce.side === 0), 45000);
            await lab.checkpoint({ check: "unknown-metadata-discovery-pinned", hash, pinned,
                dhtQueries: dhtQueries.filter(query => query.hash === hash), announces: announces.filter(item => item.hash === hash),
                scope: "Infohash may be queried on the pinned path before metadata identifies it as private" });
            // Private libtorrent seeds intentionally do not offer ut_metadata.
            // Import the matching file into the existing magnet through the normal UI API.
            try { await lab.add(fixtures[index]!.name, destination); }
            catch (error) {
                // Upstream applies metadata before reporting the duplicate-add conflict.
                assert(error instanceof Error && error.message === "WebUI torrents/add returned HTTP 409", String(error));
            }
            await waitFor("existing magnet receives private metadata", () => lab.json<Info[]>(`torrents/info?hashes=${hash}`),
                jobs => jobs.length === 1 && jobs[0]?.has_metadata === true && jobs[0].private === true);
            phase = "metadata-private";
            releaseMetadata();
        }
        const active = await waitFor("private peer uses only its pinned route", async () => {
            assert.deepEqual(errors, []);
            const [routes, jobs] = await Promise.all([status(hash), lab.json<Info[]>(`torrents/info?hashes=${hash}`)]);
            for (const peer of routes.peers) if (peer.infoHash === hash)
                assert(peer.pathId === pinned.pathId && peer.generation === pinned.generation, "Private peer escaped the pinned route");
            return { routes, job: jobs[0] };
        }, current => current.job?.has_metadata === true && current.job.private === true
            && current.routes.peers.some(peer => peer.infoHash === hash && peer.payloadDownload > 0)
            && (index !== 0 || privateHandshakes.length > 0), 45000);
        await waitFor("private payload complete", async () => {
            assert.deepEqual(errors, []); return lab.info(hash);
        }, info => info.progress === 1);
        await lab.request("torrents/stop", { hashes: hash });
        await waitFor("private torrent stopped", () => lab.info(hash), info => info.state === "stoppedUP");
        const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
        await lab.checkpoint({ check: index === 0 ? "private-discovery-policy" : "unknown-to-private-policy", hash,
            pinned, peers: active.routes.peers, privateHandshakes, verifiedBytes, exactSizes: true, hashesVerified: true,
            dhtQueries: dhtQueries.filter(query => query.hash === hash), announces: announces.filter(item => item.hash === hash) });
    }
    assert.equal(proxies[1]!.stats.uploadStreamBytes, 0, "Unpinned path carried TCP traffic");
    assert.equal(proxies[1]!.stats.downloadStreamBytes, 0, "Unpinned path returned TCP traffic");
}
catch (error) { failure = error; }
finally {
    releaseMetadata();
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    for (const socket of sockets) socket.destroy();
    await new Promise<void>(accept => probe.close(() => accept()));
    for (const tracker of trackers) tracker.stop(true);
    const seedStats: unknown[] = [];
    const closed = await Promise.allSettled([...proxies.map(proxy => proxy.close()), ...seeds.map(async (seed, index) => {
        const stats = await seed.stop(); seedStats.push({ name: fixtures[index]!.name, ...stats });
        assert.equal(stats.downloadPayloadBytes, 0, "A fixture seed unexpectedly downloaded payload");
    })]);
    for (const result of closed) if (result.status === "rejected") failure ??= result.reason;
    await Promise.all(dht.map(socket => new Promise<void>(accept => { try { socket.close(accept); } catch { accept(); } })));
    await lab.checkpoint({ check: "policy-final-observations", dhtQueries, announces, privateHandshakes, errors,
        proxies: proxies.map(proxy => proxy.stats), seedStats });
    for (const name of ["fixtures", "profile", "nodes.json", "download-0", "download-1"]) {
        const path = resolve(lab.root, name); assert(dirname(path) === resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
if (!failure && errors.length) failure = new Error(errors.join("; "));
await lab.finish(failure);
if (failure) throw failure;
