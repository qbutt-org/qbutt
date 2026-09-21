import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { rm, writeFile } from "node:fs/promises";
import { createServer, type Socket } from "node:net";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface Path {
    pathId: string; generation: number; edgeId: string; proxyName: string; open: boolean;
    reserveNames: string[]; transport: { state: string; recommended: string };
}
interface Status {
    busy: boolean; processId: number; paths: Path[];
    peers: { pathId: string; generation: number; infoHash: string; payloadDownload: number;
        peer: string; port: number; localPort: number }[];
}

const lab = await createLab("transport-reserves");
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let publicSeed: Awaited<ReturnType<typeof startSeed>> | undefined;
const dnsSockets = new Set<Socket>();
let dnsQueries = 0;
const dns = createServer(socket => {
    dnsSockets.add(socket);
    socket.once("close", () => dnsSockets.delete(socket));
    socket.on("error", () => socket.destroy());
    socket.setTimeout(5000, () => socket.destroy());
    let input = Buffer.alloc(0);
    socket.on("data", chunk => {
        input = Buffer.concat([input, chunk]);
        if (input.length > 4096) return socket.destroy();
        if (input.length < 2 || input.length < input.readUInt16BE(0) + 2) return;
        const response = Buffer.from(input.subarray(0, input.readUInt16BE(0) + 2));
        assert(response.length >= 19 && response.readUInt16BE(6) === 1);
        response.writeUInt16BE(0x8180, 4);
        dnsQueries++;
        socket.end(response);
    });
});
let failure: unknown;
try {
    await new Promise<void>((resolve, reject) => {
        dns.once("error", reject);
        dns.listen(0, "127.0.0.1", resolve);
    });
    const dnsPort = (dns.address() as { port: number }).port;
    seed = await startSeed(lab.python, lab.fixtures, "v1", lab.root, { uploadRate: 16 * 1024 });
    publicSeed = await startSeed(lab.python, lab.fixtures, "v1-public", lab.root,
        { label: "independent-public", uploadRate: 1024 });
    const credentials = { username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") };
    for (const [index, listenAddress] of ["127.0.0.20", "127.0.0.20", "127.0.0.21"].entries()) {
        const source = index === 2 ? publicSeed : seed;
        proxies.push(await startProxy({ ...credentials, listenAddress, targets: [
            ...(index === 2 ? ["127.0.0.4", "127.0.0.5"] : ["127.0.0.2", "127.0.0.3"]).map(host => ({ host, port: source!.port,
                connectHost: source!.host, connectPort: source!.port })),
            { host: "127.0.0.1", port: dnsPort },
        ] }));
    }
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, index) => ({
        name: ["primary", "reserve", "independent"][index], type: "socks5", server: proxy.host,
        port: proxy.port, ...credentials, udp: false,
    })) }));
    await lab.start();
    const status = () => lab.json<Status>("qbuttPaths/status");
    await lab.request("qbuttPaths/dns", { server: `127.0.0.1:${dnsPort}`,
        bootstrapServer: `127.0.0.1:${dnsPort}`, family: "ipv4" });
    await lab.request("qbuttPaths/open", { configPath, proxyName: "primary", reserveNames: JSON.stringify(["reserve"]),
        interfaceName: "Loopback Pseudo-Interface 1" });
    const initial = await waitFor("primary with explicit reserve", status, value => !value.busy && value.paths.some(path => path.open));
    const primary = initial.paths[0]!;
    assert.deepEqual(primary.reserveNames, ["reserve"]);
    await lab.request("qbuttPaths/open", { configPath, proxyName: "independent", interfaceName: "Loopback Pseudo-Interface 1" });
    const both = await waitFor("independent healthy edge", status, value => !value.busy && value.paths.filter(path => path.open).length === 2);
    const independent = both.paths.find(path => path.proxyName === "independent")!;
    assert.notEqual(independent.edgeId, primary.edgeId);
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const publicHash = await lab.add("v1-public", join(lab.root, "public-download"));
    await lab.request("torrents/start", { hashes: publicHash });
    await lab.request("torrents/addPeers", { hashes: publicHash,
        peers: `127.0.0.4:${publicSeed.port}|127.0.0.5:${publicSeed.port}` });
    const publicConnected = await waitFor("healthy public peer on independent edge", status, value => value.peers.some(peer =>
        peer.infoHash === publicHash && peer.pathId === independent.pathId && peer.payloadDownload > 0), 60000);
    const heldPeer = publicConnected.peers.find(peer => peer.infoHash === publicHash
        && peer.pathId === independent.pathId && peer.payloadDownload > 0)!;
    let lastPublicPayload = heldPeer.payloadDownload;
    const destination = join(lab.root, "download");
    const hash = await lab.add("v1", destination);
    const seenGenerations = new Set<number>();
    const privateStatus = async () => {
        const value = await status();
        const ongoing = value.peers.find(peer => peer.infoHash === publicHash && peer.pathId === independent.pathId
            && peer.generation === heldPeer.generation && peer.peer === heldPeer.peer
            && peer.port === heldPeer.port && peer.localPort === heldPeer.localPort);
        assert(ongoing, "Reserve replacement retired the healthy independent app peer");
        assert(ongoing.payloadDownload >= lastPublicPayload);
        lastPublicPayload = ongoing.payloadDownload;
        for (const peer of value.peers.filter(peer => peer.infoHash === hash)) {
            assert.equal(peer.pathId, primary.pathId, "Private torrent escaped its originally pinned edge");
            if (peer.payloadDownload > 0) seenGenerations.add(peer.generation);
        }
        return value;
    };
    await lab.request("torrents/start", { hashes: hash });
    await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.2:${seed.port}` });
    await waitFor("private torrent receives primary payload", privateStatus,
        value => value.peers.some(peer => peer.infoHash === hash && peer.payloadDownload >= 16384), 60000);
    assert((await lab.info(hash)).progress < 1, "Fixture completed before transport failure");
    await proxies[0]!.close();
    // A new endpoint for the same seed produces a fresh dial on the failed
    // primary, without a peer-retry backoff deciding the fixture duration.
    await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.3:${seed.port}` });
    const recovered = await waitFor("automatic reserve generation", privateStatus, value => !value.busy
        && value.paths.some(path => path.pathId === primary.pathId && path.open && path.proxyName === "reserve"), 60000);
    const replacement = recovered.paths.find(path => path.pathId === primary.pathId)!;
    assert.equal(recovered.processId, both.processId, "Reserve switch restarted the entire child");
    assert.equal(replacement.edgeId, primary.edgeId);
    assert(replacement.generation > primary.generation);
    assert.deepEqual(replacement.reserveNames, ["primary"]);
    assert(recovered.paths.some(path => path.pathId === independent.pathId && path.open && path.generation === independent.generation));
    assert(!recovered.peers.some(peer => peer.pathId === primary.pathId && peer.generation === primary.generation));
    await seed.setUploadRate(512 * 1024);
    await waitFor("private torrent receives reserve payload", privateStatus,
        value => value.peers.some(peer => peer.infoHash === hash && peer.generation === replacement.generation && peer.payloadDownload > 16384), 60000);
    await waitFor("one torrent complete after reserve recovery", async () => {
        await privateStatus();
        return await lab.info(hash);
    }, value => value.progress === 1, 90000);
    const torrents = await lab.json<{ hash: string }[]>("torrents/info");
    assert.equal(torrents.filter(torrent => torrent.hash === hash).length, 1);
    assert.equal(torrents.length, 2, "Recovery added an extra torrent/session job");
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("completed torrent stopped", () => lab.info(hash), value => value.state === "stoppedUP");
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    assert(seenGenerations.has(primary.generation) && seenGenerations.has(replacement.generation));
    assert(proxies[0]!.stats.downloadStreamBytes > 16384 && proxies[1]!.stats.downloadStreamBytes > 16384);
    assert(lastPublicPayload > heldPeer.payloadDownload, "The retained independent peer stopped making payload progress");
    assert(dnsQueries > 0, "No comparative configured-resolver response was observed");
    await lab.checkpoint({ check: "explicit-same-edge-reserve-private-torrent", automaticRecovery: true,
        faultStimulus: "primary listener closed, fresh alias of the same seed added before replacement",
        peersAddedAfterReplacement: false, naturalTorrentResumption: true,
        pathId: primary.pathId, oldGeneration: primary.generation, newGeneration: replacement.generation,
        independentGenerationUnchanged: true, independentPeerLocalPortUnchanged: true,
        independentPeerPayloadIncrease: lastPublicPayload - heldPeer.payloadDownload,
        onePrivateTorrentPreserved: true, privateScopePreserved: true,
        verifiedBytes, exactSizesAndHashes: true, configuredResolverResponses: dnsQueries,
        scope: "Real app and qbutt-net, generated legal private torrent and loopback transport fault; no WAN claim" });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    const closed = await Promise.allSettled(proxies.map(proxy => proxy.close()));
    for (const result of closed) if (result.status === "rejected") failure ??= result.reason;
    try { if (seed) await seed.stop(); } catch (error) { failure ??= error; }
    try { if (publicSeed) await publicSeed.stop(); } catch (error) { failure ??= error; }
    for (const socket of dnsSockets) socket.destroy();
    await new Promise<void>(resolve => dns.close(() => resolve()));
    for (const name of ["fixtures", "profile", "download", "public-download", "nodes.json"]) {
        const path = resolve(lab.root, name);
        assert.equal(dirname(path), resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
