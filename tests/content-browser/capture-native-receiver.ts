import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { createServer, type Server } from "node:http";
import { mkdtemp, readFile, realpath, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";

import { Database } from "bun:sqlite";

import { NativeBrowserFixture } from "./native-parent";
import { encodeFrame, FrameDecoder } from "./framing";

const qcatch = process.env.QBUTT_QCATCH_EXE;
assert(qcatch, "Set QBUTT_QCATCH_EXE to the independent qcatch native host");

type Outcome = { captureId: string; reason: string };
type FixtureMessage = {
    watchFinished?: Outcome;
    downloadFinished?: Outcome;
    watchMedia?: { identity: string; url: string; launchToken: number };
    captureProfile?: string;
    captureCompleted?: boolean;
    captureCancelled?: boolean;
    captureError?: string;
};
type DownloadJob = {
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
const expiredId = crypto.randomUUID();
const cancelledId = crypto.randomUUID();
const commands: string[] = [];
const errors: Error[] = [];
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

function event(name: "watchFinished" | "downloadFinished", captureId: string) {
    return messages.map(message => message[name]).find(value => value?.captureId === captureId);
}

function openCapture(action: "watch" | "download", captureId: string, page: string) {
    const payload = { v: 1, pageUrl: page, title: "Generated legal capture", action, captureId };
    const url = `qbutt://capture?d=${Buffer.from(JSON.stringify(payload)).toString("base64url")}`;
    bridge.forward(1, "captureOpen", { url });
}

function jobs(profile: string): DownloadJob[] {
    const database = new Database(join(profile, "content-library.sqlite"), { readonly: true });
    try {
        return database.query("SELECT snapshot FROM download_jobs").all()
            .map(row => JSON.parse((row as { snapshot: string }).snapshot) as DownloadJob);
    }
    finally {
        database.close();
    }
}

async function localMedia() {
    const ffmpeg = process.env.QBUTT_LAB_FFMPEG;
    assert(ffmpeg, "Set QBUTT_LAB_FFMPEG for the generated local media fixture");
    const path = join(root, "generated.mp4");
    const generated = Bun.spawn([ffmpeg, "-hide_banner", "-loglevel", "error",
        "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=12", "-t", "3",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", path], { stderr: "pipe" });
    const [exit, error] = await Promise.all([generated.exited, new Response(generated.stderr).text()]);
    assert.equal(exit, 0, error);
    const bytes = await readFile(path);

    mediaServer = createServer((request, response) => {
        if (request.url !== "/generated.mp4") {
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
        response.writeHead(range ? 206 : 200, {
            "Accept-Ranges": "bytes",
            "Content-Length": end - start + 1,
            "Content-Type": "video/mp4",
            ...(range ? { "Content-Range": `bytes ${start}-${end}/${bytes.length}` } : {}),
        });
        response.end(bytes.subarray(start, end + 1));
    });
    await new Promise<void>(resolve => mediaServer!.listen(0, "127.0.0.1", resolve));
    const address = mediaServer.address();
    assert(address && typeof address !== "string");
    return { url: `http://127.0.0.1:${address.port}/generated.mp4`, bytes };
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
                assert([watchId, downloadId, expiredId, cancelledId].includes(id));
                commands.push(id);
                if (id === cancelledId) {
                    delayed = command.id;
                    return;
                }
                peer!.stdin?.write(encodeFrame(id === expiredId
                    ? { id: command.id, error: { code: "capture_expired", message: "Expired selection" } }
                    : { id: command.id, result: { kind: "video", resolver: "direct",
                        url: mediaUrl, sourceUrl: page, selection: command.selection } }));
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

    openCapture("download", expiredId, page);
    await waitFor(() => !!event("downloadFinished", expiredId), "expired capture response");
    assert.equal(event("downloadFinished", expiredId).reason, "capture");
    assert.equal(jobs(profile).length, 1, "Expired capture queued a fallback download");

    openCapture("download", cancelledId, page);
    await waitFor(() => delayed !== undefined, "pending capture to cancel");
    bridge.forward(1, "captureCancel");
    await waitFor(() => messages.some(message => message.captureCancelled), "native cancellation");
    await waitFor(() => cancellationObserved, "broker cancellation");
    peer.stdin?.write(encodeFrame({ id: delayed, result: { kind: "video", resolver: "direct",
        url: mediaUrl, sourceUrl: page, selection: { captureId: cancelledId } } }));
    await Bun.sleep(500);
    assert.equal(jobs(profile).length, 1, "Cancelled capture queued a download");
    assert.equal(messages.filter(message => message.captureCompleted).length, 2,
        "Only Watch and Download may complete");
    assert.equal(messages.filter(message => message.captureError).length, 1,
        "Only the expired capture may report an error");
    assert.deepEqual(commands, [watchId, downloadId, expiredId, cancelledId]);
    assert.deepEqual(errors, []);

    console.log(JSON.stringify({ status: "passed", media: "generated",
        bytes: saved.length, sha256: createHash("sha256").update(saved).digest("hex"),
        scenarios: ["native-browser-peer", "exact-watch-gateway", "durable-download-ack",
            "one-complete-file", "same-media-bytes", "expired-no-fallback", "cancelled-no-queue"] }));
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
