import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
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
interface Peer { ip: string; port: number; downloaded: number; connection: string }
interface TcpSocket { LocalAddress: string; LocalPort: number; RemoteAddress: string; RemotePort: number; OwningProcess: number }

const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE;
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS;
assert(nativeInterface && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal), "Select the physical adapter's actual local IPv4 address");

const lab = await createLab("direct-transition");
const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let canaryHits = 0;
const canary = createServer(socket => { canaryHits++; socket.destroy(); });
let failure: unknown;

async function nativeSockets(port: number): Promise<TcpSocket[]> {
    const script = `ConvertTo-Json -Compress -InputObject @(Get-NetTCPConnection -State Established `
        + `-RemoteAddress '${nativeAddress}' -RemotePort ${port} -ErrorAction SilentlyContinue | `
        + `Where-Object { $_.LocalAddress -eq '${nativeAddress}' } | `
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
        { label: "direct", listenAddress: nativeAddress, uploadRate: 4 * 1024 }));
    seeds.push(await startSeed(lab.python, lab.fixtures, "v1-public", lab.root,
        { label: "tunnel", uploadRate: 4 * 1024 }));
    const credentials = { username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") };
    const tunneledPeer = { host: "127.0.0.2", port: seeds[1]!.port };
    proxy = await startProxy({ ...credentials, targets: [{ ...tunneledPeer, connectHost: seeds[1]!.host }] });
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: [{ name: "transition", type: "socks5",
        server: proxy.host, port: proxy.port, ...credentials, udp: false }] }));
    await new Promise<void>((resolve, reject) => {
        canary.once("error", reject);
        canary.listen(0, nativeAddress, resolve);
    });
    const address = canary.address();
    assert(address && typeof address !== "string");
    const canaryPort = address.port;
    await new Promise<void>((resolve, reject) => {
        const probe = createConnection({ host: nativeAddress, port: canaryPort });
        probe.setTimeout(5000, () => probe.destroy(new Error("Direct canary probe timed out")));
        probe.once("connect", () => { probe.destroy(); resolve(); });
        probe.once("error", reject);
    });
    await waitFor("positive Direct canary", async () => canaryHits, hits => hits === 1, 1000);

    await lab.start();
    await lab.request("app/setPreferences", { json: JSON.stringify({ current_interface_address: nativeAddress }) });
    const preferences = await lab.json<{ current_interface_address: string; bittorrent_protocol: number }>("app/preferences");
    assert(preferences.current_interface_address === nativeAddress && preferences.bittorrent_protocol === 1);
    const status = () => lab.json<Status>("qbuttPaths/status");
    const initial = await status();
    assert(!initial.pinned && initial.paths.length === 0 && initial.processId === 0,
        "Initial Direct state unexpectedly contains managed routes");
    const destination = join(lab.root, "download");
    const hash = await lab.add("v1-public", destination);
    const peers = async () => Object.values((await lab.json<{ peers: Record<string, Peer> }>(
        `sync/torrentPeers?hash=${hash}&rid=0`)).peers);
    await lab.request("torrents/start", { hashes: hash });
    await lab.request("torrents/addPeers", { hashes: hash, peers: `${nativeAddress}:${seeds[0]!.port}` });
    const direct = await waitFor("Direct native payload", peers, peers => peers.some(peer =>
        peer.ip === nativeAddress && peer.port === seeds[0]!.port && peer.downloaded > 32768), 60000);
    assert(direct.every(peer => peer.connection === "BT"), "Direct fixture used a non-TCP peer");
    const oldSockets = await nativeSockets(seeds[0]!.port);
    assert(oldSockets.length > 0, "Direct payload has no owned physical TCP socket");
    await lab.checkpoint({ check: "active-direct-before-transition", hash, peers: direct, sockets: oldSockets });
    assert((await lab.info(hash)).progress < 1, "Torrent completed before Direct to Tunnels");

    // Select fail-closed Tunnels before opening the remote path; no temporary
    // Mixed mode and no stop/start cycle are used to retire the Direct socket.
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    await waitFor("old Direct socket closed", () => nativeSockets(seeds[0]!.port), sockets => sockets.length === 0, 15000);
    await lab.request("qbuttPaths/open", { configPath, proxyName: "transition", interfaceName: "Loopback Pseudo-Interface 1" });
    const opened = await waitFor("Tunnels path ready", status,
        current => !current.busy && current.pinned && current.mode === "tunnels" && current.paths.some(path => path.open));
    assert(opened.paths.length === 1 && opened.paths[0]!.edgeId !== "native");
    const path = opened.paths[0]!;
    await lab.request("torrents/addPeers", { hashes: hash, peers: `${tunneledPeer.host}:${tunneledPeer.port}` });
    const flowing = await waitFor("same torrent tunnel payload", status, current => current.peers.some(peer =>
        peer.infoHash === hash && peer.pathId === path.pathId && peer.generation === path.generation
        && peer.peer === tunneledPeer.host && peer.port === tunneledPeer.port && peer.payloadDownload > 32768), 60000);
    const tunnelBytes = flowing.peers.find(peer => peer.infoHash === hash && peer.pathId === path.pathId)!.payloadDownload;
    await lab.request("torrents/addPeers", { hashes: hash, peers: `${nativeAddress}:${canaryPort}` });
    const canarySince = Date.now();
    const stillFlowing = await waitFor("tunnel advances while Direct canary is blocked", status, current =>
        Date.now() - canarySince >= 3000 && current.peers.some(peer => peer.infoHash === hash
            && peer.pathId === path.pathId && peer.payloadDownload >= tunnelBytes + 16384), 20000);
    assert.equal(canaryHits, 1, "Tunnels only reached the positively checked Direct canary");
    assert.equal((await nativeSockets(seeds[0]!.port)).length, 0, "Direct socket reopened during tunnel transfer");
    assert(!(await lab.info(hash)).state.startsWith("stopped"), "Policy change stopped the active torrent");
    await lab.checkpoint({ check: "active-direct-to-tunnels", hash, path, peers: stillFlowing.peers,
        nativeCanaryConnectionsAfterProbe: canaryHits - 1, observationMs: Date.now() - canarySince, relay: { ...proxy.stats } });
    assert((await lab.info(hash)).progress < 1, "Torrent completed before returning to Direct");

    await lab.request("qbuttPaths/native", {});
    await waitFor("managed path and sockets retire", status, current => !current.busy && !current.pinned
        && current.paths.length === 0 && current.peers.length === 0 && current.processId === 0
        && proxy!.stats.activeConnections === 0, 15000);
    const retiredRelay = { ...proxy.stats };
    assert(!(await lab.info(hash)).state.startsWith("stopped"), "Restoring Direct stopped the active torrent");
    await lab.request("torrents/addPeers", { hashes: hash, peers: `${nativeAddress}:${seeds[0]!.port}` });
    const resumed = await waitFor("same torrent resumes Native in Direct", peers, peers => peers.some(peer =>
        peer.ip === nativeAddress && peer.port === seeds[0]!.port && peer.downloaded > 32768), 60000);
    const newSockets = await nativeSockets(seeds[0]!.port);
    assert(newSockets.length > 0 && newSockets.every(socket => oldSockets.some(old => old.OwningProcess === socket.OwningProcess)),
        "Native did not resume in the same application process");
    await seeds[0]!.setUploadRate(256 * 1024);
    await waitFor("same torrent completes in Direct", () => lab.info(hash), info => info.progress === 1, 90000);
    await lab.request("torrents/stop", { hashes: hash });
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    assert.equal(proxy.stats.activeConnections, 0, "Managed relay reopened after restoring Direct");
    for (const field of ["acceptedConnections", "uploadStreamBytes", "downloadStreamBytes"] as const)
        assert.equal(proxy.stats[field], retiredRelay[field], `Retired relay ${field} changed during Native completion`);
    assert.equal((await lab.json<unknown[]>("torrents/info")).length, 1, "Transition replaced the original torrent");
    await lab.checkpoint({ check: "active-tunnels-to-direct-verified", hash, verifiedBytes, exactSizesAndHashes: true,
        peers: resumed, sockets: newSockets, retiredRelay, stopStartCyclesDuringTransitions: 0,
        scope: "Controlled local TCP; Direct physical source binding, no WAN, UDP/uTP, discovery or physical TUN bypass claim" });
}
catch (error) { failure = error; }
finally {
    let appStopped = false;
    try { await lab.shutdown(); appStopped = true; } catch (error) { failure ??= error; }
    if (canary.listening) await new Promise<void>(resolve => canary.close(() => resolve()));
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
