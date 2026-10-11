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
const qplay = process.env.QBUTT_LAB_QPLAY;
const localizationOnly = process.argv.includes("--localization");
if (!process.env.QBUTT_QT_ACCEPTANCE_EXE || !process.env.QBUTT_LAB_EXE || (!localizationOnly && !qplay))
    throw new Error("Set QBUTT_QT_ACCEPTANCE_EXE to the built driver, QBUTT_LAB_EXE to the portable executable and QBUTT_LAB_QPLAY to the real player.");

const root = await mkdtemp(join(tmpdir(), "qbutt-content-ui-"));
const bundle = join(root, "bundle");
const profile = join(root, "profile");
const qplayProfile = join(root, "qplay");
const fixtureLog = join(root, "fixture.jsonl");
const mpvTrace = join(root, "mpv-trace.jsonl");
const mediaPath = join(root, "generated-audio.wav");
await mkdir(bundle);
await mkdir(profile);
await mkdir(qplayProfile);
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
    await mkdir(join(qplayProfile, "config"), { recursive: true });
    const playerScript = join(qplayProfile, "config", "acceptance.lua");
    await writeFile(join(qplayProfile, "config", "mpv.conf"),
        `vo=null\nao=null\nforce-window=no\nosc=no\nkeep-open=yes\nscript="${playerScript.replaceAll("\\", "/")}"\n`);
    await writeFile(join(qplayProfile, "config", "settings.json"),
        JSON.stringify({ language: "en", resume: false, frameGeneration: false }));
    await writeFile(playerScript, `
local mp = require("mp")
local utils = require("mp.utils")
local function record(event)
    local file = assert(io.open(os.getenv("QBUTT_CONTENT_UI_MPV_TRACE"), "a"))
    file:write(utils.format_json({event=event, time=mp.get_time(),
        ipc=mp.get_property("options/input-ipc-server"), title=mp.get_property("media-title"),
        path=mp.get_property("path"), idle=mp.get_property_native("idle-active")}), "\\n")
    file:close()
end
for _, event in ipairs({"start-file", "file-loaded", "end-file", "shutdown"}) do
    mp.register_event(event, function() record(event) end)
end
record("startup")
`);
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

    const reported: Variant = { id: "reported", sourceId: "ads", sourceName: "Fixture video", title: "reported",
        url: "https://video.example/reported", transport: "video" };
    const work: Work = { id: "shared", title: "Original video fixture", category: "movies", variants: [reported] };
    const episode: Variant = { id: "collision", sourceId: "fixture-yani", sourceName: "YummyAnime",
        title: "Original episode 2", url: "https://yani.example/original", transport: "video",
        episode: "2", translation: "Studio B", player: "CVH" };
    const series: Work = { id: "original-series", title: "Original generated series", category: "anime", variants: [episode] };
    for (const locale of localizationOnly ? ["en", "ru"] : ["en"]) {
        const activeProfile = join(profile, locale);
        await mkdir(join(activeProfile, "qbutt", "config"), { recursive: true });
        await writeFile(join(activeProfile, "qbutt", "config", "qbutt.ini"), [
            "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
            "Session\\InterfaceAddress=127.0.0.1", "[Network]", "PortForwardingEnabled=false",
            "[Preferences]", `General\\Locale=${locale}`, "Advanced\\updateCheck=false",
            "WebUI\\Enabled=false", ...labAppearanceSettings("functional"), "",
        ].join("\n"));
        const evidencePath = join(root, `evidence-${locale}.json`);
        const specPath = join(root, `spec-${locale}.json`);
        await writeFile(specPath, JSON.stringify({ mode: "content-library", evidencePath, fixtureLog, mediaPath,
            locale, localizationOnly, qplay, mpvTrace,
            reportedLink: share({ mode: "exact", work, variant: reported }),
            missingLink: share({ mode: "exact", work, variant: { ...reported, id: "removed" } }),
            episodeLink: share({ mode: "exact", work: series, variant: episode }),
        }, null, 2));
        child = Bun.spawn([executable, `--profile=${activeProfile}`], {
            cwd: bundle, windowsHide: true, stdin: "ignore", stdout: "pipe", stderr: "pipe", timeout: 45000,
            env: { ...process.env, QBUTT_QT_ACCEPTANCE_SPEC: specPath, QBUTT_CONTENT_UI_FIXTURE_LOG: fixtureLog,
                QBUTT_CONTENT_UI_MEDIA: mediaPath,
                QBUTT_CONTENT_UI_MPV_TRACE: mpvTrace,
                QBUTT_LAB_PATHS: "1", QBUTT_LAB_QPLAY: qplay, QBUTT_LAB_QPLAY_PROFILE: qplayProfile,
                QT_QPA_PLATFORM: "offscreen", QT_SCALE_FACTOR: "1" },
        });
        const [exitCode, stdout, stderr] = await Promise.all([
            child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
        ]);
        await Promise.all([
            writeFile(join(root, `app-${locale}.stdout.log`), stdout),
            writeFile(join(root, `app-${locale}.stderr.log`), stderr),
        ]);
        const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
        if (exitCode !== 0 || evidence.status !== "passed")
            throw new Error(`Native content acceptance failed (${exitCode}): ${evidence.error ?? ""}\n${stdout}\n${stderr}`);
        console.log(JSON.stringify({ status: "passed", locale, evidencePath, checks: evidence.checks }, null, 2));
    }
    passed = true;
} finally {
    if (child && child.exitCode === null) {
        child.kill();
        await child.exited;
    }
    // Keep compact diagnostics, never a duplicate Qt runtime or test profile.
    const resolvedRoot = await realpath(root);
    for (const directory of [bundle, profile, qplayProfile]) {
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
