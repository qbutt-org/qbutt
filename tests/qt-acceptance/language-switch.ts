import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";
import { labAppearanceSettings } from "../appearance";
import { assertQtRuntime } from "../qt-runtime";

const source = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(source, "Set QBUTT_QT_ACCEPTANCE_EXE to the combined Qt acceptance build");
assert(process.platform === "win32", "Language acceptance requires Windows");
assert(process.argv.length === 2, "Usage: bun tests/qt-acceptance/language-switch.ts");

const root = await mkdtemp(join(tmpdir(), "qbutt-language-switch-"));
const executable = resolve(source);
const bundle = resolve(process.env.QBUTT_QT_ACCEPTANCE_BUNDLE ?? dirname(executable));
const profile = join(root, "profile");
const payload = join(root, "payload");
const temporary = join(root, "temp");
const configuration = join(profile, "qbutt", "config", "qbutt.ini");
let application: ReturnType<typeof Bun.spawn> | undefined;

try {
    assertQtRuntime(bundle);
    await mkdir(dirname(configuration), { recursive: true });
    await mkdir(payload);
    await mkdir(temporary);
    await writeFile(configuration, [
        "[BitTorrent]",
        "Session\\DHTEnabled=false",
        "Session\\LSDEnabled=false",
        "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1",
        "Session\\AddTorrentStopped=true",
        `Session\\DefaultSavePath=${payload.replaceAll("\\", "/")}`,
        "Session\\UseAutoTMM=false",
        "Session\\UseTempPathEnabled=false",
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
        "Search\\SearchEnabled=true",
        ...labAppearanceSettings("functional"),
        "",
    ].join("\n"));

    const phases = ["switch", "restart-en", "restart-ru"];
    const results: unknown[] = [];
    for (const phase of phases) {
        const evidencePath = join(root, `${phase}.json`);
        const spec = join(root, `${phase}-spec.json`);
        await writeFile(spec, JSON.stringify({ schema: 1, mode: "language-switch", phase, evidencePath, profile, payload }));
        const pathKey = Object.keys(process.env).find(key => key.toLowerCase() === "path") ?? "PATH";
        application = Bun.spawn([executable, `--profile=${profile}`], {
            cwd: bundle,
            windowsHide: true,
            timeout: 180_000,
            env: {
                ...process.env,
                [pathKey]: `${bundle};${process.env[pathKey] ?? ""}`,
                QBUTT_QT_ACCEPTANCE_SPEC: spec,
                QBUTT_QT_ACCEPTANCE_NATIVE: undefined,
                QT_QPA_PLATFORM: "offscreen",
                QT_PLUGIN_PATH: bundle,
                QML_IMPORT_PATH: join(bundle, "qml"),
                QT_QUICK_BACKEND: "software",
                QT_SCALE_FACTOR: "1",
                TEMP: temporary,
                TMP: temporary,
            },
            stdout: Bun.file(join(root, `${phase}-stdout.log`)),
            stderr: Bun.file(join(root, `${phase}-stderr.log`)),
        });
        assert.equal(await application.exited, 0, `Language ${phase} failed; inspect ${root}`);
        const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
        assert.equal(evidence.status, "passed");
        const check = evidence.checks.find((item: { name: string }) => item.name === "language-switch");
        assert(check, "Language evidence is missing");
        const expected = phase === "switch" ? ["ru", "en", "ru", "en"]
            : phase === "restart-en" ? ["en", "ru"] : ["ru"];
        assert.deepEqual(check.labels.map((label: { locale: string }) => label.locale), expected);
        assert.equal(check.savedLocale, expected.at(-1));
        assert.equal(check.torrentCount, 0);
        assert.match(await readFile(configuration, "utf8"), new RegExp(`^General\\\\Locale=${check.savedLocale}$`, "m"));
        assert.deepEqual(await readdir(payload), [], "Language fixture left downloaded payload");
        results.push(check);
    }
    await writeFile(join(root, "result.json"), JSON.stringify({
        status: "passed",
        executableSha256: createHash("sha256").update(await readFile(executable)).digest("hex"),
        results,
        nativePointerVerified: false,
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
        const resolved = await realpath(directory);
        assert(resolved.startsWith(resolvedRoot + sep), "Cleanup escaped the isolated fixture");
        await rm(resolved, { recursive: true, force: true });
    }
}
