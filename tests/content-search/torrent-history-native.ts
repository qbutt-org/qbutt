import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";
import { Database } from "bun:sqlite";

import { labAppearanceSettings } from "../appearance";
import { encode } from "../network-lab/bencode";
import { assertQtRuntime } from "../qt-runtime";

const [executableArgument, certificate, key] = process.argv.slice(2);
assert(executableArgument && certificate && key && process.argv.length === 5,
    "Usage: bun tests/content-search/torrent-history-native.ts <acceptance-exe> <server-cert.pem> <server-key.pem>");
const executable = resolve(executableArgument);
const bundle = resolve(process.env.QBUTT_QT_ACCEPTANCE_BUNDLE ?? dirname(executable));
assertQtRuntime(bundle);

const root = await mkdtemp(join(await realpath(tmpdir()), "qbutt-torrent-history-"));
const profile = join(root, "profile");
const destination = join(root, "download");
const faultPath = join(root, "malformed-metadata");
await mkdir(join(profile, "qbutt", "config"), { recursive: true });
await mkdir(destination);

function torrent(name: string) {
    const payload = Buffer.from(`qbutt authored torrent history fixture: ${name}\n`);
    const info = {
        name: Buffer.from(name), private: 1, length: payload.length,
        "piece length": 16384, pieces: createHash("sha1").update(payload).digest(),
    };
    return {
        bytes: encode({ info }),
        hash: createHash("sha1").update(encode(info)).digest("hex"),
    };
}

const magnet = torrent("magnet-history.txt");
const metadata = [torrent("metadata-first.txt"), torrent("metadata-exact.txt")];
let requestCount = 0;
let malformedResponses = 0;
let failure: unknown;
let application: ReturnType<typeof Bun.spawn> | undefined;
const server = Bun.serve({
    hostname: "127.0.0.1", port: 443,
    tls: { cert: await readFile(certificate), key: await readFile(key) },
    fetch(request) {
        try {
            const url = new URL(request.url);
            ++requestCount;
            if (url.hostname === "anilibria.top") {
                const release = {
                    id: 9700, alias: "generated-history", name: { main: "Generated history anime" },
                    year: 2026, episodes: [],
                    torrents: [9701, 9702].map(id => ({
                        id, label: `Generated exact variant ${id}`, hash: magnet.hash,
                        magnet: `magnet:?xt=urn:btih:${magnet.hash}&dn=Generated%20history`,
                    })),
                };
                if (url.pathname === "/api/v1/app/search/releases") {
                    assert.equal(url.searchParams.get("query"), "Generated history");
                    return Response.json([release]);
                }
                assert.equal(url.pathname, "/api/v1/anime/releases/9700");
                return Response.json(release);
            }

            assert.equal(url.hostname, "itorrents-igruha.org");
            const article = "https://itorrents-igruha.org/42-generated-history.html";
            let body: string;
            if (url.pathname === "/index.php") {
                assert.equal(url.searchParams.get("story"), "Generated history");
                body = `<div id='dle-content'><div class='searchpage'><form id='fullsearch'>
                    <input id='searchinput' name='story' value='Generated history'></form></div>
                    <div class='game-card'><div class='game-card__name'>
                    <a href='${article}'>Generated history game</a></div></div></div>`;
            } else if (url.pathname === "/42-generated-history.html") {
                const variants = [91, 92].map((id, index) => `
                    <div class='is-centered'><span class='size-line'>Размер: 1 KB v${index + 1}.0 | Legal</span></div>
                    <div class='dl-bar'><a class='torrent' href='https://itorrents-igruha.org/?do=download&amp;id=${id}'>Torrent</a></div>`).join("");
                body = `<html><head><link rel='canonical' href='${article}'></head><body>
                    <div id='dle-content'><div class='page-title'><h1>Generated history game</h1></div>
                    <div class='blockinfo'>${variants}</div></div></body></html>`;
            } else if (url.pathname === "/") {
                assert.equal(url.searchParams.get("do"), "download");
                const id = url.searchParams.get("id");
                assert(id === "91" || id === "92");
                body = `<div id='dle-content'><div class='dl-back'><a href='${article}'>Back</a></div>
                    <a id='btn-download' href='https://itorrents-igruha.org/engine/download.php?id=${id}'>Torrent</a></div>`;
            } else {
                assert.equal(url.pathname, "/engine/download.php");
                const id = url.searchParams.get("id");
                assert(id === "91" || id === "92");
                if (existsSync(faultPath)) {
                    ++malformedResponses;
                    return new Response("generated malformed metadata", {
                        headers: { "content-type": "application/x-bittorrent" },
                    });
                }
                return new Response(metadata[id === "91" ? 0 : 1]!.bytes, {
                    headers: { "content-type": "application/x-bittorrent" },
                });
            }
            // The production provider decodes Windows-1251. Numeric HTML
            // entities keep the authored response ASCII without another codec.
            return new Response(body.replace(/[^\x00-\x7F]/gu, character => `&#${character.codePointAt(0)};`), {
                headers: { "content-type": "text/html; charset=windows-1251" },
            });
        } catch (error) {
            failure = error;
            return new Response("Generated fixture failed", { status: 500 });
        }
    },
});

try {
    await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
        "[BitTorrent]",
        "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1", "Session\\AddTorrentStopped=true",
        `Session\\DefaultSavePath=${destination.replaceAll("\\", "/")}`,
        "Session\\AddTrackersEnabled=false", "Session\\AddTrackersFromURLEnabled=false",
        "[Network]", "PortForwardingEnabled=false",
        "[Application]", `FileLogger\\Path=${root.replaceAll("\\", "/")}`,
        "[Core]", "AutoOpenTorrentFiles=false",
        "[GUI]", "Notifications\\Enabled=false",
        "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false", "WebUI\\Enabled=false",
        "Search\\SearchEnabled=false",
        "Connection\\ResolvePeerCountries=false", "Connection\\ResolvePeerHostNames=false",
        ...labAppearanceSettings("functional"), "",
    ].join("\n"));

    const pathKey = Object.keys(process.env).find(name => name.toLowerCase() === "path") ?? "PATH";
    for (const phase of ["exercise", "restart"]) {
        const evidencePath = join(root, `${phase}.json`);
        const specPath = join(root, `${phase}-spec.json`);
        await writeFile(specPath, JSON.stringify({
            mode: "torrent-history", restart: phase === "restart", profile,
            evidencePath, faultPath, magnetHash: magnet.hash, metadataHash: metadata[1]!.hash,
        }, null, 2));
        application = Bun.spawn([executable, `--profile=${profile}`], {
            cwd: bundle, windowsHide: true, timeout: 90000,
            env: {
                ...process.env, [pathKey]: `${bundle};${process.env[pathKey] ?? ""}`,
                QBUTT_QT_ACCEPTANCE_SPEC: specPath, QBUTT_QT_ACCEPTANCE_NATIVE: undefined,
                QT_QPA_PLATFORM: "offscreen", QT_QUICK_BACKEND: "software",
                QT_PLUGIN_PATH: bundle, QML_IMPORT_PATH: join(bundle, "qml"),
                QT_SCALE_FACTOR: "1", QT_FORCE_STDERR_LOGGING: "1",
            },
            stdout: Bun.file(join(root, `${phase}-stdout.log`)),
            stderr: Bun.file(join(root, `${phase}-stderr.log`)),
        });
        const exitCode = await application.exited;
        const libraryPath = join(profile, "qbutt", "data", "content-library.sqlite");
        if (exitCode !== 0 && existsSync(libraryPath)) {
            const database = new Database(libraryPath, { readonly: true });
            try {
                const entries = database.query("SELECT entry FROM library").all() as { entry: string }[];
                await writeFile(join(root, "failed-library.json"),
                    JSON.stringify(entries.map(row => JSON.parse(row.entry)), null, 2));
            } finally {
                database.close();
            }
        }
        assert.equal(exitCode, 0,
            `Torrent history ${phase} failed; requests=${requestCount}; fixture=${String(failure ?? "ok")}; inspect ${root}`);
        if (failure) throw failure;
        const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
        assert.equal(evidence.status, "passed", JSON.stringify(evidence));
        console.log(JSON.stringify({ phase, ...evidence }, null, 2));
    }

    const database = new Database(join(profile, "qbutt", "data", "content-library.sqlite"), { readonly: true });
    try {
        const entries = database.query("SELECT entry FROM library").all() as { entry: string }[];
        assert.equal(entries.length, 2, "Expected only the two generated library works");
        const saved = entries.map(row => JSON.parse(row.entry));
        for (const entry of saved) {
            assert.equal(entry.group.publications.length, 1);
            assert.equal(entry.group.publications[0].variants.length, 2, "History lost another known variant");
            assert.deepEqual(entry.last_opened.variant.publication, entry.group.anchor,
                "Confirmed History belongs to a different work");
        }
        const anime = saved.find(entry => entry.group.anchor.id === "9700");
        const game = saved.find(entry => entry.group.anchor.id === "42-generated-history");
        assert.deepEqual(anime?.last_opened.variant.selector, { torrent: { id: 9702 } });
        assert.deepEqual(anime?.preferences.selected.selector, { torrent: { id: 9702 } },
            "History lost the original attempt's captured preference");
        assert.deepEqual(game?.last_opened.variant.selector, { torrent_hash: { info_hash: metadata[1]!.hash } });
        assert.deepEqual(game?.preferences.selected, game?.last_opened.variant);
    } finally {
        database.close();
    }
    assert.equal(malformedResponses, 1);
    console.log(JSON.stringify({ status: "passed", root, requests: requestCount, malformedResponses,
        sqliteExactVariants: true, restartExactVariants: true }, null, 2));
} finally {
    if (application?.exitCode === null) {
        application.kill();
        await application.exited;
    }
    server.stop(true);
    for (const directory of [profile, destination]) {
        const target = await realpath(directory);
        assert(target.startsWith(root + sep), "Torrent history cleanup escaped its temporary root");
        await rm(target, { recursive: true, force: true });
    }
}
