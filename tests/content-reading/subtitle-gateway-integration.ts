import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import type { ProviderContext, ResolvedContent } from "../../components/content/contracts";
import { mediaTool } from "../../components/content/media-runtime";
import { PlaybackStreams } from "../../components/content/streams";
import { parseTimedText } from "../../components/content/subtitles/timed-text";
import { ContentClient, completed } from "../content-search/client";

// Exercise the real gateway owner and main helper IPC. The existing constructor
// connector permits only this generated source, without changing production DNS policy.
const root = await mkdtemp(join(tmpdir(), "qbutt-subtitle-gateway-"));
const fixtures = join(root, "fixtures");
await mkdir(fixtures);
const appDir = resolve(process.env.QBUTT_CONTENT_APP_DIR || resolve(import.meta.dir, "../.."));
process.env.QBUTT_CONTENT_APP_DIR = appDir;
const ffmpeg = await mediaTool(appDir, "ffmpeg", new AbortController().signal);

async function command(args: string[]): Promise<string> {
    const child = Bun.spawn([ffmpeg, "-nostdin", "-hide_banner", "-v", "error", ...args], {
        cwd: fixtures, windowsHide: true, stdin: "ignore", stdout: "pipe", stderr: "pipe", timeout: 30000,
    });
    const [exit, output, error] = await Promise.all([child.exited,
        new Response(child.stdout).text(), new Response(child.stderr).text()]);
    assert.equal(exit, 0, error);
    return output;
}

await command(["-f", "lavfi", "-i", "testsrc2=s=160x90:r=4:d=2", "-c:v", "libx264",
    "-preset", "ultrafast", "-g", "4", "-sc_threshold", "0", "-hls_time", "1", "-hls_list_size", "0",
    "-hls_segment_filename", "video-%02d.ts", "video.m3u8"]);
const master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=160x90\nvideo.m3u8\n";
await writeFile(join(fixtures, "master.m3u8"), master);
const originalVtt = "WEBVTT\n\nintro\n00:00:00.500 --> 00:00:01.750 align:start\n<i>Rin</i> keeps her promise.\n\n"
    + "future\n00:03:05.000 --> 00:03:08.000\nWe will return tomorrow.\n";
const vttPath = join(fixtures, "external.vtt");
await writeFile(vttPath, originalVtt);
const originalHash = createHash("sha256").update(originalVtt).digest("hex");
const sourceHeaders = { Cookie: "session=generated-gateway-cookie", Authorization: "Bearer generated-source-token",
    "X-Source-Session": "generated-gateway-session", "User-Agent": "qbutt-generated-subtitles" };
const prefix = `/media/${crypto.randomUUID().replaceAll("-", "")}/`;
let sourceHeaderRequests = 0;
let externalRequests = 0;
let apiCalls = 0;

const external = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch(request) {
    assert.equal(new URL(request.url).pathname, prefix + "external.vtt");
    assert(!request.headers.has("cookie") && !request.headers.has("authorization")
        && !request.headers.has("x-source-session"), "Source credentials reached a different subtitle origin");
    ++externalRequests;
    return new Response(originalVtt, { headers: { "Content-Type": "text/vtt" } });
} });
const source = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch(request) {
    const path = new URL(request.url).pathname;
    assert(path.startsWith(prefix), "An unrelated local source was requested");
    assert.equal(request.headers.get("cookie"), sourceHeaders.Cookie);
    assert.equal(request.headers.get("authorization"), sourceHeaders.Authorization);
    assert.equal(request.headers.get("x-source-session"), sourceHeaders["X-Source-Session"]);
    ++sourceHeaderRequests;
    const name = path.slice(prefix.length);
    assert(/^(?:master|video)\.m3u8$|^video-\d+\.ts$/.test(name), "Unknown generated media resource");
    return new Response(Bun.file(join(fixtures, name)));
} });
const apiKey = "generated-gateway-ai-key";
const ai = Bun.serve({ hostname: "127.0.0.1", port: 0, async fetch(request) {
    assert.equal(request.headers.get("authorization"), `Bearer ${apiKey}`);
    assert(!request.headers.has("cookie") && !request.headers.has("x-source-session"), "Source credentials reached AI");
    const body = await request.json() as { messages: { content: string }[] };
    const input = JSON.parse(body.messages[1]!.content) as { blocks: { id: string; text: string }[] };
    ++apiCalls;
    return Response.json({ choices: [{ finish_reason: "stop", message: { content: JSON.stringify({
        blocks: input.blocks.map(block => ({ id: block.id,
            text: block.text === "Rin" ? "Рин" : " сдержит своё обещание." })),
        terms: [{ source: "Rin", target: "Рин" }],
    }) } }] });
} });

const original: ResolvedContent = { kind: "video", resolver: "direct", title: "Generated legal HLS",
    url: `http://127.0.0.1:${source.port}${prefix}master.m3u8`, headers: sourceHeaders,
    subtitles: [{ title: "Generated English", language: "eng",
        url: `http://127.0.0.1:${external.port}${prefix}external.vtt` }] };
const network = { async addresses(url: URL) {
    assert(url.protocol === "http:" && url.hostname === "127.0.0.1"
        && [String(source.port), String(external.port)].includes(url.port) && url.pathname.startsWith(prefix),
    "The fixture connector rejected an unrelated address");
    return [{ address: "127.0.0.1", family: 4 }];
} };
const context: ProviderContext = { signal: new AbortController().signal, dataDir: join(root, "gateway"), settings: {},
    async fetch(url, init) {
        await network.addresses(new URL(url));
        return fetch(url, init);
    } };
const streams = new PlaybackStreams(network);
const client = new ContentClient(root);

async function translate(playing: ResolvedContent): Promise<string> {
    const track = { id: 1, codec: "webvtt", language: "eng", url: playing.subtitles![0]!.url };
    const params = { identity: "legal:gateway-external", url: playing.url!, headers: playing.headers,
        track, position: 0, startTime: 0, targetLanguage: "ru" };
    const restored = streams.originalSubtitle(params.url, track.url);
    assert(restored, "The registered exact subtitle did not map to its original source");
    assert.deepEqual(restored, { url: original.url, headers: sourceHeaders, subtitleUrl: original.subtitles![0]!.url });
    // This is the dispatch transformation source-reviewed in main.ts. The
    // actual helper receives the canonical request and owns translation/cache.
    const request = { ...params, url: restored.url, headers: restored.headers,
        track: { ...track, url: restored.subtitleUrl } };
    const events = await client.request("subtitles.ahead", request);
    completed(events);
    const terminal = events.at(-1)!.data as { status: string };
    assert.equal(terminal.status, "ready");
    const asset = events.filter(event => event.event === "subtitle").at(-1);
    assert(asset, "The real helper did not publish a translated subtitle asset");
    const published = asset.data as { url: string; identity: string };
    assert.equal(published.identity, params.identity, "The helper changed the exact playback identity");
    return fileURLToPath(published.url);
}

async function cacheContainsSecret(directory: string): Promise<boolean> {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) {
            if (await cacheContainsSecret(path))
                return true;
        }
        else {
            const content = await readFile(path, "utf8");
            if ([...Object.values(sourceHeaders), apiKey].some(secret => content.includes(secret)))
                return true;
        }
    }
    return false;
}

try {
    completed(await client.request("configureSource", { id: "ai", settings: { provider: "openai-compatible",
        baseUrl: `http://127.0.0.1:${ai.port}/v1`, apiKey, model: "generated-gateway-ai" } }));
    const first = await streams.prepare(original, context);
    assert.notEqual(first.url, original.url);
    assert.notEqual(first.subtitles![0]!.url, original.subtitles![0]!.url);
    assert.equal(streams.originalSubtitle(original.url, first.subtitles![0]!.url), undefined);
    assert.equal(streams.originalSubtitle(first.url, original.subtitles![0]!.url), undefined);
    assert.equal(streams.originalSubtitle(first.url, first.subtitles![0]!.url + "?unregistered"), undefined);
    const gatewayVtt = await fetch(first.subtitles![0]!.url);
    assert.equal(await gatewayVtt.text(), originalVtt);
    const decoded = await command(["-protocol_whitelist", "http,tcp,crypto", "-rw_timeout", "5000000",
        "-i", first.url!, "-t", "1", "-map", "0:v", "-progress", "pipe:1", "-f", "null", "-"]);
    assert([...decoded.matchAll(/^frame=(\d+)$/gm)].some(match => Number(match[1]) > 0), "Generated HLS did not decode");
    const initialPath = await translate(first);
    assert.equal(apiCalls, 1);
    const translated = await readFile(initialPath, "utf8");
    const cues = parseTimedText(translated).cues;
    assert.equal(cues[0]!.startMs, 500);
    assert.equal(cues[0]!.endMs, 1750);
    assert(translated.includes("<i>Рин</i>") && translated.includes("align:start"));
    assert(cues[1]!.text.includes("We will return tomorrow."), "Translation changed a cue outside the bounded window");

    const second = await streams.prepare(original, context);
    assert.notEqual(second.url, first.url, "The gateway did not generate a fresh media capability");
    assert.notEqual(second.subtitles![0]!.url, first.subtitles![0]!.url);
    assert.equal(streams.originalSubtitle(first.url, first.subtitles![0]!.url), undefined, "A stale gateway still mapped");
    assert.equal(streams.originalSubtitle(second.url, first.subtitles![0]!.url), undefined, "A stale track mapped to a new media capability");
    const secondPath = await translate(second);
    assert.equal(secondPath, initialPath, "The fresh gateway changed the canonical subtitle cache");
    assert.equal(apiCalls, 1, "The second open made an AI request");
    assert.equal(createHash("sha256").update(await readFile(vttPath)).digest("hex"), originalHash);
    assert(sourceHeaderRequests > 0 && externalRequests > 0);
    assert(!(await cacheContainsSecret(join(root, "profile", "subtitles"))));
    assert(!(await cacheContainsSecret(join(root, "profile", "reader"))));
    await writeFile(join(root, "evidence.json"), JSON.stringify({ success: true,
        scope: "Production PlaybackStreams mapping and actual main-helper IPC; catalog.resolve source reviewed",
        freshGatewayNonce: true, sameCanonicalCache: true, apiCalls, secondOpenApiCalls: 0,
        wrongMediaAndTrackRejected: true, staleCapabilityRejected: true, decodedHls: true,
        sourceHeadersRestored: true, crossOriginCredentialsAbsent: true, aiCredentialsScoped: true,
        cacheContainsNoCredentials: true, originalUnchanged: true, timingAndStylePreserved: true,
        boundedWindow: true, initialPath, sourceHeaderRequests, externalRequests,
        completedAt: new Date().toISOString() }, null, 2));
    console.log(`PASS generated HLS/external VTT → exact gateway mapping → real helper cache; ${join(root, "evidence.json")}`);
}
finally {
    const closed = await Promise.allSettled([client.close(), streams.close(),
        source.stop(true), external.stop(true), ai.stop(true)]);
    for (const result of closed) {
        if (result.status === "rejected")
            throw result.reason;
    }
}
