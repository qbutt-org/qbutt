import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { BrowserBridge } from "../../components/content/browser/index";
import type { BrowserCommand } from "../../components/content/browser/protocol";
import { Catalog } from "../../components/content/catalog";
import { DownloadQueue, type DownloadJob } from "../../components/content/downloads";
import { resolveVariant } from "../../components/content/media";
import { mediaRuntime } from "../../components/content/media-runtime";
import { captureSelectionId } from "../../components/content/references";
import { SourceSettings } from "../../components/content/settings";
import { openLink, share } from "../../components/content/sharing";
import { LibraryStore } from "../../components/content/store";
import { encodeFrame, FrameDecoder } from "./framing";

const qcatch = process.env.QBUTT_QCATCH_EXE;
assert(qcatch, "Set QBUTT_QCATCH_EXE to the independent qcatch fixture binary");
process.env.QBUTT_LAB_PATHS = "1";
const root = await mkdtemp(join(tmpdir(), "qbutt-capture-downloads-"));
const discovery = join(root, "browser.json");
const dataDir = join(root, "data");
const downloadDir = join(root, "downloads");
const { tools: { ffmpeg } } = await mediaRuntime(root, AbortSignal.timeout(30_000));
const firstId = crypto.randomUUID();
const secondId = crypto.randomUUID();
const expiredId = crypto.randomUUID();
const requested: string[] = [];
const resolved: string[] = [];
const media = new Map<string, Buffer>();

async function command(args: string[]): Promise<string> {
    const child = Bun.spawn([ffmpeg, ...args], { cwd: root, windowsHide: true, timeout: 30000,
        stdout: "pipe", stderr: "pipe" });
    const [exit, output, error] = await Promise.all([child.exited,
        new Response(child.stdout).text(), new Response(child.stderr).text()]);
    assert.equal(exit, 0, error);
    return output;
}

for (const [id, color] of [[firstId, "blue"], [secondId, "green"]]) {
    const path = join(root, `${id}.mp4`);
    await command(["-v", "error", "-f", "lavfi", "-i", `color=c=${color}:s=160x90:r=12:d=1`,
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an", path]);
    media.set(id!, await readFile(path));
}
const server = createServer((request, response) => {
    const id = new URL(request.url!, "http://example.com").pathname.slice(1).replace(/\.mp4$/, "");
    requested.push(id);
    const payload = media.get(id);
    if (!payload) return void response.writeHead(404).end();
    response.writeHead(200, { "Content-Length": payload.length, "Content-Type": "video/mp4" });
    response.end(payload);
});
await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
const address = server.address();
assert(address && typeof address !== "string");
const origin = `http://example.com:${address.port}`;
const bridge = new BrowserBridge({ browser: "edge", discoveryPath: discovery });
const store = await LibraryStore.open(dataDir);
const catalog = new Catalog(dataDir, root, new SourceSettings(store), bridge);
let release!: () => void;
const gate = new Promise<void>(resolve => { release = resolve; });
let entered = false;
const queue = new DownloadQueue({ dataDir, downloadDir, appDir: root, store,
    network: { async addresses(url) {
        assert.equal(url.origin, origin, "The fixture cannot contact another source");
        return [{ address: "127.0.0.1", family: 4 }];
    } },
    async resolve(variant, signal) {
        if (captureSelectionId(variant) === firstId) {
            entered = true;
            await gate;
        }
        return resolveVariant(variant, catalog, signal, bridge, "");
    },
    emit() {},
});
let native: ReturnType<typeof spawn> | undefined;
async function waitFor(predicate: () => boolean, label: string) {
    const deadline = Date.now() + 30000;
    while (!predicate()) {
        assert(Date.now() < deadline, `Timed out: ${label}`);
        await Bun.sleep(20);
    }
}
function capture(captureId: string) {
    const data = { v: 1, pageUrl: "https://example.com/watch", title: "Generated capture", action: "download", captureId };
    return openLink(`qbutt://capture?d=${Buffer.from(JSON.stringify(data)).toString("base64url")}`);
}
async function completed(id: string): Promise<DownloadJob> {
    await waitFor(() => !["queued", "downloading"].includes(queue.jobs().find(job => job.id === id)!.status), "capture download");
    return queue.jobs().find(job => job.id === id)!;
}
try {
    await bridge.start();
    native = spawn(qcatch, ["--native-messaging", "chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/"], {
        env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery }, windowsHide: true,
    });
    const decoder = new FrameDecoder();
    native.stdout!.on("data", chunk => decoder.push(chunk, value => {
        const message = value as BrowserCommand;
        assert.equal(message.method, "resolve");
        const id = message.selection?.captureId;
        assert(id);
        resolved.push(id);
        native!.stdin!.write(encodeFrame(id === expiredId
            ? { id: message.id, error: { code: "capture_expired", message: "This capture expired. Choose the media again in qcatch." } }
            : { id: message.id, result: { kind: "video", resolver: "direct", url: `${origin}/${id}.mp4`, selection: message.selection } }));
    }));
    native.stdin!.write(encodeFrame({ type: "hello", browser: "edge", version: 1, userAgent: "qbutt-capture-fixture/1" }));
    await waitFor(() => bridge.status().ready, "real native relay hello");

    const first = capture(firstId);
    const firstJob = queue.enqueue({ work: first.work, variant: first.variant, title: "Capture A" });
    await waitFor(() => entered, "blocked capture A");
    const second = capture(secondId);
    const secondJob = queue.enqueue({ work: second.work, variant: second.variant, title: "Capture B" });
    assert.equal(first.work.id, second.work.id, "The page must retain one work grouping");
    assert.notEqual(first.variant!.id, second.variant!.id, "Each selected capture needs its own identity");
    assert.equal(captureSelectionId(store.setting<DownloadJob[]>("downloadJobs", [])[0]!.variant!), firstId);
    assert.throws(() => share({ mode: "exact", work: first.work, variant: first.variant }), /current browser session/);
    release();
    const [savedFirst, savedSecond] = await Promise.all([completed(firstJob.id), completed(secondJob.id)]);
    assert.equal(savedFirst.status, "complete", savedFirst.message);
    assert.equal(savedSecond.status, "complete", savedSecond.message);
    assert.deepEqual(resolved, [firstId, secondId]);
    assert.deepEqual(requested, [firstId, secondId]);
    for (const [id, saved] of [[firstId, savedFirst], [secondId, savedSecond]] as const) {
        const frame = ["-v", "error", "-i", saved.path!, "-map", "0:v:0", "-frames:v", "1", "-f", "framemd5", "-"];
        const actual = await command(frame);
        frame[3] = join(root, `${id}.mp4`);
        assert.equal(actual, await command(frame), "The saved file must decode the selected capture's exact frame");
    }
    const expired = capture(expiredId);
    const failed = await completed(queue.enqueue({ work: expired.work, variant: expired.variant }).id);
    assert.equal(failed.status, "error");
    assert.equal(failed.code, "capture_expired");
    assert.match(failed.message || "", /expired.*Choose the media again/);
    assert.equal(failed.path, undefined);
    assert.deepEqual(requested, [firstId, secondId], "Expired capture must make no fallback payload request");
    console.log(JSON.stringify({ status: "passed", scenarios: ["real-qcatch-native-relay", "queued-capture-identity",
        "same-page-work-grouping", "persisted-selection", "exact-decoded-payloads", "session-exact-share-refused", "expired-no-fallback"] }));
}
finally {
    release();
    await queue.close();
    native?.kill();
    await bridge.close();
    store.close();
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
    await rm(root, { recursive: true, force: true });
}
