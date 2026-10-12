import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";

// The acceptance build redirects only this public HTTPS origin to local TLS.
// Search, inspection, resolution and SQLite still run through native code.
const [executable, certificate, key] = process.argv.slice(2);
assert(executable && certificate && key, "Expected acceptance executable, certificate and key");

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
let searches = 0;
let inspections = 0;
let failure: unknown;

const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 443,
    tls: { cert: await readFile(certificate), key: await readFile(key) },
    fetch(request) {
        try {
            const url = new URL(request.url);
            assert.equal(url.hostname, "anilibria.top");
            if (url.pathname === "/api/v1/app/search/releases") {
                assert.equal(url.searchParams.get("query"), "Frieren");
                ++searches;
                return Response.json(releases);
            }
            const match = /^\/api\/v1\/anime\/releases\/(9542|9543|9544)$/.exec(url.pathname);
            assert(match && !url.search, `Unexpected AniLiberty path: ${url.pathname}`);
            ++inspections;
            if (match[1] === "9544")
                return Response.json({ message: "Not found" }, { status: 404 });
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
    await runScenario("library", "PASS library favorites=ok");
    await runScenario("sharing", "PASS share exact=unavailable");
    if (failure) throw failure;
    assert(searches >= 2 && inspections >= 5, "The native catalog did not repeat exact source operations");
    console.log(JSON.stringify({ ok: true, searches, inspections }));
} finally {
    server.stop(true);
}
