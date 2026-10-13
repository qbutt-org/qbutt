import assert from "node:assert/strict";
import { Database } from "bun:sqlite";
import { createHash } from "node:crypto";
import { copyFile, mkdir, readFile, stat, unlink, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

import { labAppearanceSettings } from "./appearance.ts";

const [phase, ...arguments_] = Bun.argv.slice(2);
assert((phase === "prepare" && arguments_.length === 2) || (phase === "run" && arguments_.length === 4),
    "Usage: prepare <main build> <fresh shared output>; run <VM Qt runtime> <qplay exe> <legal MP4> <prepared output>. Run opens native Windows UI; use the isolated VM.");
const output = resolve(arguments_.at(-1)!);
const repository = resolve(import.meta.dir, "..");
if (phase === "prepare")
    assert(!output.startsWith(repository + "\\"), "Keep fixture evidence outside the checkout");
await mkdir(output, { recursive: true });
let sequence = 0;

async function stopOwnedTree(child: ReturnType<typeof Bun.spawn>) {
    if (child.exitCode !== null)
        return;
    const killer = Bun.spawn(["taskkill.exe", "/PID", String(child.pid), "/T", "/F"], {
        windowsHide: true, stdin: "ignore", stdout: "pipe", stderr: "pipe",
    });
    const timer = setTimeout(() => killer.kill(), 10_000);
    const [code, stdout, stderr] = await Promise.all([
        killer.exited, new Response(killer.stdout).text(), new Response(killer.stderr).text(),
    ]).finally(() => clearTimeout(timer));
    await writeFile(join(output, `${phase}-cleanup-${++sequence}.log`), stdout + stderr);
    assert(code === 0 || child.exitCode !== null, `Cannot stop owned process tree ${child.pid}: ${stderr}`);
    await child.exited;
}

async function run(command: string[], timeoutMs = 300_000) {
    const child = Bun.spawn(command, {
        cwd: repository,
        windowsHide: true,
        env: { ...process.env, CARGO_BUILD_JOBS: "2", CMAKE_BUILD_PARALLEL_LEVEL: "2" },
        stdin: "ignore", stdout: "pipe", stderr: "pipe",
    });
    let timedOut = false;
    let termination: Promise<void> | undefined;
    let terminationError: unknown;
    const timer = setTimeout(() => {
        timedOut = true;
        termination = stopOwnedTree(child).catch(error => { terminationError = error; });
    }, timeoutMs);
    const [exitCode, stdout, stderr] = await Promise.all([
        child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ]).finally(() => clearTimeout(timer));
    await termination;
    await writeFile(join(output, `${phase}-command-${++sequence}.log`), stdout + stderr);
    if (terminationError)
        throw terminationError;
    assert(!timedOut && exitCode === 0, `${command[0]} failed: exit=${exitCode}, timeout=${timedOut}\n${stderr.slice(-3000)}`);
    return stdout.trim();
}

const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

async function prepare(build: string) {
    assert(!(await Bun.file(join(output, "prepare.json")).exists()), "Use a fresh preparation directory");
    const certificate = join(output, "fixture-cert.pem");
    const privateKey = join(output, "fixture-key.pem");
    const stagedDriver = join(output, "qbutt-save-driver.exe");
    assert(!(await Bun.file(stagedDriver).exists()), "A prepared Save driver already exists");
    await run(["C:/Program Files/Git/usr/bin/openssl.exe", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
        "-days", "2", "-subj", "/CN=anilibria.top", "-addext", "subjectAltName=DNS:anilibria.top",
        "-addext", "basicConstraints=critical,CA:FALSE",
        "-addext", "keyUsage=critical,digitalSignature,keyEncipherment",
        "-addext", "extendedKeyUsage=serverAuth",
        "-keyout", privateKey, "-out", certificate]);
    const sourcePaths = [join(repository, "src/content/src/http.rs"), join(repository, "src/content/src/download/transfer.rs")];
    const sourceStatus = await run(["git", "-C", repository, "-c", `safe.directory=${repository}`, "status", "--porcelain", "--", ...sourcePaths]);
    assert.equal(sourceStatus, "", "A fixture transport source is already changed; refusing to overwrite it");
    const transport = sourcePaths.map(path => ({ path, original: new Uint8Array(), patched: new Uint8Array(), applied: false }));
    const certPath = certificate.replaceAll("\\", "/");
    assert(!certPath.includes('"') && !certPath.includes("\n"), "Unsupported certificate path");
    const redirect = [
        '.dns_resolver(Arc::new(PublicResolver))',
        '        .resolve("anilibria.top", "127.0.0.1:443".parse().expect("fixture endpoint"))',
        `        .add_root_certificate(reqwest::Certificate::from_pem(include_bytes!(r#"${certPath}"#)).expect("fixture CA"))`,
    ].join("\n");
    for (const item of transport) {
        item.original = await readFile(item.path);
        const source = new TextDecoder().decode(item.original);
        const needle = '.dns_resolver(Arc::new(PublicResolver))';
        const expected = item.path.endsWith("http.rs") ? 2 : 1;
        assert.equal(source.split(needle).length - 1, expected, `Transport builder changed: ${item.path}`);
        item.patched = new TextEncoder().encode(source.replaceAll(needle, redirect));
    }

    const originalDigests = transport.map(item => sha256(item.original));
    let completed = false;
    try {
        for (const item of transport) {
            assert.equal(sha256(await readFile(item.path)), sha256(item.original), "Transport changed before patch admission");
            await writeFile(item.path, item.patched);
            item.applied = true;
        }
        await run(["cmake", "--build", build, "--target", "qbutt-qt-acceptance", "--parallel", "2"]);
        await copyFile(join(build, "qbutt-qt-acceptance.exe"), stagedDriver);
        await copyFile(import.meta.path, join(output, "media-save.ts"));
        await copyFile(join(import.meta.dir, "appearance.ts"), join(output, "appearance.ts"));
        await writeFile(join(output, "prepare.json"), JSON.stringify({ status: "passed",
            driverSha256: sha256(await readFile(stagedDriver)),
            sourceRevision: await run(["git", "-C", repository, "-c", `safe.directory=${repository}`, "rev-parse", "HEAD"]),
            certificateSha256: sha256(await readFile(certificate)) }, null, 4));
        completed = true;
    } finally {
        const cleanupErrors: unknown[] = [];
        const restoreErrors: unknown[] = [];
        for (const item of transport) {
            if (!item.applied)
                continue;
            try {
                assert.equal(sha256(await readFile(item.path)), sha256(item.patched),
                    `Transport changed while patched; refusing to overwrite unrelated work: ${item.path}`);
                await writeFile(item.path, item.original);
                assert.equal(sha256(await readFile(item.path)), sha256(item.original), "Transport restoration failed");
            } catch (error) {
                restoreErrors.push(error);
            }
        }
        if (!restoreErrors.length) {
            try {
                await run(["cmake", "--build", build, "--target", "qbt_app", "qbutt-qt-acceptance", "--parallel", "2"]);
            } catch (error) {
                cleanupErrors.push(error);
            }
        }
        await writeFile(join(output, "restoration.json"), JSON.stringify({ restored: restoreErrors.length === 0,
            completed, cleanupErrors: cleanupErrors.map(String), originalDigests,
            currentDigests: await Promise.all(transport.map(async item => sha256(await readFile(item.path)))) }, null, 4));
        const errors = [...restoreErrors, ...cleanupErrors];
        if (errors.length)
            throw new AggregateError(errors, "Native Save preparation restoration needs review");
    }
    console.log(JSON.stringify({ status: "prepared", output }));
}

async function runPrepared(runtime: string, qplay: string, mediaPath: string) {
    assert(!(await Bun.file(join(output, "receipt.json")).exists()), "Use an untested preparation directory");
    const prepared = JSON.parse(await readFile(join(output, "prepare.json"), "utf8"));
    const restoration = JSON.parse(await readFile(join(output, "restoration.json"), "utf8"));
    assert(prepared.status === "passed" && restoration.restored && restoration.completed
        && restoration.cleanupErrors.length === 0, "Preparation did not restore production sources/build");
    const stagedDriver = join(output, "qbutt-save-driver.exe");
    assert.equal(sha256(await readFile(stagedDriver)), prepared.driverSha256, "Prepared driver changed");
    const driver = join(runtime, "qbutt-qt-acceptance-save.exe");
    assert(!(await Bun.file(driver).exists()), "The isolated Save driver already exists");
    const certificate = join(output, "fixture-cert.pem");
    const privateKey = join(output, "fixture-key.pem");
    assert.equal(sha256(await readFile(certificate)), prepared.certificateSha256, "Fixture certificate changed");
    const profile = join(output, "profile");
    const downloads = join(output, "downloads");
    const evidencePath = join(output, "evidence.json");
    assert(!(await Bun.file(evidencePath).exists()), "The prepared fixture has already run");
    const pipeFile = join(output, "player-pipe.txt");
    const specPath = join(output, "spec.json");
    const media = Bun.file(mediaPath);
    assert((await stat(mediaPath)).isFile(), "Legal media is missing");
    const mediaHash = sha256(await media.bytes());
    let searchReads = 0;
    let publicationReads = 0;
    let payloadReads = 0;
    let failNextResolution = false;
    let injectedFailure = false;
    const publication = {
        id: 900001, alias: "native-save-legal-fixture", name: { main: "Native Save legal fixture" }, year: 2026,
        episodes: [
            { id: "save-episode-1", ordinal: 1, hls_720: "https://anilibria.top/legal.mp4" },
            { id: "save-episode-2", ordinal: 2, hls_720: "https://anilibria.top/other.mp4" },
        ],
    };
    const server = Bun.serve({
        hostname: "127.0.0.1", port: 443,
        tls: { cert: Bun.file(certificate), key: Bun.file(privateKey) },
        fetch(request) {
            const path = new URL(request.url).pathname;
            if (path === "/api/v1/app/search/releases") {
                ++searchReads;
                return Response.json([publication]);
            }
            if (path === "/api/v1/anime/releases/900001") {
                ++publicationReads;
                if (failNextResolution) {
                    failNextResolution = false;
                    injectedFailure = true;
                    return Response.json({ ...publication, episodes: [] });
                }
                return Response.json(publication);
            }
            if (path !== "/legal.mp4")
                return new Response(null, { status: 404 });
            ++payloadReads;
            const range = request.headers.get("range");
            const match = range && /^bytes=(\d+)-(\d*)$/.exec(range);
            const start = match ? Number(match[1]) : 0;
            const end = match?.[2] ? Math.min(Number(match[2]), media.size - 1) : media.size - 1;
            if ((range && !match) || start < 0 || start > end)
                return new Response(null, { status: 416 });
            const headers = new Headers({ "Content-Type": "video/mp4", "Accept-Ranges": "bytes",
                "Content-Length": String(end - start + 1) });
            if (match)
                headers.set("Content-Range", `bytes ${start}-${end}/${media.size}`);
            return new Response(media.slice(start, end + 1), { status: match ? 206 : 200, headers });
        },
    });
    let application: ReturnType<typeof Bun.spawn> | undefined;
    let completed = false;
    let qplayPid: number | undefined;
    let driverOwned = false;
    let applicationTimer: ReturnType<typeof setTimeout> | undefined;
    let applicationTermination: Promise<void> | undefined;
    let applicationTerminationError: unknown;
    try {
        driverOwned = true;
        await copyFile(stagedDriver, driver);
        await mkdir(join(profile, "qbutt/config"), { recursive: true });
        await mkdir(downloads, { recursive: true });
        await writeFile(join(profile, "qbutt/config/qbutt.ini"), [
            "[LegalNotice]", "Accepted=true", "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false",
            "Session\\PeXEnabled=false", "Session\\Port=0", "Session\\InterfaceAddress=127.0.0.1",
            `Session\\DefaultSavePath=${downloads.replaceAll("\\", "/")}`,
            "[Network]", "PortForwardingEnabled=false", "[Core]", "AutoOpenTorrentFiles=false",
            "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false", ...labAppearanceSettings(), "",
        ].join("\n"));
        await writeFile(specPath, JSON.stringify({ mode: "native-watch", source: "aniliberty", query: "Native Save legal fixture",
            workId: "aniliberty:900001", episode: "1", variantSuffix: ":episode:save-episode-1:720", contentKind: "anime",
            savePipeFile: pipeFile, saveSha256: mediaHash, saveFaultRetry: true, evidencePath, profile }, null, 4));
        application = Bun.spawn([driver, `--profile=${profile}`], {
            cwd: runtime, windowsHide: true, stdin: "ignore",
            env: { ...process.env, QT_QPA_PLATFORM: "windows", QBUTT_QT_ACCEPTANCE_NATIVE: "1",
                QT_FORCE_STDERR_LOGGING: "1", QBUTT_QT_ACCEPTANCE_SPEC: specPath,
                QBUTT_LAB_PATHS: "1", QBUTT_LAB_QPLAY: qplay, QBUTT_LAB_QPLAY_PROFILE: join(output, "qplay-profile") },
            stdout: Bun.file(join(output, "application.stdout.log")), stderr: Bun.file(join(output, "application.stderr.log")),
        });
        applicationTimer = setTimeout(() => {
            applicationTermination = stopOwnedTree(application!).catch(error => { applicationTerminationError = error; });
        }, 150_000);
        const deadline = Date.now() + 60_000;
        let pipe = "";
        while (!pipe && Date.now() < deadline && application.exitCode === null) {
            const command = `$p=Get-CimInstance Win32_Process -Filter "ParentProcessId = ${application.pid}" | Where-Object { $_.CommandLine -like '*--input-ipc-server=*' } | Select-Object -First 1; if($p -and $p.CommandLine -match 'qbutt-mpv-[0-9a-f-]+'){ @{pipe=$Matches[0];pid=$p.ProcessId}|ConvertTo-Json -Compress }`;
            const probe = Bun.spawn(["pwsh", "-NoProfile", "-Command", command], { windowsHide: true, stdout: "pipe", stderr: "pipe" });
            const [code, stdout] = await Promise.all([probe.exited, new Response(probe.stdout).text()]);
            assert.equal(code, 0, "Cannot inspect the isolated qplay child command line");
            if (stdout.trim()) {
                const child = JSON.parse(stdout);
                assert(Number.isInteger(child.pid) && child.pid > 0, "Invalid isolated qplay PID");
                qplayPid = child.pid;
                pipe = child.pipe;
            }
            if (!pipe)
                await Bun.sleep(200);
        }
        if (!pipe && application.exitCode !== null && await Bun.file(evidencePath).exists()) {
            const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
            assert.equal(application.exitCode, 0, JSON.stringify(evidence));
        }
        assert(/^qbutt-mpv-[0-9a-f-]+$/.test(pipe), "Actual isolated qplay IPC endpoint was not found");
        failNextResolution = true;
        await writeFile(pipeFile, pipe);
        const exitCode = await application.exited;
        clearTimeout(applicationTimer);
        await applicationTermination;
        if (applicationTerminationError)
            throw applicationTerminationError;
        const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
        assert.equal(exitCode, 0, JSON.stringify(evidence));
        assert.equal(evidence.status, "passed", JSON.stringify(evidence));
        assert.equal(evidence.save.jobs, 1);
        assert.equal(evidence.save.requests, 2);
        assert.equal(evidence.save.sha256, mediaHash);
        assert.equal(evidence.save.bytes, media.size);
        assert(evidence.save.queuedAckReplay && injectedFailure && payloadReads > 0);
        const libraryPath = resolve(evidence.checks[0].libraryPath, "content-library.sqlite");
        assert(libraryPath.startsWith(profile + "\\"), "Fixture database escaped its isolated profile");
        const database = new Database(libraryPath, { readonly: true });
        let persisted;
        try {
            const rows = database.query("SELECT snapshot FROM download_jobs").all() as { snapshot: string }[];
            assert.equal(rows.length, 1, "Durable queue contains a duplicate job");
            persisted = JSON.parse(rows[0].snapshot);
            assert.equal(persisted.origin.reference.publication.id, "900001");
            assert.equal(persisted.origin.reference.selector.id, "save-episode-1", "Save used the replacement catalogue selection");
        } finally {
            database.close();
        }
        await writeFile(join(output, "receipt.json"), JSON.stringify({ status: "passed", evidence, searchReads, publicationReads,
            payloadReads, injectedFailure, persisted, mediaHash, driverSha256: sha256(await readFile(driver)),
            qplaySha256: sha256(await readFile(qplay)), prepared }, null, 4) + "\n");
        completed = true;
    } catch (error) {
        await writeFile(join(output, "failure.json"), JSON.stringify({ status: "failed", error: String(error),
            searchReads, publicationReads, payloadReads, injectedFailure }, null, 4));
        throw error;
    } finally {
        clearTimeout(applicationTimer);
        const cleanupErrors: unknown[] = [];
        if (application) {
            try {
                await applicationTermination;
                if (applicationTerminationError)
                    throw applicationTerminationError;
                await stopOwnedTree(application);
            } catch (error) {
                cleanupErrors.push(error);
            }
        }
        if (qplayPid) {
            const expectedPath = qplay.replaceAll("'", "''");
            try {
                await run(["pwsh", "-NoProfile", "-Command",
                    `$p=Get-CimInstance Win32_Process -Filter "ProcessId = ${qplayPid}"; if($p){ if($p.ExecutablePath -ne '${expectedPath}' -or $p.ParentProcessId -ne ${application?.pid}){ throw 'Owned qplay identity changed' }; Stop-Process -Id $p.ProcessId; Wait-Process -Id $p.ProcessId -Timeout 10 -ErrorAction SilentlyContinue }`], 15_000);
            } catch (error) {
                cleanupErrors.push(error);
            }
        }
        server.stop(true);
        if (driverOwned) {
            try {
                await unlink(driver);
            } catch (error) {
                cleanupErrors.push(error);
            }
        }
        await writeFile(join(output, "cleanup.json"), JSON.stringify({ completed,
            cleanupErrors: cleanupErrors.map(String) }, null, 4));
        if (cleanupErrors.length)
            throw new AggregateError(cleanupErrors, "Native Save fixture cleanup needs review");
    }
    console.log(JSON.stringify({ status: "passed", receipt: join(output, "receipt.json") }));
}

if (phase === "prepare")
    await prepare(resolve(arguments_[0]));
else
    await runPrepared(resolve(arguments_[0]), resolve(arguments_[1]), resolve(arguments_[2]));
