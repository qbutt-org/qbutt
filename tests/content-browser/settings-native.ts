import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createConnection, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { encodeFrame } from "./framing";
import { NativeBrowserFixture } from "./native-parent";

assert.equal(process.platform, "win32", "The native browser settings fixture requires Windows");
const qcatch = process.env.QBUTT_QCATCH_SETUP_FIXTURE_EXE;
assert(qcatch && await Bun.file(qcatch).exists(), "Set QBUTT_QCATCH_SETUP_FIXTURE_EXE to the generated qcatch CLI fixture");

const directory = await mkdtemp(join(tmpdir(), "qbutt-browser-settings-"));
const discovery = join(directory, "browser.json");
const log = join(directory, "qcatch.log");
const pidFile = join(directory, "version-pid");
const release = join(directory, "release-connect");
process.env.QBUTT_QCATCH_SETUP_FIXTURE_LOG = log;
process.env.QBUTT_QCATCH_SETUP_FIXTURE_EXTENSION = directory;
process.env.QBUTT_QCATCH_SETUP_FIXTURE_PID_FILE = pidFile;
process.env.QBUTT_QCATCH_SETUP_FIXTURE_RELEASE = release;

type BrowserUi = {
    choice: string;
    connection: number;
    setup: number;
    failure: number;
    connectedName: string;
    extensionDirectory: string;
};

const parent = new NativeBrowserFixture({ discoveryPath: discovery });
const updates: BrowserUi[] = [];
const peers: Socket[] = [];
parent.onMessage = message => {
    if (message.browserUi)
        updates.push(message.browserUi as BrowserUi);
};

async function waitFor(predicate: () => boolean | Promise<boolean>, action: string) {
    const deadline = Date.now() + 10_000;
    while (!await predicate()) {
        assert(Date.now() < deadline, `${action} timed out; last state: ${JSON.stringify(updates.at(-1))}`);
        await Bun.sleep(20);
    }
}

async function state(predicate: (value: BrowserUi) => boolean, action: string) {
    await waitFor(() => {
        const latest = updates.at(-1);
        return latest !== undefined && predicate(latest);
    }, action);
    return updates.at(-1)!;
}

async function peer(browser: "edge" | "chrome") {
    const pipe = (JSON.parse(await readFile(discovery, "utf8")) as { pipe: string }).pipe;
    const socket = createConnection(pipe);
    peers.push(socket);
    socket.on("data", () => {});
    await new Promise<void>((resolve, reject) => {
        socket.once("connect", resolve);
        socket.once("error", reject);
    });
    socket.write(encodeFrame({ type: "hello", browser, version: 1,
        userAgent: `qbutt-generated-${browser}/1`, capabilities: [] }));
    return socket;
}

function event(name: string, data: Record<string, unknown> = {}) {
    parent.forward(1, name, data);
}

async function processExists(pid: number) {
    const task = Bun.spawn(["tasklist", "/FI", `PID eq ${pid}`, "/FO", "CSV", "/NH"],
        { stdout: "pipe", stderr: "ignore" });
    const output = await new Response(task.stdout).text();
    assert.equal(await task.exited, 0, "tasklist failed while checking the qcatch probe");
    return output.split(/\r?\n/).some(line => /^"[^"]*","(\d+)"/.exec(line)?.[1] === String(pid));
}

try {
    await parent.start(false);
    event("browserUiWatch");
    await state(value => value.connection === 4, "no installed browser");

    event("browserUiInstalled", { browser: "edge", installed: true });
    event("browserUiDefault", { browser: "edge" });
    event("qcatchFixtureInstall");
    event("browserUiWatch");
    await state(value => value.connection === 2, "installed but disconnected browser");

    event("qcatchFixtureFault", { fault: "connect" });
    event("browserUiSetup");
    await state(value => value.setup === 3 && value.failure === 4, "qcatch setup failure");
    assert.match(await readFile(log, "utf8"), /--connect-browser edge/);

    event("qcatchFixtureFault", { fault: "" });
    event("browserUiSetup");
    await state(value => value.setup === 2 && value.extensionDirectory === directory,
        "qcatch setup callback");

    await peer("edge");
    await state(value => value.connection === 1 && value.connectedName === "Microsoft Edge",
        "sole Edge connection in Auto");

    event("browserUiInstalled", { browser: "chrome", installed: true });
    event("browserUiChoice", { choice: "chrome" });
    await state(value => value.choice === "chrome" && value.connection === 2,
        "explicit Chrome must not borrow Edge");
    event("browserUiSetup");
    await waitFor(async () => (await readFile(log, "utf8")).includes("--connect-browser chrome"),
        "explicit Chrome qcatch setup");
    await state(value => value.choice === "chrome" && value.setup === 2,
        "explicit Chrome setup completes before changing choice");

    event("browserUiChoice", { choice: "auto" });
    await state(value => value.choice === "auto" && value.connection === 1
        && value.connectedName === "Microsoft Edge", "Auto returns to sole Edge peer");
    await peer("chrome");
    await state(value => value.choice === "auto" && value.connection === 3,
        "Auto with two connected browsers is ambiguous");
    const before = await readFile(log, "utf8");
    event("browserUiSetup");
    await Bun.sleep(100);
    assert.equal(await readFile(log, "utf8"), before,
        "Ambiguous Auto must not launch an arbitrary browser setup");

    event("browserUiChoice", { choice: "chrome" });
    event("browserUiChoice", { choice: "edge" });
    await state(value => value.choice === "edge" && value.connection === 1
        && value.connectedName === "Microsoft Edge", "latest browser choice owns the result");
    await Bun.sleep(100);
    assert.equal(updates.at(-1)?.choice, "edge", "late status must not restore Chrome");

    const blocked = new NativeBrowserFixture({ discoveryPath: discovery });
    const blockedUpdates: BrowserUi[] = [];
    blocked.onMessage = message => {
        if (message.browserUi)
            blockedUpdates.push(message.browserUi as BrowserUi);
    };
    try {
        await blocked.start(false);
        blocked.forward(1, "browserUiWatch");
        await waitFor(() => blockedUpdates.some(value => value.connection === 5),
            "another profile owns the broker");
        blocked.forward(1, "browserUiChoice", { choice: "chrome" });
        await waitFor(() => blockedUpdates.some(value => value.choice === "chrome"
            && value.connection === 5), "browser choice remains available during broker error");
    }
    finally {
        await blocked.close();
    }

    event("browserUiChoice", { choice: "edge" });
    for (const socket of peers)
        socket.destroy();
    await state(value => value.choice === "edge" && value.connection === 2,
        "Edge disconnects before the version probe");

    event("qcatchFixtureFault", { fault: "delay-connect" });
    const edgeStarts = (await readFile(log, "utf8")).split("--connect-browser edge").length;
    event("browserUiSetup");
    await waitFor(async () => (await readFile(log, "utf8"))
        .split("--connect-browser edge").length > edgeStarts, "delayed Edge setup starts");
    event("browserUiChoice", { choice: "chrome" });
    await state(value => value.choice === "chrome" && value.setup === 1,
        "choice change retains active setup as busy");
    const blockedChromeStarts = (await readFile(log, "utf8"))
        .split("--connect-browser chrome").length;
    event("browserUiSetup");
    await Bun.sleep(100);
    assert.equal((await readFile(log, "utf8")).split("--connect-browser chrome").length,
        blockedChromeStarts, "second setup stays blocked while the first action runs");
    await writeFile(release, "release");
    await state(value => value.choice === "chrome" && value.setup === 0
        && value.extensionDirectory === "", "old Edge setup is not attributed to Chrome");
    const chromeStarts = (await readFile(log, "utf8")).split("--connect-browser chrome").length;
    event("browserUiSetup");
    await waitFor(async () => (await readFile(log, "utf8"))
        .split("--connect-browser chrome").length > chromeStarts, "new Chrome setup starts");
    await state(value => value.choice === "chrome" && value.setup === 2
        && value.extensionDirectory === directory, "Chrome setup completes after old Edge action");

    event("browserUiChoice", { choice: "edge" });
    await state(value => value.choice === "edge" && value.setup === 0,
        "choice change clears finished setup");
    event("qcatchFixtureFault", { fault: "hang-version" });
    event("browserUiSetup");
    await state(value => value.setup === 1, "qcatch version probe begins");
    await waitFor(() => Bun.file(pidFile).exists(), "hanging version probe PID");
    const probePid = Number(await readFile(pidFile, "utf8"));
    assert(Number.isSafeInteger(probePid) && probePid > 0);
    assert(await processExists(probePid), "hanging version probe must exist before cancellation");
    event("browserUiClose");
    await waitFor(async () => !await processExists(probePid), "cancelled version probe exits");
    await state(value => value.setup === 0, "browser setup resets after probe cancellation");
    console.log("Native browser settings: choice, Auto, ambiguity, CLI setup, fault, owner error, generation, delayed setup and probe cancellation passed");
}
finally {
    for (const socket of peers)
        socket.destroy();
    await parent.close();
    await rm(directory, { recursive: true, force: true });
}
