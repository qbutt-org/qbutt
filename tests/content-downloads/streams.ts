import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, request, type ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { ProviderContext, ResolvedContent } from "../../components/content/contracts";
import { DownloadQueue, type DownloadJob } from "../../components/content/downloads";
import { LibraryStore } from "../../components/content/store";
import { PlaybackStreams } from "../../components/content/streams";

const mediaDir = Bun.argv[2];
assert(mediaDir, "Usage: bun tests/content-downloads/streams.ts <pinned-media-directory>");
const ffmpeg = join(mediaDir, "tools", "ffmpeg.exe");
const ffprobe = join(mediaDir, "profile", "tools", "bin", "ffprobe.exe");
const systemRoot = process.env.SystemRoot || "C:/Windows";
const root = await mkdtemp(join(tmpdir(), "qbutt-playback-streams-"));
console.log(JSON.stringify({ fixtureRoot: root }));
const fixtures = join(root, "fixtures");
const dataDir = join(root, "data");
const downloadDir = join(root, "downloads");
process.env.QBUTT_LAB_PATHS = "1";
process.env.QBUTT_MEDIA_RUNTIME_DIR = mediaDir;
await mkdir(fixtures);

async function command(file: string, args: string[]): Promise<string> {
    const child = Bun.spawn([file, ...args], {
        windowsHide: true, stdout: "pipe", stderr: "pipe", cwd: fixtures,
        env: { ...process.env, no_proxy: "", NO_PROXY: "",
            PATH: [mediaDir, join(systemRoot, "System32"), systemRoot].join(";") },
    });
    const timeout = setTimeout(() => child.kill(), 30000);
    try {
        const [output, error, code] = await Promise.all([
            new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
        ]);
        assert.equal(code, 0, error.replace(/https?:\/\/\S+/g, "[fixture]"));
        return output;
    }
    finally {
        clearTimeout(timeout);
    }
}

await command(ffmpeg, ["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=160x90:r=24:d=4",
    "-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-map", "0:v", "-map", "1:a",
    "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-g", "24",
    "-keyint_min", "24", "-sc_threshold", "0", "-c:a", "aac", "original.mkv"]);
await command(ffmpeg, ["-v", "error", "-i", "original.mkv", "-map", "0:v", "-c", "copy",
    "-hls_time", "1", "-hls_list_size", "0", "-hls_segment_filename", "video-%02d.ts", "video.m3u8"]);
await command(ffmpeg, ["-v", "error", "-i", "original.mkv", "-map", "0:a", "-c", "copy",
    "-hls_time", "1", "-hls_list_size", "0", "-hls_segment_filename", "audio-%02d.ts", "audio.m3u8"]);
await writeFile(join(fixtures, "subtitles.vtt"), "WEBVTT\n\n00:00:00.000 --> 00:00:03.900\nGenerated legal playback fixture.\n");
await writeFile(join(fixtures, "subtitles.m3u8"),
    "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nsubtitles.vtt\n#EXT-X-ENDLIST\n");
const master = "#EXTM3U\n#EXT-X-VERSION:3\n"
    + '#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="Generated audio",DEFAULT=YES,AUTOSELECT=YES,URI="audio.m3u8"\n'
    + '#EXT-X-STREAM-INF:BANDWIDTH=300000,RESOLUTION=160x90,AUDIO="audio"\nvideo.m3u8\n';
const mediaPlaylist = await readFile(join(fixtures, "video.m3u8"), "utf8");
const requests: { path: string; host?: string; cookie?: string; authorization?: string; secret?: string; proxy?: string }[] = [];
const held = new Map<string, ServerResponse>();
let activeSlow = 0;
let port = 0;
const server = createServer((incoming, response) => {
    const url = new URL(incoming.url!, "http://fixture.invalid");
    requests.push({ path: url.pathname, host: incoming.headers.host, cookie: incoming.headers.cookie,
        authorization: incoming.headers.authorization, secret: incoming.headers["x-source-session"] as string,
        proxy: incoming.headers["proxy-authorization"] as string });
    if (url.pathname.startsWith("/hold/")) {
        held.set(url.pathname, response);
        response.once("close", () => held.delete(url.pathname));
        return;
    }
    if (url.pathname === "/bad.m3u8") {
        response.writeHead(200).end("Not a playlist");
        return;
    }
    if (url.pathname === "/private-redirect.ts") {
        response.writeHead(302, { Location: `http://127.0.0.1:${port}/blocked.ts` }).end();
        return;
    }
    if (url.pathname === "/cross-redirect.ts") {
        response.writeHead(302, { Location: `http://second.invalid:${port}/video-00.ts` }).end();
        return;
    }
    if (url.pathname === "/stall.ts") {
        ++activeSlow;
        response.writeHead(200, { "Content-Type": "video/mp2t" });
        const timer = setInterval(() => response.write(Buffer.alloc(188)), 20);
        response.once("close", () => { clearInterval(timer); --activeSlow; });
        return;
    }
    let manifest: string | undefined;
    if (url.pathname === "/master.m3u8") manifest = master;
    if (url.pathname === "/cross.m3u8") {
        manifest = master.replaceAll('URI="audio.m3u8"', `URI="http://second.invalid:${port}/audio.m3u8"`)
            .replace("\nvideo.m3u8", `\nhttp://second.invalid:${port}/video.m3u8`);
    }
    if (url.pathname === "/private.m3u8") {
        manifest = mediaPlaylist.replace(/video-\d+\.ts/g, `http://127.0.0.1:${port}/blocked.ts`);
    }
    if (url.pathname === "/private-redirect.m3u8") manifest = mediaPlaylist.replace(/video-\d+\.ts/g, "private-redirect.ts");
    if (url.pathname === "/cross-redirect.m3u8") manifest = mediaPlaylist.replace(/video-\d+\.ts/g, "cross-redirect.ts");
    if (url.pathname === "/stall.m3u8") manifest = mediaPlaylist.replace(/video-\d+\.ts/g, "stall.ts");
    if (manifest !== undefined) {
        response.writeHead(200, { "Content-Type": "application/vnd.apple.mpegurl" }).end(manifest);
        return;
    }
    const name = url.pathname.slice(1);
    if (!/^[a-z\d.-]+$/i.test(name)) {
        response.writeHead(404).end();
        return;
    }
    void readFile(join(fixtures, name)).then(body => {
        const range = /^bytes=(\d+)-(\d*)$/.exec(incoming.headers.range || "");
        const start = range ? Number(range[1]) : 0;
        const end = range?.[2] ? Math.min(Number(range[2]), body.length - 1) : body.length - 1;
        response.writeHead(range ? 206 : 200, { "Content-Length": end - start + 1,
            ...(range ? { "Content-Range": `bytes ${start}-${end}/${body.length}` } : {}),
            "Content-Type": name.endsWith("m3u8") ? "application/vnd.apple.mpegurl"
                : name.endsWith("vtt") ? "text/vtt" : "video/mp2t" });
        response.end(incoming.method === "HEAD" ? undefined : body.subarray(start, end + 1));
    }).catch(() => response.writeHead(404).end());
});
await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
const address = server.address();
assert(address && typeof address !== "string");
port = address.port;
const origin = `http://fixture.invalid:${port}`;
const network = { async addresses(url: URL) {
    assert(["fixture.invalid", "second.invalid"].includes(url.hostname), "Private or unrelated fixture target blocked");
    assert.equal(url.protocol, "http:");
    assert.equal(url.port, String(port));
    return [{ address: "127.0.0.1", family: 4 }];
} };

function context(signal = new AbortController().signal): ProviderContext {
    return { signal, dataDir, settings: {}, async fetch(input, init = {}) {
        const target = new URL(input);
        const addresses = await network.addresses(target);
        return new Promise<Response>((resolve, reject) => {
            const outgoing = request(target, { signal, agent: false,
                headers: Object.fromEntries(new Headers(init.headers).entries()),
                lookup(_host, options, callback) {
                    if (options.all) callback(null, addresses);
                    else callback(null, addresses[0].address, addresses[0].family);
                },
            }, incoming => {
                const chunks: Buffer[] = [];
                incoming.on("data", chunk => chunks.push(chunk));
                incoming.on("error", reject);
                incoming.on("end", () => {
                    const response = new Response(Buffer.concat(chunks), { status: incoming.statusCode || 502,
                        headers: new Headers(Object.entries(incoming.headers).flatMap(([name, value]) =>
                            (Array.isArray(value) ? value : value ? [value] : []).map(value => [name, value]))) });
                    Object.defineProperty(response, "url", { value: target.href });
                    resolve(response);
                });
            });
            outgoing.setTimeout(10000, () => outgoing.destroy(new Error("Fixture source request timed out")));
            outgoing.on("error", reject);
            outgoing.end();
        });
    } };
}

async function get(resolved: ResolvedContent, target = resolved.url!, range?: string) {
    assert(resolved.proxyUrl, "Playback omitted its proxy");
    const proxy = new URL(resolved.proxyUrl);
    return new Promise<{ status: number; body: Buffer }>((resolve, reject) => {
        const outgoing = request({ hostname: proxy.hostname, port: proxy.port, path: target, agent: false,
            headers: { "Proxy-Authorization": `Basic ${Buffer.from(`${proxy.username}:${proxy.password}`).toString("base64")}`,
                ...(range ? { Range: range } : {}) } }, incoming => {
            const chunks: Buffer[] = [];
            incoming.on("data", chunk => chunks.push(chunk));
            incoming.on("error", reject);
            incoming.on("end", () => resolve({ status: incoming.statusCode || 0, body: Buffer.concat(chunks) }));
        });
        outgoing.setTimeout(5000, () => outgoing.destroy(new Error("Fixture proxy request timed out")));
        outgoing.on("error", reject);
        outgoing.end();
    });
}

async function waitFor(check: () => boolean, message: string): Promise<void> {
    const deadline = Date.now() + 10000;
    while (!check()) {
        assert(Date.now() < deadline, message);
        await Bun.sleep(20);
    }
}

async function listeners(): Promise<number[]> {
    const output = await command("netstat.exe", ["-ano", "-p", "tcp"]);
    return [...output.matchAll(/^\s*TCP\s+127\.0\.0\.1:(\d+)\s+\S+\s+LISTENING\s+(\d+)\s*$/gm)]
        .filter(match => Number(match[2]) === process.pid).map(match => Number(match[1])).sort((a, b) => a - b);
}

async function decode(resolved: ResolvedContent, start = 0): Promise<number> {
    const output = await command(ffmpeg, ["-v", "error", "-nostats", "-http_proxy", resolved.proxyUrl!,
        "-protocol_whitelist", "http,https,httpproxy,tcp,tls,crypto", "-rw_timeout", "5000000",
        "-ss", String(start), "-i", resolved.url!, "-t", "1", "-map", "0:v", "-map", "0:a?",
        "-progress", "pipe:1", "-f", "null", "-"]);
    const frames = Math.max(0, ...[...output.matchAll(/^frame=(\d+)$/gm)].map(match => Number(match[1])));
    assert(frames > 0, "Real FFmpeg produced no decoded frames");
    return frames;
}

const headers = { Cookie: "session=fixture-cookie", Authorization: "Bearer fixture-token",
    "X-Source-Session": "fixture-secret", "User-Agent": "qbutt-generated-fixture" };
const source = (path = "/master.m3u8"): ResolvedContent => ({ kind: "video", resolver: "direct",
    title: "Generated legal HLS", url: origin + path, headers });
const streams = new PlaybackStreams(network);
const store = await LibraryStore.open(dataDir);
const queue = new DownloadQueue({ dataDir, downloadDir, appDir: root, store, network,
    async resolve() { throw new Error("This scenario downloads the original playing source"); }, emit() {} });

async function finished(id: string): Promise<DownloadJob> {
    await waitFor(() => !["queued", "downloading"].includes(queue.jobs().find(job => job.id === id)!.status),
        "The original playing source did not finish downloading");
    return queue.jobs().find(job => job.id === id)!;
}

let passed = false;
try {
    const baseline = await listeners();
    assert.deepEqual(baseline, [port], "Unexpected fixture process listener at startup");
    const original = { ...source(), audioUrl: origin + "/audio.m3u8",
        subtitles: [{ title: "Generated English", language: "eng", url: origin + "/subtitles.vtt" }] };
    let playing = await streams.prepare(original, context());
    assert.notEqual(playing.url, original.url);
    assert.deepEqual(playing.headers, {});
    const masterResponse = await get(playing);
    assert.equal(masterResponse.status, 200);
    assert(!masterResponse.body.toString().includes("fixture.invalid"), "The native playlist retained upstream URLs");
    const frames = await decode(playing);
    const seekFrames = await decode(playing, 2);
    await command(ffmpeg, ["-v", "error", "-http_proxy", playing.proxyUrl!, "-i", playing.audioUrl!,
        "-t", "1", "-map", "0:a", "-f", "null", "-"]);
    assert.match((await get(playing, playing.subtitles![0].url)).body.toString(), /Generated legal playback fixture/);
    const subtitlePair = { url: original.url, headers: original.headers, subtitleUrl: original.subtitles[0].url };
    assert.deepEqual(streams.originalSubtitle(playing.url, playing.subtitles![0].url), subtitlePair);
    for (const [media, track] of [
        [playing.url, new URL("unknown.vtt", playing.subtitles![0].url).href],
        [origin + "/other.m3u8", playing.subtitles![0].url],
        [playing.url, original.subtitles[0].url],
        [playing.url, 42],
        [null, playing.subtitles![0].url],
    ]) {
        assert.equal(streams.originalSubtitle(media, track), undefined,
            "An unregistered media/subtitle pair restored source credentials");
    }

    const videoManifest = masterResponse.body.toString().split(/\r?\n/).find(line => line.startsWith("http"))!;
    const video = (await get(playing, videoManifest)).body.toString();
    const firstSegment = video.split(/\r?\n/).find(line => line.startsWith("http"))!;
    const range = await get(playing, firstSegment, "bytes=188-375");
    assert.equal(range.status, 206);
    assert.deepEqual(range.body, (await readFile(join(fixtures, "video-00.ts"))).subarray(188, 376));

    const restored = streams.original(playing);
    assert.deepEqual(JSON.parse(JSON.stringify(restored)), original);
    const completed = await finished(queue.enqueue({ title: "Downloaded playing fixture", resolved: restored }).id);
    assert.equal(completed.status, "complete", completed.message);
    const probe = JSON.parse(await command(ffprobe, ["-v", "error", "-show_streams", "-show_format", "-of", "json", completed.path!]));
    assert.deepEqual(probe.streams.map((stream: { codec_type: string }) => stream.codec_type), ["video", "audio", "subtitle"]);
    assert(Number(probe.format.duration) >= 3.9);
    await command(ffmpeg, ["-v", "error", "-i", completed.path!, "-map", "0:v", "-map", "0:a", "-f", "null", "-"]);
    const persisted = JSON.stringify(store.setting("downloadJobs", []));
    for (const forbidden of ["/_qbutt/", "127.0.0.1", new URL(playing.proxyUrl!).password,
        "fixture-cookie", "fixture-token", "fixture-secret"]) {
        assert(!persisted.includes(forbidden), "Download metadata retained a transient capability or source credential");
    }

    const firstOpen = playing;
    playing = await streams.prepare(original, context());
    assert.notEqual(new URL(playing.proxyUrl!).password, new URL(firstOpen.proxyUrl!).password);
    assert.notEqual(playing.subtitles![0].url, firstOpen.subtitles![0].url);
    assert.deepEqual(streams.originalSubtitle(playing.url, playing.subtitles![0].url), subtitlePair,
        "Reopening the same source changed its stable subtitle pair");
    for (const [media, track] of [
        [firstOpen.url, firstOpen.subtitles![0].url],
        [playing.url, firstOpen.subtitles![0].url],
        [firstOpen.url, playing.subtitles![0].url],
    ]) {
        assert.equal(streams.originalSubtitle(media, track), undefined,
            "A stale or mixed playback capability restored source credentials");
    }
    await assert.rejects(get(firstOpen), "Reopening left the previous capability reachable");

    const plain = { kind: "book" as const, url: origin + "/book.epub" };
    assert.deepEqual(await streams.prepare(plain, context()), plain);
    assert.equal((await get(playing)).status, 200, "A nonvideo resolution stopped active playback");
    await assert.rejects(streams.prepare(source("/bad.m3u8"), context()), /invalid video playlist/);
    assert.equal((await get(playing)).status, 200, "Failed preparation stopped active playback");

    const cancelled = new AbortController();
    const cancelTask = streams.prepare(source("/hold/cancel.m3u8"), context(cancelled.signal));
    const cancelResult = assert.rejects(cancelTask);
    await waitFor(() => held.has("/hold/cancel.m3u8"), "Cancelled manifest request did not reach the server");
    cancelled.abort();
    await cancelResult;
    assert.equal((await get(playing)).status, 200, "Cancelled preparation stopped active playback");
    assert.deepEqual(await listeners(), [port, Number(new URL(playing.proxyUrl!).port)].sort((a, b) => a - b));

    await assert.rejects(streams.prepare(source("/private.m3u8"), context()), /Private or unrelated/);
    assert.equal((await get(playing)).status, 200, "Unsafe preparation stopped active playback");
    const productionGuard = new PlaybackStreams();
    try {
        await assert.rejects(productionGuard.prepare(source("/private.m3u8"), context()), /local network/);
    }
    finally {
        await productionGuard.close();
    }

    const previous = playing;
    playing = await streams.prepare(source("/private-redirect.m3u8"), context());
    await assert.rejects(get(previous), "The replaced playback gateway remained reachable");
    const blockedSegment = (await get(playing)).body.toString().split(/\r?\n/).find(line => line.startsWith("http"))!;
    assert.equal((await get(playing, blockedSegment)).status, 502);
    assert(!requests.some(item => item.path === "/blocked.ts"), "A nested redirect reached a private address");

    playing = await streams.prepare(source("/cross-redirect.m3u8"), context());
    const redirectedSegment = (await get(playing)).body.toString().split(/\r?\n/).find(line => line.startsWith("http"))!;
    assert.equal((await get(playing, redirectedSegment)).status, 200);
    playing = await streams.prepare(source("/cross.m3u8"), context());
    await decode(playing);
    const crossRequests = requests.filter(item => item.host === `second.invalid:${port}`);
    assert(crossRequests.length > 3);
    assert(crossRequests.every(item => !item.cookie && !item.authorization && !item.secret), "Cross-origin media leaked source credentials");
    assert(requests.every(item => !item.proxy), "Proxy capability reached an upstream request");

    playing = await streams.prepare({ ...source(), subtitles: [{ title: "HLS subtitle", url: origin + "/subtitles.m3u8" }] }, context());
    const subtitleManifest = await get(playing, playing.subtitles![0].url);
    const subtitleResource = subtitleManifest.body.toString().split(/\r?\n/).find(line => line.startsWith("http"))!;
    assert.match((await get(playing, subtitleResource)).body.toString(), /Generated legal playback fixture/);
    assert.deepEqual(streams.originalSubtitle(playing.url, playing.subtitles![0].url),
        { url: original.url, headers: original.headers, subtitleUrl: origin + "/subtitles.m3u8" });

    const earlier = playing;
    const slowPrepare = streams.prepare(source("/hold/earlier.m3u8"), context());
    const replaced = assert.rejects(slowPrepare, error => error instanceof DOMException && error.name === "AbortError");
    await waitFor(() => held.has("/hold/earlier.m3u8"), "Concurrent manifest did not reach the server");
    playing = await streams.prepare(source(), context());
    held.get("/hold/earlier.m3u8")!.end(master.replaceAll('URI="audio.m3u8"', 'URI="/audio.m3u8"').replace("\nvideo.m3u8", "\n/video.m3u8"));
    await replaced;
    await assert.rejects(get(earlier), "The old gateway survived a newer prepared video");
    assert.equal((await get(playing)).status, 200);
    assert.deepEqual(await listeners(), [port, Number(new URL(playing.proxyUrl!).port)].sort((a, b) => a - b));

    playing = await streams.prepare(source("/stall.m3u8"), context());
    const stalledSegment = (await get(playing)).body.toString().split(/\r?\n/).find(line => line.startsWith("http"))!;
    const transfer = assert.rejects(get(playing, stalledSegment));
    await waitFor(() => activeSlow > 0, "The old payload transfer did not start");
    const pending = streams.prepare(source("/hold/close.m3u8"), context());
    const closed = assert.rejects(pending, error => error instanceof DOMException && error.name === "AbortError");
    await waitFor(() => held.has("/hold/close.m3u8"), "Close-race manifest did not reach the server");
    await streams.close();
    await transfer;
    await waitFor(() => activeSlow === 0, "Closing playback left an active upstream transfer");
    held.get("/hold/close.m3u8")!.end(master.replaceAll('URI="audio.m3u8"', 'URI="/audio.m3u8"').replace("\nvideo.m3u8", "\n/video.m3u8"));
    await closed;
    await assert.rejects(get(playing), "Close left the current gateway listening");
    assert.deepEqual(await listeners(), baseline, "A failed, cancelled or superseded preparation leaked a listener");

    const result = { status: "passed", decodedFrames: frames, seekDecodedFrames: seekFrames,
        rangeBytes: range.body.length, sameReturnedProxy: true, auxiliaryHlsAudioAndSubtitles: true,
        nestedPrivateAddressAndRedirectRejected: true, crossOriginChildAndRedirectedPayloadSecretsRemoved: true,
        failedAndCancelledPreparationRetainsPlayback: true, nonvideoRetainsPlayback: true,
        latestPreparationWins: true, settledCloseRaceLeavesNoGateways: true, leakedGateways: 0,
        downloadedStreams: probe.streams.map((stream: { codec_type: string }) => stream.codec_type),
        downloadedDuration: Number(probe.format.duration),
        downloadedSha256: createHash("sha256").update(await readFile(completed.path!)).digest("hex"),
        transientCapabilityNotPersisted: true, exactSubtitlePairRestored: true,
        unregisteredAndStaleSubtitlePairsNotRestored: true, reopenedSubtitlePairStable: true };
    await writeFile(join(root, "result.json"), JSON.stringify(result, null, 2));
    passed = true;
    console.log(JSON.stringify({ ...result, evidence: join(root, "result.json") }));
}
finally {
    await streams.close();
    await queue.close();
    store.close();
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
    if (passed) {
        for (const directory of [fixtures, dataDir, downloadDir])
            await rm(directory, { recursive: true, force: true });
    }
}
