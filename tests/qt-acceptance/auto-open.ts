import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { cp, mkdir, mkdtemp, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, sep } from "node:path";
import { labAppearanceSettings } from "../appearance";
import { encode } from "../network-lab/bencode";
import { allowLabNetwork } from "../windows-firewall";

const source = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(source, "Set QBUTT_QT_ACCEPTANCE_EXE to the combined Qt acceptance build");
assert(process.platform === "win32", "Folder auto-open currently requires Windows");
const root = await mkdtemp(join(tmpdir(), "qbutt-auto-open-"));
const bundle = join(root, "runtime");
const profile = join(root, "profile");
const watched = join(root, "watched");
const payload = join(root, "payload");
const runtime = process.env.QBUTT_QT_ACCEPTANCE_BUNDLE ?? join(dirname(source), "portable");
let application: ReturnType<typeof Bun.spawn> | undefined;
try {
    await cp(runtime, bundle, { recursive: true, filter: path => {
        if (["profile", ".git"].includes(basename(path))) return false;
        const extension = extname(path).toLowerCase();
        return !extension || [".dll", ".qm", ".conf"].includes(extension);
    } });
    const executable = join(bundle, basename(source));
    await cp(source, executable);
    for (const dependency of ["Qt6Core.dll", "Qt6Gui.dll", "Qt6Widgets.dll", "Qt6Network.dll", "platforms/qoffscreen.dll"])
        assert((await stat(join(bundle, dependency))).isFile(), `Missing runtime dependency: ${dependency}`);
    await allowLabNetwork([executable]);
    await mkdir(join(profile, "qbutt", "config"), { recursive: true });
    await mkdir(join(watched, "nested"), { recursive: true });
    await mkdir(payload);
    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1", "Session\\AddTorrentStopped=true",
        "[Network]", "PortForwardingEnabled=false", "[GUI]", "Notifications\\Enabled=false",
        "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false",
        "Connection\\ResolvePeerCountries=false", "Connection\\ResolvePeerHostNames=false",
        "General\\ExitConfirm=false", "General\\CloseToTray=false", "General\\MinimizeToTray=false",
        "General\\SystrayEnabled=false", "WebUI\\Enabled=false", ...labAppearanceSettings("functional"), "",
    ].join("\n"));
    const bytes = Buffer.from("qbutt generated legal auto-open fixture\n");
    await writeFile(join(payload, "payload.bin"), bytes);
    const torrent = join(root, "fixture.torrent");
    await writeFile(torrent, encode({ info: { length: bytes.length, name: Buffer.from("payload.bin"),
        "piece length": 16384, pieces: createHash("sha1").update(bytes).digest() } }));
    await cp(torrent, join(watched, "nested", "ignored.torrent"));
    await writeFile(join(watched, "invalid.torrent"), "not a torrent");
    const evidencePath = join(root, "evidence.json");
    const spec = join(root, "spec.json");
    await writeFile(spec, JSON.stringify({ schema: 1, mode: "auto-open", evidencePath, profile, watched, payload, torrent }));
    application = Bun.spawn([executable, `--profile=${profile}`, "--no-splash"], {
        cwd: bundle, windowsHide: true, timeout: 90000,
        env: { ...process.env, QBUTT_QT_ACCEPTANCE_SPEC: spec, QT_QPA_PLATFORM: "offscreen", QT_SCALE_FACTOR: "1" },
        stdout: Bun.file(join(root, "stdout.log")), stderr: Bun.file(join(root, "stderr.log")),
    });
    assert.equal(await application.exited, 0, `Auto-open acceptance failed; inspect ${root}`);
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert.equal(evidence.status, "passed");
    assert(evidence.checks.some((check: { name: string }) => check.name === "auto-open"));
    assert.deepEqual(await readFile(join(payload, "payload.bin")), bytes, "Auto-open changed or deleted payload");
    const saved = await readFile(join(profile, "qbutt", "config", "qbutt.ini"), "utf8");
    assert(saved.includes("AutoOpenTorrentFiles=false"), "Disabled setting was not persisted");
    assert(saved.includes("AutoOpenTorrentFolder="), "Selected folder was not persisted");
    await writeFile(join(root, "result.json"), JSON.stringify({ status: "passed", evidencePath, payloadPreserved: true,
        executableSha256: createHash("sha256").update(await readFile(executable)).digest("hex") }, null, 2));
    console.log(JSON.stringify({ status: "passed", root }));
}
finally {
    if (application && application.exitCode === null) {
        application.kill();
        await application.exited;
    }
    const resolvedRoot = await realpath(root);
    for (const directory of [bundle, profile, watched, payload]) {
        const resolved = await realpath(directory).catch(() => undefined);
        if (!resolved) continue;
        assert(resolved.startsWith(resolvedRoot + sep), "Cleanup escaped isolated fixture");
        await rm(resolved, { recursive: true, force: true });
    }
}
