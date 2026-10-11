import { cp, mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve, sep } from "node:path";
import { share } from "../../components/content/sharing";
import type { Variant, Work } from "../../components/content/contracts";
import { labAppearanceSettings } from "../appearance";
import { assertQtRuntime } from "../qt-runtime";
import { allowLabNetwork } from "../windows-firewall";

const driver = resolve(process.env.QBUTT_QT_ACCEPTANCE_EXE ?? "");
const product = resolve(process.env.QBUTT_LAB_EXE ?? "");
const runtime = resolve(process.env.QBUTT_MEDIA_RUNTIME_DIR ?? "");
if (!process.env.QBUTT_QT_ACCEPTANCE_EXE || !process.env.QBUTT_LAB_EXE || !process.env.QBUTT_MEDIA_RUNTIME_DIR)
    throw new Error("Set QBUTT_QT_ACCEPTANCE_EXE to the built driver, QBUTT_LAB_EXE to the portable executable and QBUTT_MEDIA_RUNTIME_DIR to the lab media runtime.");

const root = await mkdtemp(join(tmpdir(), "qbutt-content-ui-"));
const bundle = join(root, "bundle");
const profile = join(root, "profile");
const evidencePath = join(root, "evidence.json");
const fixtureLog = join(root, "fixture.jsonl");
const mediaPath = join(root, "generated-audio.wav");
await mkdir(bundle);
await mkdir(join(profile, "qbutt", "config"), { recursive: true });
await writeFile(fixtureLog, "");

let child: Bun.Subprocess<"ignore", "pipe", "pipe"> | undefined;
let passed = false;
try {
    for (const entry of await readdir(dirname(product), { withFileTypes: true })) {
        if (entry.isFile() && (entry.name.endsWith(".dll") || entry.name === "qt.conf"))
            await cp(join(dirname(product), entry.name), join(bundle, entry.name));
    }
    for (const folder of ["platforms", "styles", "iconengines", "imageformats", "sqldrivers"])
        await cp(join(dirname(product), folder), join(bundle, folder), { recursive: true });
    await mkdir(join(bundle, "media", "profile", "scripts"), { recursive: true });
    for (const entry of await readdir(runtime, { withFileTypes: true })) {
        if (entry.isFile() && (entry.name === "mpv.exe" || entry.name.endsWith(".dll")))
            await cp(join(runtime, entry.name), join(bundle, "media", entry.name));
    }
    await mkdir(join(bundle, "media", "profile", "tools", "bin"), { recursive: true });
    await mkdir(join(bundle, "media", "tools"));
    await cp(join(runtime, "profile", "tools", "bin", "ffprobe.exe"),
        join(bundle, "media", "profile", "tools", "bin", "ffprobe.exe"));
    await cp(join(dirname(product), "media", "profile", "scripts", "modernx.lua"),
        join(bundle, "media", "profile", "scripts", "modernx.lua"));
    await writeFile(join(bundle, "media", "profile", "mpv.conf"), "vo=null\nao=null\nforce-window=no\nosc=no\nkeep-open=yes\n");
    const samples = 16000 * 5;
    const audio = Buffer.alloc(44 + samples * 2);
    audio.write("RIFF", 0);
    audio.writeUInt32LE(audio.length - 8, 4);
    audio.write("WAVEfmt ", 8);
    audio.writeUInt32LE(16, 16);
    audio.writeUInt16LE(1, 20);
    audio.writeUInt16LE(1, 22);
    audio.writeUInt32LE(16000, 24);
    audio.writeUInt32LE(32000, 28);
    audio.writeUInt16LE(2, 32);
    audio.writeUInt16LE(16, 34);
    audio.write("data", 36);
    audio.writeUInt32LE(samples * 2, 40);
    await writeFile(mediaPath, audio);
    const executable = join(bundle, basename(driver));
    await cp(driver, executable);
    assertQtRuntime(bundle);
    await allowLabNetwork([executable]);
    const helper = Bun.spawn([process.execPath, "build", "--compile",
        join(import.meta.dir, "content-fixture.ts"), "--outfile", join(bundle, "qbutt-content.exe")],
        { stdout: "inherit", stderr: "inherit", windowsHide: true });
    if (await helper.exited !== 0)
        throw new Error("Content fixture compilation failed.");

    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1", "[Network]", "PortForwardingEnabled=false",
        "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false",
        "WebUI\\Enabled=false", ...labAppearanceSettings("functional"), "",
    ].join("\n"));
    const reported: Variant = { id: "reported", sourceId: "ads", sourceName: "Fixture video", title: "reported",
        url: "https://video.example/reported", transport: "video" };
    const work: Work = { id: "shared", title: "Original video fixture", category: "movies", variants: [reported] };
    const episode: Variant = { id: "collision", sourceId: "fixture-yani", sourceName: "YummyAnime",
        title: "Original episode 2", url: "https://yani.example/original", transport: "video",
        episode: "2", translation: "Studio B", player: "CVH" };
    const series: Work = { id: "original-series", title: "Original generated series", category: "anime", variants: [episode] };
    const specPath = join(root, "spec.json");
    await writeFile(specPath, JSON.stringify({ mode: "content-library", evidencePath, fixtureLog, mediaPath,
        reportedLink: share({ mode: "exact", work, variant: reported }),
        missingLink: share({ mode: "exact", work, variant: { ...reported, id: "removed" } }),
        episodeLink: share({ mode: "exact", work: series, variant: episode }),
    }, null, 2));
    child = Bun.spawn([executable, `--profile=${profile}`], {
        cwd: bundle, windowsHide: true, stdin: "ignore", stdout: "pipe", stderr: "pipe", timeout: 45000,
        env: { ...process.env, QBUTT_QT_ACCEPTANCE_SPEC: specPath, QBUTT_CONTENT_UI_FIXTURE_LOG: fixtureLog,
            QBUTT_CONTENT_UI_MEDIA: mediaPath,
            QBUTT_CONTENT_UI_RUNTIME: join(bundle, "media"),
            QT_QPA_PLATFORM: "offscreen", QT_SCALE_FACTOR: "1" },
    });
    const [exitCode, stdout, stderr] = await Promise.all([
        child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ]);
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    if (exitCode !== 0 || evidence.status !== "passed")
        throw new Error(`Native content acceptance failed (${exitCode}): ${evidence.error ?? ""}\n${stdout}\n${stderr}`);
    passed = true;
    console.log(JSON.stringify({ status: "passed", evidencePath, checks: evidence.checks }, null, 2));
} finally {
    if (child && child.exitCode === null) {
        child.kill();
        await child.exited;
    }
    // Keep compact diagnostics, never a duplicate Qt runtime or test profile.
    const resolvedRoot = await realpath(root);
    for (const directory of [bundle, profile]) {
        const target = await realpath(directory);
        if (!target.startsWith(resolvedRoot + sep))
            throw new Error("Content fixture cleanup escaped its temporary root.");
        await rm(target, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
    }
    if (await Bun.file(mediaPath).exists())
        await rm(mediaPath);
    if (!passed)
        console.error(`Content acceptance diagnostics: ${root}`);
}
