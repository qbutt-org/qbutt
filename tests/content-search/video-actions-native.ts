import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";

const [executable, certificate, key, ...nativeArguments] = process.argv.slice(2);
assert(executable && certificate && key, "Expected acceptance executable, certificate and key");

const releaseInspection = Promise.withResolvers<void>();
const players = ["Плеер Aksor", "Плеер CVH", "Плеер Kodik", "Плеер VK", "Плеер Alloha", "Alloha", "CVH"];
let inspections = 0;
let failure: unknown;

const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 443,
    tls: { cert: await readFile(certificate), key: await readFile(key) },
    async fetch(request) {
        try {
            const url = new URL(request.url);
            assert.equal(request.method, "GET");
            assert.equal(url.hostname, "api.yani.tv");
            assert.equal(url.pathname, "/anime/900001");
            assert.equal(url.searchParams.get("need_videos"), "true");
            assert.equal(++inspections, 1, "Rejected actions made another source request");
            await releaseInspection.promise;
            return Response.json({
                response: {
                    anime_id: 900001,
                    anime_url: "generated-video-action-fixture",
                    title: "Generated video action fixture",
                    description: "Generated HTTPS inspection completed",
                    videos: players.map((player, index) => ({
                        video_id: index + 1,
                        number: "3",
                        iframe_url: `https://api.yani.tv/generated-video/${index + 1}`,
                        data: { player, dubbing: "Fixture Studio", player_id: index + 1 },
                    })),
                },
            });
        } catch (error) {
            failure = error;
            return new Response("Generated video action fixture failed", { status: 500 });
        }
    },
});

const child = spawn(executable, [
    "--video-actions",
    ...(nativeArguments.length ? nativeArguments : ["--lang", "en"]),
], {
    windowsHide: true,
    env: {
        ...process.env,
        QT_QPA_PLATFORM: "offscreen",
        QT_QUICK_BACKEND: "software",
        QT_FORCE_STDERR_LOGGING: "1",
        QT_QPA_FONTDIR: "C:/Windows/Fonts",
    },
});
const exited = new Promise<number | null>((resolve, reject) => {
    child.once("exit", resolve);
    child.once("error", reject);
});
let output = "";
function capture(chunk: Buffer) {
    output = (output + chunk.toString("utf8")).slice(-64 * 1024);
    if (output.includes("FIXTURE release_video_inspection")) releaseInspection.resolve();
}
child.stdout.on("data", capture);
child.stderr.on("data", capture);
const timer = setTimeout(() => child.kill(), 40_000);
try {
    assert.equal(await exited, 0, output);
    if (failure) throw failure;
    assert(output.includes("PASS video_actions"), output);
    assert.equal(inspections, 1);
    console.log(output);
    console.log(JSON.stringify({ ok: true, inspections, players }));
} finally {
    clearTimeout(timer);
    releaseInspection.resolve();
    if (child.exitCode === null) {
        child.kill();
        await exited;
    }
    server.stop(true);
}
