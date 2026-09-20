import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { createConnection, createServer } from "node:net";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface Status {
    busy: boolean;
    mode: string;
    paths: { pathId: string; generation: number; edgeId: string; proxyName: string; open: boolean;
        localAddress?: string }[];
    peers: { infoHash: string; pathId: string; generation: number; peer: string; port: number;
        localAddress: string; payloadDownload: number }[];
}

const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE;
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS;
assert(nativeInterface && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal),
"Native fixture address must belong to the selected physical adapter");

const lab = await createLab("policy-transition");
const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
let nativeCanaryHits = 0;
const canary = createServer(socket => { nativeCanaryHits++; socket.destroy(); });
let failure: unknown;
async function nativeSocketCount(seedPort: number): Promise<number> {
    const script = `@(Get-NetTCPConnection -State Established -RemoteAddress '${nativeAddress}' `
        + `-RemotePort ${seedPort} -ErrorAction SilentlyContinue | `
        + `Where-Object { $_.LocalAddress -eq '${nativeAddress}' }).Count`;
    const probe = Bun.spawn(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script], {
        stdout: "pipe", stderr: "pipe", windowsHide: true,
    });
    const [exitCode, output, error] = await Promise.all([
        probe.exited, new Response(probe.stdout).text(), new Response(probe.stderr).text(),
    ]);
    assert.equal(exitCode, 0, `Native TCP socket inspection failed: ${error}`);
    assert.match(output.trim(), /^\d+$/, "Native TCP socket inspection returned invalid data");
    return Number(output.trim());
}
try {
    const source = new Map(await Promise.all(torrent.files.map(async file =>
        [file.path, await readFile(join(lab.fixtures, "seed", file.path))] as const)));
    for (const side of [0, 1, 2]) {
        const pieces = Array.from({ length: torrent.pieceCount }, (_, piece) => piece)
            .filter(piece => piece % 3 === side);
        const savePath = join(lab.root, `partial-${side}`);
        for (const file of torrent.files) {
            assert(!file.pad, "The controlled v1 torrent must have no padding files");
            const bytes = Buffer.alloc(file.size);
            for (const piece of pieces) {
                const from = Math.max(file.offset, piece * torrent.pieceLength);
                const to = Math.min(file.offset + file.size, (piece + 1) * torrent.pieceLength);
                if (to > from)
                    source.get(file.path)!.copy(bytes, from - file.offset, from - file.offset, to - file.offset);
            }
            await mkdir(dirname(join(savePath, file.path)), { recursive: true });
            await writeFile(join(savePath, file.path), bytes);
        }
        const seed = await startSeed(lab.python, lab.fixtures, torrent.name, lab.root, {
            savePath, pieces, label: `policy-partial-${side}`, uploadRate: 4 * 1024,
            listenAddress: side === 2 ? nativeAddress : undefined,
        });
        assert.deepEqual(seed.pieces, pieces, "Partial seed did not verify its exact piece bitmap");
        seeds.push(seed);
    }
    seeds.push(await startSeed(lab.python, lab.fixtures, torrent.name, lab.root,
        { label: "policy-completion", uploadRate: 4 * 1024 }));

    const credentials = [0, 1].map(() => ({ username: randomBytes(16).toString("hex"),
        password: randomBytes(24).toString("hex") }));
    for (const side of [0, 1]) {
        const targets = [{ host: `127.0.0.${side + 2}`, port: seeds[side]!.port,
            connectHost: seeds[side]!.host }];
        if (side === 0)
            targets.push({ host: "127.0.0.5", port: seeds[3]!.port, connectHost: seeds[3]!.host });
        proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`, targets }));
    }
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
        name: `policy-${side}`, type: "socks5", server: proxy.host, port: proxy.port,
        ...credentials[side], udp: false,
    })) }));

    await new Promise<void>((resolve, reject) => {
        canary.once("error", reject);
        canary.listen(0, nativeAddress, resolve);
    });
    const address = canary.address();
    assert(address && typeof address !== "string");
    const canaryPort = address.port;
    await new Promise<void>((resolve, reject) => {
        const probe = createConnection({ host: nativeAddress, port: canaryPort });
        probe.setTimeout(5000, () => probe.destroy(new Error("Native canary probe timed out")));
        probe.once("connect", () => { probe.destroy(); resolve(); });
        probe.once("error", reject);
    });
    await waitFor("Native canary reachability", async () => nativeCanaryHits, hits => hits === 1, 1000);

    await lab.start();
    const preferences = await lab.json<{ bittorrent_protocol: number }>("app/preferences");
    assert.equal(preferences.bittorrent_protocol, 1, "The policy fixture requires TCP-only peers");
    const status = () => lab.json<Status>("qbuttPaths/status");
    for (const side of [0, 1]) {
        await lab.request("qbuttPaths/open", { configPath, proxyName: `policy-${side}`,
            interfaceName: "Loopback Pseudo-Interface 1" });
        await waitFor("independent tunnel path opens", status,
            current => !current.busy && current.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface });
    const mixed = await status();
    const paths = [0, 1].map(side => mixed.paths.find(path => path.proxyName === `policy-${side}`)!);
    paths.push(mixed.paths.find(path => path.edgeId === "native" && path.localAddress === nativeAddress)!);
    assert(mixed.mode === "mixed" && paths.every(path => path?.open)
        && new Set(paths.map(path => path.pathId)).size === 3
        && new Set(paths.map(path => path.edgeId)).size === 3,
    "Mixed did not admit two distinct remote edges and the selected Native route");

    const destination = join(lab.root, "download");
    const hash = await lab.add(torrent.name, destination);
    await lab.request("torrents/start", { hashes: hash });
    const endpoints = [0, 1, 2].map(side => ({
        host: side === 2 ? nativeAddress : `127.0.0.${side + 2}`,
        port: seeds[side]!.port,
    }));
    for (const [side, endpoint] of endpoints.entries()) {
        await lab.request("torrents/addPeers", { hashes: hash, peers: `${endpoint.host}:${endpoint.port}` });
        await waitFor("each exclusive route carries active payload", status, current =>
            current.peers.some(peer => peer.infoHash === hash && peer.peer === endpoint.host
                && peer.port === endpoint.port && peer.pathId === paths[side]!.pathId
                && peer.generation === paths[side]!.generation && peer.payloadDownload > 16384), 60000);
    }
    const active = await status();
    assert(endpoints.every((endpoint, side) => active.peers.some(peer =>
        peer.infoHash === hash && peer.peer === endpoint.host && peer.port === endpoint.port
        && peer.pathId === paths[side]!.pathId && peer.generation === paths[side]!.generation
        && peer.payloadDownload > 16384)), "All three paths must flow in the same active torrent snapshot");
    assert(active.peers.some(peer => peer.infoHash === hash && peer.pathId === paths[2]!.pathId
        && peer.localAddress === nativeAddress), "Native peer did not bind the selected physical source");
    assert(proxies.every(proxy => proxy.stats.downloadStreamBytes > 0),
        "Both controlled remote relays must carry payload before retirement");
    assert(await nativeSocketCount(seeds[2]!.port) > 0,
        "The active Native peer has no established physical TCP socket");
    assert((await lab.info(hash)).progress < 1, "Torrent completed before the policy transition");
    await lab.checkpoint({ check: "active-mixed-tcp", hash, paths, peers: active.peers,
        relay: proxies.map(proxy => ({ ...proxy.stats })) });

    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const tunnels = await waitFor("Native socket retires after Mixed to Tunnels", status, current =>
        current.mode === "tunnels" && !current.paths.some(path => path.edgeId === "native")
        && !current.peers.some(peer => peer.infoHash === hash && peer.pathId === paths[2]!.pathId)
        && [0, 1].every(side => current.peers.some(peer => peer.infoHash === hash
            && peer.pathId === paths[side]!.pathId && peer.generation === paths[side]!.generation)), 15000);
    await waitFor("physical Native TCP socket closes", () => nativeSocketCount(seeds[2]!.port),
        count => count === 0, 15000);
    assert((await lab.info(hash)).progress < 1, "Torrent completed before Tunnels to Pinned");
    await lab.request("torrents/addPeers", { hashes: hash,
        peers: `${nativeAddress}:${canaryPort}` });
    await Bun.sleep(2500);
    assert.equal(nativeCanaryHits, 1, "A retired Native route reached the directly available canary");
    const tunnelFlow = await waitFor("both remote paths remain active after Native retirement", status,
        current => [0, 1].every(side => current.peers.some(peer => peer.infoHash === hash
            && peer.pathId === paths[side]!.pathId && peer.generation === paths[side]!.generation
            && peer.payloadDownload > tunnels.peers.find(previous => previous.infoHash === hash
                && previous.pathId === paths[side]!.pathId)!.payloadDownload)), 15000);
    await lab.checkpoint({ check: "active-mixed-to-tunnels", hash, retiredNative: paths[2],
        survivingPeers: tunnelFlow.peers, nativeCanaryConnectionsAfterDirectProbe: nativeCanaryHits - 1 });

    await lab.request("qbuttPaths/policy", { mode: "pinned" });
    const pinned = await waitFor("secondary socket retires after Tunnels to Pinned", status, current =>
        current.mode === "pinned" && !current.peers.some(peer => peer.infoHash === hash
            && peer.pathId === paths[1]!.pathId)
        && current.peers.some(peer => peer.infoHash === hash && peer.pathId === paths[0]!.pathId
            && peer.generation === paths[0]!.generation)
        && proxies[1]!.stats.activeConnections === 0, 15000);
    const retiredRelay = { ...proxies[1]!.stats };
    await Bun.sleep(2500);
    assert.equal(proxies[1]!.stats.acceptedConnections, retiredRelay.acceptedConnections,
        "Retired secondary route accepted a new connection");
    assert.equal(proxies[1]!.stats.uploadStreamBytes, retiredRelay.uploadStreamBytes,
        "Retired secondary route carried new outgoing bytes");
    assert.equal(proxies[1]!.stats.downloadStreamBytes, retiredRelay.downloadStreamBytes,
        "Retired secondary route carried new incoming bytes");
    assert.equal(nativeCanaryHits, 1, "Native route reopened after switching to Pinned");
    assert((await lab.info(hash)).progress < 1, "Torrent completed before the Pinned completion source");
    await lab.checkpoint({ check: "active-tunnels-to-pinned", hash, pinned: paths[0],
        retiredSecondary: paths[1], peers: pinned.peers, retiredRelay, observationMs: 2500 });

    await lab.request("torrents/addPeers", { hashes: hash,
        peers: `127.0.0.5:${seeds[3]!.port}` });
    await waitFor("completion seed uses the retained pinned route", status, current =>
        current.peers.some(peer => peer.infoHash === hash && peer.peer === "127.0.0.5"
            && peer.port === seeds[3]!.port && peer.pathId === paths[0]!.pathId
            && peer.generation === paths[0]!.generation && peer.payloadDownload > 0), 60000);
    await seeds[3]!.setUploadRate(256 * 1024);
    await waitFor("same active torrent completes on Pinned", () => lab.info(hash),
        info => info.progress === 1, 120000);
    await lab.request("torrents/stop", { hashes: hash });
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    assert.equal(nativeCanaryHits, 1, "Native canary received torrent traffic after retirement");
    assert.equal(proxies[1]!.stats.uploadStreamBytes, retiredRelay.uploadStreamBytes,
        "Retired secondary route carried outgoing bytes during completion");
    assert.equal(proxies[1]!.stats.downloadStreamBytes, retiredRelay.downloadStreamBytes,
        "Retired secondary route carried incoming bytes during completion");
    assert.equal(proxies[1]!.stats.acceptedConnections, retiredRelay.acceptedConnections,
        "Retired secondary route accepted a connection during completion");
    await lab.checkpoint({ check: "active-policy-transition-verified", hash, verifiedBytes,
        exactSizesAndHashes: true, nativeCanaryConnectionsAfterDirectProbe: nativeCanaryHits - 1,
        retiredSecondary: proxies[1]!.stats, pinnedRelay: proxies[0]!.stats,
        scope: "Controlled local TCP peers; no UDP/uTP, tracker, DHT, webseed, public egress or WAN claim" });
}
catch (error) { failure = error; }
finally {
    let appStopped = false;
    try { await lab.shutdown(); appStopped = true; } catch (error) { failure ??= error; }
    if (canary.listening)
        await new Promise<void>(resolve => canary.close(() => resolve()));
    const closed = await Promise.allSettled(proxies.map(proxy => proxy.close()));
    const stopped = await Promise.allSettled(seeds.map(seed => seed.stop()));
    for (const result of [...closed, ...stopped])
        if (result.status === "rejected") failure ??= result.reason;
    for (const [side, result] of stopped.entries()) if (result.status === "fulfilled") {
        await lab.checkpoint({ check: "policy-seed-shutdown", side, ...result.value });
        if (side === 2 && !result.value.peerAddresses.includes(nativeAddress))
            failure ??= new Error("Native seed never observed the selected physical source address");
    }
    if (appStopped && !canary.listening && [...closed, ...stopped].every(result => result.status === "fulfilled")) {
        for (const name of ["fixtures", "profile", "download", "partial-0", "partial-1", "partial-2", "nodes.json"]) {
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
