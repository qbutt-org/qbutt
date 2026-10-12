import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFile, cp, mkdir, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { ProviderContext, ResolvedContent } from "../components/content/contracts.ts";
import { kinopubProvider } from "../components/content/providers/video/kinopub.ts";
import { SearchResults } from "../components/content/results.ts";
import { openLink, share } from "../components/content/sharing.ts";
import { mediaRuntime as resolveMediaRuntime } from "../components/content/media-runtime";
import { labAppearanceSettings } from "./appearance.ts";
import { assertQtRuntime } from "./qt-runtime.ts";

// Process integration: real HTTP metadata, source/share contracts, production
// Player and packaged mpv decoding generated media. No account or remote payload.
const driver = process.env.QBUTT_QT_ACCEPTANCE_EXE;
const bundle = process.env.QBUTT_QT_ACCEPTANCE_BUNDLE;
const qplay = process.env.QBUTT_LAB_QPLAY;
assert(driver && bundle && qplay,
    "Set QBUTT_QT_ACCEPTANCE_EXE, QBUTT_QT_ACCEPTANCE_BUNDLE and QBUTT_LAB_QPLAY");
process.env.QBUTT_LAB_PATHS = "1";
const runtimeInfo = await resolveMediaRuntime(bundle, AbortSignal.timeout(30_000));
const mediaRuntime = runtimeInfo.runtimeDir;
assertQtRuntime(bundle);
const root = await mkdtemp(join(tmpdir(), "qbutt-media-audio-"));
const runtime = join(root, "runtime");
const profile = join(root, "profile");
const qplayProfile = join(root, "qplay");
const playerScript = join(qplayProfile, "config", "acceptance.lua");
const media = join(root, "generated-voices.mkv");
const tracePath = join(root, "mpv-trace.jsonl");
const evidencePath = join(root, "qt-evidence.json");
const token = "generated-fixture-session";
interface Selection {
    name: string;
    identity: string;
    audioTrack: ResolvedContent["audioTrack"];
    expectedId: number;
    rejected: boolean;
    minimumPosition: number;
    switchTrack?: number;
    resume?: boolean;
    url?: string;
    proxyUrl?: string;
    legacyIdentity?: string;
    seedLegacyPosition?: number;
    firstPositionRange?: [number, number];
}
const voices = [
    { id: 15504, index: 10, lang: "rus", codec: "aac", channels: 1, author: { title: "Studio Beta" }, type: { title: "Многоголосый" } },
    { id: 15505, index: 1, lang: "rus", codec: "aac", channels: 1, author: { title: "Studio Alpha" }, type: { title: "Многоголосый" } },
    { id: 15506, index: 4, lang: "rus", codec: "aac", channels: 1, author: { title: "Ambiguous Studio" }, type: { title: "Многоголосый" } },
    { id: 15507, index: 3, lang: "rus", codec: "aac", channels: 1, author: { title: "Missing Studio" }, type: { title: "Многоголосый" } },
    { id: 15508, index: 22, lang: "rus", codec: "aac", channels: 1, author: { title: "Studio Beta" }, type: { title: "Дублированный" } },
];
const item = { id: 77, title: "Generated voice fixture", type: "movie", year: 2026, advert: false, videos: [{
    id: 99, title: "Generated episode", audios: voices, files: ["720p", "480p"].map(quality => ({
        quality, codec: "h264", url: { hls4: `https://media.example.invalid/${quality}/master.m3u8` },
    })),
}] };
let requests = 0;
const onlineUrls = ["first%2Btoken", "refreshed%2Btoken", "restart%2Btoken"].map(signature =>
    `http://media.example.invalid/generated-voices.mkv?episode=2&signature=${signature}`);
const proxyPassword = "generated-playback-proxy";
const proxyAuthorization = `Basic ${Buffer.from(`qbutt:${proxyPassword}`).toString("base64")}`;
const mediaRequests: { url: string; range: string | null }[] = [];
const server = Bun.serve({
    hostname: "127.0.0.1", port: 0,
    fetch(request) {
        const url = new URL(request.url);
        if (url.pathname === "/generated-voices.mkv") {
            if (request.headers.get("proxy-authorization") !== proxyAuthorization)
                return new Response("Fixture proxy authentication required", { status: 407,
                    headers: { "Proxy-Authenticate": "Basic realm=\"generated-media\"" } });
            // Deliberately require the full source URL. Stripping an expiring
            // query to stabilize a bookmark must never change network requests.
            if (!onlineUrls.includes(url.href))
                return new Response("The signed fixture URL changed", { status: 403 });
            const file = Bun.file(media);
            const range = request.headers.get("range");
            const match = range && /^bytes=(\d+)-(\d*)$/.exec(range);
            const start = match ? Number(match[1]) : 0;
            const end = match?.[2] ? Math.min(Number(match[2]), file.size - 1) : file.size - 1;
            if ((range && !match) || start > end || start < 0)
                return new Response(null, { status: 416, headers: { "Content-Range": `bytes */${file.size}` } });
            mediaRequests.push({ url: url.href, range });
            const headers = new Headers({ "Content-Type": "video/x-matroska", "Accept-Ranges": "bytes",
                "Content-Length": String(end - start + 1) });
            if (match) headers.set("Content-Range", `bytes ${start}-${end}/${file.size}`);
            return new Response(request.method === "HEAD" ? null : file.slice(start, end + 1),
                { status: match ? 206 : 200, headers });
        }
        requests++;
        if (request.headers.get("authorization") !== `Bearer ${token}`)
            return new Response("Fixture authentication required", { status: 401 });
        const path = url.pathname;
        if (path === "/v1/items/search") return Response.json({ items: [item] });
        if (path === "/v1/items/77") return Response.json({ item });
        return new Response("Unknown fixture endpoint", { status: 404 });
    },
});
let application: ReturnType<typeof Bun.spawn> | undefined;
try {
    const context: ProviderContext = {
        settings: { accessToken: token, apiUrl: `${server.url}v1/` },
        dataDir: root, signal: AbortSignal.timeout(180000),
        fetch: (url, init) => fetch(url, init),
    };
    const [work] = await kinopubProvider.search({ query: "Generated", category: "movies" }, context);
    assert(work);
    assert.equal(work.variants.length, 10, "Each voice and quality needs its own variant");
    assert.equal(new Set(work.variants.map(variant => variant.id)).size, 10);
    const selected = work.variants.find(variant => variant.id.endsWith(":audio:15504") && variant.quality === "720p")!;
    assert(selected.translation?.includes("Studio Beta") && !selected.translation.includes("Studio Alpha"));
    assert.deepEqual(selected.languages, ["rus"]);
    const shared = share({ mode: "exact", work, variant: selected });
    const reopened = openLink(shared);
    assert.equal(reopened.variant?.id, selected.id);
    assert(!shared.includes(token), "Account data escaped into a share link");

    // API array order and provider indexes deliberately differ from mpv IDs.
    // Re-inspection must retain the exact API voice ID, never its array offset.
    item.videos[0].audios.reverse();
    const details = await kinopubProvider.inspect!(reopened.variant!, context);
    const exact = details.variants.find(variant => variant.id === reopened.variant!.id)!;
    assert(exact, "Shared voice was lost after source reordering");
    const resolved = await kinopubProvider.resolve!(exact, context);
    assert.deepEqual(resolved.audioTrack, { title: "Studio Beta", type: "Многоголосый", language: "rus", codec: "aac", channels: 1 });
    const selections: Selection[] = [];
    for (const [id, name, expectedId, rejected] of [
        [15504, "voice-beta", 2, false],
        [15505, "voice-alpha", 1, false],
        [15506, "voice-ambiguous", 0, true],
        [15507, "voice-missing", 0, true],
        [15508, "voice-same-author-wrong-type", 0, true],
    ] as const) {
        const variant = details.variants.find(candidate => candidate.id.endsWith(`:audio:${id}`) && candidate.quality === "720p")!;
        const resolution = await kinopubProvider.resolve!(variant, context);
        selections.push({ name, identity: `video:${variant.sourceId}:${variant.id}`, audioTrack: resolution.audioTrack, expectedId,
            rejected, minimumPosition: id === 15504 ? 2 : 1, ...(id === 15504 ? { switchTrack: 1 } : {}) });
    }
    selections.splice(1, 0, { ...selections[0], name: "voice-beta-reopen", resume: true, switchTrack: undefined });

    const fileSelection = { identity: "", audioTrack: resolved.audioTrack, expectedId: 2, rejected: false };
    selections.push(
        { ...fileSelection, name: "file-open", minimumPosition: 2 },
        { ...fileSelection, name: "file-reopen", resume: true, minimumPosition: 3 },
    );

    // The actual result owner changes the canonical work as another catalog
    // arrives. Its retained exact video must still use the same bookmark.
    const results = new SearchResults("Generated", () => {});
    assert(results.add(work));
    const originalWorkId = results.find(work)!.id;
    assert(results.add({ ...work, id: "catalog-fixture:77", variants: [
        { ...selected, id: "catalog-fixture:77:video", sourceId: "catalog-fixture" },
    ] }));
    const regrouped = results.find(work)!;
    assert.notEqual(regrouped.id, originalWorkId, "The fixture must exercise a real canonical work replacement");
    const onlineVariant = regrouped.variants.find(variant => variant.id.endsWith(":audio:15504") && variant.quality === "480p")!;
    const restartVariant = regrouped.variants.find(variant => variant.id.endsWith(":audio:15505") && variant.quality === "480p")!;
    assert(onlineVariant && restartVariant);
    const online = { identity: `video:${onlineVariant.sourceId}:${onlineVariant.id}`,
        audioTrack: resolved.audioTrack, expectedId: 2, rejected: false,
        proxyUrl: `http://qbutt:${proxyPassword}@127.0.0.1:${server.port}` };
    selections.push(
        { ...online, name: "online-legacy-import", url: onlineUrls[0], resume: true, minimumPosition: 9.5,
            legacyIdentity: `${originalWorkId}/${onlineVariant.id}`, seedLegacyPosition: 9.25,
            firstPositionRange: [9, 10] },
        { ...online, name: "online-regrouped-reopen", url: onlineUrls[1], resume: true, minimumPosition: 10.5,
            legacyIdentity: `${regrouped.id}/${onlineVariant.id}`, seedLegacyPosition: 14.25 },
        { ...online, name: "online-explicit-restart", url: onlineUrls[2],
            identity: `video:${restartVariant.sourceId}:${restartVariant.id}`,
            audioTrack: selections.find(selection => selection.name === "voice-alpha")!.audioTrack, expectedId: 1,
            minimumPosition: 1, legacyIdentity: `${originalWorkId}/${restartVariant.id}`, seedLegacyPosition: 14.25,
            firstPositionRange: [0, 0.8] },
    );
    const removed = item.videos[0].audios.findIndex(audio => audio.id === 15504);
    item.videos[0].audios.splice(removed, 1);
    await assert.rejects(() => kinopubProvider.resolve!(exact, context), /озвучка.*больше не доступна/i);

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
    await writeFile(join(qplayProfile, "config", "mpv.conf"),
        `vo=null\nao=null\nosc=no\nhwdec=no\nkeep-open=yes\nscript="${playerScript.replaceAll("\\", "/")}"\n`);
    await writeFile(playerScript, `
local mp = require("mp")
local utils = require("mp.utils")
local function record(event)
    local file = assert(io.open(os.getenv("QBUTT_MEDIA_AUDIO_TRACE"), "a"))
    file:write(utils.format_json({event=event, title=mp.get_property("media-title"),
        ipc=mp.get_property("options/input-ipc-server"), pause=mp.get_property_native("pause"),
        aid=mp.get_property_native("aid"), position=mp.get_property_number("time-pos", -1),
        tracks=mp.get_property_native("track-list")}), "\\n")
    file:close()
end
mp.add_hook("on_preloaded", 50, function() record("preloaded") end)
mp.observe_property("pause", "bool", function() record("pause") end)
mp.register_event("file-loaded", function() record("file-loaded") end)
`);
    const { ffmpeg } = runtimeInfo.tools;
    const args = [ffmpeg, "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=12:duration=30"];
    for (const frequency of [440, 660, 880, 990])
        args.push("-f", "lavfi", "-i", `sine=frequency=${frequency}:duration=30`);
    args.push("-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:a", "-map", "4:a",
        "-c:v", "libx264", "-preset", "ultrafast", "-crf", "36", "-c:a", "aac", "-b:a", "32k");
    for (const [index, title] of ["01. Многоголосый. Studio Alpha (RUS)", "10. Многоголосый. Studio Beta (RUS)",
        "04. Многоголосый. Ambiguous Studio (RUS)", "05. Многоголосый. Ambiguous Studio (RUS)"].entries())
        args.push(`-metadata:s:a:${index}`, "language=rus", `-metadata:s:a:${index}`, `title=${title}`);
    args.push(media);
    const generator = Bun.spawn(args, {
        cwd: mediaRuntime, stdin: "ignore", stdout: "pipe", stderr: "pipe", windowsHide: true, timeout: 30000,
    });
    const [generated, generatorErrors] = await Promise.all([generator.exited, new Response(generator.stderr).text()]);
    assert.equal(generated, 0, generatorErrors);
    await mkdir(join(profile, "qbutt", "config"), { recursive: true });
    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[LegalNotice]", "Accepted=true", "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false",
        "Session\\PeXEnabled=false", "Session\\Port=0", "Session\\InterfaceAddress=127.0.0.1",
        "[Network]", "PortForwardingEnabled=false", "[Core]", "AutoOpenTorrentFiles=false",
        "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false", ...labAppearanceSettings(), "",
    ].join("\n"));
    const specPath = join(root, "spec.json");
    await writeFile(specPath, JSON.stringify({ mode: "media-playback", mediaPath: media, tracePath, evidencePath, selections }));
    application = Bun.spawn([executable, `--profile=${profile}`], {
        cwd: runtime, windowsHide: true, stdin: "ignore", timeout: 120000,
        env: { ...process.env, QT_QPA_PLATFORM: "offscreen", QBUTT_QT_ACCEPTANCE_SPEC: specPath,
            QBUTT_MEDIA_AUDIO_TRACE: tracePath, QBUTT_LAB_PATHS: "1",
            QBUTT_LAB_QPLAY: qplay, QBUTT_LAB_QPLAY_PROFILE: qplayProfile },
        stdout: Bun.file(join(root, "app.stdout.log")), stderr: Bun.file(join(root, "app.stderr.log")),
    });
    const exit = await application.exited;
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert.equal(exit, 0, JSON.stringify(evidence));
    assert.equal(evidence.status, "passed", JSON.stringify(evidence));
    for (const [previousName, reopenedName] of [
        ["file-open", "file-reopen"], ["online-legacy-import", "online-regrouped-reopen"],
    ]) {
        const previous = evidence.checks.find((check: { name: string }) => check.name === previousName);
        const reopened = evidence.checks.find((check: { name: string }) => check.name === reopenedName);
        assert(previous && reopened);
        assert(Math.abs(reopened.firstPosition - previous.savedPosition) < 0.5,
            `${reopenedName}: reopening did not restore the stopped player's persisted position`);
    }
    for (const url of onlineUrls)
        assert(mediaRequests.some(request => request.url === url), `The full signed media URL was not requested: ${url}`);
    assert(mediaRequests.some(request => request.range), "Actual online playback did not exercise HTTP Range");
    assert.equal(evidence.checks.find((check: { name: string }) => check.name === "online-legacy-import").legacyPosition, null);
    for (const name of ["online-regrouped-reopen", "online-explicit-restart"])
        assert.equal(evidence.checks.find((check: { name: string }) => check.name === name).legacyPosition, 14.25);
    const trace = (await readFile(tracePath, "utf8")).trim().split("\n").map(line => JSON.parse(line));
    for (const selection of selections) {
        const events = trace.filter(event => event.title === selection.name);
        assert(events.some(event => event.event === "preloaded" && event.pause === true), `${selection.name}: voice output was not initially paused`);
        const firstOutput = events.find(event => event.pause === false);
        if (selection.rejected)
            assert(!firstOutput, `${selection.name}: a rejected voice became audible`);
        else {
            assert(firstOutput, `${selection.name}: actual playback never unpaused`);
            assert.equal(firstOutput.aid, selection.expectedId, `${selection.name}: wrong voice at first output`);
        }
    }
    await writeFile(join(root, "source-evidence.json"), JSON.stringify({ status: "passed", requests, voices: 5, variants: 10,
        selectedApiId: 15504, selectedApiIndex: 10, actualMpvId: 2, reorderedSharePreserved: true,
        unavailableApiVoiceRejected: true, sameAuthorWrongTypeRejected: true, liveKinoPub: "unverified-account-required",
        bookmarks: { originalWorkId, regroupedWorkId: regrouped.id, stableIdentity: online.identity,
            fileResume: true, changedUrlResume: true, legacyImportedOnce: true, stableWins: true, explicitRestartSkipsImport: true },
        mediaRequests,
        driver: { path: driver, sha256: createHash("sha256").update(await readFile(driver)).digest("hex") },
        mediaRuntime,
    }, null, 2));
    console.log(JSON.stringify({ status: "passed", root, evidencePath, requests, selections: selections.length }));
}
finally {
    application?.kill();
    await application?.exited;
    server.stop(true);
}
