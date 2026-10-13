import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";

const [application, peer, evidence] = process.argv.slice(2);
if (!application || !peer || !evidence || ![application, peer, evidence].every(isAbsolute)) {
    throw new Error("Pass absolute acceptance executable, Codex text peer and fresh evidence directory.");
}

await mkdir(evidence);
const runtime = await mkdtemp(join(tmpdir(), "qbutt-text-ahead-"));
const native = join(runtime, "nodejs/node_modules/@openai/codex/node_modules",
    "@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin");
await mkdir(native, { recursive: true });
await Bun.write(join(native, "codex.exe"), Bun.file(peer));
const profile = join(evidence, "profile");
await mkdir(profile);

try {
    const child = Bun.spawn([application, "--reader-text-ahead", "--lang", "ru", "--profile", profile], {
        env: {
            ...process.env,
            QBUTT_TEXT_PEER_DIRECTORY: native,
            QBUTT_TEXT_PEER_ROOT: runtime,
            QT_QPA_PLATFORM: "offscreen",
            QT_QUICK_BACKEND: "software",
            QT_QPA_FONTDIR: "C:/Windows/Fonts",
            QT_FORCE_STDERR_LOGGING: "1",
        },
        stdout: Bun.file(join(evidence, "stdout.log")),
        stderr: Bun.file(join(evidence, "stderr.log")),
        windowsHide: true,
    });
    const timeout = setTimeout(() => child.kill(), 120_000);
    const code = await child.exited;
    clearTimeout(timeout);
    const log = await readFile(join(evidence, "stderr.log"), "utf8");
    console.log(log);
    const tracePaths = [...new Bun.Glob("turn-*.json").scanSync(native)];
    const turns = await Promise.all(tracePaths.map(async path =>
        JSON.parse(await readFile(join(native, path), "utf8"))));
    await writeFile(join(evidence, "receipt.json"), JSON.stringify({
        exitCode: code,
        executable: resolve(application),
        sha256: new Bun.CryptoHasher("sha256").update(await Bun.file(application).arrayBuffer()).digest("hex"),
        turns,
    }, null, 2));
    if (code !== 0 || !log.includes("PASS reader_text_ahead stale_revision no_paid_retry")) {
        throw new Error(`Native text read-ahead failed with exit ${code}.`);
    }
} finally {
    if (dirname(runtime) !== resolve(tmpdir()) || !runtime.startsWith(join(tmpdir(), "qbutt-text-ahead-"))) {
        throw new Error("Unsafe fixture cleanup path.");
    }
    await rm(runtime, { recursive: true });
}
