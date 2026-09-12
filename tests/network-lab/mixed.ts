import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createConnection } from "node:net";
import { networkInterfaces } from "node:os";
import { dirname, join } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface PathsStatus {
    busy: boolean;
    mode: string;
    paths: { pathId: string; generation: number; edgeId: string; open: boolean; localAddress?: string;
        closedPayloadDownload?: number }[];
    peers: { pathId: string; generation: number; peer: string; port: number; localAddress: string; localPort: number;
        infoHash: string; payloadDownload: number; payloadUpload: number }[];
}

const baseline = process.argv.includes("--baseline");
const native = process.argv.includes("--native");
const tunnelsOnly = process.argv.includes("--tunnels");
assert([baseline, native, tunnelsOnly].filter(Boolean).length === 1,
    "Baseline, Native and Tunnels-only are separate integration scenarios");
const managedMode = tunnelsOnly ? "tunnels" : "mixed";
const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE;
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS;
if (native) {
    assert(nativeInterface && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
    assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
        && address.family === "IPv4" && !address.internal), "Native fixture address must belong to the selected local adapter");
}
const sides = native ? [0, 1, 2, 3] : [0, 1];
const tunnelSides = native ? [0, 1, 2] : sides;
const lab = await createLab(baseline ? "mixed-baseline" : native ? "mixed-native" : tunnelsOnly ? "tunnels-only" : "mixed");
const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
const expectedBytes = lab.manifest.payload.reduce((sum, file) => sum + file.size, 0);
const original = new Map(await Promise.all(lab.manifest.payload.map(async file =>
    [file.path, await readFile(join(lab.fixtures, "seed", file.path))] as const)));
const subsets: { pieces: number[]; bytes: number; path: string; hashes: Record<string, string> }[] = [];
const credentials = tunnelSides.map(() => ({
    username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex"),
}));
const setupSeedUploadRate = 1024;
const transferSeedUploadRate = 256 * 1024;
let failure: unknown;
try {
    for (const side of sides) {
        const pieces = Array.from({ length: torrent.pieceCount }, (_, i) => i).filter(i => i % sides.length === side);
        const path = join(lab.root, `partial-${side}`);
        const hashes: Record<string, string> = {};
        let bytes = 0;
        for (const file of torrent.files) {
            assert(!file.pad, "The v1 complementary fixture must not contain padding");
            const source = original.get(file.path)!;
            const partial = Buffer.alloc(file.size);
            for (const piece of pieces) {
                const from = Math.max(file.offset, piece * torrent.pieceLength);
                const to = Math.min(file.offset + file.size, (piece + 1) * torrent.pieceLength);
                if (to > from) {
                    source.copy(partial, from - file.offset, from - file.offset, to - file.offset);
                    bytes += to - from;
                }
            }
            await mkdir(dirname(join(path, file.path)), { recursive: true });
            await writeFile(join(path, file.path), partial);
            hashes[file.path] = sha256(partial);
        }
        subsets.push({ pieces, bytes, path, hashes });
        const seed = await startSeed(lab.python, lab.fixtures, torrent.name, lab.root,
            { savePath: path, pieces, label: `partial-${side}`, listenAddress: native && side === 3 ? nativeAddress : undefined,
                uploadRate: native ? setupSeedUploadRate : undefined });
        seeds.push(seed);
        assert.deepEqual(seed.pieces, pieces, "Native seed did not verify its exact complementary bitmap");
        assert(seed.verifiedPayloadBytes === bytes, "Native partial seed verified-byte count differs from physical layout");
        if (native && side === 3)
            continue;
        const syntheticHost = `127.0.0.${side + 2}`;
        const direct = createConnection({ host: syntheticHost, port: seed.port });
        const directlyReachable = await new Promise<boolean>((resolve, reject) => {
            direct.setTimeout(5000, () => direct.destroy(new Error("Synthetic endpoint probe timed out")));
            direct.once("connect", () => { direct.destroy(); resolve(true); });
            direct.once("error", (error: NodeJS.ErrnoException) => {
                if (error.code === "ECONNREFUSED")
                    resolve(false);
                else
                    reject(error);
            });
        });
        assert(!directlyReachable, "Complementary peer unexpectedly accepts a direct connection");
        proxies.push(await startProxy({ ...credentials[side]!, targets: [2, 4].map(offset => ({
            host: `127.0.0.${side + offset}`, port: seed.port, connectHost: seed.host, connectPort: seed.port,
        })) }));
    }
    assert(subsets.reduce((sum, subset) => sum + subset.bytes, 0) === expectedBytes, "Physical subsets do not cover the full target");
    await lab.checkpoint({ check: "complementary-native-checked-topology", pieceCount: torrent.pieceCount,
        expectedBytes, peers: seeds.map((seed, side) => ({
            endpoint: `${native && side === 3 ? seed.host : `127.0.0.${side + 2}`}:${seed.port}`, pieces: seed.pieces,
            verifiedPayloadBytes: seed.verifiedPayloadBytes,
            directConnection: native && side === 3 ? "selected-physical-interface-address" : "refused",
        })), scope: native
            ? "Controlled local TCP peers with an explicit physical Native address; no public egress, physical wire or throughput claim"
            : "Loopback TCP topology with exclusive authenticated proxy maps; no public egress claim" });

    await lab.start();
    if (native) {
        await lab.request("app/setPreferences", { json: JSON.stringify({ bittorrent_protocol: 1 }) });
        await waitFor("physical Mixed TCP peer protocol", () =>
            lab.json<Record<string, unknown>>("app/preferences"), preferences => preferences.bittorrent_protocol === 1);
    }
    if (baseline) {
        for (const side of tunnelSides) {
            const proxy = proxies[side]!;
            await lab.request("app/setPreferences", { json: JSON.stringify({
                proxy_type: "SOCKS5", proxy_ip: proxy.host, proxy_port: proxy.port,
                proxy_auth_enabled: true, proxy_username: credentials[side]!.username,
                proxy_password: credentials[side]!.password, proxy_hostname_lookup: true,
                proxy_bittorrent: true, proxy_peer_connections: true, proxy_rss: false, proxy_misc: false,
                bittorrent_protocol: 1,
            }) });
            const destination = join(lab.root, `single-path-${side}`);
            const hash = await lab.add(torrent.name, destination);
            await lab.request("torrents/start", { hashes: hash });
            await lab.request("torrents/addPeers", { hashes: hash,
                peers: seeds.map((seed, index) => `127.0.0.${index + 2}:${seed.port}`).join("|") });
            const partial = await waitFor(`single path ${side} receives its subset`, () => lab.info(hash),
                info => info.completed === subsets[side]!.bytes);
            assert(partial.progress < 1, "A single exclusive route completed complementary data");
            await waitFor("other endpoint rejected by exclusive proxy map", async () => proxy.stats.deniedConnections, count => count > 0);
            await Bun.sleep(3000);
            assert((await lab.info(hash)).completed === partial.completed, "Single path received bytes outside its subset");
            const states = await lab.json<number[]>(`torrents/pieceStates?hash=${hash}`);
            assert.deepEqual(states.flatMap((state, piece) => state === 2 ? [piece] : []), subsets[side]!.pieces,
                "Native completed bitmap differs from the path's physical subset");
            await lab.request("torrents/stop", { hashes: hash });
            await waitFor("partial download stopped", () => lab.info(hash), info => info.state === "stoppedDL");
            let verifiedBytes = 0;
            for (const file of torrent.files) {
                const ownedPieces = subsets[side]!.pieces.filter(piece => file.offset < (piece + 1) * torrent.pieceLength
                    && file.offset + file.size > piece * torrent.pieceLength);
                if (!ownedPieces.length || file.size === 0)
                    continue;
                const bytes = await readFile(join(destination, file.path));
                for (const piece of ownedPieces) {
                    const from = Math.max(file.offset, piece * torrent.pieceLength);
                    const to = Math.min(file.offset + file.size, (piece + 1) * torrent.pieceLength);
                    if (to > from) {
                        assert(bytes.subarray(from - file.offset, to - file.offset)
                            .equals(original.get(file.path)!.subarray(from - file.offset, to - file.offset)),
                        "Downloaded subset differs from expected physical payload");
                        verifiedBytes += to - from;
                    }
                }
            }
            await lab.checkpoint({ check: "single-path-cannot-complete-complementary-torrent", side,
                verifiedBytes, expectedBytes, completedPieces: subsets[side]!.pieces,
                relay: { ...proxy.stats }, outcome: "observed-incomplete-with-exact-native-bitmap" });
            await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
            await waitFor("partial job removed", () => lab.json<unknown[]>("torrents/info"), jobs => jobs.length === 0);
        }
    }
    else {
        const configPath = join(lab.root, "mixed-nodes.json");
        await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
            name: `partial-${side}`, type: "socks5", server: proxy.host, port: proxy.port,
            ...credentials[side], udp: false,
        })) }));
        const readPaths = () => lab.json<PathsStatus>("qbuttPaths/status");
        for (const side of tunnelSides) {
            await lab.request("qbuttPaths/open", { configPath, proxyName: `partial-${side}`,
                edgeId: `edge-${side}`, interfaceName: "Loopback Pseudo-Interface 1" });
            const status = await waitFor("open independent path", readPaths, status => !status.busy);
            assert(status.paths?.filter(path => path.open).length === side + 1, "Expected independent active paths");
        }
        if (tunnelsOnly) {
            const destination = join(lab.root, "active-policy-transition");
            const hash = await lab.add(torrent.name, destination);
            await lab.request("torrents/start", { hashes: hash });
            await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.2:${seeds[0]!.port}` });
            const pinned = await waitFor("pinned path obtains only its complementary subset", () => lab.info(hash),
                info => info.completed === subsets[0]!.bytes);
            assert(pinned.progress < 1, "Pinned path unexpectedly completed the complementary torrent");
            await Bun.sleep(1500);
            assert((await lab.info(hash)).completed === pinned.completed, "Pinned path received bytes outside its subset");
            await lab.request("qbuttPaths/policy", { mode: managedMode });
            await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.3:${seeds[1]!.port}` });
            await waitFor("active torrent uses the newly admitted tunnel", readPaths, status => status.peers.some(peer =>
                peer.peer === "127.0.0.3" && peer.port === seeds[1]!.port && peer.payloadDownload > 16384), 120000);
            await waitFor("active torrent completes after Tunnels-only transition", () => lab.info(hash),
                info => info.progress === 1, 120000);
            const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
            await lab.checkpoint({ check: "active-pinned-to-tunnels-only-transition", verifiedBytes, exactSizes: true });
            await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
            await waitFor("transition torrent removed", () => lab.json<unknown[]>("torrents/info"), jobs => jobs.length === 0);
        }
        await lab.request("qbuttPaths/policy", { mode: managedMode, ...(native ? { nativeInterface: nativeInterface! } : {}) });
        const selected = await readPaths();
        const expectedPaths = sides.map(side => selected.paths.find(path => native && side === 3
            ? path.edgeId === "native" && path.localAddress === nativeAddress : path.edgeId === `edge-${side}`));
        assert(selected.mode === managedMode && expectedPaths.every(path => path?.open)
            && new Set(expectedPaths.map(path => path!.pathId)).size === sides.length,
            `${managedMode} policy did not retain the distinct fixture paths`);
        for (const retry of native ? [false] : [false, true]) {
            if (retry)
                await lab.request("qbuttPaths/policy", { mode: managedMode }); // Reset per-peer route exploration.
            const destination = join(lab.root, retry ? "retry-target" : "mixed-target");
            const hash = await lab.add(torrent.name, destination);
            await lab.request("torrents/start", { hashes: hash });
            const order = retry ? [1, 0] : sides;
            const failuresBefore = proxies.map(proxy => proxy.stats.deniedConnections);
            const endpoints = order.map(side => ({ side,
                host: native && side === 3 ? nativeAddress! : `127.0.0.${side + (retry ? 4 : 2)}` }));
            // The physical scenario admits peers in path order so libtorrent's
            // asynchronous torrent iteration cannot assign the first attempts
            // to unrelated exclusive endpoints. Every seed is rate-limited,
            // keeping earlier connections active for the concurrent-flow check.
            if (!retry && !native)
                await lab.request("torrents/addPeers", { hashes: hash,
                    peers: endpoints.map(({ side, host }) => `${host}:${seeds[side]!.port}`).join("|") });
            if (native) {
                // Bound setup by one normal reconnect interval per candidate;
                // the per-seed cap keeps early peers from finishing meanwhile.
                const setupStartedAt = Date.now();
                const setupDeadlineMilliseconds = sides.length * 60000;
                const ready = await waitFor("every exclusive path connects during bounded setup", readPaths, status =>
                    endpoints.every(({ side, host }) => status.peers.some(peer => peer.peer === host
                        && peer.port === seeds[side]!.port && peer.pathId === expectedPaths[side]!.pathId
                        && peer.generation === expectedPaths[side]!.generation && peer.payloadDownload > 0)), setupDeadlineMilliseconds);
                await Promise.all(seeds.map(seed => seed.setUploadRate(transferSeedUploadRate)));
                await lab.checkpoint({ check: "native-mixed-connection-setup", setupSeedUploadRate,
                    setupDeadlineMilliseconds, setupMilliseconds: Date.now() - setupStartedAt,
                    transferSeedUploadRate, setupRateRaised: true, peers: ready.peers });
            }
            let concurrentPeers: PathsStatus["peers"] = [];
            for (const { side, host } of endpoints) {
                if (retry || native) {
                    await lab.request("torrents/addPeers", { hashes: hash, peers: `${host}:${seeds[side]!.port}` });
                }
                if (retry) {
                    await waitFor("wrong path rejected before alternative retry", async () => proxies[1 - side]!.stats.deniedConnections,
                        count => count > failuresBefore[1 - side]!);
                }
                const observation = await waitFor("peer supplies data through its exclusive path", readPaths, status =>
                    status.peers.some(peer => peer.peer === host && peer.port === seeds[side]!.port && peer.payloadDownload > 16384), 120000);
                const peer = observation.peers.find(peer => peer.peer === host && peer.port === seeds[side]!.port)!;
                assert(peer.pathId === expectedPaths[side]!.pathId && peer.generation === expectedPaths[side]!.generation,
                    "Native peer telemetry attributed payload to the wrong path or generation");
                if (native && side === 3)
                    assert(peer.localAddress === nativeAddress && peer.localPort > 0,
                        "Native socket did not bind the explicit physical source address");
                if (!retry && side === sides.at(-1))
                    concurrentPeers = observation.peers;
            }
            if (!retry) {
                assert(concurrentPeers.length === endpoints.length && endpoints.every(({ side, host }) =>
                    concurrentPeers.some(peer => peer.peer === host && peer.port === seeds[side]!.port
                        && peer.pathId === expectedPaths[side]!.pathId && peer.generation === expectedPaths[side]!.generation
                        && peer.payloadDownload > 16384)),
                "Every exact fixture peer must supply data through its own path in the same snapshot");
                assert(concurrentPeers.every(peer => peer.infoHash === hash), "Peer telemetry escaped the torrent's infohash");
                const intervalStartedAt = Date.now();
                const flowing = await waitFor("payload increases on every concurrent path", readPaths, status =>
                    concurrentPeers.every(previous => status.peers.some(peer => peer.peer === previous.peer
                        && peer.port === previous.port && peer.pathId === previous.pathId && peer.generation === previous.generation
                        && peer.payloadDownload > previous.payloadDownload)), 10000);
                await lab.checkpoint({ check: "same-torrent-concurrent-native-peer-paths", intervalMilliseconds: Date.now() - intervalStartedAt,
                    previousPeers: concurrentPeers, peers: flowing.peers });
            }
            await waitFor("complete complementary payload", () => lab.info(hash), info => info.progress === 1, 120000);
            await lab.request("torrents/stop", { hashes: hash });
            await waitFor("complete target stopped", () => lab.info(hash), info => info.state === "stoppedUP");
            const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
            const closed = await waitFor("closed peer payload is retained per path", readPaths, status =>
                tunnelSides.every(side => (status.paths.find(path => path.pathId === expectedPaths[side]!.pathId)
                    ?.closedPayloadDownload ?? 0) >= subsets[side]!.bytes * (retry ? 2 : 1)));
            await lab.checkpoint({ check: retry ? "failed-route-retries-alternative-and-completes" : "mixed-complementary-payload",
                verifiedBytes, exactSizes: true, paths: closed.paths,
                relay: proxies.map(proxy => ({ ...proxy.stats })) });
            await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
            await waitFor("completed job removed", () => lab.json<unknown[]>("torrents/info"), jobs => jobs.length === 0);
            assert(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes, "Removing the job changed payload");
        }
        await lab.request("qbuttPaths/stop", {});
        const stopped = await readPaths();
        assert(stopped.paths.every(path => !path.open), "Path shutdown retained a live listener");
        await lab.shutdown();
        await lab.start();
        const restored = await readPaths();
        assert(restored.mode === managedMode && restored.paths.every(path => !path.open),
            "Restart did not preserve the blocked managed policy");
        await lab.checkpoint({ check: "managed-policy-restart", mode: restored.mode, openPaths: 0 });
    }
    await lab.shutdown();
}
catch (error) {
    failure = error;
    try { await lab.shutdown(); }
    catch (shutdownError) { console.error(String(shutdownError)); }
}
finally {
    const closed = await Promise.allSettled(proxies.map(proxy => proxy.close()));
    const stopped = await Promise.allSettled(seeds.map(seed => seed.stop()));
    for (const result of [...closed, ...stopped]) {
        if (result.status === "rejected")
            failure ??= result.reason;
    }
    for (const [side, result] of stopped.entries()) {
        if (result.status === "fulfilled") {
            await lab.checkpoint({ check: "partial-seed-shutdown", side, ...result.value });
            if (native && side === 3 && (result.value.peerAddresses.length !== 1 || result.value.peerAddresses[0] !== nativeAddress))
                failure ??= new Error("Native peer did not observe the explicit physical source address");
        }
    }
    for (const subset of subsets) {
        for (const [path, hash] of Object.entries(subset.hashes)) {
            if (sha256(await readFile(join(subset.path, path))) !== hash)
                failure ??= new Error("Partial seed's physical data changed during the lab");
        }
    }
}
await lab.finish(failure);
if (failure)
    throw failure;
