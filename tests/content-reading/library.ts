import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";

// The acceptance build redirects AniLiberty and RuTor HTTPS origins to local TLS.
// Search, inspection, resolution and SQLite still run through native code.
const [executable, certificate, key, scenario = "all"] = process.argv.slice(2);
assert(executable && certificate && key, "Expected acceptance executable, certificate and key");
assert(["all", "library", "sharing"].includes(scenario), "Expected all, library or sharing scenario");

function release(id: number) {
    const torrents = id === 9542 ? [28083, 28084, 28085] : [38083, 38084];
    return {
        id,
        alias: `generated-frieren-${id}`,
        name: { main: `Generated Frieren ${id}` },
        description: "Generated legal acceptance metadata",
        year: 2026,
        episodes: [],
        torrents: torrents.map(torrent => {
            const hash = torrent.toString(16).padStart(40, "0");
            return {
                id: torrent,
                label: `Generated torrent ${torrent}`,
                magnet: `magnet:?xt=urn:btih:${hash}`,
                hash,
                size: torrent * 1024,
                seeders: torrent % 17,
                quality: { value: "1080p" },
            };
        }),
    };
}

const releases = [release(9542), release(9543)];
const sharedReleases = [9550, 9551].map(id => ({ ...release(id), mal: { id: 52991 } }));
const resolutionStarted = Promise.withResolvers<void>();
const cancelledSharedRetry = Promise.withResolvers<void>();
const lateGameReplies = Promise.withResolvers<void>();
const gameInspections = new Set<number>();
let sharedAnchorRequests = 0;
let partialAlternateRequests = 0;
let cancelledAlternateRequests = 0;
let preferredPublicationRequests = 0;
let searches = 0;
let inspections = 0;
let failure: unknown;

const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 443,
    tls: { cert: await readFile(certificate), key: await readFile(key) },
    async fetch(request) {
        try {
            const url = new URL(request.url);
            if (url.hostname === "rutor.info") {
                const match = /^\/torrent\/(9560|9561|9562|9563|9564)$/.exec(url.pathname);
                assert(match && !url.search, `Unexpected RuTor path: ${url.pathname}`);
                const id = Number(match[1]);
                assert(!gameInspections.has(id), "Shared game import repeated an inspection");
                gameInspections.add(id);
                ++inspections;
                if (id === 9564)
                    return new Response("Generated unavailable alternate", { status: 404 });
                if (id === 9563) {
                    await lateGameReplies.promise;
                    await Bun.sleep(600);
                }
                if (id === 9562) lateGameReplies.resolve();
                const titles: Record<number, string> = {
                    9560: "Generated Game [v 123456] (2023) PC",
                    9561: "Generated Game New Story [v 123456] (2023) PC",
                    9562: "Generated Game New Story Update v654321-RUNE",
                    9563: "Generated Game: Deluxe Edition [v 123456] (2023) PC",
                };
                const hash = id.toString(16).padStart(40, "0");
                return new Response(`<!doctype html><h1>${titles[id]}</h1>
                    <div id="download"><a href="magnet:?xt=urn:btih:${hash}">Magnet</a>
                    <a href="//d.rutor.info/download/${id}">Download</a></div>
                    <table id="details"><tr><td class="header">Категория</td><td>Игры</td></tr>
                    <tr><td class="header">Размер</td><td>1 MB (1048576 Bytes)</td></tr></table>`,
                    { headers: { "Content-Type": "text/html; charset=utf-8" } });
            }
            assert.equal(url.hostname, "anilibria.top");
            if (url.pathname === "/api/v1/app/search/releases") {
                assert.equal(url.searchParams.get("query"), "Frieren");
                ++searches;
                return Response.json(releases);
            }
            const match = /^\/api\/v1\/anime\/releases\/(9542|9543|9544|9545|9546|9547|9550|9551)$/.exec(url.pathname);
            assert(match && !url.search, `Unexpected AniLiberty path: ${url.pathname}`);
            ++inspections;
            if (match[1] === "9542") ++preferredPublicationRequests;
            if (["9544", "9547"].includes(match[1]))
                return Response.json({ message: "Not found" }, { status: 404 });
            if (match[1] === "9545" || match[1] === "9546") {
                const count = match[1] === "9545" ? ++partialAlternateRequests : ++cancelledAlternateRequests;
                if (count === 1)
                    return Response.json({ message: "Not found" }, { status: 404 });
                if (match[1] === "9546") await cancelledSharedRetry.promise;
                return Response.json({ ...release(Number(match[1])), name: { main: "Generated Frieren 9542" } });
            }
            if (match[1] === "9550") {
                if (++sharedAnchorRequests === 2) {
                    resolutionStarted.resolve();
                    await Bun.sleep(600);
                }
                return Response.json(sharedReleases[0]);
            }
            if (match[1] === "9551") {
                await resolutionStarted.promise;
                return Response.json(sharedReleases[1]);
            }
            return Response.json(releases.find(item => item.id === Number(match[1])));
        } catch (error) {
            failure = error;
            return new Response("Generated fixture failed", { status: 500 });
        }
    },
});

async function runScenario(scenario: string, expected: string) {
    const child = spawn(executable, [`--${scenario}`, "--lang", "ru"], {
        windowsHide: true,
        env: {
            ...process.env,
            QT_QPA_PLATFORM: "offscreen",
            QT_QUICK_BACKEND: "software",
            QT_FORCE_STDERR_LOGGING: "1",
        },
    });
    const exited = new Promise<number | null>((resolve, reject) => {
        child.once("exit", resolve);
        child.once("error", reject);
    });
    let output = "";
    function capture(chunk: Buffer) {
        output = (output + chunk.toString("utf8")).slice(-64 * 1024);
        if (output.includes("FIXTURE release_cancelled_shared_retry")) cancelledSharedRetry.resolve();
    }
    child.stdout.on("data", capture);
    child.stderr.on("data", capture);
    const timer = setTimeout(() => child.kill(), 240_000);
    try {
        assert.equal(await exited, 0, output);
        assert(output.includes(expected), output);
    } finally {
        clearTimeout(timer);
        if (child.exitCode === null) {
            child.kill();
            await exited;
        }
    }
}

try {
    if (scenario !== "sharing")
        await runScenario("library", "PASS library favorites=ok");
    if (scenario !== "library") {
        await runScenario("sharing", "PASS share exact=unavailable");
        assert.equal(sharedAnchorRequests, 2, "Shared action must resolve once without a retry");
        assert.equal(partialAlternateRequests, 2, "Shared retry must inspect only the failed alternate");
        assert.equal(cancelledAlternateRequests, 2, "Cancelled shared retry must still complete its exact source probe");
        if (scenario === "sharing")
            assert.equal(preferredPublicationRequests, 5, "Alternate retries restarted the shared preferred publication");
        assert.equal(gameInspections.size, 5, "Late-witness shared game publications were not inspected exactly once");
    }
    if (failure) throw failure;
    if (scenario !== "sharing")
        assert(searches >= 2, "The native catalog did not repeat search operations");
    assert(inspections >= 5, "The native catalog did not repeat exact inspections");
    console.log(JSON.stringify({ ok: true, scenario, searches, inspections }));
} finally {
    cancelledSharedRetry.resolve();
    lateGameReplies.resolve();
    server.stop(true);
}
