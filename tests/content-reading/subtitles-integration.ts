import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { mediaTool } from "../../components/content/media-runtime";
import { parseTimedText } from "../../components/content/subtitles/timed-text";

function require(value: unknown, message: string): asserts value {
    if (!value)
        throw new Error(message);
}

const codex = Bun.argv.includes("--codex");
const directory = resolve(process.env.QBUTT_SUBTITLE_QA_DIR || "C:/Temp/qbutt-subtitle-qa");
const appDirectory = resolve(process.env.QBUTT_CONTENT_APP_DIR || resolve(import.meta.dir, "../.."));
const profile = join(directory, `profile-${codex ? "codex" : "compatible"}-${crypto.randomUUID()}`);
await mkdir(directory, { recursive: true });
const source = `[Script Info]
ScriptType: v4.00+
PlayResX: 640
PlayResY: 360
[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,28,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,20,20,20,1
[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:04.00,Default,Rin,0,0,0,,{\\i1}Rin{\\i0} will find the hidden garden.
${codex ? "" : "Dialogue: 0,0:00:01.00,0:00:03.00,Default,Rin,0,0,0,,We will return tomorrow.\n"}\
${codex ? "" : "Dialogue: 0,0:00:01.00,0:00:04.00,Default,Rin,0,0,0,,{\\i1}Rin{\\i0} will find the hidden garden.\n"}\
Dialogue: 1,0:01:05.00,0:01:08.00,Default,Rin,0,0,0,,Rin keeps her promise.
Dialogue: 0,0:02:05.00,0:02:08.00,Default,Rin,0,0,0,,The hidden garden is beyond the hill.
Dialogue: 0,0:03:05.00,0:03:08.00,Default,Rin,0,0,0,,We will return tomorrow.
`;
const sourcePath = join(directory, `legal-original-${codex ? "codex" : "compatible"}.ass`);
await writeFile(sourcePath, source);
const originalHash = new Bun.CryptoHasher("sha256").update(source).digest("hex");
const ffmpeg = await mediaTool(appDirectory, "ffmpeg", new AbortController().signal);
const movie = join(directory, `legal-subtitles-${codex ? "codex" : "compatible"}.mkv`);
const generated = Bun.spawn([ffmpeg, "-nostdin", "-v", "error", "-f", "lavfi", "-i",
    "color=c=blue:s=160x90:r=1:d=240", "-i", sourcePath, "-map", "0:v", "-map", "1:s", "-c:v", "libx264",
    "-preset", "ultrafast", "-c:s", "ass", "-metadata:s:s:0", "language=eng", "-y", movie],
{ stdin: "ignore", stdout: "ignore", stderr: "pipe", windowsHide: true });
const generationError = await new Response(generated.stderr).text();
require(await generated.exited === 0, `Legal movie generation failed: ${generationError}`);

let apiCalls = 0;
let delay = false;
let observed: (() => void) | undefined;
let streamObserved: (() => void) | undefined;
let streamRequests = 0;
let unsafeChildRequests = 0;
const translatedTexts: string[] = [];
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, async fetch(request): Promise<Response> {
    const path = new URL(request.url).pathname;
    if (path === "/private-child.ts") {
        ++unsafeChildRequests;
        return new Response("A manifest must never request this private URL");
    }
    if (path.startsWith("/media/")) {
        ++streamRequests;
        streamObserved?.();
        streamObserved = undefined;
        if (path.endsWith("/unsafe.m3u8")) {
            return new Response(`#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nhttp://127.0.0.1:${server.port}/private-child.ts\n#EXT-X-ENDLIST\n`,
                { headers: { "Content-Type": "application/vnd.apple.mpegurl" } });
        }
        if (path.endsWith("/wait.mkv")) {
            await new Promise<void>(resolve => request.signal.addEventListener("abort", () => resolve(), { once: true }));
            return new Response(null, { status: 499 });
        }
        const file = Bun.file(movie);
        const range = request.headers.get("range")?.match(/^bytes=(\d+)-(\d*)$/);
        if (range) {
            const start = Number(range[1]);
            const end = range[2] ? Math.min(Number(range[2]), file.size - 1) : file.size - 1;
            return new Response(file.slice(start, end + 1), { status: 206, headers: {
                "Content-Type": "video/x-matroska", "Content-Range": `bytes ${start}-${end}/${file.size}`,
                "Accept-Ranges": "bytes", "Content-Length": String(end - start + 1),
            } });
        }
        return new Response(file, { headers: { "Content-Type": "video/x-matroska", "Accept-Ranges": "bytes" } });
    }
    require(!request.headers.has("cookie"), "Source cookie reached AI endpoint");
    const body = await request.json() as any;
    const input = JSON.parse(body.messages[1].content);
    ++apiCalls;
    observed?.();
    observed = undefined;
    if (delay) {
        await new Promise<void>(resolve => {
            const timer = setTimeout(resolve, 8000);
            request.signal.addEventListener("abort", () => { clearTimeout(timer); resolve(); }, { once: true });
        });
    }
    const blocks = input.blocks.map((block: { id: string; text: string }) => {
        translatedTexts.push(block.text);
        return { id: block.id, text: block.text === "Rin" ? "Рин"
            : block.text.includes("promise") ? "Рин сдержит обещание."
                : block.text.includes("beyond") ? "Скрытый сад находится за холмом."
                    : block.text.includes("tomorrow") ? "Мы вернёмся завтра."
                        : " найдёт скрытый сад." };
    });
    return Response.json({ choices: [{ finish_reason: "stop", message: {
        content: JSON.stringify({ blocks, terms: [{ source: "Rin", target: "Рин" }, { source: "hidden garden", target: "скрытый сад" }]
            .filter(term => input.blocks.some((block: { text: string }) => block.text.includes(term.source))) }),
    } }] });
} });

type Event = { id: number; event: string; data?: any };
const child = Bun.spawn([process.execPath, "components/content/main.ts", "--data-dir", profile,
    "--app-dir", appDirectory, "--download-dir", join(directory, "downloads")], {
    stdin: "pipe", stdout: "pipe", stderr: "pipe", windowsHide: true,
    env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: join(profile, "browser.json") },
});
let sequence = 0;
const waiting = new Map<number, { events: Event[]; resolve: (events: Event[]) => void; reject: (error: Error) => void }>();
const logs: Event[] = [];
const output = (async () => {
    let buffer = "";
    const decoder = new TextDecoder();
    for await (const bytes of child.stdout) {
        buffer += decoder.decode(bytes, { stream: true });
        let end: number;
        while ((end = buffer.indexOf("\n")) >= 0) {
            const event = JSON.parse(buffer.slice(0, end)) as Event;
            buffer = buffer.slice(end + 1);
            logs.push(event);
            const current = waiting.get(event.id);
            if (!current)
                continue;
            current.events.push(event);
            if (event.event === "complete" || event.event === "error") {
                waiting.delete(event.id);
                current.resolve(current.events);
            }
        }
    }
    for (const current of waiting.values())
        current.reject(new Error("Helper ended without a terminal response"));
})();
const errors = new Response(child.stderr).text();
function send(method: string, params: Record<string, unknown> = {}) {
    const id = ++sequence;
    const promise = new Promise<Event[]>((resolve, reject) => waiting.set(id, { events: [], resolve, reject }));
    child.stdin.write(JSON.stringify({ id, method, params }) + "\n");
    return { id, promise };
}
async function request(method: string, params: Record<string, unknown> = {}) {
    const events = await send(method, params).promise;
    const terminal = events.at(-1)!;
    require(terminal.event === "complete", `Helper failed ${method}: ${terminal.data?.message}`);
    return { events, result: terminal.data };
}
const params = { identity: "legal:subtitle-integration", url: pathToFileURL(movie).href,
    track: { id: 2, ffIndex: 1, codec: "ass", language: "eng" }, position: 0, targetLanguage: "ru" };
function translatedPath(events: Event[]): string {
    const asset = events.filter(event => event.event === "subtitle").at(-1);
    require(asset, `Subtitle asset missing: ${JSON.stringify(events.at(-1)?.data)}`);
    return fileURLToPath(asset.data.url);
}
const authPath = join(process.env.CODEX_HOME || join(process.env.USERPROFILE!, ".codex"), "auth.json");
const authBefore = codex ? createHash("sha256").update(await readFile(authPath)).digest("hex") : "";
const watchdog = setTimeout(() => child.kill(), codex ? 240_000 : 60_000);
try {
    await request("configureSource", { id: "ai", settings: codex ? { provider: "codex", model: "" }
        : { provider: "openai-compatible", baseUrl: `http://127.0.0.1:${server.port}/v1`,
            apiKey: "generated-subtitle-fixture", model: "subtitle-fixture" } });
    const initial = await request("subtitles.ahead", params);
    require(initial.result.status === "ready", "Initial subtitle window was not ready");
    const initialPath = translatedPath(initial.events);
    const initialDocument = parseTimedText(await readFile(initialPath, "utf8"));
    require(initialDocument.cues.length === (codex ? 2 : 4) && initialDocument.cues[0]!.startMs === 1000
        && initialDocument.cues.at(-1)!.startMs === 65000, "Extraction changed absolute subtitle timing or translated beyond the window");
    require(codex || initialDocument.cues[0]!.endMs === 4000 && initialDocument.cues[1]!.endMs === 3000,
        "Same-start ASS dialogue layering order changed");
    require(initialDocument.cues.every(cue => /[А-Яа-яЁё]/.test(cue.text)), "Translated window does not contain Russian subtitles");
    require((await readFile(initialPath, "utf8")).includes("{\\i1}") && (await readFile(initialPath, "utf8")).includes("{\\i0}"),
        "ASS styling was lost");
    const callsBeforeCache = apiCalls;
    const repeated = await request("subtitles.ahead", params);
    require(translatedPath(repeated.events) === initialPath && apiCalls === callsBeforeCache, "Cached window requested AI again");
    require(!repeated.events.some(event => event.event === "subtitleProgress"), "Cached window started another model turn");

    let laterPath = initialPath;
    if (!codex) {
        const later = await request("subtitles.ahead", { ...params, position: 60 });
        laterPath = translatedPath(later.events);
        const laterDocument = parseTimedText(await readFile(laterPath, "utf8"));
        require(laterDocument.cues.map(cue => cue.startMs).join(",") === "1000,1000,1000,65000,125000",
            "Second window lost translated rewind or changed cue timestamps");
        require(translatedTexts.filter(text => text.includes("promise")).length === 1,
            "Overlapping subtitle window translated the same cue again");
        await request("subtitles.ahead", { ...params, position: 180 });
        const rewind = await request("subtitles.ahead", params);
        require(parseTimedText(await readFile(translatedPath(rewind.events), "utf8")).cues.length === 6,
            "Rewind removed translated forward windows");
        const glossaryDirectory = join(profile, "reader", "glossaries");
        const glossaryFiles = Array.from(new Bun.Glob("*.json").scanSync(glossaryDirectory));
        require(glossaryFiles.length === 1 && JSON.parse(await readFile(join(glossaryDirectory, glossaryFiles[0]!), "utf8")).entries.Rin === "Рин",
            "Subtitle batches did not reuse the learned document glossary");

        const srtPath = join(directory, "external.srt");
        const srt = "1\r\n00:00:02,000 --> 00:00:05,500\r\n<i>Rin</i> keeps her promise.\r\n\r\n2\r\n00:03:05,000 --> 00:03:08,000\r\nWe will return tomorrow.\r\n";
        await writeFile(srtPath, srt);
        const external = await request("subtitles.ahead", { ...params, identity: "legal:external", track: {
            id: 3, codec: "subrip", language: "en", url: pathToFileURL(srtPath).href } });
        const externalText = await readFile(translatedPath(external.events), "utf8");
        require(externalText.includes("00:00:02,000 --> 00:00:05,500") && externalText.includes("<i>Рин</i>")
            && externalText.includes("We will return tomorrow."), "External subtitles lost timing, styling, or untranslated originals");
        require(await readFile(srtPath, "utf8") === srt, "External original was modified");

        const bitmap = await request("subtitles.ahead", { ...params, track: { id: 7, codec: "hdmv_pgs_subtitle" } });
        require(bitmap.result.status === "unsupported", "Bitmap subtitles advertised text translation");
        const russianPath = join(directory, "russian.srt");
        await writeFile(russianPath, "1\n00:00:01,000 --> 00:00:05,000\nРин найдёт скрытый сад.\n");
        const beforeRussian = apiCalls;
        const russian = await request("subtitles.ahead", { ...params, identity: "legal:russian", track: {
            id: 4, codec: "subrip", url: pathToFileURL(russianPath).href } });
        require(russian.result.status === "already-target" && apiCalls === beforeRussian, "Existing Russian subtitles were translated again");

        delay = true;
        const started = new Promise<void>(resolve => { observed = resolve; });
        const cancelled = send("subtitles.ahead", { ...params, identity: "legal:cancel" });
        await started;
        await request("cancel", { id: cancelled.id });
        const cancelledEvents = await cancelled.promise;
        delay = false;
        require(!cancelledEvents.some(event => event.event === "subtitle"), "Cancelled translation published a subtitle track");
        const afterCancellation = await request("subtitles.ahead", params);
        require(afterCancellation.result.status === "ready", "Cancellation damaged the next playback request");

        const privateStream = `http://127.0.0.1:${server.port}/media/${"a".repeat(32)}/legal.mkv`;
        const streamed = await request("subtitles.ahead", { ...params, identity: `torrent:${"a".repeat(40)}:0`, url: privateStream });
        require(streamed.result.status === "ready" && streamRequests > 0, "Owned torrent loopback subtitles could not be extracted");
        const extractionStarted = new Promise<void>(resolve => { streamObserved = resolve; });
        const heldExtraction = send("subtitles.ahead", { ...params, identity: "legal:cancel-extraction",
            url: privateStream.replace("/legal.mkv", "/wait.mkv") });
        await extractionStarted;
        const cancellationStart = performance.now();
        await request("cancel", { id: heldExtraction.id });
        const extractionEvents = await heldExtraction.promise;
        require(performance.now() - cancellationStart < 2000 && !extractionEvents.some(event => event.event === "subtitle"),
            "Cancel did not drain stalled torrent subtitle extraction promptly");
        const unsafeManifest = await send("subtitles.ahead", { ...params, identity: "legal:manifest-safety",
            url: privateStream.replace("/legal.mkv", "/unsafe.m3u8") }).promise;
        require(unsafeManifest.at(-1)?.event === "error" && unsafeChildRequests === 0,
            "Embedded manifest extraction bypassed the public network boundary");

        const utf16Path = join(directory, "utf16.srt");
        await writeFile(utf16Path, Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(srt, "utf16le")]));
        const utf16 = await request("subtitles.ahead", { ...params, identity: "legal:utf16", track: {
            id: 3, codec: "subrip", language: "en", url: pathToFileURL(utf16Path).href } });
        require(utf16.result.status === "ready", "UTF-16 BOM subtitle file could not be translated");

        const offsetMovie = join(directory, "nonzero-origin.mkv");
        const remuxed = Bun.spawn([ffmpeg, "-nostdin", "-v", "error", "-i", movie,
            "-map", "0", "-c", "copy", "-output_ts_offset", "100", "-y", offsetMovie],
        { stdin: "ignore", stdout: "ignore", stderr: "ignore", windowsHide: true });
        require(await remuxed.exited === 0, "Nonzero-origin movie generation failed");
        // Actual mpv demuxer-start-time is 99 for this pinned legal remux; its
        // original sub-start/full remains 1, unlike libavformat's reported start.
        const rebased = await request("subtitles.ahead", { ...params, identity: "legal:nonzero-origin",
            url: pathToFileURL(offsetMovie).href, startTime: 99 });
        const rebasedCues = parseTimedText(await readFile(translatedPath(rebased.events), "utf8")).cues;
        require(rebasedCues[0]!.startMs === 1000 && rebasedCues[0]!.endMs === 4000
            && rebasedCues.at(-1)!.startMs === 65000, "Nonzero source PTS was not rebased to the actual player clock");
    }
    require(new Bun.CryptoHasher("sha256").update(await readFile(sourcePath)).digest("hex") === originalHash,
        "Embedded original was modified");
    const authUnchanged = !codex || createHash("sha256").update(await readFile(authPath)).digest("hex") === authBefore;
    require(authUnchanged, "Codex authentication changed");
    const codexTurns = logs.filter(event => event.event === "subtitleProgress" && event.data?.provider === "codex"
        && event.data?.phase === "translating").length;
    require(!codex || codexTurns === 1, "Real Codex window was not translated exactly once");
    const evidence = { success: true, mode: codex ? "real-installed-codex" : "compatible-ipc", profile,
        movie, initialPath, laterPath, apiCalls, initialCues: initialDocument.cues.map(cue => ({ startMs: cue.startMs,
            endMs: cue.endMs, text: cue.text })), authUnchanged, originalUnchanged: true,
        timingPreserved: true, windowSeconds: 120, overlapCached: !codex, cancellationPassed: !codex,
        stalledExtractionCancelled: !codex, ownedTorrentStream: !codex, sameStartOrderPreserved: !codex,
        manifestChildRejected: !codex, nonzeroSourceClock: !codex, duplicateDialoguePreserved: !codex, codexTurns,
        completedAt: new Date().toISOString() };
    await writeFile(join(directory, `evidence-${codex ? "codex" : "compatible"}.json`), JSON.stringify(evidence, null, 2));
    await writeFile(join(directory, `ipc-${codex ? "codex" : "compatible"}.jsonl`), logs.filter(event => event.event !== "sources")
        .map(event => JSON.stringify(event)).join("\n"));
    console.log(JSON.stringify(evidence));
}
finally {
    clearTimeout(watchdog);
    child.stdin.end();
    const timer = setTimeout(() => child.kill(), 5000);
    await child.exited;
    clearTimeout(timer);
    await output;
    const stderr = await errors;
    if (child.exitCode)
        console.error(stderr.slice(0, 2000));
    await server.stop(true);
}
