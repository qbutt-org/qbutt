import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { createServer, request } from "node:http";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { DownloadQueue, type DownloadJob } from "../../components/content/downloads";
import { NetworkProxy } from "../../components/content/networkproxy";
import { LibraryStore } from "../../components/content/store";

const root = await mkdtemp(join(tmpdir(), "qbutt-dash-downloads-"));
const appDir = join(root, "app");
process.env.QBUTT_LAB_PATHS = "1";
const mediaRuntime = process.env.QBUTT_MEDIA_RUNTIME_DIR;
assert(mediaRuntime, "Set QBUTT_MEDIA_RUNTIME_DIR to the separately prepared pinned lab media tools");
const dataDir = join(root, "data");
const downloadDir = join(root, "saved");
const fixtures = join(root, "fixtures");
const ffmpeg = join(mediaRuntime, "tools", "ffmpeg.exe");
const ffprobe = join(mediaRuntime, "profile", "tools", "bin", "ffprobe.exe");
const systemRoot = process.env.SystemRoot || "C:/Windows";
const toolEnvironment = { ...process.env, PATH: [mediaRuntime, join(systemRoot, "System32"), systemRoot].join(";") };
await mkdir(fixtures);

async function command(file: string, args: string[]): Promise<string> {
    const child = Bun.spawn([file, ...args], {
        windowsHide: true, stdout: "pipe", stderr: "pipe", cwd: fixtures, env: toolEnvironment,
    });
    const timeout = setTimeout(() => child.kill(), 30000);
    try {
        const [out, error, exit] = await Promise.all([
            new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
        ]);
        assert.equal(exit, 0, error);
        return out;
    }
    finally {
        clearTimeout(timeout);
    }
}

const original = join(fixtures, "original.mkv");
await command(ffmpeg, ["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=160x90:r=24:d=3",
    "-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-f", "lavfi", "-i", "sine=frequency=880:duration=3",
    "-map", "0:v", "-map", "1:a", "-map", "2:a", "-c:v", "libx264", "-g", "24", "-c:a", "aac",
    "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=rus", original]);
await command(ffmpeg, ["-v", "error", "-i", original, "-map", "0", "-c", "copy", "-seg_duration", "1",
    "-use_template", "1", "-use_timeline", "0", "-init_seg_name", "init-$RepresentationID$.m4s",
    "-media_seg_name", "chunk-$RepresentationID$-$Number%05d$.m4s", "-f", "dash", join(fixtures, "generated.mpd")]);
const generated = await readFile(join(fixtures, "generated.mpd"), "utf8");
const fixtureFiles = await readdir(fixtures);
const segmentList = generated.replace(/<Representation id="(\d+)"[^>]*>[\s\S]*?<\/Representation>/g, (representation, id) =>
    representation.replace(/<SegmentTemplate[\s\S]*?<\/SegmentTemplate>/, '<SegmentList timescale="1" duration="1">'
        + `<Initialization sourceURL="init-${id}.m4s"/>`
        + fixtureFiles.filter(name => name.startsWith(`chunk-${id}-`)).sort().map(name => `<SegmentURL media="${name}"/>`).join("")
        + '</SegmentList>'));
await writeFile(join(fixtures, "subtitles.vtt"), "WEBVTT\n\n00:00:00.000 --> 00:00:02.800\nGenerated legal DASH fixture.\n");
const requests: { path: string; host?: string; cookie?: string; authorization?: string; secret?: string; proxy?: string }[] = [];
let activeSlow = 0;
let manifest = "";
let port = 0;
const server = createServer((incoming, response) => {
    const url = new URL(incoming.url!, "http://fixture.invalid");
    requests.push({ path: url.pathname, host: incoming.headers.host, cookie: incoming.headers.cookie,
        authorization: incoming.headers.authorization, secret: incoming.headers["x-source-session"] as string,
        proxy: incoming.headers["proxy-authorization"] as string });
    if (url.pathname === "/manifest-redirect") {
        response.writeHead(302, { Location: `http://second.invalid:${port}/offorigin.mpd` }).end();
        return;
    }
    if (url.pathname.endsWith(".mpd")) {
        let body = manifest;
        if (url.pathname === "/list.mpd")
            body = segmentList;
        if (url.pathname === "/unsafe.mpd")
            body = generated.replace(/(media|initialization)="[^"]*"/g, '$1="http://127.0.0.1/blocked.m4s"');
        if (url.pathname === "/external.mpd")
            body = generated.replace("<Period", '<Period xlink:href="http://127.0.0.1/blocked.xml"');
        if (url.pathname === "/live.mpd")
            body = generated.replace('type="static"', 'type="dynamic"');
        if (url.pathname === "/slow.mpd")
            body = generated.replace(/media="([^"]*)"/g, 'media="slow/$1"');
        if (url.pathname === "/redirect-private.mpd")
            body = generated.replace(/initialization="([^"]*)"/g, 'initialization="private/$1"');
        if (url.pathname === "/segment-redirect.mpd")
            body = generated.replace(/(media|initialization)="([^"]*)"/g, '$1="redirect/$2"');
        if (url.pathname === "/offorigin.mpd")
            body = generated.replace(/(media|initialization)="([^"]*)"/g, `$1="http://second.invalid:${port}/assets/$2"`);
        response.writeHead(200, { "Content-Type": "application/dash+xml" }).end(body);
        return;
    }
    if (url.pathname.startsWith("/redirect/")) {
        response.writeHead(302, { Location: `http://second.invalid:${port}/assets/${url.pathname.split("/").at(-1)}` }).end();
        return;
    }
    if (url.pathname.startsWith("/private/")) {
        response.writeHead(302, { Location: `http://127.0.0.1:${port}/blocked.m4s` }).end();
        return;
    }
    if (url.pathname.startsWith("/slow/")) {
        ++activeSlow;
        response.writeHead(200, { "Content-Type": "video/mp4" });
        const timer = setInterval(() => response.write(Buffer.alloc(1024)), 50);
        response.once("close", () => { clearInterval(timer); --activeSlow; });
        return;
    }
    const name = url.pathname.split("/").at(-1)!;
    if (!/^[a-z\d.-]+$/i.test(name)) {
        response.writeHead(404).end();
        return;
    }
    const file = join(fixtures, name);
    stat(file).then(info => {
        const range = /^bytes=(\d+)-(\d*)$/.exec(incoming.headers.range || "");
        const start = range ? Number(range[1]) : 0;
        const end = range?.[2] ? Math.min(Number(range[2]), info.size - 1) : info.size - 1;
        response.writeHead(range ? 206 : 200, { "Content-Length": end - start + 1,
            ...(range ? { "Content-Range": `bytes ${start}-${end}/${info.size}` } : {}),
            "Content-Type": name.endsWith("vtt") ? "text/vtt" : "video/mp4" });
        createReadStream(file, { start, end }).pipe(response);
    }).catch(() => response.writeHead(404).end());
});
await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
const address = server.address();
assert(address && typeof address !== "string");
port = address.port;
const origin = `http://fixture.invalid:${port}`;
manifest = generated.replace("<Period", '<BaseURL xml:base="/dash/">assets/</BaseURL>\n<Period');
const network = { async addresses(url: URL) {
    assert(["fixture.invalid", "second.invalid"].includes(url.hostname), "A nested request attempted a private or unrelated host");
    assert.equal(url.protocol, "http:");
    assert.equal(url.port, String(port));
    return [{ address: "127.0.0.1", family: 4 }];
} };
const store = await LibraryStore.open(dataDir);
const queue = new DownloadQueue({ dataDir, downloadDir, appDir, store, network,
    async resolve() { throw new Error("This fixture supplies its resolved source"); }, emit() {} });
const headers = { Cookie: "session=fixture-cookie", Authorization: "Bearer fixture-token", "X-Source-Session": "fixture-secret" };

async function finished(id: string): Promise<DownloadJob> {
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
        const job = queue.jobs().find(item => item.id === id)!;
        if (!["queued", "downloading"].includes(job.status))
            return job;
        await Bun.sleep(20);
    }
    throw new Error(`DASH job timed out; inspect ${root}`);
}

function enqueue(path: string, subtitles = false): DownloadJob {
    return queue.enqueue({ title: `Generated ${path}`, resolved: { kind: "video", url: origin + path, headers,
        ...(subtitles ? { subtitles: [{ title: "Legal subtitle", language: "eng", url: origin + "/subtitles.vtt" }] } : {}) } });
}

async function mediaHash(path: string, stream: string): Promise<string> {
    const output = await command(ffmpeg, ["-v", "error", "-i", path, "-map", stream, "-c", "copy", "-f", "hash", "-hash", "sha256", "-"]);
    return output.trim();
}

let passed = false;
const gateway = await NetworkProxy.start({ network, payload: { sourceUrl: origin + "/generated.mpd", headers, signal: new AbortController().signal } });
try {
    const completed = await finished(enqueue("/generated.mpd", true).id);
    assert.equal(completed.status, "complete", completed.message);
    const probe = JSON.parse(await command(ffprobe, ["-v", "error", "-show_streams", "-show_format", "-of", "json", completed.path!]));
    assert.deepEqual(probe.streams.map((stream: any) => stream.codec_type), ["video", "audio", "audio", "subtitle"]);
    assert(Number(probe.format.duration) >= 3);
    await command(ffmpeg, ["-v", "error", "-i", completed.path!, "-map", "0:v", "-map", "0:a", "-f", "null", "-"]);
    const hashes = [];
    for (const stream of ["0:v:0", "0:a:0", "0:a:1"]) {
        const expected = await mediaHash(join(fixtures, "generated.mpd"), stream);
        const actual = await mediaHash(completed.path!, stream);
        assert.equal(actual, expected, `DASH remux changed ${stream}`);
        hashes.push({ stream, sha256: actual.replace("SHA256=", "") });
    }
    assert(requests.some(item => item.path.startsWith("/dash/assets/chunk-0-00001")), "Padded segment templates were not substituted");
    assert(requests.filter(item => /\.m4s$/.test(item.path)).every(item => item.cookie === headers.Cookie
        && item.authorization === headers.Authorization && item.secret === headers["X-Source-Session"]));
    assert(requests.every(item => item.proxy === undefined), "Proxy credentials reached an upstream resource");

    for (const path of ["/list.mpd", "/offorigin.mpd", "/segment-redirect.mpd", "/manifest-redirect"]) {
        const crossOrigin = await finished(enqueue(path).id);
        assert.equal(crossOrigin.status, "complete", crossOrigin.message);
    }
    const offorigin = requests.filter(item => item.host === `second.invalid:${port}`);
    assert(offorigin.length > 3);
    assert(offorigin.every(item => item.cookie === undefined && item.authorization === undefined && item.secret === undefined));
    for (const path of ["/unsafe.mpd", "/external.mpd", "/live.mpd", "/redirect-private.mpd"]) {
        const rejected = await finished(enqueue(path).id);
        assert.equal(rejected.status, "error", `${path} was not rejected`);
        assert.equal(rejected.path, undefined);
    }
    assert(!requests.some(item => item.path === "/blocked.m4s"), "A redirect reached a private host");

    const slow = enqueue("/slow.mpd");
    const deadline = Date.now() + 10000;
    while (!activeSlow) {
        assert(Date.now() < deadline, "The real FFmpeg segment transfer did not start");
        await Bun.sleep(20);
    }
    queue.cancel(slow.id);
    assert.equal((await finished(slow.id)).status, "cancelled");
    const priority = await finished(enqueue("/generated.mpd").id);
    assert.equal(priority.status, "complete", priority.message);
    assert.equal(activeSlow, 0, "Cancelled segment connections remained open");

    // Exercise the same listener directly: fixed resources work, arbitrary
    // remote URLs and CONNECT cannot turn it into a general proxy.
    const registered = await gateway.resource(origin + "/assets/init-0.m4s");
    const local = new URL(gateway.url);
    const proxyGet = (target: string) => new Promise<number>((resolve, reject) => {
        const outgoing = request({ hostname: local.hostname, port: local.port, path: target, agent: false }, response => {
            response.resume();
            response.on("end", () => resolve(response.statusCode || 0));
        });
        outgoing.on("error", reject);
        outgoing.end();
    });
    assert.equal(await proxyGet(registered), 200);
    const range = await fetch(registered, { headers: { Range: "bytes=0-15" } });
    assert.equal(range.status, 206);
    assert.deepEqual(Buffer.from(await range.arrayBuffer()), (await readFile(join(fixtures, "init-0.m4s"))).subarray(0, 16));
    assert.equal(await proxyGet(origin + "/assets/init-0.m4s"), 403);
    assert.equal(await proxyGet(registered.replace(/\/_qbutt\/[^/]+\//, "/_qbutt/wrong/")), 403);
    const tunnel = connect({ host: local.hostname, port: Number(local.port) });
    const tunnelStatus = new Promise<string>(resolve => tunnel.once("data", chunk => resolve(chunk.toString())));
    tunnel.write(`CONNECT fixture.invalid:${port} HTTP/1.1\r\nHost: fixture.invalid:${port}\r\n\r\n`);
    assert.match(await tunnelStatus, /^HTTP\/1.1 403/);
    tunnel.destroy();
    await assert.rejects(() => gateway.resource("http://127.0.0.1/private.m4s"));
    const guarded = await NetworkProxy.start({ payload: { sourceUrl: "https://example.com/a.mpd", headers: {}, signal: new AbortController().signal } });
    try {
        await assert.rejects(() => guarded.resource(`http://127.0.0.1:${port}/blocked.m4s`), /local network/);
    }
    finally {
        await guarded.close();
    }
    await queue.close();
    assert(!(await readdir(downloadDir)).some(name => name.endsWith(".part")));
    assert.equal((await readdir(join(dataDir, "downloads"))).length, 0);
    const evidence = { status: "passed", duration: Number(probe.format.duration),
        streams: probe.streams.map((stream: any) => ({ type: stream.codec_type, language: stream.tags?.language })),
        bytes: (await stat(completed.path!)).size, sha256: createHash("sha256").update(await readFile(completed.path!)).digest("hex"),
        hashes, paddedTemplates: true, representationIds: true, segmentList: true, nestedBaseUrlAndXmlBase: true,
        sameOriginHeaders: true, offOriginAllCredentialsRemoved: true, privateAddressAndRedirectRejected: true,
        externalMetadataAndLiveRejected: true, fixedResourceMap: true, byteRangeVerified: true, connectRejected: true,
        cancellationDrainsConnectionsAndAllowsNextJob: true, stagingAndPartRemoved: true };
    await writeFile(join(root, "result.json"), JSON.stringify(evidence, null, 2));
    console.log(JSON.stringify({ status: "passed", root, bytes: evidence.bytes, streams: evidence.streams }));
    passed = true;
}
finally {
    if (!passed)
        await writeFile(join(root, "failed-requests.json"), JSON.stringify(requests, null, 2));
    await queue.close();
    await gateway.close();
    store.close();
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
    if (passed) {
        for (const path of [dataDir, downloadDir, fixtures])
            await rm(path, { recursive: true, force: true });
    }
}
