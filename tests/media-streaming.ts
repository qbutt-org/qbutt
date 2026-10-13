import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createConnection, type Socket } from "node:net";
import { dirname, join } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "./lab";
import { sha256, type PayloadFile, type TorrentFixture } from "./fixtures/generate";
import { mediaRuntime as resolveMediaRuntime } from "../components/content/media-runtime";

interface StreamingState {
    commandId: number;
    phase: string;
    url?: string;
    pieces: boolean[];
    prioritiesRestored: boolean;
    sequential: boolean;
    completedBytes: number;
    progress: number;
    openStreams: number;
    completionReady: boolean;
    completionActionReady: boolean;
    cancelledNativeReads?: number;
    nativeHandleReplaced?: boolean;
    files?: {fileIndex: number; nativeIndex: number; path: string; offset: number; size: number}[];
}

interface MediaFixture extends Omit<TorrentFixture, "files"> {
    files: (TorrentFixture["files"][number] & {fileIndex: number | null})[];
}

const executable = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(executable, "Set QBUTT_QT_ACCEPTANCE_EXE to the deployed Qt process driver");
const qplay = process.env.QBUTT_LAB_QPLAY;
const qplayAcceptance = process.env.QBUTT_LAB_QPLAY_NATIVE_ACCEPTANCE;
const qplayFixture = process.env.QBUTT_LAB_QPLAY_MATROSKA_FIXTURE;
const expectedAcceptanceHash = process.env.QBUTT_LAB_QPLAY_NATIVE_ACCEPTANCE_SHA256;
assert(qplay && qplayAcceptance && qplayFixture && expectedAcceptanceHash,
    "Set QBUTT_LAB_QPLAY, QBUTT_LAB_QPLAY_NATIVE_ACCEPTANCE, QBUTT_LAB_QPLAY_MATROSKA_FIXTURE and QBUTT_LAB_QPLAY_NATIVE_ACCEPTANCE_SHA256");
const qplayRoot = dirname(qplay);
const qplayLibrary = join(qplayRoot, "engine", "libmpv-2.dll");
process.env.QBUTT_LAB_PATHS = "1";
const { runtimeDir: mediaRuntime, tools: { ffmpeg, mpv } } = await resolveMediaRuntime("", AbortSignal.timeout(30_000));
process.env.QBUTT_LAB_EXE = executable;
const lab = await createLab("media-streaming");
await lab.checkpoint({check: "explicit-media-tools", mediaRuntime: await realpath(mediaRuntime),
    mpv: {path: await realpath(mpv), sha256: sha256(await readFile(mpv))},
    ffmpeg: {path: await realpath(ffmpeg), sha256: sha256(await readFile(ffmpeg))}});
const acceptanceHash = sha256(await readFile(qplayAcceptance));
assert.equal(acceptanceHash, expectedAcceptanceHash.toLowerCase(), "Native qplay acceptance executable differs from the approved build");
await lab.checkpoint({check: "native-qplay-artifacts", acceptance: await realpath(qplayAcceptance),
    acceptanceSha256: acceptanceHash, fixture: await realpath(qplayFixture),
    fixtureSha256: sha256(await readFile(qplayFixture)), qplay: await realpath(qplay),
    qplaySha256: sha256(await readFile(qplay)), runtime: await realpath(qplayLibrary),
    runtimeSha256: sha256(await readFile(qplayLibrary))});
const mediaRoot = join(lab.root, "media");
const source = join(mediaRoot, "seed", "generated-media");
let commandPath = "";
let statePath = "";
let application: Bun.Subprocess<"ignore", Bun.BunFile, Bun.BunFile> | undefined;
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let failure: unknown;
let commandId = 0;

async function run(command: string[], label: string,
    options: {cwd?: string; env?: NodeJS.ProcessEnv; timeout?: number} = {}): Promise<string> {
    const child = Bun.spawn(command, {cwd: options.cwd ?? mediaRuntime, env: options.env ?? process.env,
        stdin: "ignore", stdout: "pipe", stderr: "pipe", windowsHide: true, timeout: options.timeout ?? 120000});
    const [exit, output, errors] = await Promise.all([
        child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ]);
    assert.equal(exit, 0, `${label} exited ${exit}: ${errors}`);
    return output;
}

async function readState(): Promise<StreamingState | undefined> {
    try {
        const state = JSON.parse(await readFile(`${statePath}-${commandId}.json`, "utf8")) as StreamingState;
        assert(application?.exitCode === null || (application?.exitCode === 0 && state.phase === "finish"),
            "Qt streaming driver exited before its command completed");
        return state;
    }
    catch (error) {
        if ((error as NodeJS.ErrnoException).code === "ENOENT") {
            assert.equal(application?.exitCode, null, "Qt streaming driver exited before readiness");
            return undefined;
        }
        throw error;
    }
}

async function command(action: string, parameters: Record<string, number> = {}): Promise<StreamingState> {
    const id = ++commandId;
    await writeFile(commandPath, JSON.stringify({commandId: id, action, ...parameters}) + "\n");
    const state = await waitFor(`Qt streaming ${action}`, readState, current => current?.commandId === id, 15000);
    assert(state, "Qt streaming command returned no state");
    return state;
}

async function range(url: string, expected: Buffer, first: number, last: number, entityTag: string) {
    const response = await fetch(url, {
        headers: {Range: `bytes=${first}-${last}`, "If-Match": entityTag}, signal: AbortSignal.timeout(45000),
    });
    assert.equal(response.status, 206, "A satisfiable byte range must return Partial Content");
    assert.equal(response.headers.get("etag"), entityTag, "A range changed the complete file identity");
    assert.equal(response.headers.get("accept-ranges"), "bytes");
    assert.equal(response.headers.get("content-range"), `bytes ${first}-${last}/${expected.length}`);
    assert.equal(Number(response.headers.get("content-length")), last - first + 1);
    const bytes = Buffer.from(await response.arrayBuffer());
    const wanted = expected.subarray(first, last + 1);
    assert.equal(bytes.length, wanted.length);
    assert.equal(sha256(bytes), sha256(wanted), "Stream returned bytes outside the requested torrent range");
    return {first, last, verifiedBytes: bytes.length, sha256: sha256(bytes)};
}

function pendingRange(url: URL, offset: number) {
    const abort = new AbortController();
    const done = fetch(url, {headers: {Range: `bytes=${offset}-${offset + 65535}`}, signal: abort.signal})
        .then(async response => {
            await response.arrayBuffer();
            throw new Error("Pending range completed before cancellation");
        }).catch(error => {
            assert.equal(error.name, "AbortError", "Pending stream failed before client cancellation");
        });
    return {cancel: () => abort.abort(), done};
}

function mp4Atoms(bytes: Buffer): string[] {
    const atoms: string[] = [];
    for (let offset = 0; offset < bytes.length;) {
        const size = bytes.readUInt32BE(offset);
        assert(size >= 8 && offset + size <= bytes.length, "Generated MP4 has an invalid top-level atom");
        atoms.push(bytes.toString("ascii", offset + 4, offset + 8));
        offset += size;
    }
    return atoms;
}

async function play(url: URL, label: string, continuous = false) {
    const pipe = process.platform === "win32" ? `\\\\.\\pipe\\qbutt-media-lab-${randomUUID()}`
        : join(dirname(statePath), `mpv-${randomUUID()}.sock`);
    const child = Bun.spawn([mpv, "--no-config", "--idle=yes", "--vo=null", "--ao=null", "--no-terminal",
        "--pause=yes", ...(continuous ? [] : ["--cache=no", "--demuxer-readahead-secs=0.25"]), `--input-ipc-server=${pipe}`,
        `--log-file=${join(dirname(statePath), `mpv-${label}.log`)}`], {
        cwd: mediaRuntime, stdin: "ignore", stdout: Bun.file(join(dirname(statePath), `mpv-${label}.stdout.log`)),
        stderr: Bun.file(join(dirname(statePath), `mpv-${label}.stderr.log`)), windowsHide: true, timeout: 60000,
    });
    let socket: Socket | undefined;
    const pending = new Map<number, {resolve: (value: unknown) => void; reject: (error: Error) => void}>();
    let requestId = 0;
    let buffered = "";
    try {
        socket = await waitFor("mpv IPC readiness", async () => {
            assert.equal(child.exitCode, null, "mpv exited before opening its IPC pipe");
            return new Promise<Socket | undefined>(resolve => {
                const candidate = createConnection(pipe);
                candidate.once("connect", () => resolve(candidate));
                candidate.once("error", () => { candidate.destroy(); resolve(undefined); });
            });
        }, value => value !== undefined, 10000);
        assert(socket);
        socket.setEncoding("utf8");
        socket.on("data", chunk => {
            buffered += chunk;
            assert(buffered.length < 128 * 1024, "mpv IPC exceeded its bounded message buffer");
            for (;;) {
                const end = buffered.indexOf("\n");
                if (end < 0)
                    break;
                const message = JSON.parse(buffered.slice(0, end));
                buffered = buffered.slice(end + 1);
                const waiting = pending.get(message.request_id);
                if (waiting) {
                    pending.delete(message.request_id);
                    if (message.error === "success")
                        waiting.resolve(message.data);
                    else
                        waiting.reject(new Error(`mpv ${message.error}`));
                }
            }
        });
        socket.on("error", error => {
            for (const waiting of pending.values())
                waiting.reject(error);
            pending.clear();
        });
        const ipc = async (command: unknown[]) => {
            const id = ++requestId;
            const timer = setTimeout(() => pending.get(id)?.reject(new Error("mpv IPC command timed out")), 5000);
            try {
                return await new Promise<unknown>((resolve, reject) => {
                    pending.set(id, {resolve, reject});
                    socket!.write(JSON.stringify({command, request_id: id}) + "\n");
                });
            }
            finally {
                clearTimeout(timer);
                pending.delete(id);
            }
        };
        await ipc(["loadfile", url.href, "replace"]);
        const codecs = await waitFor("mpv opens torrent audio and video", async () => {
            try {
                return {video: await ipc(["get_property", "video-codec"]), audio: await ipc(["get_property", "audio-codec"])};
            }
            catch (error) {
                assert.equal(child.exitCode, null, String(error));
                if (!(error instanceof Error) || error.message !== "mpv property unavailable")
                    throw error;
                return undefined;
            }
        }, value => typeof value?.video === "string" && typeof value?.audio === "string", 45000);
        const seekStarted = performance.now();
        await ipc(["seek", 20, "absolute+exact"]);
        await ipc(["set_property", "pause", false]);
        const played = await waitFor("mpv decodes after a remote time seek", async () => {
            const time = await ipc(["get_property", "time-pos"]);
            const videoFrames = await ipc(["get_property", "estimated-frame-number"]);
            return {time, videoFrames};
        }, value => typeof value.time === "number" && value.time >= 21.2
            && typeof value.videoFrames === "number" && value.videoFrames > 480, 45000);
        const seekSeconds = (performance.now() - seekStarted) / 1000;
        let continuousPlayback;
        if (continuous) {
            const started = performance.now();
            await Bun.sleep(10000);
            const position = await ipc(["get_property", "time-pos"]);
            const elapsed = (performance.now() - started) / 1000;
            assert(typeof position === "number" && typeof played.time === "number");
            const advanced = position - played.time;
            assert(advanced >= elapsed - 1, `Torrent playback stalled: ${advanced.toFixed(2)} media seconds in ${elapsed.toFixed(2)} seconds`);
            continuousPlayback = {elapsedSeconds: elapsed, mediaSeconds: advanced, cache: "mpv defaults"};
        }
        socket.write(JSON.stringify({command: ["quit"]}) + "\n");
        const timer = setTimeout(() => child.kill(), 5000);
        try {
            assert.equal(await child.exited, 0, "mpv did not quit cleanly");
        }
        finally {
            clearTimeout(timer);
        }
        return {codecs, ...played, seekSeconds, continuousPlayback, renderer: "mpv null video/audio outputs", startSeconds: 20};
    }
    finally {
        socket?.destroy();
        if (child.exitCode === null) {
            child.kill();
            await child.exited;
        }
    }
}

try {
    await mkdir(source, {recursive: true});
    const mp4 = join(source, "a-faststart.mp4");
    const mkv = join(source, "b-video.mkv");
    const ass = join(source, "b-video.ass");
    const endMoov = join(source, "c-end-moov.mp4");
    await run([ffmpeg, "-hide_banner", "-nostdin", "-n", "-f", "lavfi", "-i",
        "testsrc2=size=640x360:rate=24:duration=90", "-f", "lavfi", "-i",
        "sine=frequency=1000:sample_rate=48000:duration=90", "-c:v", "libx264", "-preset", "ultrafast",
        "-b:v", "2000k", "-minrate", "2000k", "-maxrate", "2000k", "-bufsize", "2000k",
        "-x264-params", "nal-hrd=cbr:force-cfr=1", "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart", mp4],
    "Legal FFmpeg video and audio generation");
    await writeFile(ass, `[Script Info]
ScriptType: v4.00+
PlayResX: 320
PlayResY: 180

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:19.00,Default,,0,0,0,,{\\an8}{\\pos(160,35)}Long overlapping cue
Dialogue: 0,0:00:12.00,0:00:16.00,Default,,0,0,0,,Second cue
`);
    await run([ffmpeg, "-hide_banner", "-nostdin", "-n", "-i", mp4, "-i", ass,
        "-map", "0", "-map", "1:0", "-c", "copy", mkv], "Generated indexed Matroska with ASS");
    await run([ffmpeg, "-hide_banner", "-nostdin", "-n", "-i", mp4, "-map", "0", "-c", "copy", endMoov],
        "Generated MP4 with trailing moov atom");
    for (const path of [mp4, mkv, endMoov])
        await run([ffmpeg, "-hide_banner", "-nostdin", "-v", "error", "-xerror", "-i", path,
            "-map", "0:v", "-map", "0:a", "-f", "null", "-"], "Generated media decode validation");
    const authoredSubtitles = await run([ffmpeg, "-hide_banner", "-nostdin", "-v", "error", "-xerror",
        "-i", mkv, "-map", "0:s:0", "-c:s", "copy", "-f", "ass", "pipe:1"], "Generated ASS track extraction");
    assert(authoredSubtitles.includes("Style: Default,Arial,20") && authoredSubtitles.includes("Second cue"),
        "Generated Matroska lost its authored text track");
    const payload: PayloadFile[] = [];
    const sources = new Map<string, Buffer>();
    for (const name of ["a-faststart.mp4", "b-video.mkv", "c-end-moov.mp4"]) {
        const bytes = await readFile(join(source, name));
        assert(bytes.length > 16 * 1024 * 1024, "Media fixture must remain incomplete through remote seek checks");
        const path = `generated-media/${name}`;
        sources.set(path, bytes);
        payload.push({path, size: bytes.length, sha256: sha256(bytes)});
    }
    const faststartAtoms = mp4Atoms(sources.get(payload[0]!.path)!);
    const trailingAtoms = mp4Atoms(sources.get(payload[2]!.path)!);
    assert(faststartAtoms.indexOf("moov") < faststartAtoms.indexOf("mdat"), "Faststart fixture moov must precede media data");
    assert(trailingAtoms.indexOf("moov") > trailingAtoms.indexOf("mdat"), "Trailing-moov fixture must require a tail probe");
    await writeFile(join(mediaRoot, "payload.json"), JSON.stringify(payload, null, 2) + "\n");
    const generator = `
import json
import pathlib
import sys
import libtorrent as lt

assert lt.__version__ == "2.0.14.0", "Install pinned tests/fixtures/requirements.txt"
root = pathlib.Path(sys.argv[1])
descriptions = []
for variant, flags in [("v1", lt.create_torrent.v1_only), ("v2", lt.create_torrent.v2_only), ("hybrid", 0)]:
    files = lt.file_storage()
    for item in json.loads((root / "payload.json").read_text(encoding="utf-8")):
        files.add_file(item["path"], item["size"])
    creator = lt.create_torrent(files, 65536, flags)
    creator.set_creator("qbutt generated media integration fixture")
    creator.set_comment("FFmpeg testsrc2 and sine; no third-party media")
    creator.set_priv(True)
    lt.set_piece_hashes(creator, str(root / "seed"))
    metadata = creator.generate()
    metadata.pop(b"creation date", None)
    encoded = lt.bencode(metadata)
    name = "media-" + variant
    (root / (name + ".torrent")).write_bytes(encoded)
    info = lt.torrent_info(encoded)
    storage = info.files()
    mapped = []
    file_index = 0
    for i in range(storage.num_files()):
        pad = bool(storage.file_flags(i) & lt.file_storage.flag_pad_file)
        mapped.append({"index": i, "fileIndex": None if pad else file_index,
            "path": storage.file_path(i).replace("\\\\", "/"), "size": storage.file_size(i),
            "offset": storage.file_offset(i), "pad": pad})
        if not pad:
            file_index += 1
    descriptions.append({"name": name, "file": name + ".torrent", "pieceLength": info.piece_length(),
        "pieceCount": info.num_pieces(), "infoHashV1": str(info.info_hashes().v1) if info.info_hashes().has_v1() else None,
        "infoHashV2": str(info.info_hashes().v2) if info.info_hashes().has_v2() else None, "files": mapped})
print(json.dumps(descriptions))
`;
    const fixtures = JSON.parse(await run([lab.python, "-c", generator, mediaRoot], "Media torrent generation")) as MediaFixture[];
    await lab.checkpoint({check: "legal-media-generated", payload, codec: "H.264/AAC with indexed Matroska ASS",
        containers: ["MP4 faststart", "Matroska", "MP4 trailing moov"], faststartAtoms, trailingAtoms,
        formats: fixtures.map(fixture => ({name: fixture.name, pieceCount: fixture.pieceCount,
            paddingFiles: fixture.files.filter(file => file.pad).length}))});
    const entityTags = new Set<string>();
    for (const fixture of fixtures) {
        const scenario = join(lab.root, fixture.name);
        await mkdir(scenario);
        const destination = join(lab.root, "download", fixture.name);
        commandPath = join(scenario, "command.json");
        statePath = join(scenario, "state");
        const evidencePath = join(scenario, "qt-evidence.json");
        commandId = 0;
        const visibleFiles = fixture.files.filter(file => !file.pad);
        assert.equal(visibleFiles.length, payload.length);
        if (fixture.name !== "media-v1")
            assert(fixture.files.some(file => file.pad), "v2 and hybrid must exercise native padding-file mappings");
        seed = await startSeed(lab.python, mediaRoot, fixture.name, scenario, {uploadRate: 1024 * 1024});
        const specPath = join(scenario, "spec.json");
        await writeFile(specPath, JSON.stringify({mode: "media-streaming", commandPath, statePath, evidencePath,
            torrentPath: join(mediaRoot, fixture.file), destination, peer: `${seed.host}:${seed.port}`}) + "\n");
        application = Bun.spawn([executable, `--profile=${join(lab.root, "profile")}`], {
            cwd: dirname(executable), windowsHide: true, stdin: "ignore", timeout: 300000,
            env: {...process.env, QT_QPA_PLATFORM: "offscreen", QBUTT_QT_ACCEPTANCE_SPEC: specPath},
            stdout: Bun.file(join(scenario, "app.stdout.log")), stderr: Bun.file(join(scenario, "app.stderr.log")),
        });
        const ready = await waitFor("Qt media streaming readiness", readState, state => state?.phase === "ready", 45000);
        assert.deepEqual(ready?.files, visibleFiles.map(file => ({fileIndex: file.fileIndex, nativeIndex: file.index,
            path: file.path, offset: file.offset, size: file.size})), "Qt visible file mapping differs from native torrent metadata");
        await command("rate", {bytesPerSecond: 256 * 1024});
        const fileEntityTags = new Map<number, string>();
        for (const file of visibleFiles) {
            const expected = sources.get(file.path)!;
            assert(file.fileIndex !== null);
            const state = await command("open", {fileIndex: file.fileIndex});
            assert(state.url, "Stream server did not provide a loopback URL");
            const url = new URL(state.url);
            assert.equal(url.hostname, "127.0.0.1", "Streaming must use an authenticated loopback listener");
            const head = await fetch(url, {method: "HEAD", signal: AbortSignal.timeout(5000)});
            assert.equal(head.status, 200);
            assert.equal(Number(head.headers.get("content-length")), expected.length);
            assert.equal(head.headers.get("accept-ranges"), "bytes");
            const entityTag = head.headers.get("etag");
            assert(entityTag && /^"[\x21\x23-\x7e\x80-\xff]+"$/.test(entityTag),
                "The verified file needs a syntactically valid strong ETag");
            assert(!entityTags.has(entityTag), "Different torrent files shared a representation identity");
            entityTags.add(entityTag);
            fileEntityTags.set(file.fileIndex, entityTag);
            for (const condition of ['"other-file"', `W/${entityTag}`]) {
                const mismatch = await fetch(url, {
                    headers: {Range: "bytes=0-0", "If-Match": condition},
                    signal: AbortSignal.timeout(5000),
                });
                assert.equal(mismatch.status, 412, "A stale or weak entity tag allowed a torrent read");
                assert.equal((await mismatch.arrayBuffer()).byteLength, 0);
            }
            for (const condition of ["*", `"other-file", ${entityTag}`]) {
                const matched = await fetch(url, {
                    method: "HEAD", headers: {"If-Match": condition}, signal: AbortSignal.timeout(5000),
                });
                assert.equal(matched.status, 200, "A matching If-Match condition was rejected");
                assert.equal(matched.headers.get("etag"), entityTag);
            }
            const mime = file.path.endsWith(".mp4") ? "video/mp4" : "video/x-matroska";
            assert.equal(head.headers.get("content-type")?.split(";")[0], mime);
            const forbidden = new URL(url);
            const forbiddenPath = forbidden.pathname.split("/");
            forbiddenPath[2] = "invalid-stream-capability";
            forbidden.pathname = forbiddenPath.join("/");
            const rejected = await fetch(forbidden, {signal: AbortSignal.timeout(5000)});
            assert([403, 404].includes(rejected.status), "A URL without its stream capability was accepted");
            await rejected.body?.cancel();
            const invalid = await fetch(url, {headers: {Range: `bytes=${expected.length}-`}, signal: AbortSignal.timeout(5000)});
            assert.equal(invalid.status, 416);
            assert.equal(invalid.headers.get("content-range"), `bytes */${expected.length}`);
            await invalid.body?.cancel();
            if (file.fileIndex === 0) {
                await command("rate", {bytesPerSecond: 1024});
                const probeUrl = new URL(url);
                probeUrl.searchParams.set("role", "probe");
                const probe = pendingRange(probeUrl, Math.floor(expected.length * 0.32));
                let urgent: ReturnType<typeof pendingRange> | undefined;
                try {
                    await Bun.sleep(200);
                    assert.equal((await command("snapshot")).prioritiesRestored, true, "A background probe changed normal priorities");
                    urgent = pendingRange(url, Math.floor(expected.length * 0.68));
                    await waitFor("Urgent request overtakes a concurrent background probe", () => command("snapshot"),
                        snapshot => !snapshot.prioritiesRestored, 5000);
                }
                finally {
                    urgent?.cancel();
                    probe.cancel();
                    await Promise.all([urgent?.done, probe.done]);
                }
                await waitFor("Cancelled probe and urgent request release priorities", () => command("snapshot"),
                    snapshot => snapshot.prioritiesRestored, 5000);
                await command("rate", {bytesPerSecond: 256 * 1024});
                await lab.checkpoint({check: "probe-role-does-not-preempt-playback", format: fixture.name,
                    independentConcurrentConsumers: true, prioritiesRestoredAfterCancel: true});
                const priorityOffset = Math.floor(expected.length * 0.40);
                const pendingPriorityRange = range(url.href, expected, priorityOffset, priorityOffset + 65535, entityTag);
                await Bun.sleep(100);
                await command("priority", {fileIndex: file.fileIndex, priority: 0});
                await command("rate", {bytesPerSecond: 512 * 1024});
                const priorityRange = await pendingPriorityRange;
                await command("priority", {fileIndex: file.fileIndex, priority: 1});
                await waitFor("Ordinary priorities restored after a live file-priority change", () => command("snapshot"),
                    snapshot => snapshot.prioritiesRestored, 5000);
                await command("rate", {bytesPerSecond: 256 * 1024});
                await lab.checkpoint({check: "active-stream-survives-file-priority-change", format: fixture.name,
                    ignoredFileStillStreamsVerifiedBytes: priorityRange, originalPrioritiesRestored: true});
            }
            const middle = Math.floor(expected.length * 0.73);
            const tail = expected.length - 128 * 1024 - 1;
            const requestedPieces = [middle, tail].map(offset => Math.floor((file.offset + offset) / fixture.pieceLength));
            const before = await command("snapshot");
            assert(requestedPieces.every(piece => before.pieces[piece] === false),
                "Remote seek targets already downloaded; this run cannot prove seek prioritization");
            const beginning = await range(url.href, expected, 0, 32767, entityTag);
            const ranges = await Promise.all([
                range(url.href, expected, middle, middle + 64 * 1024 - 1, entityTag),
                range(url.href, expected, tail, tail + 64 * 1024 - 1, entityTag),
            ]);
            const after = await command("snapshot");
            assert(after.progress < 1, "Streaming waited for full torrent completion before serving remote seeks");
            assert(requestedPieces.every(piece => after.pieces[piece] === true), "Remote ranges were not verified by libtorrent");
            await command("rate", {bytesPerSecond: 1024});
            const cancelOffset = Math.floor(expected.length * 0.91);
            const cancelPiece = Math.floor((file.offset + cancelOffset) / fixture.pieceLength);
            assert.equal((await command("snapshot")).pieces[cancelPiece], false, "Cancellation target was already available");
            const cancelled = pendingRange(url, cancelOffset);
            await Bun.sleep(250);
            cancelled.cancel();
            await cancelled.done;
            await command("rate", {bytesPerSecond: 512 * 1024});
            const playback = await play(url, `${file.fileIndex}-incomplete`, (fixture.name === "media-v1") && (file.fileIndex === 0));
            assert((await command("snapshot")).progress < 1, "mpv waited for the whole torrent");
            if ((fixture.name === "media-v1") && file.path.endsWith(".mkv")) {
                const beforeNative = await command("snapshot");
                assert(beforeNative.progress < 1, "Native qplay subtitle playback requires an incomplete torrent");
                const profile = join(scenario, "qplay-subtitle-profile");
                const prefix = join(scenario, "qplay-embedded-subtitles");
                await mkdir(profile, {recursive: true});
                const qplayEnvironment = {
                    ...process.env,
                    Path: `${qplayRoot};${process.env.Path ?? process.env.PATH ?? ""}`,
                    QT_PLUGIN_PATH: qplayRoot,
                    QML_IMPORT_PATH: join(qplayRoot, "qml"),
                    QT_COMMAND_LINE_PARSER_NO_GUI_MESSAGE_BOXES: "1",
                    QPLAY_SUBTITLE_MKV_QA: "ass",
                };
                const seeded = await run([qplayFixture, url.href, "3", "Second cue", "ass", "12000", "19000", profile],
                    "Cache seed from the real torrent stream", {cwd: qplayRoot, env: qplayEnvironment, timeout: 60000});
                await writeFile(`${prefix}.seed.log`, seeded);
                const nativeOutput = await run([qplayAcceptance, qplayLibrary, url.href, mkv, profile, prefix],
                    "Native qplay translation from the real torrent stream",
                    {cwd: qplayRoot, env: qplayEnvironment, timeout: 60000});
                await writeFile(`${prefix}.log`, nativeOutput);
                const result = JSON.parse(await readFile(`${prefix}.json`, "utf8"));
                assert.equal(result.format, "ass");
                assert.equal(result.remoteEmbeddedTranslation, true);
                assert.equal(result.cacheStyleOriginal, true);
                assert.equal(result.gpuPixelsChanged, true);
                assert.equal(result.ownedChildren, 0);
                assert(result.selectedSubtitle > 0, "Native qplay lost the original Matroska text track");
                const afterNative = await command("snapshot");
                assert(afterNative.progress < 1, "Native qplay waited for full torrent completion");
                await lab.checkpoint({check: "native-qplay-embedded-subtitles", result,
                    evidence: `${prefix}.json`, frame: `${prefix}.embedded-subtitle.png`,
                    torrentVerifiedBefore: beforeNative.completedBytes,
                    torrentVerifiedAfter: afterNative.completedBytes,
                    torrentIncomplete: true, translationSource: "preseeded accepted cache"});
                await rm(profile, {recursive: true, force: true});
            }
            const closed = await command("close", {fileIndex: file.fileIndex});
            assert.equal(closed.openStreams, 0);
            assert.equal(closed.prioritiesRestored, true, "Stopping playback changed ordinary torrent priorities or sequential mode");
            await fetch(url, {signal: AbortSignal.timeout(5000)}).then(async retired => {
                assert([403, 404, 410].includes(retired.status), "Closed streaming URL remains usable");
                await retired.body?.cancel();
            }, error => {
                const code = error.code ?? error.cause?.code;
                assert(["ECONNREFUSED", "ConnectionRefused"].includes(code),
                    `Closed streaming listener failed unexpectedly: ${String(error)}`);
            });
            await lab.checkpoint({check: "remote-seek-concurrent-ranges-cancel-stop", format: fixture.name, file: file.path,
                beginning, ranges, verifiedBefore: before.completedBytes, verifiedAfter: after.completedBytes,
                torrentIncomplete: true, prioritiesRestored: closed.prioritiesRestored, sequential: closed.sequential,
                headMime: mime, entityTag, staleEntityRejected: true, weakEntityRejected: true,
                invalidRangeRejected: true, retiredCapabilityRejected: true, playback});
            await command("rate", {bytesPerSecond: 256 * 1024});
        }
        await command("rate", {bytesPerSecond: 0});
        await waitFor("Ordinary media download after playback stop", () => command("snapshot"), state => state.progress === 1, 120000);
        const verifiedBytes = await verifyPayload(destination, payload);
        const seedEvidence = await seed.stop();
        seed = undefined;
        assert.equal(seedEvidence.downloadPayloadBytes, 0, "Generated seed unexpectedly downloaded payload");
        await waitFor("Completed session permits its completion actions", () => command("snapshot"),
            snapshot => snapshot.completionActionReady, 15000);
        const cached = await command("open", {fileIndex: 2});
        assert(cached.url);
        assert.equal(cached.completionReady, false, "Automatic completion may interrupt an active playback capability");
        assert.equal(cached.completionActionReady, false, "Session completion actions may interrupt cached playback");
        const cancelled = await command("cancel-read-burst");
        assert(cancelled.cancelledNativeReads && cancelled.cancelledNativeReads <= 8,
            "Cancelled cached reads exceeded the in-flight memory bound");
        const recovered = await command("reload-pending-reads");
        assert.equal(recovered.nativeHandleReplaced, true);
        assert.equal(recovered.openStreams, 0, "Reload left an old playback capability active");
        const recoveredStream = await command("open", {fileIndex: 2});
        assert(recoveredStream.url);
        assert.notEqual(recoveredStream.url, cached.url, "Recovery reused a retired playback capability");
        const recoveredHead = await fetch(recoveredStream.url, {method: "HEAD", signal: AbortSignal.timeout(5000)});
        assert.equal(recoveredHead.status, 200);
        assert.equal(recoveredHead.headers.get("etag"), fileEntityTags.get(2),
            "Reopening the same verified file changed its identity");
        await Bun.sleep(2000);
        const cachedPlayback = await play(new URL(recoveredStream.url), "cached-complete");
        await command("close", {fileIndex: 2});
        await waitFor("Closing playback releases session completion actions", () => command("snapshot"),
            snapshot => snapshot.completionActionReady, 15000);
        await command("finish");
        const exit: number = await application.exited;
        const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
        assert.equal(exit, 0, `Qt streaming driver exited ${exit}: ${JSON.stringify(evidence)}`);
        assert.equal(evidence.status, "passed", JSON.stringify(evidence));
        application = undefined;
        await lab.checkpoint({check: "ordinary-download-after-stream-stop", format: fixture.name, verifiedBytes, exactSizes: true,
            exactSha256: true, seedUploadPayloadBytes: seedEvidence.uploadPayloadBytes, qtEvidence: evidencePath,
            cachedPlayback, cachedPlaybackWithoutPeers: true,
            cancelledNativeReads: cancelled.cancelledNativeReads, completionActionsDeferredDuringPlayback: true,
            reloadWithOutstandingReads: true, entityPreservedAfterReopen: true,
            scope: "Generated faststart/trailing-moov MP4 and MKV; production libtorrent, HTTP server and pinned mpv with null outputs"});
    }
}
catch (error) {
    failure = error;
}
finally {
    if (application?.exitCode === null) {
        application.kill();
        await application.exited;
    }
    try {
        await seed?.stop();
    }
    catch (error) {
        failure ??= error;
    }
    if (!failure) {
        const root = await realpath(lab.root);
        for (const name of ["profile", "fixtures", "media", "download"]) {
            const path = join(root, name);
            assert.equal(dirname(path), root, "Media lab cleanup escaped its generated root");
            await rm(path, {recursive: true, force: true});
        }
    }
}
await lab.finish(failure);
if (failure)
    throw failure;
