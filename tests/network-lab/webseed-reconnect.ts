import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { readFile, rm, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface Status {
    busy: boolean;
    paths: { pathId: string; generation: number; proxyName: string; open: boolean }[];
    peers: { infoHash: string; pathId: string; generation: number; payloadDownload: number }[];
}
const privateTorrent = process.argv.includes("--private");
const loopback = Object.entries(networkInterfaces()).find(([, addresses]) =>
    addresses?.some(address => address.internal && address.family === "IPv4"))?.[0];
assert(loopback, "A loopback interface is required");
const lab = await createLab("webseed-reconnect");
const payloads = new Map(await Promise.all(lab.manifest.payload.map(async file =>
    [file.path, await readFile(join(lab.fixtures, "seed", file.path))] as const)));
const servers: ReturnType<typeof Bun.serve>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
const requests: { side: number; first: number; last: number; path: string }[] = [];
const errors: string[] = [];
let nativeRequests = 0;
let failure: unknown;
const canary = Bun.serve({ hostname: "127.0.0.12", port: 0, fetch(request) {
    if (new URL(request.url).pathname === "/ready") return new Response("ready");
    nativeRequests++;
    return new Response("Native forbidden", { status: 403 });
} });
try {
    const url = `http://127.0.0.12:${canary.port}/`;
    assert.equal(await (await fetch(`${url}ready`, { signal: AbortSignal.timeout(2000) })).text(), "ready");
    for (const side of [0, 1]) servers.push(Bun.serve({ hostname: "127.0.0.1", port: 0, fetch(request) {
        try {
            assert(requests.length < 128, "Unbounded webseed retries");
            const path = decodeURIComponent(new URL(request.url).pathname).slice(1);
            const payload = payloads.get(path); assert(payload);
            const range = request.headers.get("range");
            const match = range ? /^bytes=(\d+)-(\d+)$/.exec(range) : null;
            assert(!range || match);
            const first = match ? Number(match[1]) : 0;
            const last = match ? Number(match[2]) : payload.length - 1;
            assert(first >= 0 && last < payload.length && (first <= last || payload.length === 0));
            requests.push({ side, first, last, path });
            const body = payload.subarray(first, last + 1);
            return new Response(body, { status: range ? 206 : 200, headers: {
                "content-length": String(last - first + 1), "accept-ranges": "bytes",
                ...(range ? { "content-range": `bytes ${first}-${last}/${payload.length}` } : {}),
            } });
        }
        catch (error) { errors.push(String(error)); return new Response("Invalid range", { status: 400 }); }
    } }));
    const credentials = { username: randomBytes(8).toString("hex"), password: randomBytes(16).toString("hex") };
    for (const side of [0, 1]) proxies.push(await startProxy({ ...credentials,
        listenAddress: `127.0.0.${side + 20}`, targets: [
            { host: "127.0.0.12", port: canary.port!, connectHost: "127.0.0.1", connectPort: servers[side]!.port! },
        ] }));
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
        name: `webseed-${side}`, type: "socks5", server: proxy.host, port: proxy.port, ...credentials,
    })) }));
    await lab.start();
    const status = () => lab.json<Status>("qbuttPaths/status");
    const openPath = (side: number) => lab.request("qbuttPaths/open", { configPath,
        proxyName: `webseed-${side}`, interfaceName: loopback });
    for (const side of [0, 1]) {
        await openPath(side);
        await waitFor("webseed path ready", status,
            current => !current.busy && current.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const destination = join(lab.root, "download");
    const hash = await lab.add(privateTorrent ? "v1" : "v1-public", destination);
    await lab.request("torrents/addWebSeeds", { hash, urls: url });
    await lab.request("torrents/setDownloadLimit", { hashes: hash, limit: "65536" });
    await lab.request("torrents/start", { hashes: hash });
    await waitFor("verified data before path retirement", () => lab.info(hash), info => info.completed >= 32768 && info.progress < 1);
    const before = await status();
    const peer = before.peers.find(peer => peer.infoHash === hash && peer.payloadDownload > 0); assert(peer);
    const retired = before.paths.find(path => path.pathId === peer.pathId); assert(retired?.open);
    const side = Number(retired.proxyName.at(-1));
    if (privateTorrent) assert.equal(side, 0, "Private webseed escaped its first path");
    await lab.request("qbuttPaths/stop", { pathId: retired.pathId });
    await waitFor("retired webseed socket closed", status, current => !current.busy
        && !current.paths.find(path => path.pathId === retired.pathId)?.open
        && !current.peers.some(peer => peer.infoHash === hash && peer.pathId === retired.pathId)
        && proxies[side]!.stats.activeConnections === 0);
    const requestBoundary = requests.length;
    if (privateTorrent) {
        // Let completed disk/hash jobs settle after the revoked socket closed.
        await Bun.sleep(1000);
        const completed = (await lab.info(hash)).completed;
        const deadline = Date.now() + 8000;
        do {
            assert.equal((await lab.info(hash)).completed, completed, "Private webseed kept downloading after revocation");
            assert.equal(requests.length, requestBoundary, "Private webseed migrated to another path");
            assert.equal(nativeRequests, 0);
            await Bun.sleep(200);
        } while (Date.now() < deadline);
        await lab.checkpoint({ check: "active-private-webseed-fails-closed", completed,
            observationMs: 8000, otherPathReady: (await status()).paths.some(path => path.open), nativeRequests });
        await openPath(side);
        const reopened = await waitFor("same private path reopened", status, current => !current.busy
            && current.paths.some(path => path.pathId === retired.pathId && path.open && path.generation > retired.generation));
        assert(reopened.paths.some(path => path.open && path.pathId !== retired.pathId));
    }
    await lab.request("torrents/setDownloadLimit", { hashes: hash, limit: "0" });
    await waitFor("webseed resumes without restarting torrent", async () => {
        assert.deepEqual(errors, []); assert.equal(nativeRequests, 0); return lab.info(hash);
    }, info => info.progress === 1, 90000);
    const resumed = requests.slice(requestBoundary);
    assert(resumed.length > 0 && resumed.every(request => request.side === (privateTorrent ? side : 1 - side)),
        "Webseed reconnected through an unexpected path");
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("completed torrent stopped", () => lab.info(hash), info => info.state === "stoppedUP");
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    await lab.checkpoint({ check: "webseed-mid-transfer-reconnect", privateTorrent, retired,
        verifiedBytes, exactSizesAndHashes: true, resumedRequests: resumed.length,
        resumedSide: resumed[0]!.side, nativeRequests, torrentRestarted: false });
}
catch (error) {
    failure = error;
    try { await lab.checkpoint({ check: "webseed-failure-log", log: await lab.json("log/main") }); }
    catch {}
}
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    for (const proxy of proxies) { try { await proxy.close(); } catch (error) { failure ??= error; } }
    for (const server of servers) server.stop(true);
    canary.stop(true);
    await lab.checkpoint({ check: "webseed-reconnect-observations", privateTorrent, requests, errors, nativeRequests });
    for (const name of ["fixtures", "profile", "download", "nodes.json"]) {
        const path = resolve(lab.root, name); assert.equal(dirname(path), resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
