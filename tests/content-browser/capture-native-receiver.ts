import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { createServer, type Server } from "node:http";
import { mkdtemp, readFile, readdir, realpath, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";

import { Database } from "bun:sqlite";

import { NativeBrowserFixture } from "./native-parent";
import { encodeFrame, FrameDecoder } from "./framing";

const qcatch = process.env.QBUTT_QCATCH_EXE;
assert(qcatch, "Set QBUTT_QCATCH_EXE to the independent qcatch native host");
const ffmpeg = process.env.QBUTT_LAB_FFMPEG;
const ffprobe = process.env.QBUTT_LAB_FFPROBE;
assert(ffmpeg && ffprobe, "Set QBUTT_LAB_FFMPEG and QBUTT_LAB_FFPROBE to verified qplay tools");

type Outcome = { captureId: string; reason: string };
type FixtureMessage = {
    watchFinished?: Outcome;
    downloadFinished?: Outcome;
    watchMedia?: { identity: string; url: string; launchToken: number };
    captureProfile?: string;
    captureCompleted?: boolean;
    captureCancelled?: boolean;
    captureError?: string;
    downloadSnapshot?: {
        state: string;
        pageCount: number;
        jobs: { id: string; status: string; bytes: number; path: string }[];
    };
};
type DownloadJob = {
    id: string;
    status: string;
    path: string;
    bytes: number;
    origin: { kind: string; source_host: string };
};

const root = await mkdtemp(join(tmpdir(), "qbutt-capture-native-"));
const discovery = join(root, "browser.json");
const bridge = new NativeBrowserFixture({ browser: "edge", discoveryPath: discovery });
const messages: FixtureMessage[] = [];
bridge.onMessage = message => messages.push(message as FixtureMessage);

const watchId = crypto.randomUUID();
const downloadId = crypto.randomUUID();
const hlsId = crypto.randomUUID();
const dashId = crypto.randomUUID();
const externalId = crypto.randomUUID();
const expiredId = crypto.randomUUID();
const cancelledId = crypto.randomUUID();
const commands: string[] = [];
const errors: Error[] = [];
const fixtureSession = "generated-legal-session";
let peer: ReturnType<typeof spawn> | undefined;
let mediaServer: Server | undefined;

async function waitFor(predicate: () => boolean | Promise<boolean>, label: string, timeout = 30_000) {
    const deadline = Date.now() + timeout;
    while (!await predicate()) {
        const events = messages.map(message => Object.keys(message).join(",")).join(" | ");
        const reasons = messages.flatMap(message =>
            [message.watchFinished?.reason, message.downloadFinished?.reason].filter(Boolean));
        assert(Date.now() < deadline,
            `Timed out: ${label}; events=${events}; reasons=${reasons}; resolves=${commands.length}; errors=${errors.length}`);
        await Bun.sleep(25);
    }
}

async function mediaCommand(file: string, args: string[]): Promise<Buffer> {
    const process = Bun.spawn([file, ...args], {
        cwd: root, windowsHide: true, stdout: "pipe", stderr: "pipe",
    });
    const timer = setTimeout(() => process.kill(), 30_000);
    try {
        const [output, error, exit] = await Promise.all([
            new Response(process.stdout).arrayBuffer(), new Response(process.stderr).text(), process.exited,
        ]);
        assert.equal(exit, 0, error);
        return Buffer.from(output);
    }
    finally {
        clearTimeout(timer);
    }
}

function event(name: "watchFinished" | "downloadFinished", captureId: string) {
    return messages.map(message => message[name]).find(value => value?.captureId === captureId);
}

function openCapture(action: "watch" | "download", captureId: string, page: string) {
    const payload = { v: 1, pageUrl: page, title: "Generated legal capture", action, captureId };
    const url = `qbutt://capture?d=${Buffer.from(JSON.stringify(payload)).toString("base64url")}`;
    bridge.forward(1, "captureOpen", { url });
}

function resolvedMedia(captureId: string, page: string, media: Awaited<ReturnType<typeof localMedia>>) {
    const source = { kind: "video", resolver: "direct", sourceUrl: page };
    const headers = { "X-Source-Session": fixtureSession };
    if (captureId === hlsId)
        return { ...source, url: media.assetUrl("master.m3u8"), quality: "90p", headers };
    if (captureId === dashId)
        return { ...source, url: media.assetUrl("manifest.mpd"), quality: "90p", headers };
    if (captureId === externalId) {
        return { ...source, url: media.url, audioUrl: media.assetUrl("audio.m4a"), headers,
            subtitles: [{ title: "Accent", language: "en", url: media.assetUrl("accent.ass") }] };
    }
    return { ...source, url: media.url };
}

function jobs(profile: string): DownloadJob[] {
    const database = new Database(join(profile, "content-library.sqlite"), { readonly: true });
    try {
        database.exec("PRAGMA busy_timeout = 5000");
        return database.query("SELECT snapshot FROM download_jobs").all()
            .map(row => JSON.parse((row as { snapshot: string }).snapshot) as DownloadJob);
    }
    finally {
        database.close();
    }
}

async function localMedia() {
    async function generate(args: string[]) {
        await mediaCommand(ffmpeg, ["-hide_banner", "-loglevel", "error", ...args]);
    }

    const video = join(root, "generated.mp4");
    const audio = join(root, "audio.m4a");
    await generate(["-f", "lavfi", "-i", "testsrc2=size=160x90:rate=12", "-t", "3",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", video]);
    await generate(["-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-c:a", "aac", audio]);
    await generate(["-i", video, "-map", "0:v:0", "-c", "copy", "-f", "hls", "-hls_time", "1",
        "-hls_list_size", "0", "-hls_segment_filename", join(root, "video-%03d.ts"),
        join(root, "video.m3u8")]);
    await generate(["-i", audio, "-map", "0:a:0", "-c", "copy", "-f", "hls", "-hls_time", "1",
        "-hls_list_size", "0", "-hls_segment_filename", join(root, "audio-%03d.ts"),
        join(root, "audio.m3u8")]);
    await generate(["-i", video, "-i", audio, "-map", "0:v:0", "-map", "1:a:0", "-c", "copy",
        "-f", "dash", "-seg_duration", "1", "-use_template", "1", "-use_timeline", "1",
        "-init_seg_name", "init-stream$RepresentationID$.m4s",
        "-media_seg_name", "chunk-stream$RepresentationID$-$Number%05d$.m4s",
        join(root, "manifest.mpd")]);
    await writeFile(join(root, "master.m3u8"),
        "#EXTM3U\n#EXT-X-VERSION:3\n"
        + '#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="Generated audio",DEFAULT=YES,LANGUAGE="en",URI="audio.m3u8"\n'
        + '#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=320x180,AUDIO="audio"\nmissing.m3u8\n'
        + '#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=160x90,AUDIO="audio"\nvideo.m3u8\n');
    await writeFile(join(root, "accent.ass"),
        "[Script Info]\nTitle: Generated subtitle fixture\nScriptType: v4.00+\nPlayResX: 160\nPlayResY: 90\n\n"
        + "[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
        + "Style: Accent,Arial,18,&H0000FFFF,&H0000FFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1\n\n"
        + "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
        + "Dialogue: 0,0:00:00.00,0:00:02.00,Accent,,0,0,0,,Generated subtitle\n");
    const assets = new Map<string, Buffer>();
    for (const name of await readdir(root)) {
        if (/^[a-z0-9._-]+\.(?:mp4|m4a|m3u8|ts|mpd|m4s|ass)$/i.test(name))
            assets.set(name, await readFile(join(root, name)));
    }
    const requested: { path: string; session?: string }[] = [];

    mediaServer = createServer((request, response) => {
        const name = new URL(request.url!, "http://fixture.invalid").pathname.slice(1);
        const bytes = assets.get(name);
        requested.push({ path: name, session: request.headers["x-source-session"] as string | undefined });
        if (!bytes) {
            response.writeHead(404).end();
            return;
        }
        const range = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range ?? "");
        const start = range ? Number(range[1]) : 0;
        const end = range ? Math.min(Number(range[2] || bytes.length - 1), bytes.length - 1) : bytes.length - 1;
        if (start > end || end >= bytes.length) {
            response.writeHead(416).end();
            return;
        }
        const contentType = name.endsWith(".m3u8") ? "application/vnd.apple.mpegurl"
            : name.endsWith(".mpd") ? "application/dash+xml"
            : name.endsWith(".ass") ? "text/x-ass"
            : name.endsWith(".mp4") ? "video/mp4"
            : name.endsWith(".m4a") ? "audio/mp4" : "application/octet-stream";
        response.writeHead(range ? 206 : 200, {
            "Accept-Ranges": "bytes",
            "Content-Length": end - start + 1,
            "Content-Type": contentType,
            ...(range ? { "Content-Range": `bytes ${start}-${end}/${bytes.length}` } : {}),
        });
        response.end(bytes.subarray(start, end + 1));
    });
    await new Promise<void>(resolve => mediaServer!.listen(0, "127.0.0.1", resolve));
    const address = mediaServer.address();
    assert(address && typeof address !== "string");
    const assetUrl = (name: string) => `http://127.0.0.1:${address.port}/${name}`;
    return { url: assetUrl("generated.mp4"), bytes: assets.get("generated.mp4")!, assetUrl, requested };
}

try {
    const generated = await localMedia();
    const mediaUrl = generated.url;
    const page = "https://example.com/watch";

    await bridge.start();
    bridge.forward(1, "browserRegistration", { browser: "edge", registered: true });
    peer = spawn(qcatch, ["--native-messaging", "chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/"], {
        env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery },
        windowsHide: true,
    });
    peer.stderr?.resume();
    peer.once("error", error => errors.push(error));
    const decoder = new FrameDecoder();
    let delayed: number | undefined;
    let cancellationObserved = false;
    peer.stdout?.on("data", chunk => {
        try {
            decoder.push(chunk, value => {
                const command = value as { id: number; method: string; url?: string;
                    selection?: { captureId?: string } };
                if (command.method === "cancel") {
                    cancellationObserved = true;
                    return;
                }
                assert.equal(command.method, "resolve");
                assert.equal(command.url, page);
                const id = command.selection?.captureId;
                assert(id, "Resolve lost the selected capture ID");
                assert([watchId, downloadId, hlsId, dashId, externalId, expiredId, cancelledId].includes(id));
                commands.push(id);
                if (id === cancelledId) {
                    delayed = command.id;
                    return;
                }
                peer!.stdin?.write(encodeFrame(id === expiredId
                    ? { id: command.id, error: { code: "capture_expired", message: "Expired selection" } }
                    : { id: command.id, result: { ...resolvedMedia(id, page, generated),
                        selection: command.selection } }));
            });
        }
        catch (error) {
            errors.push(error as Error);
        }
    });
    peer.stdin?.write(encodeFrame({ type: "hello", browser: "edge", version: 1,
        userAgent: "qbutt-generated-capture/1", capabilities: ["resolve", "cancel"] }));

    await waitFor(async () => (await bridge.status()).ready, "registered peer connection", 15_000);
    assert.equal((await bridge.status()).connectedBrowser, "edge");
    const profile = messages.find(message => typeof message.captureProfile === "string")?.captureProfile;
    assert(profile, "The native fixture did not report its isolated profile");

    openCapture("watch", watchId, page);
    await waitFor(() => {
        const finished = event("watchFinished", watchId);
        assert(!finished?.reason, `Native Watch failed: ${finished?.reason}`);
        return !!finished && !!messages.find(message =>
            message.watchMedia?.identity === `capture:${watchId}`);
    }, "native Watch and gateway");
    assert.equal(event("watchFinished", watchId).reason, "");
    const watchMedia = messages.find(message => message.watchMedia?.identity === `capture:${watchId}`).watchMedia;
    const protectedUrl = new URL(watchMedia.url);
    const authorization = `Basic ${Buffer.from(`${decodeURIComponent(protectedUrl.username)}:${decodeURIComponent(protectedUrl.password)}`).toString("base64")}`;
    protectedUrl.username = "";
    protectedUrl.password = "";
    const gateway = await fetch(protectedUrl, { headers: { Authorization: authorization } });
    assert.equal(gateway.status, 200, "The guarded gateway did not serve the selected MP4");
    const watched = Buffer.from(await gateway.arrayBuffer());
    assert(watched.length > 1024);
    assert.deepEqual(watched, generated.bytes);
    bridge.forward(1, "captureReleaseWatch", { launchToken: watchMedia.launchToken });

    openCapture("download", downloadId, page);
    await waitFor(() => !!event("downloadFinished", downloadId), "durable Download acknowledgement");
    assert.equal(event("downloadFinished", downloadId).reason, "");
    assert.equal(jobs(profile).length, 1, "The acknowledgement requires one durable job");
    await waitFor(() => jobs(profile)[0]?.status === "complete", "download completion");
    const [job] = jobs(profile);
    assert.equal(job.origin.kind, "browser");
    assert.equal(job.origin.source_host, new URL(page).hostname);
    assert(!JSON.stringify(job).includes(downloadId), "Capture ID entered persistent provenance");
    const saved = await readFile(job.path);
    assert.equal((await stat(job.path)).size, saved.length);
    assert.equal(saved.length, job.bytes);
    assert.equal(createHash("sha256").update(saved).digest("hex"),
        createHash("sha256").update(watched).digest("hex"), "Watch and Download received different bytes");

    for (const [captureId, expectedPaths] of [
        [hlsId, ["master.m3u8", "video.m3u8", "audio.m3u8"]],
        [dashId, ["manifest.mpd", "init-stream0.m4s", "init-stream1.m4s"]],
        [externalId, ["generated.mp4", "audio.m4a", "accent.ass"]],
    ] as const) {
        const previous = new Set(jobs(profile).map(job => job.id));
        const requestStart = generated.requested.length;
        openCapture("download", captureId, page);
        await waitFor(() => !!event("downloadFinished", captureId), "segmented durable acknowledgement");
        assert.equal(event("downloadFinished", captureId).reason, "");
        const newlyQueued = jobs(profile).filter(job => !previous.has(job.id));
        assert.equal(newlyQueued.length, 1, "Acknowledgement did not persist exactly one job");
        const jobId = newlyQueued[0].id;
        await waitFor(() => jobs(profile).some(job => job.id === jobId && job.status === "complete"),
            "segmented download completion");
        const complete = jobs(profile).find(job => job.id === jobId)!;
        assert.equal(complete.origin.kind, "browser");
        assert.equal(complete.origin.source_host, new URL(page).hostname);
        assert(complete.path.endsWith(".mkv"));
        assert.equal((await stat(complete.path)).size, complete.bytes);
        assert(!JSON.stringify(complete).includes(captureId));
        assert(!JSON.stringify(complete).includes(fixtureSession));
        assert(!JSON.stringify(complete).includes("127.0.0.1"), "Acquisition URL persisted in job");

        const requests = generated.requested.slice(requestStart);
        for (const path of expectedPaths)
            assert(requests.some(request => request.path === path), `Missing ${path} child request`);
        assert(requests.every(request => request.session === fixtureSession),
            "Manifest and child requests did not share the selected session");
        assert(!requests.some(request => request.path === "missing.m3u8"),
            "The unselected HLS rendition was fetched");

        const probe = JSON.parse((await mediaCommand(ffprobe, ["-v", "error", "-count_packets",
            "-show_streams", "-of", "json", complete.path])).toString()) as {
            streams: { codec_type: string; codec_name: string; nb_read_packets?: string }[] };
        for (const kind of ["video", "audio"])
            assert(probe.streams.some(stream => stream.codec_type === kind
                && Number(stream.nb_read_packets) > 0), `Saved ${kind} stream has no packets`);
        await mediaCommand(ffmpeg, ["-v", "error", "-i", complete.path, "-f", "null", "-"]);
        if (captureId === externalId) {
            assert(probe.streams.some(stream => stream.codec_type === "subtitle"
                && stream.codec_name === "ass" && Number(stream.nb_read_packets) > 0));
            const subtitle = join(root, "extracted.ass");
            await mediaCommand(ffmpeg, ["-y", "-v", "error", "-i", complete.path,
                "-map", "0:s:0", "-c:s", "copy", "-f", "ass", subtitle]);
            assert((await readFile(subtitle, "utf8")).includes("Style: Accent"));
        }
        await waitFor(() => messages.some(message => message.downloadSnapshot?.jobs.some(row =>
            row.id === jobId && row.status === "complete")
            && message.downloadSnapshot.pageCount >= previous.size + 1), "Qt Downloads state");
        const snapshot = messages.findLast(message => message.downloadSnapshot?.jobs.some(row =>
            row.id === jobId && row.status === "complete"))!.downloadSnapshot!;
        const row = snapshot.jobs.find(row => row.id === jobId)!;
        assert.equal(snapshot.state, "ready");
        assert.equal(row.path, complete.path);
        assert.equal(row.bytes, complete.bytes);
    }

    openCapture("download", expiredId, page);
    await waitFor(() => !!event("downloadFinished", expiredId), "expired capture response");
    assert.equal(event("downloadFinished", expiredId).reason, "capture");
    assert.equal(jobs(profile).length, 4, "Expired capture queued a fallback download");

    openCapture("download", cancelledId, page);
    await waitFor(() => delayed !== undefined, "pending capture to cancel");
    bridge.forward(1, "captureCancel");
    await waitFor(() => messages.some(message => message.captureCancelled), "native cancellation");
    await waitFor(() => cancellationObserved, "broker cancellation");
    peer.stdin?.write(encodeFrame({ id: delayed, result: { kind: "video", resolver: "direct",
        url: mediaUrl, sourceUrl: page, selection: { captureId: cancelledId } } }));
    await Bun.sleep(500);
    assert.equal(jobs(profile).length, 4, "Cancelled capture queued a download");
    assert.equal(messages.filter(message => message.captureCompleted).length, 5,
        "Only Watch and accepted Downloads may complete");
    assert.equal(messages.filter(message => message.captureError).length, 1,
        "Only the expired capture may report an error");
    assert.deepEqual(commands, [watchId, downloadId, hlsId, dashId, externalId, expiredId, cancelledId]);
    assert.deepEqual(errors, []);

    console.log(JSON.stringify({ status: "passed", media: "generated",
        bytes: saved.length, sha256: createHash("sha256").update(saved).digest("hex"),
        scenarios: ["native-browser-peer", "exact-watch-gateway", "durable-download-ack",
            "one-complete-file", "same-media-bytes", "hls", "static-dash", "external-audio-ass",
            "qt-download-state", "expired-no-fallback", "cancelled-no-queue"] }));
}
finally {
    if (peer && peer.exitCode === null) {
        peer.stdin?.end();
        await Promise.race([new Promise(resolve => peer!.once("exit", resolve)),
            Bun.sleep(10_000).then(() => peer!.kill())]);
    }
    await bridge.close();
    if (mediaServer) {
        mediaServer.closeAllConnections();
        await new Promise<void>(resolve => mediaServer!.close(() => resolve()));
    }
    const temporary = await realpath(tmpdir());
    const target = await realpath(root);
    assert(!relative(temporary, target).startsWith("..") && target !== temporary);
    await rm(root, { recursive: true, force: true });
}
