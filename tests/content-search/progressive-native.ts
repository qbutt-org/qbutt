import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";

const [executable, certificate, key, scenario = "success"] = process.argv.slice(2);
assert(executable && certificate && key, "Expected acceptance executable, certificate and key");
assert(["success", "failure", "initial", "missing", "split", "split-anchor", "empty"].includes(scenario), "Unknown scenario");

const releaseLate = Promise.withResolvers<void>();
const releaseInspection = Promise.withResolvers<void>();
const releaseCachedFast = Promise.withResolvers<void>();
const releaseCachedLate = Promise.withResolvers<void>();
if (scenario === "initial") releaseLate.resolve();
let searches = 0;
let fastInspections = 0;
let lateInspections = 0;
let failure: unknown;

const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 443,
    tls: { cert: await readFile(certificate), key: await readFile(key) },
    async fetch(request) {
        try {
            const url = new URL(request.url);
            if (url.hostname === "anilibria.top") {
                let torrentIds = [9660, 9662];
                if (scenario === "empty") torrentIds = [];
                else if (scenario === "missing" && fastInspections >= 2) torrentIds = [9660];

                const release = {
                    id: 9660,
                    alias: "generated-progressive",
                    name: { main: "Generated progressive" },
                    year: scenario === "split-anchor" && fastInspections >= 2 ? 2027 : 2026,
                    mal: { id: 9660 },
                    episodes: [],
                    torrents: torrentIds.map(id => ({
                        id,
                        label: `Exact variant ${id}`,
                        hash: String(id).padStart(40, "0"),
                        magnet: `magnet:?xt=urn:btih:${String(id).padStart(40, "0")}`,
                    })),
                };
                if (url.pathname === "/api/v1/app/search/releases") {
                    assert.equal(url.searchParams.get("query"), "Generated progressive");
                    ++searches;
                    return Response.json([release]);
                }
                assert.equal(url.pathname, "/api/v1/anime/releases/9660");
                ++fastInspections;
                if (fastInspections === 3) await releaseCachedFast.promise;
                return Response.json(release);
            }
            assert.equal(url.hostname, "api.yani.tv");
            const publication = {
                anime_id: 9661,
                anime_url: "generated-progressive",
                title: "Generated progressive",
                year: 2026,
                remote_ids: { myanimelist_id: 9660 },
            };
            if (url.pathname === "/anime") {
                assert.equal(url.searchParams.get("q"), "Generated progressive");
                ++searches;
                await releaseLate.promise;
                return Response.json({ response: [publication] });
            }
            assert.equal(url.pathname, "/anime/9661");
            ++lateInspections;
            await (lateInspections === 1 ? releaseInspection.promise : releaseCachedLate.promise);
            if (scenario === "failure") {
                return new Response("Generated unavailable source", { status: 503 });
            }
            return Response.json({
                response: {
                    ...publication,
                    year: scenario === "split" && lateInspections > 1 ? 2027 : 2026,
                    videos: [{
                        video_id: 9661,
                        number: "1",
                        iframe_url: "https://kodik.info/serial/9661/generated/720p",
                        data: { player: "Kodik", dubbing: "Озвучка Generated", player_id: 1 },
                    }],
                },
            });
        } catch (error) {
            failure = error;
            return new Response("Generated fixture failed", { status: 500 });
        }
    },
});

const child = spawn(executable, ["--progressive-selection", "--lang", "en"], {
    windowsHide: process.env.QBUTT_PROGRESSIVE_POINTER !== "1",
    env: {
        ...process.env,
        QT_QPA_PLATFORM: process.env.QBUTT_PROGRESSIVE_POINTER === "1" ? "windows" : "offscreen",
        QT_QUICK_BACKEND: "software",
        QT_FORCE_STDERR_LOGGING: "1",
        QT_QPA_FONTDIR: "C:/Windows/Fonts",
        QBUTT_PROGRESSIVE_FAILURE: scenario === "failure" ? "1" : "0",
        QBUTT_PROGRESSIVE_INITIAL: scenario === "initial" ? "1" : "0",
        QBUTT_PROGRESSIVE_MISSING: scenario === "missing" ? "1" : "0",
        QBUTT_PROGRESSIVE_SPLIT: scenario.startsWith("split") ? "1" : "0",
        QBUTT_PROGRESSIVE_SPLIT_ANCHOR: scenario === "split-anchor" ? "1" : "0",
        QBUTT_PROGRESSIVE_EMPTY: scenario === "empty" ? "1" : "0",
    },
});
const exited = new Promise<number | null>((resolve, reject) => {
    child.once("exit", resolve);
    child.once("error", reject);
});
let output = "";
function capture(chunk: Buffer) {
    if (process.env.QBUTT_PROGRESSIVE_POINTER === "1") process.stderr.write(chunk);
    output = (output + chunk.toString("utf8")).slice(-64 * 1024);
    if (output.includes("FIXTURE release_late_search")) releaseLate.resolve();
    if (output.includes("FIXTURE release_late_inspection")) releaseInspection.resolve();
    if (output.includes("FIXTURE release_cached_fast")) releaseCachedFast.resolve();
    if (output.includes("FIXTURE release_cached_late")) releaseCachedLate.resolve();
}
child.stdout.on("data", capture);
child.stderr.on("data", capture);
const timer = setTimeout(() => child.kill(), 40_000);
try {
    assert.equal(await exited, 0, output);
    if (failure) throw failure;
    assert(output.includes("PASS progressive_selection"), output);
    assert.equal(searches, 2);
    let expectedFastInspections = 4;
    if (scenario === "empty") expectedFastInspections = 2;
    else if (["missing", "split", "split-anchor"].includes(scenario)) expectedFastInspections = 3;
    assert.equal(fastInspections, expectedFastInspections);
    assert.equal(lateInspections, scenario.startsWith("split") ? 3 : 2);
    console.log(output);
    console.log(JSON.stringify({ ok: true, scenario, searches, fastInspections, lateInspections }));
} finally {
    clearTimeout(timer);
    releaseLate.resolve();
    releaseInspection.resolve();
    releaseCachedFast.resolve();
    releaseCachedLate.resolve();
    if (child.exitCode === null) {
        child.kill();
        await exited;
    }
    server.stop(true);
}
