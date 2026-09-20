import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { compact, decode, encode, type Value } from "./bencode";
import { startProxy } from "./proxy";

interface Status {
    busy: boolean;
    mode: string;
    paths: { pathId: string; generation: number; edgeId: string; proxyName: string; open: boolean }[];
    peers: { infoHash: string; pathId: string; generation: number; peer: string; port: number; payloadDownload: number }[];
}
const protocol = process.env.QBUTT_DISCOVERY_PROTOCOL ?? "tcp";
assert(protocol === "tcp" || protocol === "both", "QBUTT_DISCOVERY_PROTOCOL must be tcp or both");
const pexMode = process.argv.includes("--pex");
const lab = await createLab("discovery", { pex: pexMode });
const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
const infoHash = Buffer.from(torrent.infoHashV1!, "hex");
const loopback = Object.entries(networkInterfaces()).find(([, addresses]) =>
    addresses?.some(address => address.internal && address.family === "IPv4"))?.[0];
assert(loopback, "A local loopback interface is required");
const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
const datagrams: ReturnType<typeof createSocket>[] = [];
const errors: string[] = [];
const dhtQueries: { side: number; query: string; nodeId: string; readOnly: boolean; matchingInfoHash: boolean }[] = [];
const trackerQueries = { http: 0, udp: 0 };
let tracker: ReturnType<typeof Bun.serve> | undefined;
let failure: unknown;
try {
    const source = new Map(await Promise.all(torrent.files.map(async file =>
        [file.path, await readFile(join(lab.fixtures, "seed", file.path))] as const)));
    const owners = pexMode ? 2 : 4;
    for (const side of (pexMode ? [1, 0] : [0, 1, 2, 3])) {
        const pieces = Array.from({ length: torrent.pieceCount }, (_, index) => index).filter(index => index % owners === side);
        const savePath = join(lab.root, `partial-${side}`);
        for (const file of torrent.files) {
            assert(!file.pad);
            const bytes = Buffer.alloc(file.size);
            for (const piece of pieces) {
                const from = Math.max(file.offset, piece * torrent.pieceLength);
                const to = Math.min(file.offset + file.size, (piece + 1) * torrent.pieceLength);
                if (to > from) source.get(file.path)!.copy(bytes, from - file.offset, from - file.offset, to - file.offset);
            }
            await mkdir(dirname(join(savePath, file.path)), { recursive: true });
            await writeFile(join(savePath, file.path), bytes);
        }
        const neighbor = pexMode ? { host: `127.0.0.${side === 0 ? 3 : 2}`,
            port: side === 0 ? seeds[1]!.port : 0 } : undefined;
        seeds[side] = await startSeed(lab.python, lab.fixtures, torrent.name, lab.root,
            { savePath, pieces, label: `discovery-${side}`, uploadRate: 2048,
                listenAddress: pexMode ? `127.0.0.${side + 2}` : undefined, neighbor });
        assert.deepEqual(seeds[side]!.pieces, pieces);
    }
    const endpoints = seeds.map((seed, side) => ({ host: `127.0.0.${side + 2}`, port: seed.port }));
    let bootstrapPort = 0;
    let udpTrackerPort = 0;
    if (!pexMode) {
        for (let side = 0; side < 2; side++) {
            const node = createSocket("udp4"); datagrams.push(node);
            const nodeId = randomBytes(20);
            node.on("message", (packet, remote) => {
                try {
                    assert(dhtQueries.length < 4096, "DHT fixture query limit");
                    const message = decode(packet);
                    if (!Buffer.isBuffer(message.y) || message.y.toString() !== "q") return;
                    assert(Buffer.isBuffer(message.t) && Buffer.isBuffer(message.q));
                    const args = message.a as Record<string, Value>; assert(args && Buffer.isBuffer(args.id));
                    const query = message.q.toString();
                    const matchingInfoHash = Buffer.isBuffer(args.info_hash) && args.info_hash.equals(infoHash);
                    dhtQueries.push({ side, query, nodeId: args.id.toString("hex"), readOnly: message.ro === 1, matchingInfoHash });
                    assert(message.ro === 1, "Managed outgoing DHT must set ro=1");
                    assert(query !== "announce_peer", "Outgoing-only DHT must never publish a public endpoint");
                    const result: Record<string, Value> = { id: nodeId };
                    if (query === "get_peers") {
                        result.token = Buffer.from("fixture");
                        // Bootstrap liveness probes may use another target; only the
                        // fixture torrent can discover our bounded peer endpoints.
                        if (matchingInfoHash) result.values = [compact(endpoints[side]!.host, endpoints[side]!.port)];
                        else result.nodes = Buffer.alloc(0);
                    }
                    else if (query === "find_node")
                        result.nodes = Buffer.concat([nodeId, compact("127.0.0.10", bootstrapPort)]);
                    else assert(query === "ping", "Unexpected DHT query");
                    node.send(encode({ r: result, t: message.t, y: Buffer.from("r") }), remote.port, remote.address);
                }
                catch (error) { errors.push(String(error)); }
            });
            await new Promise<void>(accept => node.bind(0, "127.0.0.1", accept));
            if (side === 0) bootstrapPort = node.address().port;
        }
        tracker = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch(request) {
            try {
                const encodedHash = /[?&]info_hash=([^&]*)/.exec(request.url)?.[1]; assert(encodedHash);
                const hash = Buffer.from(encodedHash.replace(/%([0-9a-f]{2})/gi, (_, byte) =>
                    String.fromCharCode(Number.parseInt(byte, 16))), "latin1");
                assert(hash.equals(infoHash)); trackerQueries.http++;
                return new Response(encode({ interval: 30, "min interval": 1, complete: 1, incomplete: 0,
                    peers: compact(endpoints[2]!.host, endpoints[2]!.port) }));
            }
            catch (error) { errors.push(String(error)); return new Response("Invalid fixture announce", { status: 400 }); }
        } });
        const udpTracker = createSocket("udp4"); datagrams.push(udpTracker);
        const connectionId = 0x123456789abcdef0n;
        udpTracker.on("message", (packet, remote) => {
            try {
                assert(packet.length >= 16 && packet.length <= 1024);
                const action = packet.readUInt32BE(8), transaction = packet.readUInt32BE(12);
                const reply = Buffer.alloc(action === 0 ? 16 : action === 2 ? 20 : 26);
                reply.writeUInt32BE(action); reply.writeUInt32BE(transaction, 4);
                if (action === 0) {
                    assert.equal(packet.readBigUInt64BE(0), 0x41727101980n);
                    reply.writeBigUInt64BE(connectionId, 8);
                }
                else if (action === 2) {
                    assert(packet.length === 36 && packet.readBigUInt64BE(0) === connectionId);
                    assert(packet.subarray(16).equals(infoHash));
                    reply.writeUInt32BE(1, 8);
                }
                else {
                    assert(action === 1 && packet.length >= 98 && packet.readBigUInt64BE(0) === connectionId);
                    assert(packet.subarray(16, 36).equals(infoHash)); trackerQueries.udp++;
                    reply.writeUInt32BE(30, 8); reply.writeUInt32BE(1, 16);
                    compact(endpoints[3]!.host, endpoints[3]!.port).copy(reply, 20);
                }
                udpTracker.send(reply, remote.port, remote.address);
            }
            catch (error) { errors.push(String(error)); }
        });
        await new Promise<void>(accept => udpTracker.bind(0, "127.0.0.1", accept));
        udpTrackerPort = udpTracker.address().port;
    }
    const credentials = [0, 1].map(() => ({ username: randomBytes(8).toString("hex"), password: randomBytes(16).toString("hex") }));
    for (let side = 0; side < 2; side++) {
        const targets = pexMode ? [{ ...endpoints[side]!, connectHost: seeds[side]!.host }] : [
            { host: "127.0.0.10", port: bootstrapPort, connectHost: "127.0.0.1", connectPort: datagrams[side]!.address().port },
            { host: "127.0.0.11", port: tracker!.port!, connectHost: "127.0.0.1" },
            { host: "127.0.0.12", port: udpTrackerPort, connectHost: "127.0.0.1" },
            ...endpoints.map((endpoint, index) => ({ ...endpoint, connectHost: seeds[index]!.host })),
        ];
        proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`, udp: !pexMode, targets }));
    }
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
        name: `discovery-${side}`, type: "socks5", server: proxy.host, port: proxy.port, udp: !pexMode, ...credentials[side],
    })) }));
    await lab.start();
    const preferences = await lab.json<{ bittorrent_protocol: number }>("app/preferences");
    assert.equal(preferences.bittorrent_protocol, 1, "Controlled seeds require TCP peers");
    if (protocol === "both") {
        await lab.request("app/setPreferences", { json: JSON.stringify({ bittorrent_protocol: 0 }) });
        const current = await lab.json<{ bittorrent_protocol: number }>("app/preferences");
        assert.equal(current.bittorrent_protocol, 0);
    }
    await lab.checkpoint({ check: "discovery-transport", protocol });
    for (let side = 0; side < 2; side++) {
        await lab.request("qbuttPaths/open", { configPath, proxyName: `discovery-${side}`, interfaceName: loopback });
        await waitFor("discovery path open", () => lab.json<Status>("qbuttPaths/status"),
            current => !current.busy && current.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    if (pexMode) {
        const destination = join(lab.root, "download");
        const hash = await lab.add(torrent.name, destination); assert.equal(hash, infoHash.toString("hex"));
        const pathStatus = () => lab.json<Status>(`qbuttPaths/status?hash=${hash}`);
        const paths = (await pathStatus()).paths;
        const pathA = paths.find(path => path.proxyName === "discovery-0");
        const pathB = paths.find(path => path.proxyName === "discovery-1");
        assert(pathA?.open && pathB?.open && pathA.pathId !== pathB.pathId && pathA.edgeId !== pathB.edgeId);
        await lab.request("torrents/start", { hashes: hash });
        await lab.request("torrents/addPeers", { hashes: hash, peers: `${endpoints[0]!.host}:${endpoints[0]!.port}` });
        const observations = await waitFor("PEX peer B over its separate path", async () => {
            assert.deepEqual(errors, []);
            const [routes, peers] = await Promise.all([pathStatus(), lab.json<{ peers: Record<string,
                { ip: string; port: number; flags: string }> }>(`sync/torrentPeers?hash=${hash}`)]);
            const fromPex = Object.values(peers.peers).some(peer => peer.ip === endpoints[1]!.host
                && peer.port === endpoints[1]!.port && peer.flags.includes("X"));
            return { routes, fromPex };
        }, ({ routes, fromPex }) => fromPex && endpoints.every((endpoint, side) => routes.peers.some(peer =>
            peer.infoHash === hash && peer.peer === endpoint.host && peer.port === endpoint.port
                && peer.pathId === (side === 0 ? pathA.pathId : pathB.pathId)
                && peer.generation === (side === 0 ? pathA.generation : pathB.generation)
                && peer.payloadDownload > 0)), 90000);
        assert.equal(observations.routes.mode, "tunnels");
        const livePeers = await lab.json<{ peers: Record<string, { connection: string }> }>(`sync/torrentPeers?hash=${hash}`);
        assert(endpoints.every(endpoint => livePeers.peers[`${endpoint.host}:${endpoint.port}`]?.connection === "BT"),
            "PEX-discovered seeds must use controlled TCP paths");
        await lab.checkpoint({ check: "pex-cross-path-admission", infoHash: hash, bootstrap: endpoints[0],
            discovered: endpoints[1], sourcePEX: observations.fromPex, paths: observations.routes.paths,
            peers: observations.routes.peers, peerTransport: "TCP", manualBootstrapA: true, manualPeerB: false,
            dhtQueries, trackerQueries });
        await Promise.all(seeds.map(seed => seed.setUploadRate(256 * 1024)));
        await waitFor("PEX peers complete complementary pieces", () => lab.info(hash), info => info.progress === 1, 60000);
        await lab.request("torrents/stop", { hashes: hash });
        const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
        await lab.checkpoint({ check: "pex-payload-verification", verifiedBytes, exactSizes: true,
            hashesVerified: true, sourcePEX: true, paths: observations.routes.paths });
    }
    else {
        await lab.request("app/setPreferences", { json: JSON.stringify({ dht_bootstrap_nodes: `127.0.0.10:${bootstrapPort}`,
            dht: false, pex: false, lsd: false, announce_to_all_trackers: true, announce_to_all_tiers: true }) });
        const destination = join(lab.root, "download");
        const hash = await lab.add(torrent.name, destination); assert.equal(hash, infoHash.toString("hex"));
        const status = () => lab.json<Status>(`qbuttPaths/status?hash=${hash}`);
        await lab.request("torrents/addTrackers", { hash, urls:
            `http://127.0.0.11:${tracker!.port}/announce\nudp://127.0.0.12:${udpTrackerPort}/announce` });
        await lab.request("torrents/start", { hashes: hash });
        await waitFor("fixture torrent active before DHT bootstrap", () => lab.info(hash), info =>
            info.state === "downloading" || info.state === "stalledDL");
        await lab.request("app/setPreferences", { json: JSON.stringify({ dht: true }) });
        const diagnosticAt = Date.now() + 10000;
        let diagnosticWritten = false;
        const simultaneous = await waitFor("DHT and tracker peer union in one active torrent", async () => {
            assert.deepEqual(errors, []);
            const current = await status();
            if (!diagnosticWritten && Date.now() >= diagnosticAt) {
                diagnosticWritten = true;
                const [info, trackers, peers] = await Promise.all([lab.info(hash),
                    lab.json(`torrents/trackers?hash=${hash}`), lab.json(`sync/torrentPeers?hash=${hash}`)]);
                await lab.checkpoint({ check: "discovery-admission", info, trackers, peers, paths: current,
                    endpoints, proxy: proxies.map(proxy => proxy.stats), dhtQueries, trackerQueries });
            }
            return current;
        }, current => endpoints.every(endpoint => current.peers.some(peer => peer.infoHash === hash
            && peer.peer === endpoint.host && peer.port === endpoint.port && peer.payloadDownload > 0)), 60000);
        assert.equal(simultaneous.mode, "tunnels");
        assert(simultaneous.paths.filter(path => path.open).length === 2);
        assert(new Set(simultaneous.peers.map(peer => peer.pathId)).size === 2);
        for (const peer of simultaneous.peers) assert(simultaneous.paths.some(path =>
            path.pathId === peer.pathId && path.generation === peer.generation));
        const livePeers = await lab.json<{ peers: Record<string, { connection: string }> }>(`sync/torrentPeers?hash=${hash}`);
        assert(endpoints.every(endpoint => livePeers.peers[`${endpoint.host}:${endpoint.port}`]?.connection === "BT"),
            "All controlled seeds must be reached over TCP, including automatic retry mode");
        assert(dhtQueries.some(query => query.side === 0 && query.query === "get_peers" && query.matchingInfoHash)
            && dhtQueries.some(query => query.side === 1 && query.query === "get_peers" && query.matchingInfoHash));
        const identities = [0, 1].map(side => new Set(dhtQueries.filter(query => query.side === side).map(query => query.nodeId)));
        assert([...identities[0]!].every(id => !identities[1]!.has(id)), "DHT paths reused a node ID");
        assert(trackerQueries.http > 0 && trackerQueries.udp > 0);
        await lab.checkpoint({ check: "route-local-discovery-union", infoHash: hash, dhtQueries, trackerQueries,
            protocol, peerTransport: "TCP", paths: simultaneous.paths, peers: simultaneous.peers,
            manualPeerInjection: false, pex: "not-tested" });
        await Promise.all(seeds.map(seed => seed.setUploadRate(256 * 1024)));
        await waitFor("discovered peers complete complementary pieces", () => lab.info(hash), info => info.progress === 1, 45000);
        await lab.request("torrents/stop", { hashes: hash });
        const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
        await lab.checkpoint({ check: "discovery-payload-verification", verifiedBytes, exactSizes: true, hashesVerified: true,
            relay: proxies.map(proxy => proxy.stats), scope: "Controlled local DHT + HTTP/UDP tracker union; PEX and real-network discovery unverified" });
    }
}
catch (error) { failure = error; }
finally {
    try {
        await lab.checkpoint({ check: "discovery-final-paths", paths:
            await lab.json(`qbuttPaths/status?hash=${infoHash.toString("hex")}`) });
    }
    catch (error) { failure ??= error; }
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    const [closed, stopped] = await Promise.all([Promise.allSettled(proxies.map(proxy => proxy.close())),
        Promise.allSettled(seeds.map(seed => seed.stop()))]);
    for (const result of [...closed, ...stopped]) if (result.status === "rejected") failure ??= result.reason;
    for (const [side, result] of stopped.entries()) if (result.status === "fulfilled") {
        await lab.checkpoint({ check: "discovery-seed-stopped", side, ...result.value });
        if (pexMode) {
            try {
                const owned = Array.from({ length: torrent.pieceCount }, (_, index) => index)
                    .filter(index => index % 2 === side);
                assert.deepEqual(result.value.pieces, owned, "PEX fixture peer changed its checked piece subset");
                assert.equal(result.value.downloadPayloadBytes, 0, "PEX fixture peers exchanged payload with each other");
                assert(result.value.peerAddresses.includes(`127.0.0.${side === 0 ? 3 : 2}`),
                    "PEX fixture peers lost their controlled neighbor link");
            }
            catch (error) { failure ??= error; }
        }
    }
    tracker?.stop(true);
    await Promise.all(datagrams.map(socket => new Promise<void>(accept => socket.close(accept))));
    await lab.checkpoint({ check: "discovery-observations", dhtQueries, trackerQueries, errors,
        proxy: proxies.map(proxy => proxy.stats) });
    for (const name of ["fixtures", "profile", "download", "nodes.json", "partial-0", "partial-1", "partial-2", "partial-3"]) {
        const path = resolve(lab.root, name); assert(path.startsWith(resolve(lab.root) + "\\") || path.startsWith(resolve(lab.root) + "/"));
        try { await rm(path, { recursive: true, force: true }); }
        catch (error) { failure ??= error; }
    }
}
if (!failure && errors.length) failure = new Error(`Discovery fixture errors: ${errors.join("; ")}`);
await lab.finish(failure);
if (failure) throw failure;
