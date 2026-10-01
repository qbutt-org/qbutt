import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { rm, writeFile } from "node:fs/promises";
import { createConnection, createServer } from "node:net";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface Status {
    busy: boolean;
    pinned: boolean;
    mode: string;
    processId: number;
    paths: { pathId: string; generation: number; edgeId: string; open: boolean }[];
    peers: { infoHash: string; pathId: string; generation: number; peer: string; port: number;
        payloadDownload: number }[];
}
interface Peer { ip: string; port: number; downloaded: number; client: string; progress: number;
    connection: string; flags: string }
interface TcpSocket { LocalAddress: string; LocalPort: number; RemoteAddress: string; RemotePort: number; OwningProcess: number }

const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE;
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS;
assert(nativeInterface && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal), "Select the physical adapter's actual local IPv4 address");

const useUtp = process.argv.includes("--utp");
const lab = await createLab(`direct-transition${useUtp ? "-utp" : ""}`, { protocol: useUtp ? "UTP" : "TCP" });
const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let canaryHits = 0;
const canary = createServer(socket => { canaryHits++; socket.destroy(); });
const udpCanary = createSocket("udp4");
udpCanary.on("message", () => { canaryHits++; });
let udpCanaryBound = false;
let failure: unknown;

async function nativeSockets(port: number): Promise<TcpSocket[]> {
    const owner = lab.pid;
    assert(owner, "Native socket inspection needs the running app process");
    const script = `ConvertTo-Json -Compress -InputObject @(Get-NetTCPConnection -State Established `
        + `-RemoteAddress '${nativeAddress}' -RemotePort ${port} -ErrorAction SilentlyContinue | `
        + `Where-Object { $_.LocalAddress -eq '${nativeAddress}' -and $_.OwningProcess -eq ${owner} } | `
        + "Select-Object LocalAddress,LocalPort,RemoteAddress,RemotePort,OwningProcess)";
    const child = Bun.spawn(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script], {
        stdout: "pipe", stderr: "pipe", timeout: 10000, windowsHide: true });
    const [exit, output, error] = await Promise.all([child.exited,
        new Response(child.stdout).text(), new Response(child.stderr).text()]);
    assert.equal(exit, 0, `Owned TCP socket inspection failed: ${error}`);
    const sockets = JSON.parse(output);
    assert(Array.isArray(sockets), "TCP inspection did not return an array");
    return sockets;
}

try {
    seeds.push(await startSeed(lab.python, lab.fixtures, "v1-public", lab.root,
        { label: "direct", listenAddress: nativeAddress, uploadRate: 4 * 1024,
            transport: useUtp ? "utp" : "tcp" }));
    seeds.push(await startSeed(lab.python, lab.fixtures, "v1-public", lab.root,
        { label: "tunnel", uploadRate: 4 * 1024, transport: useUtp ? "utp" : "tcp" }));
    const credentials = { username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") };
    const tunneledPeer = { host: "127.0.0.2", port: seeds[1]!.port };
    proxy = await startProxy({ ...credentials, udp: useUtp, remoteAddresses: [nativeAddress], targets: [
        { ...tunneledPeer, connectHost: seeds[1]!.host },
        { host: nativeAddress, port: seeds[0]!.port },
    ] });
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: [{ name: "transition", type: "socks5",
        server: proxy.host, port: proxy.port, ...credentials, udp: useUtp }] }));
    let canaryPort: number;
    if (useUtp) {
        await new Promise<void>((resolve, reject) => {
            udpCanary.once("error", reject);
            udpCanary.bind(0, nativeAddress, resolve);
        });
        udpCanaryBound = true;
        canaryPort = udpCanary.address().port;
        const probe = createSocket("udp4");
        try {
            await new Promise<void>((resolve, reject) => {
                probe.once("error", reject);
                probe.bind(0, nativeAddress, resolve);
            });
            await new Promise<void>((resolve, reject) => probe.send(Buffer.from("qbutt-direct-canary"),
                canaryPort, nativeAddress, error => error ? reject(error) : resolve()));
        }
        finally { await new Promise<void>(resolve => probe.close(resolve)); }
    }
    else {
        await new Promise<void>((resolve, reject) => {
            canary.once("error", reject);
            canary.listen(0, nativeAddress, resolve);
        });
        const address = canary.address();
        assert(address && typeof address !== "string");
        canaryPort = address.port;
        await new Promise<void>((resolve, reject) => {
            const probe = createConnection({ host: nativeAddress, port: canaryPort });
            probe.setTimeout(5000, () => probe.destroy(new Error("Direct canary probe timed out")));
            probe.once("connect", () => { probe.destroy(); resolve(); });
            probe.once("error", reject);
        });
    }
    await waitFor("positive Direct canary", async () => canaryHits, hits => hits === 1, 1000);

    await lab.start();
    await lab.request("app/setPreferences", { json: JSON.stringify({ current_interface_address: nativeAddress,
        enable_multi_connections_from_same_ip: true }) });
    const preferences = await lab.json<{ current_interface_address: string; bittorrent_protocol: number;
        enable_multi_connections_from_same_ip: boolean }>("app/preferences");
    assert(preferences.current_interface_address === nativeAddress
        && preferences.bittorrent_protocol === (useUtp ? 2 : 1)
        && preferences.enable_multi_connections_from_same_ip);
    const status = () => lab.json<Status>("qbuttPaths/status");
    const initial = await status();
    assert(!initial.pinned && initial.paths.length === 0 && initial.processId === 0,
        "Initial Direct state unexpectedly contains managed routes");
    const destination = join(lab.root, "download");
    const hash = await lab.add("v1-public", destination);
    const peers = async () => Object.values((await lab.json<{ peers: Record<string, Peer> }>(
        `sync/torrentPeers?hash=${hash}&rid=0`)).peers);
    const verifiedPieces = async () => (await lab.json<number[]>(`torrents/pieceStates?hash=${hash}`))
        .filter(state => state === 2).length;
    await lab.request("torrents/start", { hashes: hash });
    await lab.request("torrents/addPeers", { hashes: hash, peers: `${nativeAddress}:${seeds[0]!.port}` });
    const direct = await waitFor("Direct native payload", peers, peers => peers.some(peer =>
        peer.ip === nativeAddress && peer.port === seeds[0]!.port && peer.downloaded > 32768), 60000);
    assert(direct.every(peer => peer.connection === (useUtp ? "μTP" : "BT")
        && peer.flags.includes("P") === useUtp),
        "Direct fixture used the wrong peer transport");
    const oldSockets = useUtp ? [] : await nativeSockets(seeds[0]!.port);
    if (!useUtp)
        assert(oldSockets.length > 0, "Direct payload has no owned physical TCP socket");
    await lab.checkpoint({ check: "active-direct-before-transition", hash, peers: direct, sockets: oldSockets });
    assert((await lab.info(hash)).progress < 1, "Torrent completed before Direct to Tunnels");

    if (useUtp) {
        const beforeMixed = await verifiedPieces();
        await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface });
        const mixed = await waitFor("Mixed Native route ready", status, current => current.mode === "mixed"
            && current.paths.some(path => path.edgeId === "native" && path.open), 15000);
        const nativePath = mixed.paths.find(path => path.edgeId === "native")!;
        const recovered = await waitFor("same peer resumes and verifies over managed Native", async () =>
            ({ status: await status(), verifiedPieces: await verifiedPieces() }), current =>
            current.verifiedPieces > beforeMixed && current.status.peers.some(peer =>
                peer.infoHash === hash && peer.peer === nativeAddress
                && peer.port === seeds[0]!.port && peer.pathId === nativePath.pathId
                && peer.generation === nativePath.generation && peer.payloadDownload > 16384), 20000);
        await lab.checkpoint({ check: "active-direct-to-mixed-native-utp", hash, route: nativePath,
            peers: recovered.status.peers, verifiedPiecesBefore: beforeMixed,
            verifiedPiecesAfter: recovered.verifiedPieces });
        assert((await lab.info(hash)).progress < 1, "Torrent completed before Mixed to Tunnels");
    }

    // Select fail-closed Tunnels before opening the remote path. The UTP
    // variant first proves the same warm peer survives ordinary Native to Mixed.
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    if (!useUtp)
        await waitFor("old Direct socket closed", () => nativeSockets(seeds[0]!.port), sockets => sockets.length === 0, 15000);
    else
        await waitFor("unready Tunnels blocks Native peer", status, current => current.mode === "tunnels"
            && !current.paths.some(path => path.edgeId === "native")
            && !current.peers.some(peer => peer.infoHash === hash && peer.peer === nativeAddress), 15000);
    if (useUtp) {
        await Bun.sleep(2500);
        const blocked = await status();
        assert(blocked.paths.length === 0 && !blocked.peers.some(peer => peer.infoHash === hash),
            "Unready Tunnels resumed a peer before a relay was available");
    }
    await lab.request("qbuttPaths/open", { configPath, proxyName: "transition", interfaceName: "Loopback Pseudo-Interface 1" });
    const opened = await waitFor("Tunnels path ready", status,
        current => !current.busy && current.pinned && current.mode === "tunnels" && current.paths.some(path => path.open));
    assert(opened.paths.length === 1 && opened.paths[0]!.edgeId !== "native");
    const path = opened.paths[0]!;
    await lab.request("torrents/addPeers", { hashes: hash, peers: `${tunneledPeer.host}:${tunneledPeer.port}` });
    const flowing = await waitFor("same torrent tunnel payload", status, current => current.peers.some(peer =>
        peer.infoHash === hash && peer.pathId === path.pathId && peer.generation === path.generation
        && peer.peer === tunneledPeer.host && peer.port === tunneledPeer.port && peer.payloadDownload > 32768), 60000);
    const relayedNative = await waitFor("retired Native peer reconnects through ready relay", status, current =>
        current.peers.some(peer => peer.infoHash === hash && peer.pathId === path.pathId
            && peer.peer === nativeAddress && peer.port === seeds[0]!.port && peer.payloadDownload > 16384), 20000);
    assert(proxy.stats[useUtp ? "downloadDatagramBytes" : "downloadStreamBytes"] > 0,
        "Ready relay did not carry the selected peer transport");
    const tunnelBytes = flowing.peers.find(peer => peer.infoHash === hash && peer.pathId === path.pathId
        && peer.peer === tunneledPeer.host && peer.port === tunneledPeer.port)!.payloadDownload;
    await lab.request("torrents/addPeers", { hashes: hash, peers: `${nativeAddress}:${canaryPort}` });
    const canarySince = Date.now();
    const stillFlowing = await waitFor("tunnel advances while Direct canary is blocked", status, current =>
        Date.now() - canarySince >= 3000 && current.peers.some(peer => peer.infoHash === hash
            && peer.pathId === path.pathId && peer.peer === tunneledPeer.host
            && peer.port === tunneledPeer.port && peer.payloadDownload >= tunnelBytes + 16384), 20000);
    assert.equal(canaryHits, 1, "Tunnels only reached the positively checked Direct canary");
    if (!useUtp)
        assert.equal((await nativeSockets(seeds[0]!.port)).length, 0, "Direct socket reopened during tunnel transfer");
    assert(!(await lab.info(hash)).state.startsWith("stopped"), "Policy change stopped the active torrent");
    await lab.checkpoint({ check: "active-direct-to-tunnels", hash, path, peers: stillFlowing.peers,
        relayedNativePeer: relayedNative.peers.find(peer => peer.infoHash === hash && peer.peer === nativeAddress),
        nativeCanaryConnectionsAfterProbe: canaryHits - 1, observationMs: Date.now() - canarySince, relay: { ...proxy.stats } });
    assert((await lab.info(hash)).progress < 1, "Torrent completed before returning to Direct");

    const returnedAt = Date.now();
    const returnDeadline = returnedAt + 20000;
    const returnTimeLeft = () => {
        const remaining = returnDeadline - Date.now();
        assert(remaining > 0, "Native reconnect exceeded the 20-second deadline");
        return remaining;
    };
    await lab.request("qbuttPaths/native", {});
    await waitFor("managed path and sockets retire", status, current => !current.busy && !current.pinned
        && current.paths.length === 0 && current.peers.length === 0 && current.processId === 0
        && proxy!.stats.activeConnections === 0, returnTimeLeft());
    const retiredRelay = { ...proxy.stats };
    assert(!(await lab.info(hash)).state.startsWith("stopped"), "Restoring Direct stopped the active torrent");
    let lastVerified = -1;
    let stableSamples = 0;
    const beforeReturn = await waitFor("queued relay pieces settle after Native return", async () => {
        await Bun.sleep(250);
        return verifiedPieces();
    }, count => {
        stableSamples = (count === lastVerified) ? stableSamples + 1 : 1;
        lastVerified = count;
        return stableSamples >= 3;
    }, returnTimeLeft());
    const nativePeer = (peer: Peer) => peer.ip === nativeAddress && peer.port === seeds[0]!.port
        && peer.connection === (useUtp ? "μTP" : "BT") && peer.flags.includes("P") === useUtp
        && peer.client.startsWith("libtorrent/") && peer.progress === 1;
    const firstNative = await waitFor("Native seed completes a new peer handshake", peers,
        current => current.some(nativePeer), returnTimeLeft());
    const downloadedBefore = firstNative.find(nativePeer)!.downloaded;
    const resumed = await waitFor("same Native peer reconnects and verifies without addPeers", async () =>
        ({ peers: await peers(), verifiedPieces: await verifiedPieces() }), current =>
        current.verifiedPieces > beforeReturn && current.peers.some(peer =>
            nativePeer(peer) && peer.downloaded >= downloadedBefore + 16384), returnTimeLeft());
    const recoveryMs = Date.now() - returnedAt;
    assert(recoveryMs <= 20000, `Native reconnect and verification took ${recoveryMs} ms`);
    const newSockets = useUtp ? [] : await nativeSockets(seeds[0]!.port);
    if (!useUtp)
        assert(newSockets.length > 0 && newSockets.every(socket => oldSockets.some(old =>
            old.OwningProcess === socket.OwningProcess)),
            "Native did not resume a physical TCP socket in the same application process");
    await seeds[0]!.setUploadRate(256 * 1024);
    await waitFor("same torrent completes in Direct", () => lab.info(hash), info => info.progress === 1, 90000);
    await lab.request("torrents/stop", { hashes: hash });
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    assert.equal(proxy.stats.activeConnections, 0, "Managed relay reopened after restoring Direct");
    const retiredFields = useUtp
        ? ["acceptedConnections", "uploadDatagramBytes", "downloadDatagramBytes"] as const
        : ["acceptedConnections", "uploadStreamBytes", "downloadStreamBytes"] as const;
    for (const field of retiredFields)
        assert.equal(proxy.stats[field], retiredRelay[field], `Retired relay ${field} changed during Native completion`);
    assert.equal((await lab.json<unknown[]>("torrents/info")).length, 1, "Transition replaced the original torrent");
    await lab.checkpoint({ check: "active-tunnels-to-direct-verified", hash, verifiedBytes, exactSizesAndHashes: true,
        peers: resumed.peers, sockets: newSockets, retiredRelay, stopStartCyclesDuringTransitions: 0,
        verifiedPiecesBefore: beforeReturn, verifiedPiecesAfter: resumed.verifiedPieces,
        nativePeerDownloadedBefore: downloadedBefore,
        nativePeerDownloadedAfter: resumed.peers.find(nativePeer)!.downloaded, recoveryMs,
        scope: `Controlled local ${useUtp ? "UTP" : "TCP"}; no WAN, discovery or physical TUN bypass claim` });
}
catch (error) { failure = error; }
finally {
    let appStopped = false;
    try { await lab.shutdown(); appStopped = true; } catch (error) { failure ??= error; }
    if (canary.listening) await new Promise<void>(resolve => canary.close(() => resolve()));
    if (udpCanaryBound) await new Promise<void>(resolve => udpCanary.close(resolve));
    const results = await Promise.allSettled([...(proxy ? [proxy.close()] : []), ...seeds.map(seed => seed.stop())]);
    for (const result of results) if (result.status === "rejected") failure ??= result.reason;
    if (appStopped && results.every(result => result.status === "fulfilled")) {
        for (const name of ["fixtures", "download", "nodes.json", "profile"]) {
            if (name === "profile" && failure) continue;
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
