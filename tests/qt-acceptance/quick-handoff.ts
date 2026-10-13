import assert from "node:assert/strict";
import { Database } from "bun:sqlite";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, realpath, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";
import { labAppearanceSettings } from "../appearance";
import { assertQtRuntime } from "../qt-runtime";

const source = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(source, "Set QBUTT_QT_ACCEPTANCE_EXE to the combined Qt acceptance build");
assert(process.platform === "win32", "Native torrent handoff acceptance currently requires Windows");
const input = process.argv.slice(2);
const mainContent = input.includes("--main");
assert(input.filter(arg => arg === "--main").length <= 1, "Repeated --main option");
const args = input.filter(arg => arg !== "--main");
assert(args.length === 0 || (args.length === 2 && args[0] === "--source"),
    "Usage: bun tests/qt-acceptance/quick-handoff.ts [--source aniliberty|rutor|limetorrents|nnmclub|rutracker|dodi|byrut|freetp|fitgirl|onlinefix|igruha] [--main]");
const selectedSource = args[1] ?? "aniliberty";
assert(["aniliberty", "rutor", "limetorrents", "nnmclub", "rutracker", "dodi", "byrut", "freetp", "fitgirl", "onlinefix", "igruha"].includes(selectedSource), "Unsupported handoff source");

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
        `Session\\DefaultSavePath=${payload.replaceAll("\\", "/")}`,
        "Session\\UseAutoTMM=false",
        "Session\\UseTempPathEnabled=false",
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
        `Search\\SearchEnabled=${mainContent}`,
        ...labAppearanceSettings("functional"),
        "",
    ].join("\n"));

    const evidencePath = join(root, "evidence.json");
    const spec = join(root, "spec.json");
    await writeFile(spec, JSON.stringify({
        schema: 1,
        mode: mainContent ? "main-content" : "quick-handoff",
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
    const category = selectedSource === "rutor" ? "software"
        : ["dodi", "byrut", "freetp", "fitgirl", "onlinefix", "igruha"].includes(selectedSource) ? "games"
        : selectedSource === "aniliberty" ? "anime" : "movies";
    assert.equal(check.category, category);
    assert.equal(check.mainContent, mainContent);
    if (mainContent) {
        assert.equal(check.favoriteWritePendingAtExit, true);
        const libraryPath = resolve(check.libraryPath);
        assert(libraryPath.startsWith(resolve(profile) + sep), "Library escaped the selected profile");
        const database = new Database(join(libraryPath, "content-library.sqlite"), { readonly: true });
        try {
            assert.equal((database.query("PRAGMA integrity_check").get() as { integrity_check: string }).integrity_check, "ok");
            const records = database.query("SELECT entry FROM library").all() as { entry: string }[];
            assert.equal(records.length, 1, "Unexpected persisted content entries");
            const entry = JSON.parse(records[0].entry);
            assert.equal(entry.favorite, true, "Accepted favorite write was lost at MainWindow shutdown");
            const group = entry.group;
            const anchor = group.publications.find((publication: { reference: unknown }) =>
                JSON.stringify(publication.reference) === JSON.stringify(group.anchor));
            assert(anchor?.title.length > 0, "Favorite lost its anchor publication");
            assert(group.references.some((reference: unknown) =>
                JSON.stringify(reference) === JSON.stringify(entry.preferences.selected.publication)),
                "Favorite lost the selected publication reference");
            const variants = group.publications.flatMap((publication: { variants: unknown[] }) => publication.variants);
            assert(variants.length >= check.variantCount, "Favorite lost known variants");
            const selected = variants.find((variant: { reference: unknown }) =>
                JSON.stringify(variant.reference) === JSON.stringify(entry.preferences.selected));
            assert(selected, "Favorite did not retain its exact selected variant");
            assert.equal(selected.info_hash, selectedSource === "dodi" ? null : check.infoHash);
        }
        finally {
            database.close();
        }
    }
    assert(check.projectedRow >= 0, "Scenario did not select a projected category row");
    if (selectedSource === "dodi") {
        assert.equal(check.variantCount, 1);
        assert.equal(check.selectedRow, 0);
        assert.equal(check.workId, "dodi:2643--devil-may-cry-5");
        assert.equal(check.variantId, `${check.workId}:torrent:vpe2b0x0a9zw`);
        assert.equal(check.infoHash, "b34fa7dfc9c95baf8509f6bd90a4ce5e1fde22e1");
        assert(check.metadataBytes > 0 && check.metadataBytes <= 10 * 1024 * 1024);
    }
    else if (selectedSource === "byrut") {
        assert.equal(check.variantCount, 1);
        assert.equal(check.workId, "byrut:2799-stardew-valley");
        assert.equal(check.infoHash, "942e9eeace33f7cfefa4fbcc129a1a9f12097638");
        assert.equal(check.variantId, `${check.workId}:torrent:${check.infoHash}`);
        assert(check.metadataBytes > 0 && check.metadataBytes <= 10 * 1024 * 1024);
    }
    else if (selectedSource === "freetp") {
        assert.equal(check.workId, "freetp:6327");
        assert.equal(check.infoHash, "e54e92ed62a49401fc224642e65f8ca03709c95e");
        assert.equal(check.variantId, `${check.workId}:torrent:${check.infoHash}`);
        assert(check.metadataBytes > 0 && check.metadataBytes <= 10 * 1024 * 1024);
    }
    else if (selectedSource === "onlinefix") {
        assert.equal(check.workId, "onlinefix:games/sandbox/16274-stardew-valley-po-seti.html");
        assert.equal(check.infoHash, "063043892eac8f94c5eb249669548a3f93b7c3a5");
        assert.equal(check.variantId, `${check.workId}:torrent:${check.infoHash}`);
    }
    else if (selectedSource === "fitgirl") {
        assert.equal(check.workId, "fitgirl:cyberpunk-2077");
        assert(check.variantCount >= 2 && check.selectedRow > 0);
        assert.equal(check.infoHash, "f081eec38f08f5d663db807b39c5c33bb1049e2f");
        assert.equal(check.variantId, `${check.workId}:torrent:${check.infoHash}`);
        assert.notEqual(check.defaultInfoHash, check.infoHash);
    }
    else if (selectedSource === "igruha") {
        assert.equal(check.variantCount, 1);
        assert.equal(check.selectedRow, 0);
        assert.equal(check.workId, "igruha:7861-open-transport-tycoon-deluxe");
        assert.equal(check.infoHash, "1ea103b38144f9bd91cb8e979949de4341010484");
        assert.equal(check.variantId, `${check.workId}:torrent:${check.infoHash}`);
        assert(check.metadataBytes > 0, "Igruha did not hand off torrent metadata");
        assert.match(check.metadataSha256, /^[0-9a-f]{64}$/);
    }
    else if (["rutor", "limetorrents", "nnmclub", "rutracker"].includes(selectedSource)) {
        assert.equal(check.variantCount, 1);
        assert.equal(check.selectedRow, 0);
        if (selectedSource === "rutor")
            assert.match(check.workId, /^rutor:[1-9][0-9]*$/);
        else if (selectedSource === "nnmclub") {
            assert.equal(check.workId, "nnmclub:906013");
            assert.equal(check.infoHash, "eeddbd381bc734373bccf170a17d3bba23fa33a3");
        }
        else if (selectedSource === "rutracker") {
            assert.equal(check.workId, "rutracker:5026149");
            assert.equal(check.infoHash, "cfcdab5d3c5efb529590160ea84fc419977fbd67");
        }
        else {
            assert.equal(check.workId,
                "limetorrents:Big-Buck-Bunny-4K-UHD-HFR-60fps-FLAC-WEBDL-2160p-X264-mkv-torrent-9616433.html");
        }
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
