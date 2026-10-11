import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { DownloadQueue, type DownloadJob } from "../../components/content/downloads";
import type { AudioTrack } from "../../components/content/audio";
import { LibraryStore } from "../../components/content/store";

const root = await mkdtemp(join(tmpdir(), "qbutt-audio-downloads-"));
const fixtures = join(root, "fixtures");
const appDir = join(root, "app");
process.env.QBUTT_LAB_PATHS = "1";
const mediaRuntime = process.env.QBUTT_MEDIA_RUNTIME_DIR;
assert(mediaRuntime, "Set QBUTT_MEDIA_RUNTIME_DIR to the separately prepared pinned lab media tools");
const dataDir = join(root, "data");
const downloadDir = join(root, "saved");
const ffmpeg = join(mediaRuntime, "tools", "ffmpeg.exe");
const ffprobe = join(mediaRuntime, "profile", "tools", "bin", "ffprobe.exe");
const systemRoot = process.env.SystemRoot || "C:/Windows";
const toolEnvironment = { ...process.env, PATH: [mediaRuntime, join(systemRoot, "System32"), systemRoot].join(";") };
await mkdir(fixtures);

async function command(file: string, args: string[]): Promise<string> {
    const child = Bun.spawn([file, ...args], {
        windowsHide: true, stdout: "pipe", stderr: "pipe", cwd: fixtures, env: toolEnvironment,
    });
    const timeout = setTimeout(() => child.kill(), 30000);
    try {
        const [output, error, exit] = await Promise.all([
            new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
        ]);
        assert.equal(exit, 0, error);
        return output;
    }
    finally { clearTimeout(timeout); }
}
const titles = ["01. Многоголосый. Studio Alpha (RUS)", "02. Многоголосый. Studio Beta (RUS)",
    "03. Многоголосый. Ambiguous (RUS)", "04. Многоголосый. Ambiguous (RUS)"];
const original = join(fixtures, "voices.mkv");
await command(ffmpeg, ["-v", "error", "-f", "lavfi", "-i", "color=c=blue:s=160x90:r=24:d=2",
    ...[440, 880, 660, 550].flatMap(frequency => ["-f", "lavfi", "-i", `sine=frequency=${frequency}:duration=2`]),
    "-map", "0:v", ...[1, 2, 3, 4].flatMap(index => ["-map", `${index}:a`]), "-c:v", "libx264", "-g", "24", "-c:a", "aac",
    ...titles.flatMap((title, index) => [`-metadata:s:a:${index}`, `title=${title}`, `-metadata:s:a:${index}`, "language=rus"]), original]);
await command(ffmpeg, ["-v", "error", "-i", original, "-map", "0:v", "-c", "copy", "-hls_time", "1", "-hls_list_size", "0", "video.m3u8"]);
for (let index = 0; index < titles.length; ++index) {
    await command(ffmpeg, ["-v", "error", "-i", original, "-map", `0:a:${index}`, "-c", "copy",
        "-hls_time", "1", "-hls_list_size", "0", `voice-${index}.m3u8`]);
}
await writeFile(join(fixtures, "master.m3u8"), "#EXTM3U\n" + titles.map((title, index) =>
    `#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="voices",NAME="${title}",LANGUAGE="ru",DEFAULT=${index ? "NO" : "YES"},URI="voice-${index}.m3u8"`).join("\n")
    + '\n#EXT-X-STREAM-INF:BANDWIDTH=100000,AUDIO="voices"\nvideo.m3u8\n');
const requests: string[] = [];
const server = createServer(async (incoming, response) => {
    const name = new URL(incoming.url!, "http://fixture.invalid").pathname.slice(1);
    requests.push(name);
    if (!/^[a-z\d.-]+$/i.test(name)) {
        response.writeHead(404).end();
        return;
    }
    try {
        const body = await readFile(join(fixtures, name));
        response.writeHead(200, { "Content-Type": name.endsWith("m3u8") ? "application/vnd.apple.mpegurl" : "application/octet-stream" });
        response.end(body);
    }
    catch { response.writeHead(404).end(); }
});
await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
const address = server.address();
assert(address && typeof address !== "string");
const origin = `http://fixture.invalid:${address.port}`;
const store = await LibraryStore.open(dataDir);
const queue = new DownloadQueue({ appDir, dataDir, downloadDir, store,
    network: { async addresses(url: URL) {
        assert.equal(url.hostname, "fixture.invalid");
        assert.equal(url.protocol, "http:");
        assert.equal(url.port, String(address.port));
        return [{ address: "127.0.0.1", family: 4 }];
    } }, async resolve() { throw new Error("Resolved fixture only"); }, emit() {} });
const beta: AudioTrack = { title: "Studio Beta", type: "Многоголосый", language: "ru", codec: "aac", channels: 1 };
async function save(path: string, selection: AudioTrack): Promise<DownloadJob> {
    const enqueued = queue.enqueue({ title: "Selected voice", resolved: { kind: "video", url: origin + path, audioTrack: selection } });
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
        const job = queue.jobs().find(item => item.id === enqueued.id)!;
        if (!["queued", "downloading"].includes(job.status))
            return job;
        await Bun.sleep(20);
    }
    throw new Error(`Audio download timed out; inspect ${root}`);
}
async function audioHash(path: string, stream: string): Promise<string> {
    return (await command(ffmpeg, ["-v", "error", "-i", path, "-map", stream, "-c", "copy", "-f", "hash", "-hash", "sha256", "-"])).trim();
}
let passed = false;
try {
    const verified = [];
    for (const path of ["/voices.mkv", "/master.m3u8"]) {
        const job = await save(path, beta);
        assert.equal(job.status, "complete", job.message);
        const probe = JSON.parse(await command(ffprobe, ["-v", "error", "-show_streams", "-of", "json", job.path!]));
        const audio = probe.streams.filter((stream: any) => stream.codec_type === "audio");
        assert.equal(audio.length, 1, "Only the explicitly selected voice should be saved");
        assert.equal(audio[0].disposition.default, 1);
        const expected = await audioHash(original, "0:a:1");
        const actual = await audioHash(job.path!, "0:a:0");
        assert.equal(actual, expected, "Default Alpha audio replaced explicitly selected Beta");
        await command(ffmpeg, ["-v", "error", "-i", job.path!, "-f", "null", "-"]);
        verified.push({ source: path, bytes: (await stat(job.path!)).size, audioSha256: actual.replace("SHA256=", "") });
    }
    assert(!requests.some(path => /^voice-(?:0|2|3)/.test(path)), "Unselected HLS voices were downloaded");
    for (const path of ["/voices.mkv", "/master.m3u8"]) {
        for (const selection of [{ ...beta, title: "Ambiguous" }, { ...beta, type: "Дублированный" },
            { ...beta, title: "Missing" }, { ...beta, channels: 2 }, { ...beta, codec: "ac3" }]) {
            const job = await save(path, selection);
            assert.equal(job.status, "error", `${path}: incorrect voice was accepted`);
            assert.equal(job.path, undefined);
        }
    }
    await queue.close();
    assert(!(await readdir(downloadDir)).some(path => path.endsWith(".part")));
    assert.equal((await readdir(join(dataDir, "downloads"))).length, 0);
    await writeFile(join(root, "result.json"), JSON.stringify({ status: "passed", verified,
        actualDefaultAlpha: true, exactBetaSavedAsDefault: true, unselectedHlsNotDownloaded: true,
        ambiguousMissingTypeChannelsCodecRejected: true, stagingRemoved: true }, null, 2));
    console.log(JSON.stringify({ status: "passed", root, verified }));
    passed = true;
}
finally {
    await queue.close();
    store.close();
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
    if (passed) {
        for (const path of [dataDir, downloadDir, fixtures])
            await rm(path, { recursive: true, force: true });
    }
}
