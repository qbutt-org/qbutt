import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { NativeBrowserFixture } from "./native-parent";
import { FrameDecoder, encodeFrame } from "./framing";
import { openLink } from "../../components/content/sharing";
import type { BrowserCommand } from "../../components/content/browser/protocol";

const qcatch = process.env.QBUTT_QCATCH_EXE;
assert(qcatch, "Set QBUTT_QCATCH_EXE to the independent native host");
const root = await mkdtemp(join(tmpdir(), "qbutt-helper-relay-"));
const discovery = join(root, "browser.json");
const parent = new NativeBrowserFixture({ browser: "edge", discoveryPath: discovery });
const children: ReturnType<typeof spawn>[] = [];
const commands: BrowserCommand[] = [];
const replies: { generation: number; id: number }[] = [];
let generation = 1;
let helper: ReturnType<typeof spawn> | undefined;

async function waitFor(condition: () => boolean | Promise<boolean>, operation: string) {
    const deadline = Date.now() + 10_000;
    while (!await condition()) {
        assert(Date.now() < deadline, `${operation} timed out`);
        await Bun.sleep(10);
    }
}

function startHelper(choice: "edge" | "chrome") {
    const session = ++generation;
    const process = spawn(Bun.which("bun")!, [join(import.meta.dir, "../../components/content/main.ts"),
        "--data-dir", join(root, `helper-${session}`), "--app-dir", root, "--download-dir", join(root, "downloads"),
        "--browser-choice", choice],
        { windowsHide: true });
    children.push(process);
    const messages: { id: number; event: string; data?: any }[] = [];
    let input = "";
    process.stderr.resume();
    process.stdout.setEncoding("utf8");
    process.stdout.on("data", (chunk: string) => {
        input += chunk;
        let end: number;
        while ((end = input.indexOf("\n")) >= 0) {
            const message = JSON.parse(input.slice(0, end));
            input = input.slice(end + 1);
            if (message.id === 0 && ["browserRequest", "browserCancel"].includes(message.event))
                parent.forward(session, message.event, message.data);
            else
                messages.push(message);
        }
    });
    process.once("exit", () => parent.forward(session, "browserSessionClosed"));
    return { process, messages, send(id: number, method: string, params = {}) {
        process.stdin.write(JSON.stringify({ id, method, params }) + "\n");
    } };
}

try {
    await parent.start();
    const owned = JSON.parse(await readFile(discovery, "utf8"));
    const peer = spawn(qcatch, ["--native-messaging", "chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/"], {
        env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery }, windowsHide: true,
    });
    children.push(peer);
    peer.stderr.resume();
    const decoder = new FrameDecoder();
    peer.stdout.on("data", chunk => decoder.push(chunk, value => commands.push(value as BrowserCommand)));
    const sendPeer = (value: unknown) => peer.stdin.write(encodeFrame(value));
    sendPeer({ type: "hello", browser: "edge", version: 1, userAgent: "qbutt-helper-relay/1",
        capabilities: ["cookies", "resolve", "cancel"] });
    await waitFor(async () => (await parent.status()).ready, "extension connection");
    parent.onSessionReply = (session, response) => {
        replies.push({ generation: session, id: response.id });
        if (session === generation && helper?.exitCode === null && helper.signalCode === null)
            helper?.stdin.write(JSON.stringify({ browserReply: response }) + "\n");
    };
    let current = startHelper("chrome");
    helper = current.process;
    current.send(5, "chooseBrowser", { browser: "edge" });
    await waitFor(() => current.messages.some(message => message.id === 5 && message.event === "complete"), "live browser choice");
    assert.equal((await parent.status()).connectedBrowser, "edge");
    const captureId = crypto.randomUUID();
    const capture = openLink("qbutt://capture?d=" + Buffer.from(JSON.stringify({ v: 1,
        pageUrl: "https://example.com/watch", title: "Generated capture", action: "watch", captureId })).toString("base64url"));
    current.send(2, "resolve", { variant: capture.variant });
    await waitFor(() => commands.some(command => command.method === "resolve"), "helper resolve forwarding");
    const active = commands.find(command => command.method === "resolve")!;
    current.send(3, "cancel", { id: 2 });
    await waitFor(() => current.messages.some(message => message.id === 2 && message.event === "error"), "terminal cancel response");
    await waitFor(() => commands.some(command => command.method === "cancel" && command.id === active.id), "extension cancel frame");
    // A late peer response must neither revive the request nor resolve a new helper's ID.
    sendPeer({ id: active.id, result: { kind: "video", resolver: "direct",
        url: "https://example.com/generated.mp4", selection: { captureId } } });
    await Bun.sleep(100);
    assert.equal(current.messages.filter(message => message.id === 2 && ["complete", "error"].includes(message.event)).length, 1);
    current.process.stdin.end();
    await waitFor(() => current.process.exitCode !== null, "first helper shutdown");
    assert.equal(current.process.exitCode, 0);
    current = startHelper("edge");
    helper = current.process;
    assert.equal((await parent.status()).connectedBrowser, "edge");
    assert.deepEqual(JSON.parse(await readFile(discovery, "utf8")), owned, "Helper restart must retain the sole native broker");
    current.send(2, "resolve", { variant: capture.variant });
    await waitFor(() => commands.filter(command => command.method === "resolve").length === 2, "restarted helper resolve");
    const second = commands.filter(command => command.method === "resolve")[1]!;
    assert.notEqual(second.id, active.id);
    current.send(5, "chooseBrowser", { browser: "chrome" });
    await waitFor(() => current.messages.some(message => message.id === 5 && message.event === "complete"), "choice during request");
    sendPeer({ id: second.id, result: { kind: "video", resolver: "direct", url: "https://example.com/generated.mp4", selection: { captureId } } });
    await waitFor(() => current.messages.some(message => message.id === 2 && message.event === "complete"), "exact capture terminal response");
    assert.equal(current.messages.filter(message => message.id === 2 && message.event === "resolved").length, 1);
    current.send(6, "resolve", { variant: capture.variant });
    await waitFor(() => current.messages.some(message => message.id === 6 && message.event === "error"), "new choice without connection");
    assert.equal(commands.filter(command => command.method === "resolve").length, 2,
        "Chrome request was sent to the old Edge session");
    current.send(7, "chooseBrowser", { browser: "edge" });
    await waitFor(() => current.messages.some(message => message.id === 7 && message.event === "complete"), "restore browser choice");
    current.send(4, "resolve", { variant: capture.variant });
    await waitFor(() => commands.filter(command => command.method === "resolve").length === 3, "request before helper crash");
    const abandoned = commands.filter(command => command.method === "resolve")[2]!;
    current.process.kill();
    await waitFor(() => current.process.exitCode !== null || current.process.signalCode !== null, "crashed helper exit");
    await waitFor(() => commands.some(command => command.method === "cancel" && command.id === abandoned.id), "helper exit cancels native request");
    current = startHelper("edge");
    helper = current.process;
    sendPeer({ id: abandoned.id, result: { kind: "video", resolver: "direct", url: "https://example.com/generated.mp4", selection: { captureId } } });
    await Bun.sleep(100);
    assert.equal(current.messages.filter(message => ["complete", "error"].includes(message.event)).length, 0,
        "Late response must not reach the restarted helper");
    current.process.stdin.end();
    await waitFor(() => current.process.exitCode !== null, "restarted helper shutdown");
    assert.equal(current.process.exitCode, 0);
    peer.stdin.end();
    await parent.close();
    await assert.rejects(readFile(discovery), { code: "ENOENT" });
    console.log(JSON.stringify({ ok: true, scenarios: [
        "native-parent-helper-stdio", "browser-choice-startup-and-live", "inflight-session-pinned",
        "exact-capture", "cancel-to-native-host", "one-terminal-response", "late-response-isolated",
        "helper-restart-single-owner", "helper-crash-cancels-native-request",
        "crash-restart-late-response-isolated", "owned-discovery-cleanup",
    ] }));
}
finally {
    for (const child of children) if (child.exitCode === null) child.kill();
    await parent.close();
    await rm(root, { recursive: true, force: true });
}
