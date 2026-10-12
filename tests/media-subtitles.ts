import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFile, cp, mkdir, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { labAppearanceSettings } from "./appearance.ts";
import { assertQtRuntime } from "./qt-runtime.ts";
import { mediaRuntime as resolveMediaRuntime } from "../components/content/media-runtime";

// Production Player and real mpv decode generated legal videos and subtitles.
// The default helper controls cancellation and deliberately late replies.
// QBUTT_SUBTITLE_REAL_HELPER exercises installed Codex and cached reopening.
const driver = process.env.QBUTT_QT_ACCEPTANCE_EXE;
const bundle = process.env.QBUTT_QT_ACCEPTANCE_BUNDLE;
const qplay = process.env.QBUTT_LAB_QPLAY;
const realHelper = process.env.QBUTT_SUBTITLE_REAL_HELPER;
const locale = process.argv.find(argument => argument.startsWith("--locale="))?.slice(9) || "en";
assert(["en", "ru"].includes(locale), "Use --locale=en or --locale=ru");
const localizationOnly = process.argv.includes("--localization");
const selectedScenario = process.argv.find(argument => argument.startsWith("--scenario="))?.slice(11);
assert(driver && bundle && qplay,
    "Set QBUTT_QT_ACCEPTANCE_EXE, QBUTT_QT_ACCEPTANCE_BUNDLE and QBUTT_LAB_QPLAY");
process.env.QBUTT_LAB_PATHS = "1";
const runtimeInfo = await resolveMediaRuntime(bundle, AbortSignal.timeout(30_000));
const mediaRuntime = runtimeInfo.runtimeDir;
assertQtRuntime(bundle);
const root = await mkdtemp(join(tmpdir(), "qbutt-media-subtitles-"));
const runtime = join(root, "runtime");
const profile = join(root, "profile");
const qplayProfile = join(root, "qplay");
const playerScript = join(qplayProfile, "config", "acceptance.lua");
const helperTrace = join(root, "helper-trace.jsonl");
const mpvTrace = join(root, "mpv-trace.jsonl");
const evidencePath = join(root, "qt-evidence.json");
const originalSrt = join(root, "original.srt");
const originalAss = join(root, "original.ass");
const originalRu = join(root, "existing-ru.srt");
const bare = join(root, "generated-bare.mkv");
const ass = join(root, "generated-ass.mkv");
const srt = join(root, "generated-srt.mkv");
const russian = join(root, "generated-russian.mkv");
const foreign = join(root, "generated-foreign.mkv");
const nonzero = join(root, "generated-nonzero.mkv");
const authPath = join(process.env.CODEX_HOME || join(process.env.USERPROFILE!, ".codex"), "auth.json");
const authBefore = realHelper ? createHash("sha256").update(await readFile(authPath)).digest("hex") : "";
let application: ReturnType<typeof Bun.spawn> | undefined;

async function run(command: string[], timeout = 30000) {
    const child = Bun.spawn(command, { cwd: mediaRuntime, stdin: "ignore", stdout: "pipe", stderr: "pipe", windowsHide: true, timeout });
    const [exit, output, error] = await Promise.all([child.exited,
        new Response(child.stdout).text(), new Response(child.stderr).text()]);
    assert.equal(exit, 0, `${command[0]} exited ${exit}\n${output}\n${error}`);
}

try {
    await mkdir(runtime);
    await mkdir(join(qplayProfile, "config"), { recursive: true });
    for (const entry of await readdir(bundle, { withFileTypes: true })) {
        if (entry.isFile() && (/\.dll$/i.test(entry.name)
            || ["qt.conf", "qbutt-net.exe", "qbutt-search.exe"].includes(entry.name)))
            await copyFile(join(bundle, entry.name), join(runtime, entry.name));
    }
    for (const name of ["platforms", "styles", "imageformats", "sqldrivers"])
        await cp(join(bundle, name), join(runtime, name), { recursive: true });
    const executable = join(runtime, "qbutt-qt-acceptance.exe");
    await copyFile(driver, executable);
    if (realHelper)
        await copyFile(realHelper, join(runtime, "qbutt-content.exe"));
    else
        await run([process.execPath, "build", "--compile", "--target=bun-windows-x64",
            join(import.meta.dir, "qt-acceptance", "subtitles-fixture.ts"), "--outfile", join(runtime, "qbutt-content.exe")]);
    await writeFile(join(qplayProfile, "config", "mpv.conf"),
        `vo=null\nao=null\nosc=no\nhwdec=no\nkeep-open=yes\nscript="${playerScript.replaceAll("\\", "/")}"\n`);
    await writeFile(join(qplayProfile, "config", "settings.json"),
        JSON.stringify({ language: locale, resume: false, frameGeneration: false }));
    await writeFile(playerScript, `
local mp = require("mp")
local utils = require("mp.utils")
local function record()
    local file = assert(io.open(os.getenv("QBUTT_SUBTITLE_MPV_TRACE"), "a"))
    file:write(utils.format_json({title=mp.get_property("media-title"),
        ipc=mp.get_property("options/input-ipc-server"), position=mp.get_property_number("time-pos", -1),
        sid=mp.get_property_native("sid"), text=mp.get_property("sub-text"),
        subtitles=mp.get_property_native("user-data/qbutt-subtitles"),
        tracks=mp.get_property_native("track-list")}), "\\n")
    file:close()
end
mp.register_event("file-loaded", function()
    if mp.get_property("media-title") == "early-user-off" then
        mp.set_property("sid", "no")
    end
    record()
end)
mp.observe_property("track-list", "native", record)
mp.observe_property("sub-text", "string", record)
mp.observe_property("user-data/qbutt-subtitles", "native", record)
`);

    await writeFile(originalSrt, "1\n00:00:00,000 --> 00:05:00,000\nOriginal English subtitle.\n\n");
    await writeFile(originalRu, "1\n00:00:00,000 --> 00:05:00,000\nИмеющиеся русские субтитры.\n\n");
    await writeFile(originalAss, `[Script Info]
ScriptType: v4.00+
PlayResX: 320
PlayResY: 180
[V4+ Styles]
Format: Name,Fontname,Fontsize,PrimaryColour,SecondaryColour,OutlineColour,BackColour,Bold,Italic,Underline,StrikeOut,ScaleX,ScaleY,Spacing,Angle,BorderStyle,Outline,Shadow,Alignment,MarginL,MarginR,MarginV,Encoding
Style: Default,Arial,18,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1
[Events]
Format: Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text
Dialogue: 0,0:00:00.00,0:05:00.00,Default,,0,0,0,,{\\i1}${realHelper ? "Rin will meet us by the river tomorrow." : "Original English ASS subtitle."}{\\i0}
`);
    const { ffmpeg } = runtimeInfo.tools;
    await run([ffmpeg, "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:size=320x180:rate=2:duration=300",
        "-f", "lavfi", "-i", "anullsrc=r=8000:cl=mono", "-t", "300", "-c:v", "libx264", "-preset", "ultrafast",
        "-crf", "40", "-c:a", "aac", "-b:a", "8k", bare]);
    for (const [input, output] of [[originalAss, ass], [originalSrt, srt]]) {
        await run([ffmpeg, "-hide_banner", "-loglevel", "error", "-i", bare, "-i", input,
            "-map", "0", "-map", "1", "-c", "copy", "-metadata:s:s:0", "language=eng",
            "-disposition:s:0", "default", output]);
    }
    await run([ffmpeg, "-hide_banner", "-loglevel", "error", "-i", ass, "-i", originalRu,
        "-map", "0", "-map", "1", "-c", "copy", "-metadata:s:s:1", "language=rus", "-disposition:s:1", "0", russian]);
    await run([ffmpeg, "-hide_banner", "-loglevel", "error", "-i", ass, "-i", originalSrt,
        "-map", "0", "-map", "1", "-c", "copy", "-metadata:s:s:1", "language=jpn", "-disposition:s:1", "0", foreign]);
    await run([ffmpeg, "-hide_banner", "-loglevel", "error", "-i", ass, "-map", "0", "-c", "copy",
        "-output_ts_offset", "100", nonzero]);
    const failures = [
        ["ai_unavailable", "Codex is unavailable. Install it and sign in, or choose another AI service in Sources.",
            "Codex недоступен. Установите его и войдите в аккаунт или выберите другой AI-сервис в разделе «Источники»."],
        ["ai_authentication", "Sign in to your AI service or check its API key in Sources.",
            "Войдите в AI-сервис или проверьте его API-ключ в разделе «Источники»."],
        ["ai_configuration", "Check the AI translation settings in Sources.", "Проверьте настройки AI-перевода в разделе «Источники»."],
        ["ai_timeout", "AI translation took too long. Retry.", "AI-перевод занял слишком много времени. Повторите попытку."],
        ["ai_provider", "The AI service could not complete the translation. Retry or check its settings in Sources.",
            "AI-сервис не смог завершить перевод. Повторите попытку или проверьте его настройки в разделе «Источники»."],
        ["ai_invalid-response", "The AI service returned an invalid translation. Retry or choose another model.",
            "AI-сервис вернул некорректный перевод. Повторите попытку или выберите другую модель."],
        ["ai_capability", "This AI service cannot translate with the current settings. Choose another service or model in Sources.",
            "AI-сервис не может выполнить перевод с текущими настройками. Выберите другой сервис или модель в разделе «Источники»."],
        ["ai_future", "Specific future AI failure.", "Specific future AI failure."],
    ];
    const authenticationMessage = failures[1]![locale === "ru" ? 2 : 1]!;
    const fixtureScenarios = (localizationOnly ? failures.map(([failureCode, en, ru]) => ({
        name: `localized-${failureCode}`, mode: "unavailable", mediaPath: ass, failureCode,
        expectedFailure: locale === "ru" ? ru : en,
    })) : [
        { name: "embedded-ass", mode: "reload", mediaPath: ass, expectedTracks: 4 },
        { name: "embedded-srt", mode: "reload", mediaPath: srt, expectedTracks: 4 },
        { name: "external-srt", mode: "reload", mediaPath: bare, externalSubtitle: originalSrt, expectedTracks: 4 },
        { name: "user-original", mode: "original", mediaPath: ass, expectedTracks: 4 },
        { name: "user-off", mode: "off", mediaPath: ass, expectedTracks: 4 },
        { name: "early-user-off", mode: "early-off", mediaPath: ass },
        { name: "pending-user-original", mode: "pending-original", mediaPath: ass },
        { name: "seek-pending", mode: "seek", mediaPath: ass },
        { name: "eof-rewind", mode: "eof-rewind", mediaPath: ass },
        { name: "eof-user-off", mode: "eof-off", mediaPath: ass },
        { name: "eof-user-hidden", mode: "eof-hidden", mediaPath: ass },
        { name: "subtitle-delay", mode: "delay", mediaPath: ass },
        { name: "nonzero-origin", mode: "reload", mediaPath: nonzero, expectedTracks: 4 },
        { name: "navigate-pending", mode: "navigate", mediaPath: ass },
        { name: "stop-pending", mode: "stop", mediaPath: ass },
        { name: "unavailable-nonfatal", mode: "unavailable", mediaPath: ass, expectedFailure: authenticationMessage },
        { name: "detected-russian", mode: "already-target", mediaPath: ass },
        { name: "no-subtitles", mode: "no-track", mediaPath: bare },
        { name: "existing-russian", mode: "existing-ru", mediaPath: russian },
        { name: "action-original", mode: "action-original", mediaPath: ass, expectedTracks: 4 },
        { name: "action-off", mode: "action-off", mediaPath: ass, expectedTracks: 4 },
        { name: "action-race", mode: "action-race", mediaPath: ass, expectedTracks: 4 },
        { name: "action-original-seek", mode: "action-original-seek", mediaPath: ass, expectedTracks: 4 },
        { name: "action-retarget", mode: "action-retarget", mediaPath: foreign, expectedTracks: 5 },
        { name: "retry-ai", mode: "retry", mediaPath: ass, expectedFailure: authenticationMessage },
        { name: "retry-player-load", mode: "retry-load", mediaPath: ass },
        { name: "late-track-after-eof", mode: "late-track", mediaPath: bare, lateSubtitle: originalSrt },
    ]).map(scenario => ({ ...scenario, identity: `subtitles:${scenario.name}` }))
        .filter(scenario => !selectedScenario || scenario.name === selectedScenario);
    assert(fixtureScenarios.length > 0, "Unknown subtitle scenario");
    const scenarios = realHelper ? ["real-codex", "real-codex-cached"].map(name => ({
        name, mode: "real-codex", mediaPath: ass, expectedTracks: 4, identity: "subtitles:real-codex",
    })) : fixtureScenarios;

    await mkdir(join(profile, "qbutt", "config"), { recursive: true });
    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[LegalNotice]", "Accepted=true", "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false",
        "Session\\PeXEnabled=false", "Session\\Port=0", "Session\\InterfaceAddress=127.0.0.1",
        "[Network]", "PortForwardingEnabled=false", "[Core]", "AutoOpenTorrentFiles=false",
        "[Preferences]", `General\\Locale=${locale}`, "Advanced\\updateCheck=false", ...labAppearanceSettings(), "",
    ].join("\n"));
    const specPath = join(root, "spec.json");
    const originalHash = createHash("sha256").update(await readFile(ass)).digest("hex");
    await writeFile(specPath, JSON.stringify({ mode: "subtitles-playback", realCodex: !!realHelper,
        scenarios, locale, mediaRuntime, helperTrace, mpvTrace, evidencePath }));
    application = Bun.spawn([executable, `--profile=${profile}`], {
        cwd: runtime, windowsHide: true, stdin: "ignore", timeout: realHelper ? 240000 : 180000,
        env: { ...process.env, QT_QPA_PLATFORM: "offscreen", QBUTT_QT_ACCEPTANCE_SPEC: specPath,
            QBUTT_SUBTITLE_FIXTURE_SPEC: specPath, QBUTT_SUBTITLE_MPV_TRACE: mpvTrace,
            QBUTT_BROWSER_BRIDGE_DISCOVERY: join(root, "browser.json"),
            QBUTT_LAB_PATHS: "1",
            QBUTT_LAB_QPLAY: qplay, QBUTT_LAB_QPLAY_PROFILE: qplayProfile },
        stdout: Bun.file(join(root, "app.stdout.log")), stderr: Bun.file(join(root, "app.stderr.log")),
    });
    const exit = await application.exited;
    let evidence;
    try {
        evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    }
    catch (error) {
        throw new Error(`Native subtitle acceptance exited ${exit}; missing or invalid evidence ${evidencePath}. `
            + `Logs: ${join(root, "app.stdout.log")}, ${join(root, "app.stderr.log")}`, { cause: error });
    }
    assert.equal(exit, 0, `${evidence.error}; exit ${exit}; evidence ${evidencePath}`);
    assert.equal(evidence.status, "passed", `Native acceptance failed; evidence ${evidencePath}`);
    assert.equal(evidence.checks.length, scenarios.length);
    assert.equal(createHash("sha256").update(await readFile(ass)).digest("hex"), originalHash, "The original media changed");
    if (realHelper) {
        assert(createHash("sha256").update(await readFile(authPath)).digest("hex") === authBefore,
            "Installed Codex authentication changed");
        await writeFile(join(root, "runtime-evidence.json"), JSON.stringify({ originalUnchanged: true,
            authUnchanged: true, helperSha256: createHash("sha256").update(await readFile(join(runtime, "qbutt-content.exe"))).digest("hex"),
            driverSha256: createHash("sha256").update(await readFile(executable)).digest("hex"),
            cachedReopen: true, codexTurns: evidence.checks.at(-1).codexTurns }, null, 2));
        console.log(`PASS real Player → real helper → installed Codex → decoded Russian subtitles and cached reopen; evidence ${evidencePath}`);
    }
    else {
        const requests = (await readFile(helperTrace, "utf8")).trim().split("\n")
            .map(line => JSON.parse(line)).filter(event => event.event === "request");
        for (const scenario of fixtureScenarios) {
            const own = requests.filter(request => request.identity === scenario.identity);
            const reposition = ["seek", "delay"].includes(scenario.mode);
            const eof = ["eof-rewind", "eof-off", "eof-hidden"].includes(scenario.mode);
            const explicit = scenario.mode.startsWith("action-") || ["retry", "retry-load"].includes(scenario.mode);
            const limit = scenario.mode === "eof-rewind" ? 3 : (reposition || eof || explicit ? 2 : 1);
            assert(own.length <= limit, `${scenario.name}: repeated ahead work`);
            if (explicit) assert.equal(own.length, 2, `${scenario.name}: explicit Translate/Retry did not start exactly one new request`);
            if (reposition) assert.equal(own.length, 2, "Reposition did not start exactly one new bounded request");
            if (eof) {
                const afterRewind = own.filter(request => request.position >= 180 && request.position < 300);
                assert.equal(afterRewind.length, scenario.mode === "eof-rewind" ? 1 : 0,
                    `${scenario.name}: rewind did not preserve automatic/user subtitle selection`);
            }
            const check = evidence.checks.find((item: any) => item.identity === scenario.identity);
            for (const request of own)
                assert.equal(request.startTime, scenario.externalSubtitle || scenario.lateSubtitle ? 0 : check.demuxerStartTime,
                    `${scenario.name}: actual mpv origin was not passed to the helper`);
            if (scenario.name === "nonzero-origin")
                assert(check.demuxerStartTime > 0, "The generated container had no nonzero origin");
            if (explicit || scenario.mode === "late-track") {
                const progress = check.states.map((state: any) => state.progress).filter(Boolean);
                assert(progress.some((value: any) => value.phase === "connecting" && value.provider === "codex"));
                assert(progress.some((value: any) => value.phase === "translating" && value.model === "fixture-model"));
                assert(progress.every((value: any) => value.completed === undefined && value.total === undefined),
                    "Subtitle progress invented work counts");
            }
        }
        console.log(`PASS ${scenarios.length} real Player/mpv subtitle lifecycle scenarios; evidence ${evidencePath}`);
    }
}
finally {
    if (application && application.exitCode === null) {
        application.kill();
        await application.exited;
    }
}
