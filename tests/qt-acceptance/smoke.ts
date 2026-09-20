import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { cp, mkdir, mkdtemp, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { closeSync, openSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { connect, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, resolve, sep } from "node:path";
import { generateFixtures } from "../fixtures/generate";
import { allowLabNetwork } from "../windows-firewall";
import { labAppearanceSettings } from "../appearance";

const sourceExecutable = process.env.QBUTT_QT_ACCEPTANCE_EXE;
const appearanceOnly = process.argv.includes("--appearance-only");
const python = process.env.QBUTT_LAB_PYTHON;
assert(sourceExecutable, "Set QBUTT_QT_ACCEPTANCE_EXE");
if (!appearanceOnly)
    assert(python, "Set QBUTT_LAB_PYTHON for the full Qt acceptance suite");
assert((await stat(sourceExecutable)).isFile(), "Qt acceptance executable is missing");
const siblingPortable = join(dirname(sourceExecutable), "portable");
const runtimeSource = process.env.QBUTT_QT_ACCEPTANCE_BUNDLE
    ?? await stat(siblingPortable).then(value => value.isDirectory() ? siblingPortable : dirname(sourceExecutable))
        .catch(() => dirname(sourceExecutable));
assert((await stat(runtimeSource)).isDirectory(), "Qt acceptance runtime bundle is missing");

function readBytes(socket: Socket, count: number): Promise<Buffer> {
    return new Promise((resolveRead, rejectRead) => {
        let data = Buffer.alloc(0);
        const timeout = setTimeout(() => finish(new Error("SOCKS fixture response timed out")), 5000);
        const finish = (error?: Error) => {
            clearTimeout(timeout);
            socket.off("data", onData);
            socket.off("error", onError);
            socket.off("close", onClose);
            if (error)
                rejectRead(error);
            else
                resolveRead(data.subarray(0, count));
        };
        const onData = (chunk: Buffer) => {
            data = Buffer.concat([data, chunk]);
            if (data.length >= count)
                finish();
        };
        const onError = (error: Error) => finish(error);
        const onClose = () => finish(new Error("SOCKS fixture closed before replying"));
        socket.on("data", onData);
        socket.once("error", onError);
        socket.once("close", onClose);
    });
}

async function authenticate(port: number, suppliedPassword: string, probePayload = false): Promise<boolean> {
    const socket = connect({ host: "127.0.0.1", port });
    try {
        await once(socket, "connect");
        const methodReply = readBytes(socket, 2);
        socket.write(Buffer.from([5, 1, 2]));
        assert.deepEqual(await methodReply, Buffer.from([5, 2]));
        const user = Buffer.from("acceptance-user");
        const password = Buffer.from(suppliedPassword);
        const authReply = readBytes(socket, 2);
        socket.write(Buffer.concat([Buffer.from([1, user.length]), user, Buffer.from([password.length]), password]));
        const reply = await authReply;
        assert.equal(reply[0], 1);
        if (reply[1] !== 0)
            return false;
        if (probePayload) {
            const connectReply = readBytes(socket, 10);
            socket.write(Buffer.from([5, 1, 0, 1, 127, 0, 0, 1, 0, 9]));
            assert.deepEqual(await connectReply, Buffer.from([5, 0, 0, 1, 127, 0, 0, 1, 0, 0]));
            const payload = Buffer.from("qbutt-authenticated-payload-boundary");
            const echoed = readBytes(socket, payload.length);
            socket.write(payload);
            assert.deepEqual(await echoed, payload, "SOCKS fixture did not preserve the authenticated payload boundary");
        }
        return true;
    }
    finally {
        socket.destroy();
    }
}

async function probePayloadAuthentication(path: string, signal: AbortSignal): Promise<void> {
    const deadline = Date.now() + 120000;
    let opened: { port: number }[] = [];
    while (!signal.aborted && (Date.now() < deadline)) {
        opened = await readFile(path, "utf8").then(value => JSON.parse(value).opened ?? []).catch(() => []);
        if (opened.length === 3)
            break;
        await Bun.sleep(10);
    }
    if (signal.aborted)
        return;
    assert.equal(opened.length, 3, "Timed out waiting for the three payload listeners");
    for (const endpoint of opened) {
        assert(Number.isSafeInteger(endpoint.port) && endpoint.port > 0 && endpoint.port <= 65535,
            "Transport evidence did not contain a valid listener port");
        assert.equal(await authenticate(endpoint.port, "wrong-password"), false,
            "A payload listener accepted invalid credentials");
        assert.equal(await authenticate(endpoint.port, "QBUTT_ACCEPTANCE_SECRET", true), true,
            "A payload listener rejected its private credentials");
    }
}

async function removeOwnedDirectory(parent: string, name: string) {
    const path = join(parent, name);
    try {
        const expected = join(await realpath(parent), name);
        assert.equal(await realpath(path), expected, "Fixture cleanup escaped its owned directory");
        await rm(path, { recursive: true, force: true });
    }
    catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
}

const root = await mkdtemp(join(tmpdir(), "qbutt-qt-acceptance-"));
assert((await realpath(root)).startsWith(await realpath(tmpdir()) + sep)
    && basename(root).startsWith("qbutt-qt-acceptance-"), "Unexpected Qt fixture root");
// Keep executable paths stable so the firewall preflight creates one reusable
// rule per process while every profile, payload and evidence tree stays fresh.
const bundle = join(tmpdir(), "qbutt-qt-acceptance-runtime");
const runtimeLock = `${bundle}.lock`;
let runtimeLockHeld = false;
const releaseRuntimeLock = () => {
    if (runtimeLockHeld) {
        runtimeLockHeld = false;
        try { unlinkSync(runtimeLock); } catch {}
    }
};
process.once("exit", releaseRuntimeLock);
let application: ReturnType<typeof Bun.spawn> | undefined;
let compiler: ReturnType<typeof Bun.spawn> | undefined;
let authenticationAbort: AbortController | undefined;
let authentication: Promise<void> | undefined;
let failure: unknown;
let result: Record<string, unknown> | undefined;
try {
    for (;;) {
        try {
            const descriptor = openSync(runtimeLock, "wx");
            writeFileSync(descriptor, String(process.pid));
            closeSync(descriptor);
            break;
        }
        catch (error) {
            if ((error as NodeJS.ErrnoException).code !== "EEXIST")
                throw error;
            const owner = Number(readFileSync(runtimeLock, "utf8"));
            try {
                process.kill(owner, 0);
                throw new Error(`Qt acceptance runtime is already owned by process ${owner}`);
            }
            catch (ownerError) {
                if ((ownerError as NodeJS.ErrnoException).code !== "ESRCH")
                    throw ownerError;
                unlinkSync(runtimeLock);
            }
        }
    }
    runtimeLockHeld = true;
    await removeOwnedDirectory(tmpdir(), basename(bundle));
    await mkdir(bundle);
    await cp(runtimeSource, bundle, { recursive: true, filter: path => {
        if (["profile", ".git"].includes(basename(path)))
            return false;
        const extension = extname(path).toLowerCase();
        return !extension || [".exe", ".dll", ".qm", ".conf"].includes(extension);
    } });
    const executable = join(bundle, basename(sourceExecutable));
    await cp(sourceExecutable, executable);
    if (appearanceOnly) {
        // Three real application processes exercise first-run defaults and restart
        // persistence. No torrents, transport child or Python fixture are needed.
        await allowLabNetwork([executable]);
        const layoutDefaults = resolve(import.meta.dir, "../../docs/ui-default-layout.json");
        assert((await stat(layoutDefaults)).isFile(), "Captured default layout is missing");
        const screenshots = join(root, "screenshots");
        await mkdir(screenshots);
        const retainedState = join(root, "saved-layout.json");
        const results: { phase: string; evidence: string }[] = [];
        for (const phase of ["product", "retained", "functional"] as const) {
            const profile = join(root, phase === "functional" ? "functional-profile" : "product-profile");
            if (phase !== "retained") {
                const config = join(profile, "qbutt", "config");
                await mkdir(config, { recursive: true });
                await writeFile(join(config, "qbutt.ini"), [
                    "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
                    "Session\\InterfaceAddress=127.0.0.1", "Session\\AddTorrentStopped=true",
                    "[Network]", "PortForwardingEnabled=false",
                    "[GUI]", "Notifications\\Enabled=false",
                    "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false",
                    "Connection\\ResolvePeerCountries=false", "Connection\\ResolvePeerHostNames=false",
                    "General\\ExitConfirm=false", "General\\CloseToTray=false", "General\\MinimizeToTray=false",
                    "General\\SystrayEnabled=false", "WebUI\\Enabled=false",
                    // Product starts without any theme or layout override.
                    ...(phase === "functional" ? labAppearanceSettings("functional") : []), "",
                ].join("\n"));
            }
            const evidencePath = join(root, `${phase}-evidence.json`);
            const spec = join(root, `${phase}-spec.json`);
            await writeFile(spec, JSON.stringify({ schema: 1, mode: "appearance", appearance: phase,
                evidencePath, profile, screenshots, layoutDefaults, retainedState }, null, 2));
            application = Bun.spawn([executable, `--profile=${profile}`, "--no-splash", "--confirm-legal-notice"], {
                cwd: bundle, windowsHide: true,
                env: { ...process.env, QT_QPA_PLATFORM: "offscreen", QT_SCALE_FACTOR: "1",
                    QBUTT_QT_ACCEPTANCE_SPEC: spec },
                stdout: Bun.file(join(root, `${phase}-stdout.log`)),
                stderr: Bun.file(join(root, `${phase}-stderr.log`)), timeout: 90000,
            });
            const exitCode = await application.exited;
            assert.equal(exitCode, 0, `Appearance ${phase} exited ${exitCode} (signal ${application.signalCode}); inspect ${root}`);
            const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
            assert.equal(evidence.status, "passed", `Appearance ${phase} failed; inspect ${evidencePath}`);
            assert(evidence.checks.some((check: { name: string }) => check.name === `appearance-${phase}`),
                "Acceptance executable did not run the requested appearance phase");
            results.push({ phase, evidence: evidencePath });
        }
        const executableSha256 = createHash("sha256").update(await readFile(executable)).digest("hex");
        result = { status: "passed", suite: "appearance", executableSha256, results, screenshots };
    }
    else {
        const child = join(bundle, "qbutt-net.exe");
        compiler = Bun.spawn([process.execPath, "build", "--compile", join(import.meta.dir, "fake-child.ts"), "--outfile", child],
            { stdout: "pipe", stderr: "pipe", windowsHide: true });
        const [compileCode, compileOut, compileErr] = await Promise.all([
            compiler.exited, new Response(compiler.stdout).text(), new Response(compiler.stderr).text(),
        ]);
        assert.equal(compileCode, 0, `Cannot compile Qt transport fixture: ${compileOut}\n${compileErr}`);
        await allowLabNetwork([executable, child]);

        assert(python, "Set QBUTT_LAB_PYTHON for torrent fixtures");
        const fixtures = await generateFixtures(python, join(root, "fixtures"));
        const manifest = JSON.parse(await readFile(join(fixtures, "manifest.json"), "utf8")) as {
            torrents: { name: string; file: string }[];
        };
        const torrent = manifest.torrents.find(candidate => candidate.name === "v1-public");
        assert(torrent, "Generated v1-public fixture is missing");
        const profile = join(root, "profile");
        const config = join(profile, "qbutt", "config");
        await mkdir(config, { recursive: true });
        await writeFile(join(config, "qbutt.ini"), [
            "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
            "Session\\AddTorrentStopped=true", "Session\\AddExtensionToIncompleteFiles=false", "Session\\UseUnwantedFolder=false",
            "Session\\QueueingSystemEnabled=false", "Session\\InterfaceAddress=127.0.0.1", "Session\\ResumeDataStorageType=SQLite",
            "[Network]", "PortForwardingEnabled=false",
            "[GUI]", "Notifications\\Enabled=false",
            "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false", "Connection\\ResolvePeerCountries=false",
            "Connection\\ResolvePeerHostNames=false", "General\\ExitConfirm=false", "General\\CloseToTray=false",
            "General\\MinimizeToTray=false", "General\\SystrayEnabled=false", "WebUI\\Enabled=false", "",
            ...labAppearanceSettings(),
        ].join("\n"));

        const largeRoot = join(root, "large-source");
        await mkdir(largeRoot);
        for (let offset = 0; offset < 30000; offset += 512) {
            await Promise.all(Array.from({ length: Math.min(512, 30000 - offset) }, (_, index) => {
                const number = offset + index;
                return writeFile(join(largeRoot, `candidate-${number.toString().padStart(5, "0")}.bin`), "x");
            }));
        }
        const destination = join(root, "destination");
        await mkdir(destination);
        await writeFile(join(destination, "unknown.keep"), "must survive repair");
        const subscription = join(root, "subscription.yaml");
        await writeFile(subscription, "proxies: []\n");
        const evidencePath = join(root, "evidence.json");
        const childEvidence = join(root, "child-evidence.json");
        const spec = join(root, "spec.json");
        await writeFile(spec, JSON.stringify({
            schema: 1, evidencePath, childEvidence, torrentPath: join(fixtures, torrent.file),
            sourceRoot: join(fixtures, "seed"), largeRoot, destination, subscription,
            screenshots: join(root, "screenshots"), fixtureRoot: root, profile, bulkRows: 2000,
        }, null, 2));
        await mkdir(join(root, "screenshots"));

        application = Bun.spawn([executable, `--profile=${profile}`, "--no-splash", "--confirm-legal-notice"], {
            cwd: bundle, windowsHide: true,
            env: { ...process.env, QT_QPA_PLATFORM: "offscreen", QBUTT_QT_ACCEPTANCE_SPEC: spec,
                QBUTT_QT_CHILD_EVIDENCE: childEvidence },
            stdout: Bun.file(join(root, "stdout.log")), stderr: Bun.file(join(root, "stderr.log")),
            timeout: 600000,
        });
        authenticationAbort = new AbortController();
        let authenticationError: unknown;
        authentication = probePayloadAuthentication(childEvidence, authenticationAbort.signal)
            .catch(error => { authenticationError = error; });
        const exitCode = await application.exited;
        if (exitCode !== 0)
            authenticationAbort.abort();
        await authentication;
        assert.equal(exitCode, 0, `Qt acceptance process exited ${exitCode}; inspect ${root}`);
        if (authenticationError)
            throw authenticationError;
        const evidence = JSON.parse(await readFile(evidencePath, "utf8")) as Record<string, unknown>;
        assert.equal(evidence.status, "passed", `Qt acceptance failed; inspect ${root}`);
        const transport = JSON.parse(await readFile(childEvidence, "utf8")) as {
            protocol: number; hello: number; listed: number; status: number; authenticated: number; rejectedCredentials: number;
            payloadBoundaries: number; delayedStatus: number; statusPending: boolean;
            eofObserved: boolean;
            opened: { pathId: string; generation: number; proxyName: string; port: number }[];
            closed: { pathId: string; generation: number }[];
            retiredOnEof: { pathId: string; generation: number }[];
        };
        assert.equal(transport.protocol, 5, "The Qt acceptance transport did not use the pinned v5 contract");
        assert.equal(transport.eofObserved, true, "The transport child did not observe parent EOF and finish cleanup");
        assert(transport.hello >= 1 && transport.listed >= 1, "The production app did not negotiate and list the transport child");
        assert(transport.status >= 1, "The production app did not poll bounded transport counters");
        assert.equal(transport.opened.length, 3, "The production app did not open exactly three acceptance paths");
        const opened = new Set(transport.opened.map(path => `${path.pathId}:${path.generation}`));
        assert.equal(opened.size, transport.opened.length, "Acceptance paths did not have independent id/generation pairs");
        const retired = [...transport.closed, ...transport.retiredOnEof].map(path => `${path.pathId}:${path.generation}`);
        assert.equal(new Set(retired).size, retired.length, "A path generation was retired more than once");
        assert.deepEqual(new Set(retired), opened, "Native restoration did not retire the exact active path generations");
        assert(transport.authenticated >= 3, "Authenticated payload probes did not reach every listener");
        assert.equal(transport.rejectedCredentials, 3, "Invalid credentials were not rejected by every listener");
        assert.equal(transport.payloadBoundaries, 3, "Authenticated SOCKS payloads did not cross every listener boundary");
        assert.equal(transport.delayedStatus, 1, "The queued foreground request race was not exercised exactly once");
        assert.equal(transport.statusPending, false, "The delayed status request did not complete");
        const bytes = await readFile(executable);
        result = { status: "passed", evidence: evidencePath, executable: resolve(sourceExecutable),
            executableSha256: createHash("sha256").update(bytes).digest("hex"),
            transport: { protocol: transport.protocol, opened: 3, retired: retired.length,
                authenticated: transport.authenticated, payloadBoundaries: transport.payloadBoundaries } };
    }
}
catch (error) { failure = error; }
finally {
    authenticationAbort?.abort();
    const cleanupErrors: unknown[] = [];
    const clean = async (operation: () => Promise<unknown>) => {
        try { await operation(); } catch (error) { cleanupErrors.push(error); }
    };
    if (application?.exitCode === null)
        await clean(async () => { application!.kill(); await application!.exited; });
    if (compiler?.exitCode === null)
        await clean(async () => { compiler!.kill(); await compiler!.exited; });
    if (authentication) await clean(() => authentication!);
    if (runtimeLockHeld) {
        await clean(() => removeOwnedDirectory(tmpdir(), basename(bundle)));
        releaseRuntimeLock();
    }
    for (const name of ["fixtures", "large-source", "profile", "destination", "product-profile", "functional-profile"])
        await clean(() => removeOwnedDirectory(root, name));
    if (cleanupErrors.length)
        failure = new AggregateError(failure ? [failure, ...cleanupErrors] : cleanupErrors,
            "Qt acceptance cleanup failed");
}
if (failure) throw failure;
if (result) console.log(JSON.stringify(result));
