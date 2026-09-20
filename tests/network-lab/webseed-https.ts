import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, waitFor } from "../lab";
import { sha256 } from "../fixtures/generate";
import { encode } from "./bencode";

const SOURCE = "https://releases.ubuntu.com/24.04.5/SHA256SUMS";
const SOURCE_SHA256 = "728064ecf411f4ab702d9c3ca0a938ce672771424c028a1b52b2449f7eb5a068";
const SOURCE_BYTES = 893;
const configPath = process.env.QBUTT_HTTPS_PROXY_CONFIG;
const proxyName = process.env.QBUTT_HTTPS_PROXY_NAME;
const interfaceName = process.env.QBUTT_HTTPS_INTERFACE;
assert(configPath && proxyName && interfaceName, "Set QBUTT_HTTPS_PROXY_CONFIG, QBUTT_HTTPS_PROXY_NAME and QBUTT_HTTPS_INTERFACE");
assert(networkInterfaces()[interfaceName]?.some(address => !address.internal), "Selected physical interface does not exist");
interface Status {
    busy: boolean;
    mode: string;
    paths: { pathId: string; generation: number; open: boolean; wire?: { relayDownloadBytes: number } }[];
    diagnostics: { routes: { pathId: string; generation: number; payloadDownload: number; verifiedDownload: number }[] };
}
const lab = await createLab("webseed-https");
let failure: unknown;
try {
    // This preliminary public download supplies only metadata hashes; qbutt has
    // its own empty destination and must fetch the payload again through its path.
    const response = await fetch(SOURCE, { redirect: "error", signal: AbortSignal.timeout(15000) });
    assert(response.ok && response.body, "Official HTTPS fixture is unavailable");
    const chunks: Uint8Array[] = []; let size = 0;
    for await (const chunk of response.body) {
        size += chunk.length; assert(size <= 65536, "HTTPS fixture exceeds 64 KiB"); chunks.push(chunk);
    }
    const payload = Buffer.concat(chunks);
    assert.equal(payload.length, SOURCE_BYTES); assert.equal(sha256(payload), SOURCE_SHA256);
    const pieces = [];
    for (let offset = 0; offset < payload.length; offset += 16384)
        pieces.push(createHash("sha1").update(payload.subarray(offset, offset + 16384)).digest());
    const info = { name: Buffer.from("SHA256SUMS"), length: payload.length,
        "piece length": 16384, pieces: Buffer.concat(pieces) };
    const hash = createHash("sha1").update(encode(info)).digest("hex");
    const torrent = join(lab.root, "https.torrent");
    await writeFile(torrent, encode({ info, "url-list": Buffer.from(SOURCE) }));
    const destination = join(lab.root, "downloads"); await mkdir(destination);
    await lab.start();
    const preferences = await lab.json<{ validate_https_tracker_certificate: boolean }>("app/preferences");
    assert.equal(preferences.validate_https_tracker_certificate, true, "Certificate validation must remain enabled");
    const status = () => lab.json<Status>("qbuttPaths/status");
    await lab.request("qbuttPaths/open", { configPath, proxyName, interfaceName, edgeId: "https-fixture" });
    const opened = await waitFor("HTTPS path ready", status, current => !current.busy && current.paths.length === 1 && current.paths[0]!.open);
    const { pathId, generation } = opened.paths[0]!;
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const form = new FormData(); form.set("torrents", Bun.file(torrent)); form.set("savepath", destination);
    form.set("stopped", "true"); form.set("autoTMM", "false"); form.set("contentLayout", "Original");
    await lab.request("torrents/add", form);
    await waitFor("HTTPS torrent registered", () => lab.json<{ hash: string }[]>(`torrents/info?hashes=${hash}`), jobs => jobs.length === 1);
    assert.deepEqual(await lab.json<{ url: string }[]>(`torrents/webseeds?hash=${hash}`), [{ url: SOURCE }]);
    await lab.request("torrents/start", { hashes: hash });
    await lab.checkpoint({ check: "https-webseed-started", source: SOURCE, sourceSha256: SOURCE_SHA256,
        sourceBytes: SOURCE_BYTES, hash, pathId, generation });
    await waitFor("HTTPS-only webseed download", () => lab.info(hash), current => current.progress === 1, 60000);
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("HTTPS torrent stopped", () => lab.info(hash), current => current.state === "stoppedUP");
    const result = await readFile(join(destination, "SHA256SUMS"));
    assert.equal(result.length, SOURCE_BYTES); assert.equal(sha256(result), SOURCE_SHA256);
    const counters = await waitFor("HTTPS verified bytes attributed to selected path", status, current =>
        current.diagnostics.routes.some(route => route.pathId === pathId && route.generation === generation
            && route.verifiedDownload === SOURCE_BYTES));
    assert.equal(counters.mode, "tunnels");
    for (const route of counters.diagnostics.routes) if (route.pathId !== pathId || route.generation !== generation)
        assert(route.payloadDownload === 0 && route.verifiedDownload === 0, "HTTPS data used another or Native route");
    const wire = counters.paths.find(path => path.pathId === pathId && path.generation === generation)?.wire;
    assert(wire && wire.relayDownloadBytes >= SOURCE_BYTES, "Transport child did not carry the HTTPS payload");
    await lab.checkpoint({ check: "real-path-https-webseed", source: SOURCE, sourceSha256: SOURCE_SHA256,
        verifiedBytes: result.length, exactSize: true, hashesVerified: true, pathId, generation,
        certificateValidationEnabled: true, routes: counters.diagnostics.routes, wire,
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
