import assert from "node:assert/strict";
import { mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { createLab, verifyPayload, waitFor } from "../lab";
import { startControlledPeer } from "../network-lab/controlled-peer";

assert.equal(process.platform, "win32", "The disk wait uses a real Windows oplock");
const executable = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(executable, "Set QBUTT_QT_ACCEPTANCE_EXE to the stable Qt process driver");
process.env.QBUTT_LAB_EXE = executable;
const lab = await createLab("diagnostic-waits");
const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
const destination = join(lab.root, "download");
const payload = Buffer.alloc(torrent.files.reduce((size, file) => Math.max(size, file.offset + file.size), 0));
const errors: string[] = [];
let peer: Awaited<ReturnType<typeof startControlledPeer>> | undefined;
let application: Bun.Subprocess<"ignore", Bun.BunFile, Bun.BunFile> | undefined;
let diskWait: Bun.Subprocess<"ignore", Bun.BunFile, Bun.BunFile> | undefined;
let failure: unknown;
const commandPath = join(lab.root, "command.json");
const diskEvidence = join(lab.root, "disk-wait.json");
const diskRelease = join(lab.root, "release-disk.json");
const evidencePath = join(lab.root, "qt-evidence.json");
try {
    for (const file of torrent.files) {
        assert(!file.pad);
        (await readFile(join(lab.fixtures, "seed", file.path))).copy(payload, file.offset);
        await mkdir(dirname(join(destination, file.path)), {recursive: true});
        // Empty targets have no verified pieces. The oplock only delays the native writer.
        await writeFile(join(destination, file.path), Buffer.alloc(0));
    }
    const profile = join(lab.root, "profile");
    const iniPath = join(profile, "qbutt", "config", "qbutt.ini");
    const ini = await readFile(iniPath, "utf8");
    await writeFile(iniPath, ini.replace("[BitTorrent]", ["[BitTorrent]", "Session\\Encryption=2",
        "Session\\DiskQueueSize=16384"].join("\n")));
    peer = await startControlledPeer(torrent, payload, errors);
    const specPath = join(lab.root, "spec.json");
    await writeFile(specPath, JSON.stringify({mode: "diagnostic-waits", evidencePath, commandPath,
        diskEvidence, diskRelease, torrentPath: join(lab.fixtures, torrent.file), destination,
        peer: `127.0.0.1:${peer.port}`, screenshots: lab.root}));
    application = Bun.spawn([executable, `--profile=${profile}`, "--no-splash", "--confirm-legal-notice"], {
        cwd: dirname(executable), windowsHide: true, stdin: "ignore",
        env: {...process.env, QT_QPA_PLATFORM: "offscreen", QBUTT_QT_ACCEPTANCE_SPEC: specPath},
        stdout: Bun.file(join(lab.root, "app.stdout.log")), stderr: Bun.file(join(lab.root, "app.stderr.log")),
    });
    const waitCommand = (phase: string, timeout = 30000) => waitFor(phase, async () => {
        assert.equal(application!.exitCode, null, "Qt diagnostic driver exited before its phase");
        assert.deepEqual(errors, []);
        return readFile(commandPath, "utf8").then(text => JSON.parse(text).phase).catch(() => "");
    }, current => current === phase, timeout);
    await waitCommand("disk-arm");
    diskWait = Bun.spawn([lab.python, join(import.meta.dir, "disk-wait.py"),
        join(destination, "bundle", "skip.bin"), diskEvidence, diskRelease], {
        windowsHide: true, stdin: "ignore", stdout: Bun.file(join(lab.root, "disk.stdout.log")),
        stderr: Bun.file(join(lab.root, "disk.stderr.log")),
    });
    await waitCommand("rate-release");
    peer.release();
    await waitCommand("payload-complete", 150000);
    assert.equal(peer.counts.connections, 1, "Diagnostic waits caused a peer reconnect");
    assert(peer.counts.requests > 0 && peer.counts.payloadBytes >= payload.length);
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    assert.deepEqual(errors, [], "Peer failed during payload transfer or verification");
    await peer.close();
    const peerCounts = {...peer.counts};
    peer = undefined;
    assert.deepEqual(errors, [], "Peer failed before orderly teardown completed");
    await writeFile(commandPath, JSON.stringify({phase: "peer-closed"}));
    const timeout = setTimeout(() => application?.kill(), 15000);
    let exit: number;
    try { exit = await application.exited; }
    finally { clearTimeout(timeout); }
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert.equal(exit, 0, `Qt driver exited ${exit}: ${JSON.stringify(evidence)}`);
    assert.equal(evidence.status, "passed", JSON.stringify(evidence));
    assert.equal(await diskWait.exited, 0, "Oplock helper failed or reached its safety timeout");
    assert.equal(JSON.parse(await readFile(diskEvidence, "utf8")).state, "released");
    assert.deepEqual(errors, []);
    await lab.checkpoint({check: "real-disk-and-bandwidth-diagnostics", verifiedBytes,
        peer: peerCounts, qtEvidence: evidencePath, diskEvidence,
        scope: "One generated torrent, default asynchronous disk worker blocked by a Windows oplock, real per-torrent rate limit"});
}
catch (error) { failure = error; }
finally {
    // Release before waiting for app teardown: libtorrent may be draining the blocked write.
    await writeFile(diskRelease, "{}\n");
    if (diskWait?.exitCode === null) {
        const timeout = setTimeout(() => diskWait?.kill(), 5000);
        try { await diskWait.exited; }
        finally { clearTimeout(timeout); }
    }
    if (application?.exitCode === null) { application.kill(); await application.exited; }
    await peer?.close();
    const root = await realpath(lab.root);
    for (const name of ["profile", "fixtures", "download"]) {
        const path = join(root, name);
        assert.equal(dirname(path), root);
        await rm(path, {recursive: true, force: true});
    }
}
await lab.finish(failure);
if (failure) throw failure;
