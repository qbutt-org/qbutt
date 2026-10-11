import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { BrowserBridge } from "../../components/content/browser/index.ts";
import { encodeFrame, FrameDecoder } from "../../components/content/browser/framing.ts";
import { CHROMIUM_EXTENSION_ID, MAX_MESSAGE_BYTES, type BrowserCommand } from "../../components/content/browser/protocol.ts";

if (process.platform !== "win32") throw new Error("This integration scenario requires Windows named pipes.");
const directory = await mkdtemp(join(tmpdir(), "qbutt-browser-ipc-"));
const discovery = join(directory, "bridge.json");
const bridge = new BrowserBridge({ browser: "edge", discoveryPath: discovery });
const children: ReturnType<typeof spawn>[] = [];
const waitFor = async (predicate: () => boolean) => {
    for (let attempt = 0; attempt < 100; attempt++) {
        if (predicate()) return;
        await Bun.sleep(50);
    }
    throw new Error("Integration deadline elapsed");
};

function extension(origin = `chrome-extension://${CHROMIUM_EXTENSION_ID}/`) {
    const child = spawn(process.execPath, [join(import.meta.dir, "native-runner.ts"), "--native-messaging", origin], {
        env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery }, windowsHide: true,
    });
    children.push(child);
    const messages: BrowserCommand[] = [];
    const decoder = new FrameDecoder();
    child.stdout.on("data", (chunk) => decoder.push(chunk, (message) => messages.push(message as BrowserCommand)));
    return { child, messages, send: (message: unknown) => child.stdin.write(encodeFrame(message)) };
}

try {
    await bridge.start();
    assert.equal(bridge.status().error, undefined, "Secure broker must start");
    const pipe = JSON.parse(await readFile(discovery, "utf8")).pipe as string;
    const securityScript = `
        $client = [IO.Pipes.NamedPipeClientStream]::new('.', '${pipe.split("\\").at(-1)}', [IO.Pipes.PipeDirection]::InOut)
        try {
            $client.Connect(5000)
            $acl = $client.GetAccessControl()
            $rules = $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])
            @{
                owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
                current = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
                identities = @($rules | ForEach-Object { $_.IdentityReference.Value })
                protected = $acl.AreAccessRulesProtected
            } | ConvertTo-Json -Compress
        } finally { $client.Dispose() }
    `;
    const security = JSON.parse((await promisify(execFile)("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", securityScript], { windowsHide: true })).stdout);
    assert.equal(security.owner, security.current);
    assert.equal(security.protected, true);
    assert.deepEqual(security.identities, [security.current], "Cookies pipe must grant access only to the current user");
    assert.deepEqual(await bridge.cookies("https://fixture.qbutt.test/"), [], "Public sources require no extension");
    await assert.rejects(bridge.cookies("http://100.64.0.1/source"), /публичный/);
    const first = extension();
    first.send({ type: "hello", browser: "edge", version: 1, userAgent: "qbutt-ipc-fixture/1" });
    await waitFor(() => bridge.status().ready);
    assert.equal(bridge.status().userAgent, "qbutt-ipc-fixture/1");

    const cookiePromise = bridge.cookies("https://fixture.qbutt.test/source");
    await waitFor(() => first.messages.length === 1);
    first.send({ id: first.messages.shift()!.id, result: [{ name: "session", value: "fixture-session", domain: "fixture.qbutt.test", path: "/", secure: true }] });
    assert.equal((await cookiePromise)[0]?.value, "fixture-session");

    const fetchPromise = bridge.fetch("https://fixture.qbutt.test/source");
    await waitFor(() => first.messages.length === 1);
    first.send({ id: first.messages.shift()!.id, result: { status: 200, headers: { "content-type": "text/html" }, body: "<h1>Legal generated fixture — Привет 世界 🐈</h1>", url: "https://fixture.qbutt.test/source" } });
    const response = await fetchPromise;
    assert.equal(response.url, "https://fixture.qbutt.test/source");
    assert.match(await response.text(), /Привет 世界 🐈/);

    const selection = { season: "1", episode: "2", translationId: "17", player: "alloha" };
    const resolvePromise = bridge.resolve("https://fixture.qbutt.test/source", selection);
    await waitFor(() => first.messages.length === 1);
    first.send({ id: first.messages.shift()!.id, result: { kind: "video", resolver: "direct", url: "https://fixture.qbutt.test/episode-2-voice-17.m3u8", selection } });
    assert.match((await resolvePromise).url!, /episode-2-voice-17/);

    const wrongPromise = bridge.resolve("https://fixture.qbutt.test/source", selection);
    const wrongCheck = assert.rejects(wrongPromise, /another media selection/);
    await waitFor(() => first.messages.length === 1);
    first.send({ id: first.messages.shift()!.id, result: { kind: "video", resolver: "direct", url: "https://fixture.qbutt.test/episode-1.m3u8", selection: { ...selection, episode: "1" } } });
    await wrongCheck;

    const controller = new AbortController();
    const cancelPromise = bridge.fetch("https://fixture.qbutt.test/slow", {}, { signal: controller.signal });
    const cancelCheck = assert.rejects(cancelPromise);
    await waitFor(() => first.messages.length === 1);
    const canceledId = first.messages.shift()!.id;
    controller.abort();
    await cancelCheck;
    await waitFor(() => first.messages.length === 1);
    assert.deepEqual(first.messages.shift(), { id: canceledId, method: "cancel" });

    const malformed = extension();
    const oversized = Buffer.alloc(4);
    oversized.writeUInt32LE(MAX_MESSAGE_BYTES + 1);
    malformed.child.stdin.write(oversized);
    await waitFor(() => malformed.child.exitCode !== null);
    assert.equal(malformed.child.exitCode, 1, "Oversized frame must fail, not allocate");

    const denied = extension("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/");
    await waitFor(() => denied.child.exitCode !== null);
    assert.equal(denied.child.exitCode, 1, "An unlisted extension cannot connect");

    const invalidHello = extension();
    invalidHello.send(null);
    await waitFor(() => invalidHello.child.exitCode !== null);
    assert.equal(bridge.status().ready, true, "Malformed peer must not stop the connected browser");
    const unicodeUrl = "https://fixture.qbutt.test/Привет?voice=世界";
    const unicodePromise = bridge.cookies(unicodeUrl);
    await waitFor(() => first.messages.length === 1);
    const unicodeCommand = first.messages.shift()!;
    assert.equal(unicodeCommand.url, unicodeUrl, "Native request URL must retain Unicode");
    first.send({ id: unicodeCommand.id, result: [] });
    await unicodePromise;

    const pendingPromise = bridge.fetch("https://fixture.qbutt.test/pending");
    const disconnectCheck = assert.rejects(pendingPromise, /отключился/);
    await waitFor(() => first.messages.length === 1);
    first.child.stdin.end();
    await disconnectCheck;
    assert.equal(bridge.status().ready, false);
    await bridge.close();
    await assert.rejects(readFile(discovery), { code: "ENOENT" });
    console.log(JSON.stringify({ ok: true, scenarios: ["current-user-only-pipe-dacl", "no-extension-public-flow", "official-cookie-payload", "browser-fetch", "unicode-bidirectional", "exact-media-selection", "wrong-selection-rejected", "cancel", "oversized-frame", "unknown-extension", "malformed-peer-isolated", "disconnect", "owned-discovery-cleanup"] }));
} finally {
    for (const child of children) child.kill();
    await bridge.close();
    await rm(directory, { recursive: true, force: true });
}
