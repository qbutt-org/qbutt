import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { Database } from "bun:sqlite";

const [application, peer, fixtures, evidence] = process.argv.slice(2);
if (!application || !peer || !fixtures || !evidence || ![application, peer, fixtures, evidence].every(isAbsolute)) {
    throw new Error("Pass absolute acceptance executable, manga protocol peer, authored page fixtures and fresh evidence directory.");
}

await mkdir(evidence);
const runtime = await mkdtemp(join(tmpdir(), "qbutt-manga-ahead-"));
try {
    const windows = process.platform === "win32";
    const native = windows
        ? join(runtime, "nodejs/node_modules/@openai/codex/node_modules",
            "@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin")
        : runtime;
    await mkdir(native, { recursive: true });
    const executable = join(native, windows ? "codex.exe" : "codex");
    await Bun.write(executable, Bun.file(peer));
    if (!windows) {
        await chmod(executable, 0o700);
    }
    await writeFile(join(native, "case.txt"), "ahead");

    const pageFixtures = join(runtime, "pages");
    await mkdir(pageFixtures);
    await Bun.write(join(pageFixtures, "generated-cleanup.png"), Bun.file(join(fixtures, "generated-cleanup.png")));

    const proposal = await Bun.file(join(fixtures, "proposal.json")).json();
    const region = proposal.regions[0];
    proposal.terms = [{
        source: "Ren",
        target: "Рен",
        forms: [],
        evidence: { region_id: region.id, quote: "Ren" },
    }];
    proposal.facts = [{
        subject: "Ren",
        attribute: "status",
        value: "awaited",
        evidence: { region_id: region.id, quote: region.sourceText },
    }];
    await writeFile(join(pageFixtures, "proposal.json"), JSON.stringify(proposal));
    await writeFile(join(native, "fixture-directory.txt"), pageFixtures);
    await writeFile(join(native, "trace.jsonl"), "");

    const profile = join(evidence, "profile");
    await mkdir(profile);

    const prepared = Bun.spawn([peer, "--prepare-ahead", join(fixtures, "original.png"), profile], {
        stdout: "pipe",
        stderr: "pipe",
        windowsHide: true,
    });
    const [preparedCode, preparedError] = await Promise.all([
        prepared.exited, new Response(prepared.stderr).text(),
    ]);
    if (preparedCode !== 0) {
        throw new Error(`Authored CBZ preparation failed with exit ${preparedCode}: ${preparedError}`);
    }

    const child = Bun.spawn([application, "--reader-manga-ahead", "--lang", "ru", "--profile", profile], {
        env: {
            ...process.env,
            QBUTT_MANGA_PEER_DIRECTORY: native,
            QBUTT_MANGA_PEER_ROOT: runtime,
            QT_QPA_PLATFORM: "offscreen",
            QT_QUICK_BACKEND: "software",
            ...(windows ? { QT_QPA_FONTDIR: "C:/Windows/Fonts" } : {}),
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

    const tracePaths = [...new Bun.Glob("turn-*.json").scanSync(native)].sort();
    const turns = await Promise.all(tracePaths.map(async path =>
        JSON.parse(await readFile(join(native, path), "utf8"))));
    let cache;
    if (code === 0) {
        const database = new Database(join(profile, "content-library.sqlite"), { readonly: true });
        try {
            const integrity = database.query("PRAGMA quick_check").get() as { quick_check: string };
            if (integrity.quick_check !== "ok") {
                throw new Error(`Native reader cache failed integrity check: ${integrity.quick_check}`);
            }
            cache = database.query("SELECT (SELECT count(*) FROM translated_images) AS images, (SELECT count(*) FROM translation_context) AS contexts").get();
        } finally {
            database.close();
        }
    }

    await writeFile(join(evidence, "receipt.json"), JSON.stringify({
        exitCode: code,
        executable: resolve(application),
        sha256: new Bun.CryptoHasher("sha256").update(await Bun.file(application).arrayBuffer()).digest("hex"),
        peer: resolve(peer),
        peerSha256: new Bun.CryptoHasher("sha256").update(await Bun.file(peer).arrayBuffer()).digest("hex"),
        checks: log.split("\n").filter(line => line.startsWith("PASS reader_manga_ahead")),
        attemptedPagesPolicy: "Each page is prefetched once within the chapter and translation-settings lifecycle. Failed or canceled targets are not retried automatically after other prefetches. Visiting a target may retry foreground work; reader, chapter and settings resets clear attempts.",
        liveInferenceCalls: 0,
        nativePointerVerified: false,
        cache,
        turns,
    }, null, 2));
    await writeFile(join(evidence, "trace.jsonl"), await readFile(join(native, "trace.jsonl")));

    if (code !== 0 || turns.length !== 25
        || !log.includes("PASS reader_manga_ahead chapter_cancel background_failure foreground_retry cleanup")
        || !log.includes("PASS reader_manga_ahead foreground_decline service_pause retained_pages explicit_retry original offline_cache chapter_pause")) {
        throw new Error(`Native manga read-ahead failed with exit ${code}.`);
    }
} finally {
    if (dirname(runtime) !== resolve(tmpdir()) || !runtime.startsWith(join(tmpdir(), "qbutt-manga-ahead-"))) {
        throw new Error("Unsafe fixture cleanup path.");
    }
    await rm(runtime, { recursive: true });
}
