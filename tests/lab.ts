import assert from "node:assert/strict";
import { pbkdf2Sync, randomBytes, randomInt } from "node:crypto";
import { createSocket } from "node:dgram";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { generateFixtures, sha256, type FixtureManifest, type PayloadFile } from "./fixtures/generate";
import { allowLabNetwork } from "./windows-firewall";
import { labAppearanceSettings } from "./appearance";

export interface TorrentStatus {
    hash: string;
    state: string;
    progress: number;
    completed: number;
}

export interface TorrentFile {
    index: number;
    name: string;
    progress: number;
    priority: number;
}

async function withTimeout<T>(operation: Promise<T>, timeoutMs: number, message: string): Promise<T> {
    let timer: ReturnType<typeof setTimeout>;
    try {
        return await Promise.race([operation, new Promise<never>((_, reject) => {
            timer = setTimeout(() => reject(new Error(message)), timeoutMs);
        })]);
    }
    finally {
        clearTimeout(timer!);
    }
}

export async function waitFor<T>(label: string, read: () => Promise<T>, accepts: (value: T) => boolean, timeoutMs = 45000): Promise<T> {
    const deadline = Date.now() + timeoutMs;
    let last: T;
    do {
        last = await read();
        if (accepts(last))
            return last;
        await Bun.sleep(150);
    } while (Date.now() < deadline);
    throw new Error(`${label} timed out; last observation: ${JSON.stringify(last!)}`);
}

async function freePort(): Promise<number> {
    const server = createServer();
    await new Promise<void>((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", resolve);
    });
    const address = server.address();
    assert(address && typeof address !== "string", "No local port assigned");
    await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    return address.port;
}

async function freePeerPort(webPort: number): Promise<number> {
    for (let attempt = 0; attempt < 32; ++attempt) {
        // Port 0 may repeatedly pick the other protocol's reserved range.
        const port = randomInt(40000, 49152);
        if (port === webPort)
            continue;
        const udp = createSocket("udp4");
        const tcp = createServer();
        try {
            await new Promise<void>((resolve, reject) => {
                udp.once("error", reject);
                udp.bind(port, "127.0.0.1", resolve);
            });
            await new Promise<void>((resolve, reject) => {
                tcp.once("error", reject);
                tcp.listen(port, "127.0.0.1", resolve);
            });
            return port;
        }
        catch (error) {
            if (!["EADDRINUSE", "EACCES"].includes((error as NodeJS.ErrnoException).code ?? ""))
                throw error;
        }
        finally {
            await new Promise<void>(resolve => udp.close(resolve));
            if (tcp.listening)
                await new Promise<void>((resolve, reject) => tcp.close(error => error ? reject(error) : resolve()));
        }
    }
    throw new Error("No local peer port available for both TCP and UDP");
}

export async function createLab(name: string, options: { pex?: boolean; protocol?: "TCP" | "UTP" } = {}) {
    const executable = process.env.QBUTT_LAB_EXE;
    const python = process.env.QBUTT_LAB_PYTHON;
    const appName = process.env.QBUTT_LAB_APP_NAME ?? "qbutt";
    const resumeBackend = process.env.QBUTT_LAB_RESUME_BACKEND ?? "Legacy";
    assert(executable && python, "Set QBUTT_LAB_EXE and QBUTT_LAB_PYTHON");
    assert(appName === "qbutt" || appName === "qBittorrent", "QBUTT_LAB_APP_NAME must be qbutt or qBittorrent");
    assert(resumeBackend === "Legacy" || resumeBackend === "SQLite", "QBUTT_LAB_RESUME_BACKEND must be Legacy or SQLite");
    assert((await stat(executable)).isFile(), "Native executable is missing");
    const networkChild = join(dirname(executable), "qbutt-net.exe");
    await allowLabNetwork([process.execPath, executable, python, ...(existsSync(networkChild) ? [networkChild] : [])]);
    const root = await mkdtemp(join(tmpdir(), `qbutt-${name}-`));
    const fixtures = await generateFixtures(python, join(root, "fixtures"));
    const manifest = JSON.parse(await readFile(join(fixtures, "manifest.json"), "utf8")) as FixtureManifest;
    const port = await freePort();
    const peerPort = await freePeerPort(port);
    const profile = join(root, "profile");
    const config = join(profile, appName, "config");
    await mkdir(config, { recursive: true });
    const password = randomBytes(32).toString("hex");
    const salt = randomBytes(16);
    const passwordHash = `${salt.toString("base64")}:${pbkdf2Sync(password, salt, 100000, 64, "sha512").toString("base64")}`;
    await writeFile(join(config, `${appName}.ini`), [
        "[BitTorrent]",
        `Session\\ResumeDataStorageType=${resumeBackend}`,
        "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", `Session\\PeXEnabled=${options.pex === true}`,
        `Session\\BTProtocol=${options.protocol ?? "TCP"}`, "Session\\InterfaceAddress=127.0.0.1", `Session\\Port=${peerPort}`,
        "Session\\IgnoreLimitsOnLAN=false", "Session\\AddExtensionToIncompleteFiles=false",
        "Session\\UseUnwantedFolder=false", "Session\\QueueingSystemEnabled=false",
        "[Network]", "PortForwardingEnabled=false",
        "[Preferences]", "General\\ExitConfirm=false", "General\\Locale=en",
        "Advanced\\updateCheck=false", "Connection\\ResolvePeerCountries=false", "Connection\\ResolvePeerHostNames=false",
        "General\\CloseToTray=false", "General\\MinimizeToTray=false",
        "WebUI\\Enabled=true", "WebUI\\Address=127.0.0.1", `WebUI\\Port=${port}`,
        "WebUI\\Username=lab", `WebUI\\Password_PBKDF2=@ByteArray(${passwordHash})`,
        "WebUI\\LocalHostAuth=true", "WebUI\\UseUPnP=false",
        "WebUI\\ServerDomains=127.0.0.1", "WebUI\\HostHeaderValidation=true", "WebUI\\CSRFProtection=true",
        ...labAppearanceSettings(),
        "",
    ].join("\n"));
    const origin = `http://127.0.0.1:${port}`;
    let processHandle: ReturnType<typeof Bun.spawn> | undefined;
    let launch = 0;
    let cookie = "";
    const evidence: Record<string, unknown> = {
        schema: 1, suite: name, status: "running", executable: resolve(executable),
        executableSha256: sha256(await readFile(executable)), generator: manifest.generator,
        profile, resumeBackend, startedAt: new Date().toISOString(), checks: [],
    };

    async function request(path: string, body?: Record<string, string> | FormData): Promise<Response> {
        const response = await fetch(`${origin}/api/v2/${path}`, {
            method: body ? "POST" : "GET",
            headers: { Origin: origin, Referer: `${origin}/`, Cookie: cookie },
            body: body instanceof FormData ? body : body ? new URLSearchParams(body) : undefined,
            signal: AbortSignal.timeout(5000),
        });
        if (!response.ok)
            throw new Error(`WebUI ${path.split("?")[0]} returned HTTP ${response.status}`);
        return response;
    }
    async function json<T>(path: string): Promise<T> {
        return (await request(path)).json() as Promise<T>;
    }
    async function start() {
        assert(!processHandle, "Lab process is already running");
        ++launch;
        processHandle = Bun.spawn([executable!, `--profile=${profile}`, `--webui-port=${port}`, "--no-splash", "--confirm-legal-notice"], {
            env: { ...process.env, QT_QPA_PLATFORM: "offscreen" },
            stdout: Bun.file(join(root, `app-${launch}.stdout.log`)),
            stderr: Bun.file(join(root, `app-${launch}.stderr.log`)),
        });
        await waitFor("WebUI readiness", async () => {
            assert(processHandle!.exitCode === null, `Native app exited ${processHandle!.exitCode}; inspect ${root}`);
            try {
                const response = await request("auth/login", { username: "lab", password });
                await response.text();
                cookie = response.headers.get("set-cookie")?.split(";")[0] ?? "";
                return cookie ? "ready" : "";
            }
            catch { return ""; }
        }, version => version.length > 0, 30000);
        evidence.appVersion = await (await request("app/version")).text();
        evidence.buildInfo = await json("app/buildInfo");
        const preferences = await json<Record<string, unknown>>("app/preferences");
        assert(preferences.resume_data_storage_type === resumeBackend, "Native resume backend differs from requested fixture");
        assert(preferences.dht === false && preferences.lsd === false && preferences.pex === (options.pex === true),
            "Isolated lab discovery settings differ from its pre-launch profile");
    }
    async function shutdown() {
        if (!processHandle)
            return;
        const child = processHandle;
        try {
            if (child.exitCode === null)
                await request("app/shutdown", {});
            const exitCode = await withTimeout(child.exited, 20000, "Native app clean shutdown timed out");
            assert(exitCode === 0, `Native app exited ${exitCode}`);
        }
        finally {
            if (child.exitCode === null) {
                child.kill();
                await child.exited;
            }
            processHandle = undefined;
        }
    }
    async function markCompletionPreview(torrentID: string) {
        assert(!processHandle, "Resume data must only be changed while the native app is stopped");
        const markerScript = `
import libtorrent as lt
import pathlib
import sqlite3
import sys

backend, data_path, torrent_id = sys.argv[1:]
data_path = pathlib.Path(data_path)
if backend == "SQLite":
    connection = sqlite3.connect(data_path / "torrents.db")
    try:
        row = connection.execute(
            "SELECT libtorrent_resume_data FROM torrents WHERE torrent_id = ?",
            (torrent_id,),
        ).fetchone()
        assert row is not None, f"Missing SQLite resume record for {torrent_id}"
        resume = lt.bdecode(row[0])
        resume[b"qbutt-completion-policy-preview"] = 1
        cursor = connection.execute(
            "UPDATE torrents SET libtorrent_resume_data = ? WHERE torrent_id = ?",
            (lt.bencode(resume), torrent_id),
        )
        assert cursor.rowcount == 1
        connection.commit()
    finally:
        connection.close()
else:
    path = data_path / "BT_backup" / f"{torrent_id}.fastresume"
    resume = lt.bdecode(path.read_bytes())
    resume[b"qbutt-completion-policy-preview"] = 1
    path.write_bytes(lt.bencode(resume))
`;
        const marker = Bun.spawn([python!, "-c", markerScript, resumeBackend, join(profile, appName, "data"), torrentID], {
            stdout: "pipe", stderr: "pipe",
        });
        assert(await marker.exited === 0, await new Response(marker.stderr).text());
    }
    async function info(hash: string): Promise<TorrentStatus> {
        const torrents = await json<TorrentStatus[]>(`torrents/info?hashes=${hash}`);
        assert(torrents.length === 1, `Expected one torrent for ${hash}`);
        return torrents[0]!;
    }
    async function add(torrentName: string, destination: string): Promise<string> {
        await mkdir(destination, { recursive: true });
        const torrent = manifest.torrents.find(item => item.name === torrentName);
        assert(torrent, `Unknown fixture ${torrentName}`);
        const data = new FormData();
        data.set("torrents", Bun.file(join(fixtures, torrent.file)));
        data.set("savepath", destination);
        data.set("stopped", "true");
        data.set("autoTMM", "false");
        data.set("contentLayout", "Original");
        await request("torrents/add", data);
        // qBittorrent uses libtorrent get_best(): truncated v2 for hybrid too.
        const hash = torrent.infoHashV2?.slice(0, 40) ?? torrent.infoHashV1!;
        await waitFor("torrent add", () => json<TorrentStatus[]>(`torrents/info?hashes=${hash}`), items => items.length === 1);
        return hash;
    }
    async function checkpoint(check: Record<string, unknown>) {
        (evidence.checks as unknown[]).push(check);
        await writeFile(join(root, "evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
        console.log(JSON.stringify({ suite: name, ...check }));
    }
    async function finish(error?: unknown) {
        evidence.status = error ? "failed" : "passed";
        evidence.finishedAt = new Date().toISOString();
        if (error)
            evidence.error = error instanceof Error ? error.message : String(error);
        await writeFile(join(root, "evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
        console.log(JSON.stringify({ status: evidence.status, evidence: join(root, "evidence.json") }));
    }
    return { root, fixtures, manifest, python, origin, request, json, info, add, start, shutdown, markCompletionPreview, checkpoint, finish,
        get pid() { return processHandle?.pid; },
        get exitCode() { return processHandle?.exitCode; } };
}

export async function verifyPayload(root: string, expected: PayloadFile[]): Promise<number> {
    let verified = 0;
    for (const file of expected) {
        const bytes = await readFile(join(root, file.path));
        assert(bytes.length === file.size, `${file.path}: expected exact size ${file.size}, got ${bytes.length}`);
        assert(sha256(bytes) === file.sha256, `${file.path}: SHA-256 mismatch`);
        verified += bytes.length;
    }
    return verified;
}

export async function startSeed(python: string, fixtures: string, name: string, logs: string,
    options: { savePath?: string; pieces?: number[]; label?: string; listenAddress?: string; uploadRate?: number;
        neighbor?: { host: string; port: number }; transport?: "tcp" | "utp"; allowedPeerAddresses?: string[] } = {}) {
    const label = options.label ?? name;
    const child = Bun.spawn([python, join(import.meta.dir, "network-lab", "seed.py"), join(fixtures, `${name}.torrent`),
        options.savePath ?? join(fixtures, "seed"), JSON.stringify(options.pieces ?? null),
        options.listenAddress ?? "127.0.0.1", String(options.uploadRate ?? 256 * 1024),
        JSON.stringify(options.neighbor ?? null), options.transport ?? "tcp", JSON.stringify(options.allowedPeerAddresses ?? [])], {
        stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(logs, `seed-${label}.stderr.log`)),
    });
    const reader = child.stdout.getReader();
    let text = "";
    let unreadOffset = 0;
    const readLine = async (timeoutMs: number, message: string) => {
        for (;;) {
            const newline = text.indexOf("\n", unreadOffset);
            if (newline >= 0) {
                const line = text.slice(unreadOffset, newline);
                unreadOffset = newline + 1;
                return line;
            }
            const result = await withTimeout(reader.read(), timeoutMs, message);
            assert(!result.done, `Seed ${label} ended before sending a complete response`);
            text += new TextDecoder().decode(result.value);
        }
    };
    try {
        const ready = JSON.parse(await readLine(35000, "Seed readiness timeout")) as {
            ready: boolean; host: string; port: number; pieces: number[]; verifiedPayloadBytes: number;
        };
        assert(ready.ready && ready.port > 0, "Seed failed readiness");
        let controlId = 0;
        let control = Promise.resolve();
        let stopPromise: Promise<{ uploadPayloadBytes: number; downloadPayloadBytes: number;
            pieces: number[]; peerAddresses: string[]; outgoingPeerAddresses: string[] }> | undefined;
        return {
            ...ready,
            setUploadRate(bytesPerSecond: number) {
                assert(Number.isInteger(bytesPerSecond) && bytesPerSecond >= 1024 && bytesPerSecond <= 1024 * 1024,
                    "Seed upload rate must be between 1 KiB/s and 1 MiB/s");
                const requestId = ++controlId;
                control = control.then(async () => {
                    child.stdin.write(`${JSON.stringify({ controlId: requestId, uploadRate: bytesPerSecond })}\n`);
                    await child.stdin.flush();
                    const response = JSON.parse(await readLine(5000, "Seed upload-rate acknowledgement timed out")) as {
                        controlId: number; uploadRate: number;
                    };
                    assert(response.controlId === requestId && response.uploadRate === bytesPerSecond,
                        "Seed acknowledged a different upload-rate command");
                });
                return control;
            },
            stop() {
                stopPromise ??= (async () => {
                    try { await control; }
                    catch (error) {
                        if (child.exitCode === null)
                            child.kill();
                        await child.exited;
                        await reader.closed.catch(() => {});
                        try { reader.releaseLock(); } catch {}
                        throw error;
                    }
                    child.stdin.end();
                    let exitCode: number;
                    try {
                        exitCode = await withTimeout(child.exited, 15000, "Seed shutdown timed out");
                    }
                    finally {
                        if (child.exitCode === null) {
                            child.kill();
                            await child.exited;
                        }
                    }
                    assert(exitCode === 0, `Seed exited ${exitCode}`);
                    for (;;) {
                        const final = await reader.read();
                        if (final.done)
                            break;
                        text += new TextDecoder().decode(final.value);
                    }
                    await writeFile(join(logs, `seed-${label}.jsonl`), text);
                    reader.releaseLock();
                    return JSON.parse(text.trimEnd().split("\n").at(-1)!) as {
                        uploadPayloadBytes: number; downloadPayloadBytes: number; pieces: number[]; peerAddresses: string[];
                        outgoingPeerAddresses: string[];
                    };
                })();
                return stopPromise;
            },
        };
    }
    catch (error) {
        child.kill();
        await child.exited;
        reader.releaseLock();
        throw error;
    }
}
