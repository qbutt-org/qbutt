import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { readFile, rm, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface Status {
    busy: boolean;
    mode: string;
    paths: { pathId: string; generation: number; edgeId: string; proxyName: string; open: boolean }[];
    peers: { infoHash: string }[];
}
const lab = await createLab("webseed");
const loopback = Object.entries(networkInterfaces()).find(([, addresses]) =>
    addresses?.some(address => address.internal && address.family === "IPv4"))?.[0];
assert(loopback, "A loopback interface is required");
const files = new Map(await Promise.all(lab.manifest.payload.map(async item =>
    [item.path, await readFile(join(lab.fixtures, "seed", item.path))] as const)));
const totalBytes = lab.manifest.payload.reduce((sum, item) => sum + item.size, 0);
const requests: { side: number; path: string; first: number; last: number; bytes: number }[] = [];
const errors: string[] = [];
const servers: ReturnType<typeof Bun.serve>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
let nativeRequests = 0, responseBytes = 0;
let failure: unknown;
// The URL's actual numeric destination is reachable. Managed paths redirect it
// to their own responders; any accidental direct connection reaches this canary.
const canary = Bun.serve({ hostname: "127.0.0.12", port: 0, fetch(request) {
    if (new URL(request.url).pathname === "/ready") return new Response("ready");
    nativeRequests++;
    errors.push("Webseed bypassed the managed paths and reached the Native canary");
    return new Response("Native bypass forbidden", { status: 403 });
} });
try {
    const url = `http://127.0.0.12:${canary.port}/`;
    assert.equal(await (await fetch(`${url}ready`, { signal: AbortSignal.timeout(5000) })).text(), "ready");
    for (const side of [0, 1]) servers.push(Bun.serve({ hostname: "127.0.0.1", port: 0, fetch(request) {
        try {
            assert(requests.length < 128 && responseBytes <= totalBytes * 4, "Webseed fixture request limit");
            const path = decodeURIComponent(new URL(request.url).pathname).slice(1);
            const payload = files.get(path); assert(payload, `Unexpected webseed path: ${path}`);
            const range = request.headers.get("range");
            const match = range ? /^bytes=(\d+)-(\d+)$/.exec(range) : null;
            assert(!range || match, "Unsupported webseed byte range");
            const first = match ? Number(match[1]) : 0;
            const last = match ? Number(match[2]) : payload.length - 1;
            assert(Number.isSafeInteger(first) && Number.isSafeInteger(last) && first >= 0
                && last < payload.length && (first <= last || payload.length === 0), "Range outside fixture file");
            const body = payload.subarray(first, last + 1);
            requests.push({ side, path, first, last, bytes: body.length });
            responseBytes += body.length;
            return new Response(body, { status: range ? 206 : 200, headers: {
                "accept-ranges": "bytes", "content-length": String(body.length),
                ...(range ? { "content-range": `bytes ${first}-${last}/${payload.length}` } : {}),
            } });
        }
        catch (error) { errors.push(String(error)); return new Response("Invalid controlled request", { status: 400 }); }
    } }));
    const credentials = [0, 1].map(() => ({ username: randomBytes(8).toString("hex"), password: randomBytes(16).toString("hex") }));
    for (const side of [0, 1]) proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`, targets: [
        { host: "127.0.0.12", port: canary.port!, connectHost: "127.0.0.1", connectPort: servers[side]!.port! },
    ] }));
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({ name: `webseed-${side}`,
        type: "socks5", server: proxy.host, port: proxy.port, ...credentials[side] })) }));
    await lab.start();
    const status = () => lab.json<Status>("qbuttPaths/status");
    for (const side of [0, 1]) {
        await lab.request("qbuttPaths/open", { configPath, proxyName: `webseed-${side}`, interfaceName: loopback });
        await waitFor("webseed path ready", status, current => !current.busy && current.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const pinned = (await status()).paths[0]!;
    assert(pinned.open && pinned.proxyName === "webseed-0");
    async function download(name: string, destination: string) {
        const hash = await lab.add(name, destination);
        await lab.request("torrents/addWebSeeds", { hash, urls: url });
        await lab.request("torrents/start", { hashes: hash });
        await waitFor("HTTP webseed download complete", async () => {
            assert.deepEqual(errors, []); return lab.info(hash);
        }, info => info.progress === 1);
        await lab.request("torrents/stop", { hashes: hash });
        await waitFor("webseed torrent stopped", () => lab.info(hash), info => info.state === "stoppedUP");
        return verifyPayload(destination, lab.manifest.payload);
    }
    const verifiedBytes = await download("v1", join(lab.root, "downloads"));
    assert(requests.length > 0 && requests.every(request => request.side === 0));
    assert(proxies[0]!.stats.authenticatedConnections > 0 && proxies[0]!.stats.downloadStreamBytes >= verifiedBytes);
    assert.equal(proxies[1]!.stats.downloadStreamBytes, 0);
    assert.equal(nativeRequests, 0);
    await lab.checkpoint({ check: "private-http-webseed", pinned, verifiedBytes, exactSizes: true, hashesVerified: true,
        requests: [...requests], routes: await status(), proxies: proxies.map(proxy => ({ ...proxy.stats })) });

    await lab.request("qbuttPaths/stop", { pathId: pinned.pathId });
    const retired = await waitFor("first path retired while second remains ready", status, current => !current.busy
        && current.paths[0]?.open === false && current.paths[1]?.open === true);
    assert(retired.paths[0]!.pathId === pinned.pathId && retired.paths[0]!.generation === pinned.generation
        && retired.paths[0]!.edgeId === pinned.edgeId && retired.mode === "tunnels", "Pinned route identity was replaced");
    const blockedHash = await lab.add("v1-64k", join(lab.root, "blocked"));
    await lab.request("torrents/addWebSeeds", { hash: blockedHash, urls: url });
    await waitFor("blocked torrent has a registered webseed", () => lab.json<{ url: string }[]>(`torrents/webseeds?hash=${blockedHash}`),
        seeds => seeds.some(seed => seed.url === url));
    await lab.request("torrents/start", { hashes: blockedHash });
    await waitFor("blocked torrent is active", () => lab.info(blockedHash), info => !info.state.startsWith("stopped"));
    const beforeBlocked = { requests: requests.length, responseBytes, relayBytes: proxies.map(proxy => proxy.stats.downloadStreamBytes),
        connections: proxies.map(proxy => proxy.stats.acceptedConnections) };
    const deadline = Date.now() + 8000;
    do {
        assert.deepEqual(errors, []);
        const [info, routes] = await Promise.all([lab.info(blockedHash), status()]);
        assert(info.completed === 0 && info.progress === 0 && !info.state.startsWith("stopped"));
        assert(routes.paths[0]?.pathId === pinned.pathId && !routes.paths[0].open && routes.paths[1]?.open === true);
        assert(!routes.peers.some(peer => peer.infoHash === blockedHash), "Blocked private webseed acquired a peer route");
        assert.equal(requests.length, beforeBlocked.requests);
        assert.equal(responseBytes, beforeBlocked.responseBytes);
        assert.deepEqual(proxies.map(proxy => proxy.stats.downloadStreamBytes), beforeBlocked.relayBytes);
        assert.deepEqual(proxies.map(proxy => proxy.stats.acceptedConnections), beforeBlocked.connections);
        await Bun.sleep(200);
    } while (Date.now() < deadline);
    assert.equal(nativeRequests, 0);
    await lab.checkpoint({ check: "retired-private-webseed-fails-closed", pinned, blockedHash, observationMs: 8000,
        secondPathStillReady: true, nativeRequests, newHttpRequests: requests.length - beforeBlocked.requests,
        verifiedBytes: 0, routes: await status() });

    // Prove the surviving route really serves data while the private torrent
    // remains blocked; an open flag alone is not a positive control.
    const publicBytes = await download("v1-public", join(lab.root, "public"));
    const publicRequests = requests.slice(beforeBlocked.requests);
    assert(publicRequests.length > 0 && publicRequests.every(request => request.side === 1));
    assert(proxies[1]!.stats.authenticatedConnections > 0 && proxies[1]!.stats.downloadStreamBytes >= publicBytes);
    assert.equal((await lab.info(blockedHash)).completed, 0);
    assert.equal(nativeRequests, 0);
    await lab.checkpoint({ check: "public-webseed-uses-surviving-path", verifiedBytes: publicBytes,
        exactSizes: true, hashesVerified: true, privateTorrentStillBlocked: true, requests: publicRequests, routes: await status() });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    const closed = await Promise.allSettled(proxies.map(proxy => proxy.close()));
    for (const result of closed) if (result.status === "rejected") failure ??= result.reason;
    for (const server of servers) server.stop(true);
    canary.stop(true);
    await lab.checkpoint({ check: "webseed-final-observations", requests, responseBytes, nativeRequests, errors,
        proxies: proxies.map(proxy => proxy.stats), scope: "HTTP webseeds and private path pinning; HTTPS is not exercised" });
    for (const name of ["fixtures", "profile", "nodes.json", "downloads", "blocked", "public"]) {
        const path = resolve(lab.root, name); assert(dirname(path) === resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
if (!failure && errors.length) failure = new Error(errors.join("; "));
await lab.finish(failure);
if (failure) throw failure;
