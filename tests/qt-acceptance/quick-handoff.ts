import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, realpath, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";
import { labAppearanceSettings } from "../appearance";
import { assertQtRuntime } from "../qt-runtime";

const source = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(source, "Set QBUTT_QT_ACCEPTANCE_EXE to the combined Qt acceptance build");
assert(process.platform === "win32", "Native torrent handoff acceptance currently requires Windows");
const args = process.argv.slice(2);
assert(args.length === 0 || (args.length === 2 && args[0] === "--source"),
    "Usage: bun tests/qt-acceptance/quick-handoff.ts [--source aniliberty|rutor]");
const selectedSource = args[1] ?? "aniliberty";
assert(selectedSource === "aniliberty" || selectedSource === "rutor", "Unsupported handoff source");

const root = await mkdtemp(join(tmpdir(), "qbutt-quick-handoff-"));
const executable = resolve(source);
const bundle = resolve(process.env.QBUTT_QT_ACCEPTANCE_BUNDLE ?? dirname(executable));
const profile = join(root, "profile");
const payload = join(root, "payload");
const temporary = join(root, "temp");
const native = process.env.QBUTT_QT_ACCEPTANCE_NATIVE === "1";
let application: ReturnType<typeof Bun.spawn> | undefined;

try {
    assertQtRuntime(bundle);
    for (const library of ["Qt6Quick.dll", "Qt6Qml.dll", "Qt6QuickWidgets.dll"])
        assert((await stat(join(bundle, library))).isFile(), `Missing Qt Quick runtime library: ${library}`);

    await mkdir(join(profile, "qbutt", "config"), { recursive: true });
    await mkdir(payload);
    await mkdir(temporary);
    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[BitTorrent]",
        "Session\\DHTEnabled=false",
        "Session\\LSDEnabled=false",
        "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1",
        "Session\\AddTorrentStopped=true",
        "Session\\AddTrackersEnabled=false",
        "Session\\AddTrackersFromURLEnabled=false",
        "[Network]",
        "PortForwardingEnabled=false",
        "[Core]",
        "AutoOpenTorrentFiles=false",
        "[GUI]",
        "Notifications\\Enabled=false",
        "[Preferences]",
        "General\\Locale=ru",
        "Advanced\\updateCheck=false",
        "Connection\\ResolvePeerCountries=false",
        "Connection\\ResolvePeerHostNames=false",
        "WebUI\\Enabled=false",
        "Search\\SearchEnabled=false",
        ...labAppearanceSettings("functional"),
        "",
    ].join("\n"));

    const evidencePath = join(root, "evidence.json");
    const spec = join(root, "spec.json");
    await writeFile(spec, JSON.stringify({
        schema: 1,
        mode: "quick-handoff",
        source: selectedSource,
        evidencePath,
        profile,
        payload,
        screenshots: root,
    }));
    const pathKey = Object.keys(process.env).find(key => key.toLowerCase() === "path") ?? "PATH";
    application = Bun.spawn([executable, `--profile=${profile}`], {
        cwd: bundle,
        windowsHide: !native,
        timeout: 300_000,
        env: {
            ...process.env,
            [pathKey]: `${bundle};${process.env[pathKey] ?? ""}`,
            QBUTT_QT_ACCEPTANCE_SPEC: spec,
            QBUTT_QT_ACCEPTANCE_NATIVE: native ? "1" : undefined,
            QT_QPA_PLATFORM: native ? "windows" : "offscreen",
            QT_PLUGIN_PATH: bundle,
            QT_QUICK_BACKEND: native ? undefined : "software",
            QT_SCALE_FACTOR: "1",
            TEMP: temporary,
            TMP: temporary,
        },
        stdout: Bun.file(join(root, "stdout.log")),
        stderr: Bun.file(join(root, "stderr.log")),
    });
    assert.equal(await application.exited, 0, `Native torrent handoff failed; inspect ${root}`);
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert.equal(evidence.status, "passed");
    const check = evidence.checks.find((item: { name: string }) => item.name === "quick-handoff");
    assert(check, "Native handoff evidence is missing");
    assert.equal(check.handoffs, 1);
    assert.equal(check.cancellations, 1);
    assert.equal(check.exactDialogHash, true);
    assert.equal(check.torrentCount, 0);
    assert.equal(check.payloadEmpty, true);
    assert.equal(check.metadataPayloadEmpty, true);
    assert.equal(check.source, selectedSource);
    assert.equal(check.category, selectedSource === "rutor" ? "software" : "anime");
    assert(check.projectedRow >= 0, "Scenario did not select a projected category row");
    if (selectedSource === "rutor") {
        assert.equal(check.variantCount, 1);
        assert.equal(check.selectedRow, 0);
        assert.match(check.workId, /^rutor:[1-9][0-9]*$/);
        assert.equal(check.variantId, `${check.workId}:torrent:${check.infoHash}`);
    }
    else {
        assert.equal(check.workId, "aniliberty:9542");
        assert(check.variantCount >= 2 && check.selectedRow > 0, "Scenario did not select a non-default variant");
        assert.notEqual(check.infoHash, check.defaultInfoHash, "Scenario resolved the default variant");
    }
    assert.match(check.infoHash, /^[0-9a-f]{40}$/);
    assert.deepEqual(await readdir(payload), [], "Cancelled handoff left payload files after application shutdown");

    await writeFile(join(root, "result.json"), JSON.stringify({
        status: "passed",
        evidencePath,
        source: selectedSource,
        infoHash: check.infoHash,
        payloadEmptyAfterShutdown: true,
        executableSha256: createHash("sha256").update(await readFile(executable)).digest("hex"),
    }, null, 2));
    console.log(JSON.stringify({ status: "passed", root }));
}
finally {
    if (application && application.exitCode === null) {
        application.kill();
        await application.exited;
    }
    const resolvedRoot = await realpath(root);
    for (const directory of [profile, payload, temporary]) {
        const resolved = await realpath(directory).catch(() => undefined);
        if (!resolved)
            continue;
        assert(resolved.startsWith(resolvedRoot + sep), "Cleanup escaped the isolated fixture");
        await rm(resolved, { recursive: true, force: true });
    }
}
