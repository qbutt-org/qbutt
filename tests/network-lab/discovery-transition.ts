import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { rm, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { decode, encode, type Value } from "./bencode";
import { startProxy } from "./proxy";

interface Counters {
    dhtPackets: number;
    dhtGetPeers: number;
    httpRequests: number;
    udpPackets: number;
    udpAnnounces: number;
    udpTunnels: number;
    udpPinned: number;
}

interface Status {
    mode: string;
    busy: boolean;
    paths: { pathId: string; generation: number; edgeId: string; proxyName: string;
        open: boolean; localAddress?: string }[];
    peers: { infoHash: string; pathId: string; generation: number; peer: string; port: number;
        payloadDownload: number }[];
}

const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE;
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS;
assert(nativeInterface && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal),
"Native discovery address must belong to the selected physical adapter");
const loopback = Object.entries(networkInterfaces()).find(([, addresses]) =>
    addresses?.some(address => address.internal && address.family === "IPv4"))?.[0];
assert(loopback, "A loopback adapter is required for the controlled tunnel paths");

const lab = await createLab("discovery-transition");
const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
assert(torrent.infoHashV1, "The public fixture must have a v1 infohash");
const infoHash = Buffer.from(torrent.infoHashV1, "hex");
const counters: Counters[] = Array.from({ length: 3 }, () => ({
    dhtPackets: 0, dhtGetPeers: 0, httpRequests: 0, udpPackets: 0, udpAnnounces: 0,
    udpTunnels: 0, udpPinned: 0,
}));
const errors: string[] = [];
const dht: ReturnType<typeof createSocket>[] = [];
const udpTrackers: ReturnType<typeof createSocket>[] = [];
const httpTrackers: ReturnType<typeof Bun.serve>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let failure: unknown;

const snapshot = () => counters.map(counter => ({ ...counter }));
const trafficIncreased = (current: Counters, previous: Counters,
    phase: "udpTunnels" | "udpPinned") =>
    current.dhtGetPeers > previous.dhtGetPeers
        && current.httpRequests > previous.httpRequests
        && current[phase] > previous[phase];

try {
    seed = await startSeed(lab.python, lab.fixtures, torrent.name, lab.root,
        { label: "discovery-transition", uploadRate: 256 * 1024 });
    for (const side of [0, 1, 2]) {
        const bindAddress = side === 2 ? nativeAddress : "127.0.0.1";
        const node = createSocket("udp4");
        const nodeId = randomBytes(20);
        dht.push(node);
        node.on("message", (packet, remote) => {
            counters[side]!.dhtPackets++;
            try {
                assert(counters[side]!.dhtPackets < 4096, "DHT fixture packet limit");
                const message = decode(packet);
                if (!Buffer.isBuffer(message.y) || message.y.toString() !== "q") return;
                assert(Buffer.isBuffer(message.t) && Buffer.isBuffer(message.q));
                const args = message.a as Record<string, Value>;
                assert(args && Buffer.isBuffer(args.id));
                const query = message.q.toString();
                if (query === "get_peers" && Buffer.isBuffer(args.info_hash)
                    && args.info_hash.equals(infoHash))
                    counters[side]!.dhtGetPeers++;
                const result: Record<string, Value> = { id: nodeId, nodes: Buffer.alloc(0) };
                if (query === "get_peers") result.token = Buffer.from("fixture");
                node.send(encode({ r: result, t: message.t, y: Buffer.from("r") }),
                    remote.port, remote.address);
            }
            catch (error) { errors.push(String(error)); }
        });
        await new Promise<void>((accept, reject) => {
            node.once("error", reject);
            node.bind(0, bindAddress, accept);
        });

        httpTrackers.push(Bun.serve({ hostname: bindAddress, port: 0, fetch(request) {
            counters[side]!.httpRequests++;
            try {
                assert(counters[side]!.httpRequests < 512, "HTTP tracker fixture request limit");
                const encoded = /[?&]info_hash=([^&]*)/.exec(request.url)?.[1];
                assert(encoded, "HTTP announce omitted infohash");
                const announced = Buffer.from(encoded.replace(/%([0-9a-f]{2})/gi, (_, byte) =>
                    String.fromCharCode(Number.parseInt(byte, 16))), "latin1");
                assert(announced.equals(infoHash), "HTTP announce used another infohash");
                return new Response(encode({ interval: 30, "min interval": 1,
                    complete: 1, incomplete: 0, peers: Buffer.alloc(0) }));
            }
            catch (error) { errors.push(String(error)); return new Response("Invalid fixture announce", { status: 400 }); }
        } }));

        const tracker = createSocket("udp4");
        udpTrackers.push(tracker);
        const connectionId = 0x123456789abcdef0n;
        tracker.on("message", (packet, remote) => {
            counters[side]!.udpPackets++;
            try {
                assert(counters[side]!.udpPackets < 1024, "UDP tracker fixture packet limit");
                assert(packet.length >= 16 && packet.length <= 1024);
                const action = packet.readUInt32BE(8);
                const transaction = packet.readUInt32BE(12);
                assert(action === 0 || action === 1 || action === 2, "Unexpected UDP tracker action");
                const reply = Buffer.alloc(action === 0 ? 16 : 20);
                reply.writeUInt32BE(action);
                reply.writeUInt32BE(transaction, 4);
                if (action === 0) {
                    assert.equal(packet.readBigUInt64BE(0), 0x41727101980n);
                    reply.writeBigUInt64BE(connectionId, 8);
                }
                else {
                    assert.equal(packet.readBigUInt64BE(0), connectionId);
                    assert(packet.subarray(16, 36).equals(infoHash), "UDP tracker used another infohash");
                    if (action === 1) {
                        assert(packet.length >= 98);
                        counters[side]!.udpAnnounces++;
                        if (packet.includes(Buffer.from("phase=tunnels"))) counters[side]!.udpTunnels++;
                        if (packet.includes(Buffer.from("phase=pinned"))) counters[side]!.udpPinned++;
                    }
                    reply.writeUInt32BE(30, 8);
                    reply.writeUInt32BE(1, 16);
                }
                tracker.send(reply, remote.port, remote.address, error => {
                    if (error) errors.push(String(error));
                });
            }
            catch (error) { errors.push(String(error)); }
        });
        await new Promise<void>((accept, reject) => {
            tracker.once("error", reject);
            tracker.bind(0, bindAddress, accept);
        });
    }

    const bootstrapPort = dht[2]!.address().port;
    const httpPort = httpTrackers[2]!.port!;
    const udpPort = udpTrackers[2]!.address().port;
    assert(new Set([bootstrapPort, httpPort, udpPort]).size === 3,
        "Native discovery responders require distinct numeric ports");
    const credentials = [0, 1].map(() => ({
        username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex"),
    }));
    for (const side of [0, 1]) {
        const targets = [
            { host: nativeAddress, port: bootstrapPort, connectHost: "127.0.0.1", connectPort: dht[side]!.address().port },
            { host: nativeAddress, port: httpPort, connectHost: "127.0.0.1", connectPort: httpTrackers[side]!.port! },
            { host: nativeAddress, port: udpPort, connectHost: "127.0.0.1", connectPort: udpTrackers[side]!.address().port },
        ];
        if (side === 0)
            targets.push({ host: "127.0.0.2", port: seed.port,
                connectHost: seed.host, connectPort: seed.port });
        proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`,
            udp: true, targets }));
    }
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
        name: `discovery-transition-${side}`, type: "socks5", server: proxy.host, port: proxy.port,
        udp: true, ...credentials[side],
    })) }));

    await lab.start();
    const status = () => lab.json<Status>("qbuttPaths/status");
    await lab.request("app/setPreferences", { json: JSON.stringify({
        dht: false, pex: false, lsd: false, dht_bootstrap_nodes: `${nativeAddress}:${bootstrapPort}`,
        announce_to_all_trackers: true, announce_to_all_tiers: true,
    }) });
    const preferences = await lab.json<{ dht: boolean; dht_bootstrap_nodes: string;
        bittorrent_protocol: number; announce_to_all_trackers: boolean;
        announce_to_all_tiers: boolean }>("app/preferences");
    assert(!preferences.dht && preferences.dht_bootstrap_nodes === `${nativeAddress}:${bootstrapPort}`
        && preferences.bittorrent_protocol === 1 && preferences.announce_to_all_trackers
        && preferences.announce_to_all_tiers,
    "The fixture must begin TCP-only with DHT disabled and the controlled bootstrap node");
    await lab.request("qbuttPaths/dns", { server: "127.0.0.1:53",
        bootstrapServer: "127.0.0.1:53", family: "ipv4" });
    for (const side of [0, 1]) {
        await lab.request("qbuttPaths/open", { configPath, proxyName: `discovery-transition-${side}`,
            interfaceName: loopback });
        await waitFor("discovery tunnel path opens", status,
            current => !current.busy && current.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface });
    const mixed = await status();
    const paths = [0, 1].map(side => mixed.paths.find(path => path.proxyName === `discovery-transition-${side}`)!);
    paths.push(mixed.paths.find(path => path.edgeId === "native" && path.localAddress === nativeAddress)!);
    assert(mixed.mode === "mixed" && paths.every(path => path?.open)
        && new Set(paths.map(path => path.pathId)).size === 3
        && new Set(paths.map(path => path.edgeId)).size === 3,
    "Mixed did not admit the three independent controlled routes");

    const destination = join(lab.root, "download");
    const hash = await lab.add(torrent.name, destination);
    assert.equal(hash, torrent.infoHashV1);
    await lab.request("torrents/addTrackers", { hash, urls:
        `http://${nativeAddress}:${httpPort}/announce\nudp://${nativeAddress}:${udpPort}/announce` });
    await lab.request("torrents/start", { hashes: hash });
    await waitFor("public torrent active before DHT", () => lab.info(hash),
        info => info.state === "downloading" || info.state === "stalledDL");
    await lab.request("app/setPreferences", { json: JSON.stringify({ dht: true }) });
    assert((await lab.json<{ dht: boolean }>("app/preferences")).dht,
        "Controlled DHT was not enabled for the active public torrent");
    const initial = await waitFor("all Mixed routes deliver DHT and tracker traffic", async () => {
        assert.deepEqual(errors, []); return snapshot();
    }, current => current.every(side => side.dhtGetPeers > 0
        && side.httpRequests > 0 && side.udpAnnounces > 0), 60000);
    await lab.checkpoint({ check: "active-mixed-discovery", hash, paths, counters: initial,
        bootstrap: `${nativeAddress}:${bootstrapPort}`, trackerHost: nativeAddress });

    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    await waitFor("Mixed to Tunnels route catalog", status, current => current.mode === "tunnels"
        && !current.paths.some(path => path.edgeId === "native")
        && paths.slice(0, 2).every(path => current.paths.some(candidate =>
            candidate.open && candidate.pathId === path.pathId && candidate.generation === path.generation)));
    const beforeNativeGrace = snapshot();
    await Bun.sleep(2000);
    const afterNativeGrace = snapshot();
    await lab.request("torrents/addTrackers", { hash,
        urls: `udp://${nativeAddress}:${udpPort}/announce?phase=tunnels` });
    await lab.request("torrents/reannounce", { hashes: hash });
    const tunnels = await waitFor("both retained tunnels deliver fresh DHT and tracker traffic", async () => {
        assert.deepEqual(errors, []); return snapshot();
    }, current => [0, 1].every(side =>
        trafficIncreased(current[side]!, afterNativeGrace[side]!, "udpTunnels")), 30000);
    await Bun.sleep(3000);
    const afterTunnelsHold = snapshot();
    assert.deepEqual(afterTunnelsHold[2], afterNativeGrace[2],
        "Retired Native path reached a controlled DHT or tracker responder");
    await lab.checkpoint({ check: "active-mixed-to-tunnels-discovery", hash,
        graceMs: 2000, holdMs: 3000, beforeGrace: beforeNativeGrace,
        afterGrace: afterNativeGrace, afterForced: tunnels, afterHold: afterTunnelsHold });

    await lab.request("qbuttPaths/policy", { mode: "pinned" });
    await waitFor("Tunnels to Pinned route catalog", status, current => current.mode === "pinned"
        && paths.slice(0, 2).every(path => current.paths.some(candidate =>
            candidate.open && candidate.pathId === path.pathId && candidate.generation === path.generation)));
    const beforeSecondaryGrace = snapshot();
    await Bun.sleep(2000);
    const afterSecondaryGrace = snapshot();
    assert.deepEqual(afterSecondaryGrace[2], afterNativeGrace[2],
        "Native discovery resumed between the two policy transitions");
    await lab.request("torrents/addTrackers", { hash,
        urls: `udp://${nativeAddress}:${udpPort}/announce?phase=pinned` });
    await lab.request("torrents/reannounce", { hashes: hash });
    const pinned = await waitFor("retained Pinned route delivers fresh DHT and tracker traffic", async () => {
        assert.deepEqual(errors, []); return snapshot();
    }, current => trafficIncreased(current[0]!, afterSecondaryGrace[0]!, "udpPinned"), 30000);
    await Bun.sleep(3000);
    const afterPinnedHold = snapshot();
    assert.deepEqual(afterPinnedHold[1], afterSecondaryGrace[1],
        "Retired secondary path reached a controlled DHT or tracker responder");
    assert.deepEqual(afterPinnedHold[2], afterNativeGrace[2],
        "Native discovery resumed after switching to Pinned");
    await lab.checkpoint({ check: "active-tunnels-to-pinned-discovery", hash,
        graceMs: 2000, holdMs: 3000, beforeGrace: beforeSecondaryGrace,
        afterGrace: afterSecondaryGrace, afterForced: pinned, afterHold: afterPinnedHold });

    await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.2:${seed.port}` });
    await waitFor("retained Pinned path accepts controlled peer", status, current =>
        current.peers.some(peer => peer.infoHash === hash && peer.peer === "127.0.0.2"
            && peer.port === seed.port && peer.pathId === paths[0]!.pathId
            && peer.generation === paths[0]!.generation && peer.payloadDownload > 0), 60000);
    await waitFor("same torrent completes after discovery transition", () => lab.info(hash),
        info => info.progress === 1, 90000);
    await lab.request("torrents/stop", { hashes: hash });
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    assert.deepEqual(errors, []);
    const finalCounters = snapshot();
    assert.deepEqual(finalCounters[1], afterSecondaryGrace[1],
        "Secondary discovery resumed during Pinned payload completion");
    assert.deepEqual(finalCounters[2], afterNativeGrace[2],
        "Native discovery resumed during Pinned payload completion");
    await lab.checkpoint({ check: "discovery-transition-payload-verified", hash, verifiedBytes,
        exactSizesAndHashes: true, finalCounters,
        scope: "Controlled local DHT, forced HTTP, fresh URL UDP tracker egress and TCP payload; "
            + "no prior UDP response/reannounce, public DHT, WAN or uTP claim" });
}
catch (error) { failure = error; }
finally {
    let appStopped = false;
    try { await lab.shutdown(); appStopped = true; } catch (error) { failure ??= error; }
    const closed = await Promise.allSettled(proxies.map(proxy => proxy.close()));
    const stopped = seed ? await Promise.allSettled([seed.stop()]) : [];
    for (const result of [...closed, ...stopped])
        if (result.status === "rejected") failure ??= result.reason;
    if (stopped[0]?.status === "fulfilled") {
        try { await lab.checkpoint({ check: "discovery-transition-seed-stopped", ...stopped[0].value }); }
        catch (error) { failure ??= error; }
    }
    let respondersStopped = true;
    for (const tracker of httpTrackers) {
        try { tracker.stop(true); } catch (error) { failure ??= error; respondersStopped = false; }
    }
    const datagramsClosed = await Promise.allSettled([...dht, ...udpTrackers].map(socket =>
        new Promise<void>((accept, reject) => {
            try { socket.close(accept); } catch (error) { reject(error); }
        })));
    for (const result of datagramsClosed) if (result.status === "rejected") {
        failure ??= result.reason; respondersStopped = false;
    }
    try { await lab.checkpoint({ check: "discovery-transition-final-counters", counters: snapshot(), errors }); }
    catch (error) { failure ??= error; }
    if (!failure && errors.length)
        failure = new Error(errors.join("; "));
    if (!failure && appStopped && respondersStopped
        && [...closed, ...stopped].every(result => result.status === "fulfilled")) {
        for (const name of ["fixtures", "profile", "download", "nodes.json"]) {
            try {
                const target = resolve(lab.root, name);
                assert.equal(dirname(target), resolve(lab.root), "Cleanup escaped the owned fixture");
                await rm(target, { recursive: true, force: true });
            }
            catch (error) { failure ??= error; }
        }
    }
}
await lab.finish(failure);
if (failure) throw failure;
