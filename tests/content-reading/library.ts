import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";

// The acceptance build redirects only this public HTTPS origin to local TLS.
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
let sharedAnchorRequests = 0;
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
            assert.equal(url.hostname, "anilibria.top");
            if (url.pathname === "/api/v1/app/search/releases") {
                assert.equal(url.searchParams.get("query"), "Frieren");
                ++searches;
                return Response.json(releases);
            }
            const match = /^\/api\/v1\/anime\/releases\/(9542|9543|9544|9550|9551)$/.exec(url.pathname);
            assert(match && !url.search, `Unexpected AniLiberty path: ${url.pathname}`);
            ++inspections;
            if (match[1] === "9544")
                return Response.json({ message: "Not found" }, { status: 404 });
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
    }
    if (failure) throw failure;
    if (scenario !== "sharing")
        assert(searches >= 2, "The native catalog did not repeat search operations");
    assert(inspections >= 5, "The native catalog did not repeat exact inspections");
    console.log(JSON.stringify({ ok: true, scenario, searches, inspections }));
} finally {
    server.stop(true);
}
