import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { NativeBrowserFixture } from "./native-parent";
import { FrameDecoder, encodeFrame } from "./framing";
import type { BrowserCommand } from "../../components/content/browser/protocol";

const qcatch = process.env.QBUTT_QCATCH_EXE;
assert(qcatch, "Set QBUTT_QCATCH_EXE to the independent native host");
const directory = await mkdtemp(join(tmpdir(), "qbutt-browser-startup-"));
const discovery = join(directory, "bridge.json");
const parent = new NativeBrowserFixture({ browser: "edge", discoveryPath: discovery });
const captureId = crypto.randomUUID();
const source = "https://example.com/watch";
const selection = { captureId };
let peer: ReturnType<typeof spawn> | undefined;
let reconnect: ReturnType<typeof setTimeout> | undefined;
let commands = 0;

try {
    await parent.start(false);
    await assert.rejects(readFile(discovery), { code: "ENOENT" });
    parent.forward(1, "browserInitialize");
    await parent.status();
    const missingStarted = performance.now();
    await assert.rejects(parent.resolve(source, selection), (error: any) => error.code === "browser_required");
    const missingMilliseconds = performance.now() - missingStarted;
    assert(missingMilliseconds < 2_000, "Proven missing native host must not wait for reconnect");
    parent.forward(1, "browserRegistration", { browser: "edge", registered: true });
    await parent.status();
    const started = performance.now();
    const result = parent.resolve(source, selection);
    reconnect = setTimeout(() => {
        peer = spawn(qcatch, ["--native-messaging", "chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/"], {
            env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery }, windowsHide: true,
        });
        peer.stderr.resume();
        const decoder = new FrameDecoder();
        peer.stdout.on("data", chunk => decoder.push(chunk, value => {
            const command = value as BrowserCommand;
            assert.equal(command.method, "resolve");
            assert.equal(command.url, source);
            assert.deepEqual(command.selection, selection);
            ++commands;
            peer!.stdin.write(encodeFrame({ id: command.id, result: { kind: "video", resolver: "direct",
                url: "https://example.com/generated.mp4", selection } }));
        }));
        peer.stdin.write(encodeFrame({ type: "hello", browser: "edge", version: 1, userAgent: "qcatch-delayed-startup/1",
            capabilities: ["resolve", "cancel"] }));
    }, 31_000);
    assert.equal((await result).url, "https://example.com/generated.mp4");
    const reconnectMilliseconds = performance.now() - started;
    assert(reconnectMilliseconds >= 30_000 && reconnectMilliseconds < 45_000);
    assert.equal(commands, 1, "Automatic reconnect must dispatch the original capture exactly once");
    peer!.stdin.end();
    await parent.close();
    await assert.rejects(readFile(discovery), { code: "ENOENT" });
    console.log(JSON.stringify({ ok: true, missingMilliseconds, reconnectMilliseconds,
        scenarios: ["status-does-not-create-owner", "production-async-initialization", "missing-host-immediate-action", "registered-host-delayed-over-one-alarm", "first-exact-capture-no-manual-retry", "one-command", "owned-cleanup"] }));
}
finally {
    clearTimeout(reconnect);
    if (peer?.exitCode === null) peer.kill();
    await parent.close();
    await rm(directory, { recursive: true, force: true });
}
