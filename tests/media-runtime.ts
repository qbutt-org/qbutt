import assert from "node:assert/strict";
import { copyFile, mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

import { mediaRuntime } from "../components/content/media-runtime";
import { assertQtRuntime } from "./qt-runtime";

// Production Player probes actual qplay, then decodes a generated legal clip.
// Native fault executables exercise the same process boundary without Bun IPC.
const driver = process.env.QBUTT_QT_ACCEPTANCE_EXE;
const bundle = process.env.QBUTT_QT_ACCEPTANCE_BUNDLE;
const qplay = process.env.QBUTT_LAB_QPLAY;
assert(driver && bundle && qplay,
    "Set QBUTT_QT_ACCEPTANCE_EXE, QBUTT_QT_ACCEPTANCE_BUNDLE and QBUTT_LAB_QPLAY");
assertQtRuntime(bundle);
process.env.QBUTT_LAB_PATHS = "1";
const verified = await mediaRuntime(bundle, AbortSignal.timeout(30_000));
const root = await mkdtemp(join(tmpdir(), "qbutt-native-media-runtime-"));
const executable = join(bundle, basename(driver));
if (resolve(driver) !== resolve(executable))
    await copyFile(driver, executable);
const qplayProfile = join(root, "qplay");
const media = join(root, "legal.mkv");
const generator = Bun.spawn([verified.tools.ffmpeg, "-hide_banner", "-loglevel", "error",
    "-f", "lavfi", "-i", "color=c=black:size=160x90:rate=12:duration=30",
    "-f", "lavfi", "-i", "sine=frequency=440:duration=30", "-c:v", "libx264",
    "-preset", "ultrafast", "-crf", "36", "-c:a", "aac", "-b:a", "32k", media],
{ stdin: "ignore", stdout: "pipe", stderr: "pipe", windowsHide: true, timeout: 30_000 });
const [generated, generationError] = await Promise.all([generator.exited, new Response(generator.stderr).text()]);
assert.equal(generated, 0, generationError);

const probeSource = join(root, "probe.cpp");
const probe = join(root, "probe.exe");
await writeFile(probeSource, `
#include <windows.h>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <string>
int main()
{
    if (FILE *trace = std::fopen(std::getenv("QBUTT_NATIVE_PROBE_TRACE"), "a"))
    {
        std::fprintf(trace, "%lu\\n", GetCurrentProcessId());
        std::fclose(trace);
    }
    const std::string mode = std::getenv("QBUTT_NATIVE_PROBE_MODE");
    if (mode == "timeout") { Sleep(60000); return 0; }
    if (mode == "delay") Sleep(300);
    if (mode == "exit") return 7;
    if (mode == "oversize")
    {
        const std::string output(65537, 'x');
        std::fwrite(output.data(), 1, output.size(), stdout);
        std::fflush(stdout);
        Sleep(60000);
        return 0;
    }
    std::ifstream file(std::getenv("QBUTT_NATIVE_PROBE_RESPONSE"), std::ios::binary);
    const std::string response((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
    std::fwrite(response.data(), 1, response.size(), stdout);
    return 0;
}
`);
const compiler = Bun.spawn(["cl.exe", "/nologo", "/EHsc", probeSource, `/Fe:${probe}`, `/Fo:${join(root, "probe.obj")}`],
    { cwd: root, windowsHide: true, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
const [compiled, compilerOutput, compilerError] = await Promise.all([compiler.exited,
    new Response(compiler.stdout).text(), new Response(compiler.stderr).text()]);
assert.equal(compiled, 0, `${compilerOutput}\n${compilerError}`);

const metadataProbe = Bun.spawn([qplay, "--runtime-info-json"],
    { stdin: "ignore", stdout: "pipe", stderr: "ignore", windowsHide: true, timeout: 30_000 });
const [metadataExit, metadataOutput] = await Promise.all([metadataProbe.exited, new Response(metadataProbe.stdout).text()]);
assert.equal(metadataExit, 0);
const info = JSON.parse(metadataOutput);
const corruptExecutable = join(root, "invalid.exe");
await writeFile(corruptExecutable, "This is not a Windows executable.");
const cases = [
    { name: "missing-player", player: join(root, "missing.exe"), error: "Install qplay" },
    { name: "failed-start", player: corruptExecutable, error: "could not start" },
    { name: "failed-probe", mode: "exit", error: "could not start" },
    { name: "old-version", info: { ...info, version: "1.0.0" }, error: "or newer" },
    { name: "wrong-protocol", info: { ...info, protocolVersion: String(info.protocolVersion) }, error: "or newer" },
    { name: "incomplete-tools", info: { ...info, tools: { ...info.tools, ytDlp: join(root, "missing-tool.exe") } }, error: "incomplete" },
    { name: "oversize", mode: "oversize", error: "invalid response" },
    { name: "timeout", mode: "timeout", error: "did not respond", timeoutMs: 35_000 },
    { name: "cancel-running-probe", mode: "timeout", stopDuringPreparation: true },
    { name: "shared-probe-cancel-player", mode: "delay", selections: [
        { name: "shared-probe-cancel-player", stopDuringPreparation: true, sharedProbe: true },
    ] },
    { name: "cancel-and-retry", mode: "timeout", selections: [
        { name: "cancel-and-retry", cancelPreparationAndRetry: true, contentKind: "anime", checkDownload: true },
    ] },
    { name: "native-playback", player: qplay, selections: [
        { name: "pending-stop", stopBeforeLoaded: true },
        { name: "reentrant-anime", supersedePending: true, reentrantSupersede: true, contentKind: "anime" },
        { name: "movie-replacement", supersedePlaying: true, contentKind: "movie" },
        { name: "ordinary-playback", legacyLaunch: true, contentKind: "movie" },
    ] },
];
const results = [];
const selectedCase = process.argv.find(argument => argument.startsWith("--case="))?.slice(7);
assert(!selectedCase || cases.some(scenario => scenario.name === selectedCase), "Unknown runtime scenario");
for (const scenario of cases.filter(scenario => !selectedCase || scenario.name === selectedCase)) {
    const directory = join(root, scenario.name);
    const profile = join(directory, "profile");
    await mkdir(join(profile, "qbutt", "config"), { recursive: true });
    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[LegalNotice]", "Accepted=true", "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false",
        "Session\\PeXEnabled=false", "Session\\Port=0", "Session\\InterfaceAddress=127.0.0.1", "[Network]",
        "PortForwardingEnabled=false", "[Core]", "AutoOpenTorrentFiles=false", "[Preferences]", "General\\Locale=en",
        "Advanced\\updateCheck=false", "WebUI\\Enabled=false", "Search\\SearchEnabled=false", "",
    ].join("\n"));
    const response = join(directory, "response.json");
    const probeTracePath = join(directory, "probes.txt");
    const evidencePath = join(directory, "evidence.json");
    const specPath = join(directory, "spec.json");
    await writeFile(response, JSON.stringify(scenario.info || info));
    const selections = (scenario.selections || [{ name: scenario.name, rejected: !scenario.stopDuringPreparation,
        stopDuringPreparation: scenario.stopDuringPreparation, timeoutMs: scenario.timeoutMs }])
        .map(selection => ({ identity: "video:generated:exact", expectedId: 1, minimumPosition: 1, ...selection }));
    await writeFile(specPath, JSON.stringify({ mode: "media-playback", mediaPath: media, evidencePath, probeTracePath,
        retryPlayerPath: qplay, selections }));
    const began = Date.now();
    const application = Bun.spawn([executable, `--profile=${profile}`], {
        cwd: bundle, windowsHide: true, stdin: "ignore", timeout: 50_000,
        env: { ...process.env, QT_QPA_PLATFORM: "offscreen", QSG_RHI_BACKEND: "software", QBUTT_QT_ACCEPTANCE_SPEC: specPath,
            QBUTT_LAB_PATHS: "1", QBUTT_LAB_QPLAY: scenario.player || probe, QBUTT_LAB_QPLAY_PROFILE: qplayProfile,
            QBUTT_NATIVE_PROBE_MODE: scenario.mode || "metadata", QBUTT_NATIVE_PROBE_RESPONSE: response,
            QBUTT_NATIVE_PROBE_TRACE: probeTracePath },
        stdout: Bun.file(join(directory, "stdout.log")), stderr: Bun.file(join(directory, "stderr.log")),
    });
    const exit = await application.exited;
    const elapsedMs = Date.now() - began;
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert.equal(exit, 0, JSON.stringify(evidence));
    assert.equal(evidence.status, "passed", JSON.stringify(evidence));
    if (scenario.error)
        assert(evidence.checks[0].error.includes(scenario.error), JSON.stringify(evidence));
    if (scenario.name === "timeout")
        assert(elapsedMs >= 29_000 && elapsedMs < 40_000, `Unexpected timeout duration ${elapsedMs}`);
    const pids = await readFile(probeTracePath, "utf8").then(text => text.trim().split("\n").map(Number)).catch(() => []);
    if (scenario.name === "shared-probe-cancel-player")
        assert.equal(pids.length, 1, "Concurrent playback and download tools requests started separate probes");
    results.push({ name: scenario.name, elapsedMs, pids, evidencePath });
}
const resultPath = join(root, "result.json");
await writeFile(resultPath, JSON.stringify({ status: "passed", results }, null, 2));
console.log(JSON.stringify({ status: "passed", root, resultPath }));
