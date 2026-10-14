import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";

const [executable, certificate, key, scenario = "success"] = process.argv.slice(2);
assert(executable && certificate && key, "Expected acceptance executable, certificate and key");
assert(["success", "failure"].includes(scenario), "Expected success or failure scenario");

const releaseLate = Promise.withResolvers<void>();
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
                const release = {
                    id: 9660,
                    alias: "generated-progressive",
                    name: { main: "Generated progressive" },
                    year: 2026,
                    mal: { id: 9660 },
                    episodes: [],
                    torrents: [9660, 9662].map(id => ({
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
            if (scenario === "failure") {
                return new Response("Generated unavailable source", { status: 503 });
            }
            return Response.json({
                response: {
                    ...publication,
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
    windowsHide: true,
    env: {
        ...process.env,
        QT_QPA_PLATFORM: "offscreen",
        QT_QUICK_BACKEND: "software",
        QT_FORCE_STDERR_LOGGING: "1",
        QT_QPA_FONTDIR: "C:/Windows/Fonts",
        QBUTT_PROGRESSIVE_FAILURE: scenario === "failure" ? "1" : "0",
    },
});
const exited = new Promise<number | null>((resolve, reject) => {
    child.once("exit", resolve);
    child.once("error", reject);
});
let output = "";
function capture(chunk: Buffer) {
    output = (output + chunk.toString("utf8")).slice(-64 * 1024);
    if (output.includes("FIXTURE release_late_search")) releaseLate.resolve();
}
child.stdout.on("data", capture);
child.stderr.on("data", capture);
const timer = setTimeout(() => child.kill(), 40_000);
try {
    assert.equal(await exited, 0, output);
    if (failure) throw failure;
    assert(output.includes("PASS progressive_selection"), output);
    assert.equal(searches, 2);
    assert.equal(fastInspections, 1);
    assert.equal(lateInspections, 1);
    console.log(output);
    console.log(JSON.stringify({ ok: true, scenario, searches, fastInspections, lateInspections }));
} finally {
    clearTimeout(timer);
    releaseLate.resolve();
    if (child.exitCode === null) {
        child.kill();
        await exited;
    }
    server.stop(true);
}
