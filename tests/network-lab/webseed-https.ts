import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, waitFor } from "../lab";
import { sha256 } from "../fixtures/generate";
import { encode } from "./bencode";

const reconnect = process.argv.includes("--reconnect");
const privateTorrent = process.argv.includes("--private");
assert(!privateTorrent || reconnect, "--private requires --reconnect");
const ninja = JSON.parse(await readFile(resolve(import.meta.dir, "../../upstream-lock.json"), "utf8")).windows.ninja as {
    url: string; sha256: string;
};
const SOURCE = reconnect ? ninja.url : "https://releases.ubuntu.com/24.04.5/SHA256SUMS";
const SOURCE_SHA256 = reconnect ? ninja.sha256 : "728064ecf411f4ab702d9c3ca0a938ce672771424c028a1b52b2449f7eb5a068";
const SOURCE_BYTES = reconnect ? 275253 : 893;
const NAME = reconnect ? "ninja-win.zip" : "SHA256SUMS";
const sourceFile = process.env.QBUTT_HTTPS_SOURCE_FILE;
const configPath = process.env.QBUTT_HTTPS_PROXY_CONFIG;
const proxyName = process.env.QBUTT_HTTPS_PROXY_NAME;
const secondProxyName = process.env.QBUTT_HTTPS_PROXY_NAME_2;
const interfaceName = process.env.QBUTT_HTTPS_INTERFACE;
assert(configPath && proxyName && interfaceName, "Set QBUTT_HTTPS_PROXY_CONFIG, QBUTT_HTTPS_PROXY_NAME and QBUTT_HTTPS_INTERFACE");
assert(!reconnect || (sourceFile && secondProxyName && secondProxyName !== proxyName),
    "Reconnect requires QBUTT_HTTPS_SOURCE_FILE and a distinct QBUTT_HTTPS_PROXY_NAME_2");
assert(networkInterfaces()[interfaceName]?.some(address => !address.internal), "Selected physical interface does not exist");
interface Status {
    busy: boolean;
    mode: string;
    paths: { pathId: string; generation: number; open: boolean; wire?: { relayDownloadBytes: number } }[];
    peers: { infoHash: string; pathId: string; generation: number; payloadDownload: number }[];
    diagnostics: { routes: { pathId: string; generation: number; payloadDownload: number; verifiedDownload: number }[] };
}
const lab = await createLab("webseed-https");
let failure: unknown;
try {
    // The reconnect fixture reuses the verified build dependency cache solely
    // for metadata. qbutt must fetch a separate payload through its own routes.
    let payload: Buffer;
    if (reconnect) payload = await readFile(sourceFile!);
    else {
        const response = await fetch(SOURCE, { redirect: "error", signal: AbortSignal.timeout(15000) });
        assert(response.ok && response.body, "Official HTTPS fixture is unavailable");
        const chunks: Uint8Array[] = []; let size = 0;
        for await (const chunk of response.body) {
            size += chunk.length; assert(size <= 65536, "HTTPS fixture exceeds 64 KiB"); chunks.push(chunk);
        }
        payload = Buffer.concat(chunks);
    }
    assert.equal(payload.length, SOURCE_BYTES); assert.equal(sha256(payload), SOURCE_SHA256);
    const pieces = [];
    for (let offset = 0; offset < payload.length; offset += 16384)
        pieces.push(createHash("sha1").update(payload.subarray(offset, offset + 16384)).digest());
    const info = { name: Buffer.from(NAME), length: payload.length,
        "piece length": 16384, pieces: Buffer.concat(pieces), ...(privateTorrent ? { private: 1 } : {}) };
    const hash = createHash("sha1").update(encode(info)).digest("hex");
    const torrent = join(lab.root, "https.torrent");
    await writeFile(torrent, encode({ info, "url-list": Buffer.from(SOURCE) }));
    const destination = join(lab.root, "downloads"); await mkdir(destination);
    await lab.start();
    const preferences = await lab.json<{ validate_https_tracker_certificate: boolean }>("app/preferences");
    assert.equal(preferences.validate_https_tracker_certificate, true, "Certificate validation must remain enabled");
    const status = () => lab.json<Status>("qbuttPaths/status");
    await lab.request("qbuttPaths/open", { configPath, proxyName, interfaceName });
    const opened = await waitFor("HTTPS path ready", status, current => !current.busy && current.paths.length === 1 && current.paths[0]!.open);
    const { pathId, generation } = opened.paths[0]!;
    if (reconnect) {
        await lab.request("qbuttPaths/open", { configPath, proxyName: secondProxyName!, interfaceName });
        await waitFor("Second HTTPS path ready", status, current => !current.busy && current.paths.filter(path => path.open).length === 2);
    }
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const form = new FormData(); form.set("torrents", Bun.file(torrent)); form.set("savepath", destination);
    form.set("stopped", "true"); form.set("autoTMM", "false"); form.set("contentLayout", "Original");
    await lab.request("torrents/add", form);
    await waitFor("HTTPS torrent registered", () => lab.json<{ hash: string }[]>(`torrents/info?hashes=${hash}`), jobs => jobs.length === 1);
    assert.deepEqual(await lab.json<{ url: string }[]>(`torrents/webseeds?hash=${hash}`), [{ url: SOURCE }]);
    if (reconnect) await lab.request("torrents/setDownloadLimit", { hashes: hash, limit: "8192" });
    await lab.request("torrents/start", { hashes: hash });
    await lab.checkpoint({ check: "https-webseed-started", source: SOURCE, sourceSha256: SOURCE_SHA256,
        sourceBytes: SOURCE_BYTES, hash, pathId, generation, reconnect, privateTorrent });
    let retired: Status["paths"][number] | undefined;
    let resumedPath = { pathId, generation };
    if (reconnect) {
        await waitFor("Verified HTTPS piece before retirement", () => lab.info(hash), current => current.completed >= 16384 && current.progress < 1);
        const before = await waitFor("HTTPS verified route credit before retirement", status, current =>
            current.diagnostics.routes.some(route => route.verifiedDownload >= 16384));
        const active = before.peers.find(peer => peer.infoHash === hash && peer.payloadDownload > 0);
        assert(active, "No active HTTPS payload socket before retirement");
        retired = before.paths.find(path => path.pathId === active.pathId && path.generation === active.generation);
        assert(retired?.open, "HTTPS payload route is already closed");
        assert(before.diagnostics.routes.some(route => route.pathId === retired!.pathId && route.generation === retired!.generation
            && route.verifiedDownload >= 16384), "Active HTTPS path has no verified piece before retirement");
        if (privateTorrent) assert.equal(retired.pathId, pathId, "Private HTTPS webseed escaped its first path");
        await lab.checkpoint({ check: "https-before-retirement", retired: { pathId: retired.pathId, generation: retired.generation },
            completed: (await lab.info(hash)).completed, routes: before.diagnostics.routes });
        await lab.request("qbuttPaths/stop", { pathId: retired.pathId });
        const retiredId = retired.pathId;
        const stopped = await waitFor("Retired HTTPS socket closed", status, current => !current.busy
            && !current.paths.find(path => path.pathId === retiredId)?.open
            && !current.peers.some(peer => peer.infoHash === hash && peer.pathId === retiredId));
        const survivor = stopped.paths.find(path => path.open); assert(survivor, "The other HTTPS route is unavailable");
        resumedPath = { pathId: survivor.pathId, generation: survivor.generation };
        if (privateTorrent) {
            await Bun.sleep(1000); // Drain already completed disk/hash jobs.
            const completed = (await lab.info(hash)).completed;
            const deadline = Date.now() + 8000;
            do {
                const current = await status();
                assert.equal((await lab.info(hash)).completed, completed, "Private HTTPS data advanced after revocation");
                assert(!current.peers.some(peer => peer.infoHash === hash), "Private HTTPS socket migrated after revocation");
                assert(current.diagnostics.routes.filter(route => route.pathId !== retiredId)
                    .every(route => route.payloadDownload === 0 && route.verifiedDownload === 0), "Private HTTPS payload escaped its pinned path");
                await Bun.sleep(200);
            } while (Date.now() < deadline);
            await lab.checkpoint({ check: "private-https-retirement-holds", completed, observationMs: 8000, survivingPath: resumedPath });
            await lab.request("qbuttPaths/open", { configPath, proxyName, interfaceName });
            const reopened = await waitFor("Private HTTPS path reopened", status, current => !current.busy
                && current.paths.some(path => path.pathId === retiredId && path.open && path.generation > retired!.generation));
            const newPath = reopened.paths.find(path => path.pathId === retiredId)!;
            resumedPath = { pathId: newPath.pathId, generation: newPath.generation };
        }
        await lab.request("torrents/setDownloadLimit", { hashes: hash, limit: "0" });
    }
    await waitFor("HTTPS-only webseed download", () => lab.info(hash), current => current.progress === 1, 60000);
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("HTTPS torrent stopped", () => lab.info(hash), current => current.state === "stoppedUP");
    const result = await readFile(join(destination, NAME));
    assert.equal(result.length, SOURCE_BYTES); assert.equal(sha256(result), SOURCE_SHA256);
    const allowed = retired ? [{ pathId: retired.pathId, generation: retired.generation }, resumedPath] : [resumedPath];
    const counters = await waitFor("HTTPS verified bytes attributed to selected paths", status, current =>
        current.diagnostics.routes.filter(route => allowed.some(path => path.pathId === route.pathId && path.generation === route.generation))
            .reduce((sum, route) => sum + route.verifiedDownload, 0) === SOURCE_BYTES);
    assert.equal(counters.mode, "tunnels");
    for (const route of counters.diagnostics.routes) if (!allowed.some(path => path.pathId === route.pathId && path.generation === route.generation))
        assert(route.payloadDownload === 0 && route.verifiedDownload === 0, "HTTPS data used another or Native route");
    for (const path of allowed)
        assert(counters.diagnostics.routes.some(route => route.pathId === path.pathId && route.generation === path.generation
            && route.payloadDownload > 0 && route.verifiedDownload > 0), "An expected HTTPS path carried no verified data");
    const wire = counters.paths.find(path => path.pathId === resumedPath.pathId && path.generation === resumedPath.generation)?.wire;
    const resumedCounters = counters.diagnostics.routes.find(route => route.pathId === resumedPath.pathId && route.generation === resumedPath.generation)!;
    assert(wire && wire.relayDownloadBytes >= resumedCounters.payloadDownload, "Transport child did not carry the HTTPS payload");
    await lab.checkpoint({ check: "real-path-https-webseed", source: SOURCE, sourceSha256: SOURCE_SHA256,
        verifiedBytes: result.length, exactSize: true, hashesVerified: true, allowed, reconnect, privateTorrent,
        certificateValidationEnabled: true, routes: counters.diagnostics.routes, wire,
        ...(reconnect ? { torrentRestarted: false } : {}),
        nativeRoutePayloadBytes: 0, scope: "Normal public TLS certificate, Native exclusion measured by engine route accounting" });
}
catch (error) {
    failure = error;
    try {
        const current = await lab.json<Status>("qbuttPaths/status");
        await lab.checkpoint({ check: "https-failure-route-counters", routes: current.diagnostics.routes });
    }
    catch { /* The app may have been stopped before producing diagnostics. */ }
}
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    for (const name of ["fixtures", "profile", "https.torrent", "downloads"]) {
        const path = resolve(lab.root, name); assert(dirname(path) === resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
