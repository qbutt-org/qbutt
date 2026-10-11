import { cp, mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve, sep } from "node:path";

import { labAppearanceSettings } from "../appearance";
import { assertQtRuntime } from "../qt-runtime";

const driver = process.env.QBUTT_QT_ACCEPTANCE_EXE;
const product = process.env.QBUTT_LAB_EXE;
if (!driver || !product)
    throw new Error("Set QBUTT_QT_ACCEPTANCE_EXE to the built driver and QBUTT_LAB_EXE to the deployed portable executable.");

const root = await mkdtemp(join(tmpdir(), "qbutt-reader-ui-"));
const bundle = join(root, "bundle");
const profile = join(root, "profile");
const fixtureImages = join(root, "images");
const imageDir = join(root, "screenshots");
const fixtureLog = join(root, "fixture.jsonl");
await mkdir(bundle);
await mkdir(fixtureImages);
await mkdir(imageDir);
await mkdir(join(profile, "qbutt", "config"), { recursive: true });
await writeFile(fixtureLog, "");

let child: Bun.Subprocess<"ignore", "pipe", "pipe"> | undefined;
try {
    const productDir = dirname(resolve(product));
    for (const entry of await readdir(productDir, { withFileTypes: true })) {
        if (entry.isFile() && (entry.name.endsWith(".dll") || entry.name === "qt.conf"))
            await cp(join(productDir, entry.name), join(bundle, entry.name));
    }
    for (const folder of ["platforms", "styles", "iconengines", "imageformats", "sqldrivers"])
        await cp(join(productDir, folder), join(bundle, folder), { recursive: true });
    const executable = join(bundle, basename(driver));
    await cp(resolve(driver), executable);
    assertQtRuntime(bundle);
    const compile = Bun.spawn([process.execPath, "build", "--compile", join(import.meta.dir, "reader-flow-fixture.ts"),
        "--outfile", join(bundle, "qbutt-content.exe")], { stdout: "inherit", stderr: "inherit", windowsHide: true });
    if (await compile.exited !== 0)
        throw new Error("Reader JSON-lines fixture compilation failed.");

    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1", "[Network]", "PortForwardingEnabled=false",
        "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false", "WebUI\\Enabled=false",
        ...labAppearanceSettings("functional"), "",
    ].join("\n"));

    for (const readerSettingsProbe of [false, true]) {
        const specPath = join(root, readerSettingsProbe ? "spec-persistence.json" : "spec.json");
        const evidencePath = join(root, readerSettingsProbe ? "evidence-persistence.json" : "evidence.json");
        await writeFile(specPath, JSON.stringify({
            mode: "reader-flow", readerSettingsProbe, fixtureImages, imageDir, fixtureLog, evidencePath,
        }, null, 2));
        child = Bun.spawn([executable, `--profile=${profile}`], {
            cwd: bundle, windowsHide: true, stdin: "ignore", stdout: "pipe", stderr: "pipe", timeout: 90000,
            env: { ...process.env, QBUTT_QT_ACCEPTANCE_SPEC: specPath, QBUTT_QT_ACCEPTANCE_NATIVE: undefined,
                QBUTT_READER_UI_FIXTURE_LOG: fixtureLog, QBUTT_READER_UI_FIXTURE_IMAGES: fixtureImages,
                QT_QPA_PLATFORM: "offscreen", QT_SCALE_FACTOR: "1" },
        });
        const [exitCode, stdout, stderr] = await Promise.all([
            child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
        ]);
        const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
        if (exitCode !== 0 || evidence.status !== "passed")
            throw new Error(`Native Reader lifecycle acceptance failed (${exitCode}): ${evidence.error ?? ""}\n${stdout}\n${stderr}`);
        console.log(JSON.stringify({
            status: "passed", readerSettingsProbe, evidencePath, screenshots: evidence.screenshots, checks: evidence.checks,
        }, null, 2));
    }
} finally {
    if (child && child.exitCode === null) {
        child.kill();
        await child.exited;
    }
    const resolvedRoot = await realpath(root);
    for (const directory of [bundle, profile]) {
        const target = await realpath(directory);
        if (!target.startsWith(resolvedRoot + sep))
            throw new Error("Reader fixture cleanup escaped its temporary root.");
        await rm(target, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
    }
    // Retain only legal source images and compact IPC/evidence, not a second runtime or profile.
    console.log(`Reader lifecycle diagnostics: ${root}`);
}
