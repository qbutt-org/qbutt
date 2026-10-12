import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";

import { labAppearanceSettings } from "../appearance";
import { encode } from "../network-lab/bencode";
import { assertQtRuntime } from "../qt-runtime";

const executable = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(process.argv.length === 2 || (process.argv.length === 3 && process.argv[2] === "--torrent-data"),
    "Usage: bun tests/qt-acceptance/metadata-cancel.ts [--torrent-data]");
const mode = process.argv[2] === "--torrent-data" ? "torrent-data" : "metadata-cancel";
assert(executable, "Set QBUTT_QT_ACCEPTANCE_EXE to the built Qt acceptance driver");
const bundle = resolve(process.env.QBUTT_QT_ACCEPTANCE_BUNDLE ?? dirname(executable));
assertQtRuntime(bundle);

const root = await mkdtemp(join(await realpath(tmpdir()), "qbutt-metadata-cancel-"));
const profile = join(root, "profile");
const destination = join(root, "download");
await mkdir(join(profile, "qbutt", "config"), { recursive: true });
await mkdir(destination);
let application: ReturnType<typeof Bun.spawn> | undefined;
try {
    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[BitTorrent]",
        "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1",
        "Session\\AddTorrentStopped=true",
        `Session\\DefaultSavePath=${destination.replaceAll("\\", "/")}`,
        "Session\\AddTrackersEnabled=false", "Session\\AddTrackersFromURLEnabled=false",
        "[Network]", "PortForwardingEnabled=false",
        "[Core]", "AutoOpenTorrentFiles=false",
        "[GUI]", "Notifications\\Enabled=false",
        "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false", "WebUI\\Enabled=false",
        "Search\\SearchEnabled=false",
        "Connection\\ResolvePeerCountries=false", "Connection\\ResolvePeerHostNames=false",
        ...labAppearanceSettings("functional"), "",
    ].join("\n"));

    const payload = Buffer.from("qbutt generated legal metadata cancellation fixture\n");
    const torrentPath = join(root, "fixture.torrent");
    await writeFile(torrentPath, encode({ info: {
        name: Buffer.from("metadata-cancel.txt"), private: 1, length: payload.length,
        "piece length": 16384, pieces: createHash("sha1").update(payload).digest(),
    } }));
    const evidencePath = join(root, "evidence.json");
    const specPath = join(root, "spec.json");
    await writeFile(specPath, JSON.stringify({
        mode, evidencePath, torrentPath, destination,
    }, null, 2));

    const pathKey = Object.keys(process.env).find(key => key.toLowerCase() === "path") ?? "PATH";
    application = Bun.spawn([resolve(executable), `--profile=${profile}`], {
        cwd: bundle, windowsHide: true, timeout: 45000,
        env: {
            ...process.env,
            [pathKey]: `${bundle};${process.env[pathKey] ?? ""}`,
            QBUTT_QT_ACCEPTANCE_SPEC: specPath, QBUTT_QT_ACCEPTANCE_NATIVE: undefined,
            QT_QPA_PLATFORM: "offscreen", QT_PLUGIN_PATH: bundle, QT_SCALE_FACTOR: "1",
        },
        stdout: Bun.file(join(root, "stdout.log")), stderr: Bun.file(join(root, "stderr.log")),
    });
    assert.equal(await application.exited, 0, `Metadata cancellation acceptance failed; inspect ${root}`);
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert.equal(evidence.status, "passed");
    assert(evidence.checks.some((check: { name: string }) => check.name === mode));
    console.log(JSON.stringify({ status: "passed", evidencePath, checks: evidence.checks }, null, 2));
}
finally {
    if (application?.exitCode === null) {
        application.kill();
        await application.exited;
    }
    for (const directory of [profile, destination]) {
        const target = await realpath(directory);
        assert(target.startsWith(root + sep), "Metadata fixture cleanup escaped its temporary root");
        await rm(target, { recursive: true, force: true });
    }
}
