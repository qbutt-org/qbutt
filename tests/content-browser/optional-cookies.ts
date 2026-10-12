import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { NativeBrowserFixture } from "./native-parent";
import { FrameDecoder, encodeFrame } from "./framing";
import type { BrowserCommand } from "../../components/content/browser/protocol";

const qcatch = process.env.QBUTT_QCATCH_EXE;
assert(qcatch);
const directory = await mkdtemp(join(tmpdir(), "qbutt-optional-cookies-"));
const discovery = join(directory, "browser.json");
const bridge = new NativeBrowserFixture({ browser: "edge", discoveryPath: discovery });
const peers: ReturnType<typeof spawn>[] = [];
let other: NativeBrowserFixture | undefined;

async function waitFor(condition: () => boolean | Promise<boolean>) {
    const deadline = Date.now() + 5_000;
    while (!await condition()) {
        assert(Date.now() < deadline, "Cookie scenario timed out");
        await Bun.sleep(10);
    }
}

function extension(browser: "edge" | "chrome") {
    const child = spawn(qcatch!, ["--native-messaging", "chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/"], {
        env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery }, windowsHide: true,
    });
    peers.push(child);
    child.stderr.resume();
    const decoder = new FrameDecoder();
    const messages: BrowserCommand[] = [];
    child.stdout.on("data", chunk => decoder.push(chunk, value => messages.push(value as BrowserCommand)));
    const send = (value: unknown) => child.stdin.write(encodeFrame(value));
    send({ type: "hello", browser, version: 1, userAgent: `qbutt-optional-${browser}/1`, capabilities: ["cookies", "resolve", "cancel"] });
    return { child, messages, send };
}

try {
    await bridge.start();
    const first = extension("edge");
    await waitFor(async () => (await bridge.status()).ready);
    const status = bridge.status.bind(bridge);
    let once = true;
    bridge.status = async (signal, initialize) => {
        const snapshot = await status(signal, initialize);
        if (once) {
            once = false;
            first.child.stdin.end();
            await waitFor(async () => !(await status()).ready);
        }
        return snapshot;
    };
    const started = performance.now();
    assert.deepEqual(await bridge.cookies("https://example.com/"), []);
    const disconnectedMilliseconds = performance.now() - started;
    assert(disconnectedMilliseconds < 2_000, "Optional cookies must not wait for reconnection after stale ready status");
    bridge.status = status;
    const edge = extension("edge");
    await waitFor(async () => (await bridge.status()).ready);
    const invalid = bridge.cookies("https://example.com/");
    const rejected = assert.rejects(invalid, (error: any) => error.code === "browser_invalid_response");
    await waitFor(() => edge.messages.length === 1);
    edge.send({ id: edge.messages.shift()!.id, result: [{ name: "session", value: "generated", domain: "another.example.com",
        path: "/", secure: true, hostOnly: true }] });
    await rejected;
    bridge.setBrowser("auto");
    extension("chrome");
    await waitFor(async () => !(await bridge.status()).ready);
    assert.deepEqual(await bridge.cookies("https://example.com/"), []);
    assert.equal(edge.messages.length, 0);
    await assert.rejects(bridge.resolve("https://example.com/watch"), (error: any) => error.code === "browser_choice_required");
    const owned = JSON.parse(await readFile(discovery, "utf8"));
    other = new NativeBrowserFixture({ discoveryPath: discovery });
    await other.start(false);
    await assert.rejects(other.status(undefined, true), (error: any) => error.code === "browser_owner_exists");
    assert.deepEqual(await other.cookies("https://example.com/"), []);
    await other.close();
    assert.deepEqual(JSON.parse(await readFile(discovery, "utf8")), owned);
    console.log(JSON.stringify({ ok: true, disconnectedMilliseconds,
        scenarios: ["stale-ready-disconnect-no-reconnect-wait", "invalid-cookie-response-not-swallowed", "auto-ambiguous-import-skipped",
            "mandatory-ambiguity-preserved", "other-owner-optional-import-skipped", "foreign-owner-discovery-preserved"] }));
}
finally {
    for (const peer of peers) if (peer.exitCode === null) peer.kill();
    await other?.close();
    await bridge.close();
    await rm(directory, { recursive: true, force: true });
}
