import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

import { DownloadQueue, type DownloadJob } from "../../components/content/downloads";
import type { ResolvedContent, Variant, Work } from "../../components/content/contracts";
import { LibraryStore } from "../../components/content/store";
import { mediaRuntime as resolveMediaRuntime } from "../../components/content/media-runtime";

const root = await mkdtemp(join(tmpdir(), "qbutt-content-downloads-"));
const dataDir = join(root, "data");
const downloadDir = join(root, "saved files");
const appDir = join(root, "app");
process.env.QBUTT_LAB_PATHS = "1";
const runtimeInfo = await resolveMediaRuntime(appDir, AbortSignal.timeout(30_000));
const mediaRuntime = runtimeInfo.runtimeDir;
const fixtures = join(root, "fixtures");
const { ffmpeg, ffprobe } = runtimeInfo.tools;
await mkdir(fixtures);

async function command(file: string, args: string[]): Promise<string> {
    const child = Bun.spawn([file, ...args], { cwd: mediaRuntime, windowsHide: true, stdout: "pipe", stderr: "pipe" });
    const [output, error, exit] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
    assert.equal(exit, 0, error);
    return output;
}

await command(ffmpeg, ["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=blue:s=320x180:r=24:d=2",
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an", join(fixtures, "video.mp4")]);
await command(ffmpeg, ["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
    "-c:a", "aac", join(fixtures, "audio.m4a")]);
await command(ffmpeg, ["-hide_banner", "-loglevel", "error", "-i", join(fixtures, "video.mp4"),
    "-c", "copy", "-hls_time", "1", "-hls_list_size", "0", "-hls_segment_filename", join(fixtures, "segment-%02d.ts"),
    join(fixtures, "video.m3u8")]);
await writeFile(join(fixtures, "master.m3u8"), "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=60000\nunused-low-quality.m3u8\n"
    + "#EXT-X-STREAM-INF:BANDWIDTH=120000,RESOLUTION=320x180\nvideo.m3u8?token=temporary-download-secret\n");
await writeFile(join(fixtures, "unsafe.m3u8"), "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nhttp://127.0.0.1/blocked.ts\n#EXT-X-ENDLIST\n");
await writeFile(join(fixtures, "subtitles.vtt"), "WEBVTT\n\n00:00:00.000 --> 00:00:01.800\nGenerated legal subtitle.\n");
const payload = Buffer.alloc(25 * 1024 * 1024 + 17, 0x51);
payload.write("qbutt generated legal streaming download fixture\n");
const payloadHash = createHash("sha256").update(payload).digest("hex");
const compressed = gzipSync(payload);
const requests: { path: string; cookie?: string; authorization?: string; host?: string }[] = [];
const server = createServer((request, response) => {
    const url = new URL(request.url!, "http://fixture.invalid");
    requests.push({ path: url.pathname, cookie: request.headers.cookie, authorization: request.headers.authorization, host: request.headers.host });
    if (url.pathname === "/payload.bin" || url.pathname === "/headers.bin") {
        response.writeHead(200, { "Content-Length": payload.length, "Content-Type": "application/octet-stream" });
        response.end(payload);
    }
    else if (url.pathname === "/redirect") {
        response.writeHead(302, { Location: `http://second.invalid:${port}/headers.bin` });
        response.end();
    }
    else if (url.pathname === "/slow.bin") {
        response.writeHead(200, { "Content-Length": 64 * 1024 * 1024 });
        const timer = setInterval(() => response.write(Buffer.alloc(65536, 0x23)), 10);
        response.once("close", () => clearInterval(timer));
    }
    else if (url.pathname === "/compressed.bin") {
        response.writeHead(200, { "Content-Length": compressed.length, "Content-Encoding": "gzip" });
        response.end(compressed);
    }
    else if (url.pathname === "/tiny.bin") {
        response.writeHead(200, { "Content-Length": 3 });
        response.end("own");
    }
    else if (url.pathname === "/incomplete.bin") {
        response.writeHead(200, { "Content-Length": 10000 });
        response.end("incomplete");
        response.socket?.end();
    }
    else if (url.pathname === "/verification") {
        response.writeHead(200, { "Content-Type": "text/html" });
        response.end("<html><body>Sign in before downloading</body></html>");
    }
    else {
        const name = url.pathname.slice(1);
        if (!/^[a-z0-9.-]+$/i.test(name)) {
            response.writeHead(404).end();
            return;
        }
        const file = join(fixtures, name);
        stat(file).then(info => {
            response.writeHead(200, { "Content-Length": info.size,
                "Content-Type": name.endsWith("m3u8") ? "application/vnd.apple.mpegurl" : "application/octet-stream" });
            createReadStream(file).pipe(response);
        }).catch(() => response.writeHead(404).end());
    }
});
await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
const address = server.address();
assert(address && typeof address !== "string");
const port = address.port;
const origin = `http://fixture.invalid:${port}`;
const fixtureNetwork = { async addresses(url: URL) {
    assert(["fixture.invalid", "second.invalid"].includes(url.hostname), "The fixture connector cannot reach an unrelated host");
    assert.equal(url.protocol, "http:");
    assert.equal(url.port, String(port));
    return [{ address: "127.0.0.1", family: 4 }];
} };
const store = await LibraryStore.open(dataDir);
const events: { event: string; data: DownloadJob }[] = [];
let resolutions = 0;
const variant: Variant = { id: "legal-file", sourceId: "fixture", sourceName: "Generated fixture", title: "Legal fixture",
    url: `${origin}/publication/1?token=source-page-secret`, transport: "file" };
const work: Work = { id: "legal-work", title: "Legal fixture", category: "books", variants: [variant] };
const options = { dataDir, downloadDir, appDir, store, network: fixtureNetwork,
    async resolve(selected: Variant): Promise<ResolvedContent> {
        ++resolutions;
        return { kind: selected.transport, url: `${origin}/${selected.id === "legal-video" ? "master.m3u8" : "payload.bin"}?token=temporary-download-secret`,
            headers: { Cookie: "session=private-fixture-cookie", Authorization: "Bearer private-fixture-token" },
            ...(selected.transport === "video" ? { audioUrl: `${origin}/audio.m4a`,
                subtitles: [{ title: "Fixture English", language: "eng", url: `${origin}/subtitles.vtt` }] } : {}) };
    },
    emit(event: "job" | "progress", data: DownloadJob) { events.push({ event, data }); },
};
let queue = new DownloadQueue(options);

async function finished(id: string): Promise<DownloadJob> {
    const deadline = Date.now() + 60000;
    while (Date.now() < deadline) {
        const job = queue.jobs().find(item => item.id === id)!;
        if (!["queued", "downloading"].includes(job.status))
            return job;
        await Bun.sleep(20);
    }
    throw new Error(`Download did not finish: ${id}; inspect ${root}`);
}

let passed = false;
try {
    const enqueued = queue.enqueue({ work, variant, resolved: { kind: "file", url: "https://expired.invalid/secret" } });
    assert.equal(enqueued.status, "queued");
    assert.equal(resolutions, 0, "Enqueue must return before resolution or download work");
    const first = await finished(enqueued.id);
    assert.equal(first.status, "complete", first.message);
    assert.equal((await stat(first.path!)).size, payload.length);
    assert.equal(createHash("sha256").update(await readFile(first.path!)).digest("hex"), payloadHash);

    const compressedFile = await finished(queue.enqueue({ title: "Compressed response", url: `${origin}/compressed.bin` }).id);
    assert.equal(compressedFile.status, "complete", compressedFile.message);
    assert.equal(createHash("sha256").update(await readFile(compressedFile.path!)).digest("hex"), payloadHash);
    for (const path of ["incomplete.bin", "verification"]) {
        const failed = await finished(queue.enqueue({ title: "Must not publish", url: `${origin}/${path}` }).id);
        assert.equal(failed.status, "error");
        assert.equal(failed.path, undefined);
    }
    assert.equal(resolutions, 1, "The original variant must be resolved instead of persisting a CDN address");
    assert(events.some(event => event.event === "progress" && event.data.id === first.id));

    const second = await finished(queue.enqueue({ work, variant }).id);
    assert.equal(second.status, "complete", second.message);
    assert.notEqual(first.path, second.path, "An existing destination must never be replaced");
    assert.equal(createHash("sha256").update(await readFile(first.path!)).digest("hex"), payloadHash);

    const redirect = await finished(queue.enqueue({ title: "Redirect", resolved: { kind: "file", url: `${origin}/redirect`,
        headers: { Cookie: "session=private-fixture-cookie", Authorization: "Bearer private-fixture-token" } } }).id);
    assert.equal(redirect.status, "complete", redirect.message);
    const redirected = requests.find(item => item.path === "/headers.bin")!;
    assert.equal(redirected.cookie, undefined);
    assert.equal(redirected.authorization, undefined);

    const slow = queue.enqueue({ title: "Cancelled", url: `${origin}/slow.bin` });
    const progressDeadline = Date.now() + 5000;
    while (!events.some(event => event.event === "progress" && event.data.id === slow.id)) {
        assert(Date.now() < progressDeadline, "The slow transfer did not begin");
        await Bun.sleep(20);
    }
    queue.cancel(slow.id);
    assert.equal((await finished(slow.id)).status, "cancelled");

    // Real filesystem collisions keep publication in flight after the response
    // has finished, covering a stop that arrives during final filename selection.
    const collisions = ["Publication race.bin", ...Array.from({ length: 1500 }, (_, index) => `Publication race (${index + 2}).bin`)];
    for (const name of collisions)
        await writeFile(join(downloadDir, name), "existing file");
    const publication = queue.enqueue({ title: "Publication race", url: `${origin}/tiny.bin` });
    const publicationDeadline = Date.now() + 5000;
    while (!events.some(event => event.event === "progress" && event.data.id === publication.id)) {
        assert(Date.now() < publicationDeadline, "Publication transfer did not begin");
        await Bun.sleep(1);
    }
    await Bun.sleep(25);
    assert.equal(queue.jobs().find(item => item.id === publication.id)?.status, "downloading", "The collision fixture must still be publishing");
    queue.cancel(publication.id);
    assert.equal((await finished(publication.id)).status, "cancelled");

    const videoVariant = { ...variant, id: "legal-video", title: "Legal video", transport: "video" as const };
    const video = await finished(queue.enqueue({ variant: videoVariant, title: "Legal video" }).id);
    assert.equal(video.status, "complete", video.message);
    const probe = JSON.parse(await command(ffprobe, ["-v", "error", "-show_streams", "-show_format", "-of", "json", video.path!]));
    assert(probe.streams.some((stream: { codec_type: string }) => stream.codec_type === "video"));
    assert(probe.streams.some((stream: { codec_type: string }) => stream.codec_type === "audio"));
    assert(probe.streams.some((stream: { codec_type: string; tags?: { language?: string } }) => stream.codec_type === "subtitle" && stream.tags?.language === "eng"));
    assert(Number(probe.format.duration) >= 1.9);
    assert.equal(queue.jobs().find(item => item.id === publication.id)?.status, "cancelled");
    assert.equal((await readdir(downloadDir)).filter(name => name.startsWith("Publication race")).length, collisions.length,
        "Cancellation during publication must not create another final file");
    for (const name of collisions)
        await rm(join(downloadDir, name));
    assert(!requests.some(request => request.path === "/unused-low-quality.m3u8"), "Only the selected master rendition should be fetched");
    await command(ffmpeg, ["-v", "error", "-i", video.path!, "-f", "null", "-"]);
    const unsafe = await finished(queue.enqueue({ title: "Unsafe nested stream", resolved: { kind: "video", url: `${origin}/unsafe.m3u8` } }).id);
    assert.equal(unsafe.status, "error");
    assert.equal(unsafe.path, undefined);

    assert.throws(() => queue.enqueue({ variant: { ...variant, transport: "torrent" } }), /torrent session/);
    const persisted = JSON.stringify(store.setting("downloadJobs", []));
    for (const secret of ["temporary-download-secret", "private-fixture-cookie", "private-fixture-token", "source-page-secret", "expired.invalid"])
        assert(!persisted.includes(secret), `Persisted download metadata leaked ${secret}`);
    await queue.close();
    assert(!(await readdir(downloadDir)).some(name => name.endsWith(".part")));
    assert.equal((await readdir(join(dataDir, "downloads"))).length, 0);

    const interruptedId = crypto.randomUUID();
    await writeFile(join(downloadDir, `.qbutt-${interruptedId}.part`), "owned interrupted bytes");
    await mkdir(join(dataDir, "downloads", interruptedId));
    store.setSetting("downloadJobs", [...store.setting<DownloadJob[]>("downloadJobs", []),
        { id: interruptedId, title: "Interrupted", status: "downloading", variant: { ...variant, url: `${origin}/publication/1` } }]);
    queue = new DownloadQueue(options);
    assert.equal(queue.jobs().find(item => item.id === interruptedId)?.status, "error");
    assert.match(queue.jobs().find(item => item.id === interruptedId)?.message || "", /interrupted/i);
    const retry = await finished(queue.enqueue({ variant: queue.jobs().find(item => item.id === interruptedId)!.variant }).id);
    assert.equal(retry.status, "complete", retry.message);
    await queue.close();

    const guarded = new DownloadQueue({ ...options, network: undefined });
    const blocked = guarded.enqueue({ url: `http://127.0.0.1:${port}/payload.bin`, title: "Blocked local address" });
    const deadline = Date.now() + 5000;
    while (guarded.jobs().find(job => job.id === blocked.id)?.status === "queued"
        || guarded.jobs().find(job => job.id === blocked.id)?.status === "downloading") {
        assert(Date.now() < deadline, "The public network guard did not finish");
        await Bun.sleep(20);
    }
    assert.equal(guarded.jobs().find(job => job.id === blocked.id)?.status, "error");
    assert.match(guarded.jobs().find(job => job.id === blocked.id)?.message || "", /local network/i);
    await guarded.close();
    await writeFile(join(root, "result.json"), JSON.stringify({ status: "passed", verifiedFileBytes: payload.length,
        sha256: payloadHash, immediateEnqueue: true, uniquePublication: true, crossOriginSecretsRemoved: true,
        cancelledPartRemoved: true, cancelledPublicationRemoved: true, interruptedRestartAndRetry: true, publicNetworkGuard: true,
        compressedPayloadVerified: true, incompleteAndVerificationRejected: true,
        nestedHlsAddressGuard: true, selectedHlsRenditionOnly: true,
        videoStreams: probe.streams.map((stream: { codec_type: string }) => stream.codec_type),
        videoDuration: Number(probe.format.duration) }, null, 2));
    passed = true;
    console.log(JSON.stringify({ status: "passed", root, verifiedFileBytes: payload.length }));
}
finally {
    await queue.close();
    store.close();
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
    if (passed) {
        for (const directory of [dataDir, downloadDir, fixtures])
            await rm(directory, { recursive: true, force: true });
    }
}
