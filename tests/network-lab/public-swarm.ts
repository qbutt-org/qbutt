import assert from "node:assert/strict";
import { createHash, pbkdf2Sync, randomBytes } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, open, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { createSocket } from "node:dgram";
import { createServer } from "node:net";
import { networkInterfaces, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { allowLabNetwork } from "../windows-firewall";
import { labAppearanceSettings } from "../appearance";

type Mode = "upstream-native" | "qbutt-native" | "qbutt-one-tunnel" | "qbutt-mixed";

interface TorrentInfo {
    hash: string;
    state: string;
    downloaded_session: number;
    dlspeed: number;
    num_complete: number;
    num_incomplete: number;
    num_leechs: number;
    num_seeds: number;
}

interface TransferInfo {
    connection_status: string;
    dht_nodes: number;
    dl_info_data: number;
}

interface PathsStatus {
    busy: boolean;
    paths: { open: boolean }[];
    diagnostics: { routes: PathCounters[] };
}

interface PathCounters {
    pathId: string;
    generation: number;
    payloadDownload: number;
    verifiedDownload: number;
}

interface RunResult {
    mode: Mode;
    round: number;
    ordinal: number;
    attempt: number;
    peerPort: number;
    executableSha256: string;
    appVersion: string;
    connectionSetupMilliseconds: number;
    measurementMilliseconds: number;
    verifiedBytesBeforeWindow: number;
    verifiedBytesAfterWindow: number;
    measuredVerifiedBytes: number;
    measuredVerifiedBytesPerSecond: number;
    verifiedPiecesAfterWindow: number;
    clientTransferDataDelta: number;
    torrentDownloadedSessionDelta: number;
    uiProbeMilliseconds: { count: number; median: number; p95: number; maximum: number };
    publicPeers: { connectedAtWarmup: number; seedsReported: number; leechesReported: number; dhtNodes: number };
    pathMeasurements?: (PathCounters & { verifiedBytesPerSecond: number })[];
    evidenceRoot: string;
}

const SOURCE_URL = "https://releases.ubuntu.com/24.04.5/ubuntu-24.04.5-live-server-amd64.iso.torrent";
const CHECKSUM_URL = "https://releases.ubuntu.com/24.04.5/SHA256SUMS";
const SOURCE_SHA256 = "81d2234a304fbed595c78df9ec243ae7695263f591f3dabedcb808e5cd7615c8";
const INFO_HASH = "a3b4136a174e30b121a6bb6ab75b577ea7eda5f3";
const PAYLOAD_NAME = "ubuntu-24.04.5-live-server-amd64.iso";
const PAYLOAD_BYTES = 4080486400;
const PAYLOAD_SHA256 = "97f3d7ffb032c3eb3b23d2c8be9cc76e60c2c1f2c0146ba5ba9fe01cafae0fd8";
const PIECE_LENGTH = 262144;
const PIECE_COUNT = 15566;
const CONTROL_SHA256 = "70322489c36a613eec5788688355fca26268a520d74e3f41ebb2d90c1c8beb0f";
const CONTROL_REVISION = "0b63c3d17373f6132ea211c9dcd4241284ccdfaf";

const qbuttOnly = process.argv.includes("--qbutt-only");
const controlExecutable = resolve(process.env.QBUTT_PUBLIC_SWARM_CONTROL_EXE ?? "");
const qbuttExecutable = process.env.QBUTT_PUBLIC_SWARM_QBUTT_EXE
    ? resolve(process.env.QBUTT_PUBLIC_SWARM_QBUTT_EXE) : undefined;
const proxyConfig = process.env.QBUTT_PUBLIC_SWARM_PROXY_CONFIG
    ? resolve(process.env.QBUTT_PUBLIC_SWARM_PROXY_CONFIG) : undefined;
const proxyNames = (process.env.QBUTT_PUBLIC_SWARM_PROXY_NAMES ?? "")
    .split("|").map(value => value.trim()).filter(Boolean);
const nativeInterface = process.env.QBUTT_PUBLIC_SWARM_NATIVE_INTERFACE ?? "";
const nativeAddress = process.env.QBUTT_PUBLIC_SWARM_NATIVE_ADDRESS ?? "";
const rounds = Number(process.env.QBUTT_PUBLIC_SWARM_ROUNDS ?? (qbuttExecutable ? 4 : 3));
const warmupRate = Number(process.env.QBUTT_PUBLIC_SWARM_WARMUP_RATE ?? 128 * 1024);
const measuredRate = Number(process.env.QBUTT_PUBLIC_SWARM_RATE ?? 4 * 1024 * 1024);
const measurementMilliseconds = Number(process.env.QBUTT_PUBLIC_SWARM_WINDOW_MS ?? 15000);
const minimumWarmupPieces = Number(process.env.QBUTT_PUBLIC_SWARM_WARMUP_PIECES ?? 2);
const peerPort = Number(process.env.QBUTT_PUBLIC_SWARM_PEER_PORT ?? 45123);
const attemptsPerWindow = Number(process.env.QBUTT_PUBLIC_SWARM_ATTEMPTS ?? 3);

assert(qbuttOnly ? qbuttExecutable : process.env.QBUTT_PUBLIC_SWARM_CONTROL_EXE,
    "Set QBUTT_PUBLIC_SWARM_CONTROL_EXE, or use --qbutt-only with QBUTT_PUBLIC_SWARM_QBUTT_EXE");
assert(nativeInterface && nativeAddress,
    "Set QBUTT_PUBLIC_SWARM_NATIVE_INTERFACE and QBUTT_PUBLIC_SWARM_NATIVE_ADDRESS");
assert(networkInterfaces()[nativeInterface]?.some(address => address.family === "IPv4"
    && address.address === nativeAddress && !address.internal),
"The public-swarm Native address must belong to its selected non-loopback interface");
assert(Number.isInteger(rounds) && rounds >= 3 && rounds <= 8, "QBUTT_PUBLIC_SWARM_ROUNDS must be 3..8");
assert(Number.isInteger(warmupRate) && warmupRate >= 64 * 1024 && warmupRate <= 512 * 1024,
    "QBUTT_PUBLIC_SWARM_WARMUP_RATE must be 64..512 KiB/s");
assert(Number.isInteger(measuredRate) && measuredRate >= 512 * 1024 && measuredRate <= 8 * 1024 * 1024,
    "QBUTT_PUBLIC_SWARM_RATE must be 512 KiB/s..8 MiB/s");
assert(Number.isInteger(measurementMilliseconds)
    && measurementMilliseconds >= 10000 && measurementMilliseconds <= 60000,
    "QBUTT_PUBLIC_SWARM_WINDOW_MS must be 10000..60000");
assert(Number.isInteger(minimumWarmupPieces) && minimumWarmupPieces >= 1 && minimumWarmupPieces <= 8,
    "QBUTT_PUBLIC_SWARM_WARMUP_PIECES must be 1..8");
assert(Number.isInteger(peerPort) && peerPort >= 1024 && peerPort <= 49151,
    "QBUTT_PUBLIC_SWARM_PEER_PORT must be an unprivileged non-ephemeral port");
assert(Number.isInteger(attemptsPerWindow) && attemptsPerWindow >= 1 && attemptsPerWindow <= 3,
    "QBUTT_PUBLIC_SWARM_ATTEMPTS must be 1..3");
assert(!proxyConfig || qbuttExecutable, "A path config requires QBUTT_PUBLIC_SWARM_QBUTT_EXE");
assert(!proxyConfig || proxyNames.length > 0, "Path modes require QBUTT_PUBLIC_SWARM_PROXY_NAMES");
assert(proxyConfig || proxyNames.length === 0, "Proxy names require QBUTT_PUBLIC_SWARM_PROXY_CONFIG");

function sha256(bytes: Uint8Array): string {
    return createHash("sha256").update(bytes).digest("hex");
}

function quantile(values: number[], fraction: number): number {
    assert(values.length > 0, "Cannot summarize an empty sample");
    return [...values].sort((left, right) => left - right)[Math.ceil(values.length * fraction) - 1]!;
}

function median(values: number[]): number {
    const sorted = [...values].sort((left, right) => left - right);
    const middle = Math.floor(sorted.length / 2);
    return sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}

function summarizeLatencies(values: number[]) {
    return { count: values.length, median: quantile(values, 0.5),
        p95: quantile(values, 0.95), maximum: Math.max(...values) };
}

function pieceBytes(index: number): number {
    assert(index >= 0 && index < PIECE_COUNT, `Invalid piece index ${index}`);
    return index === PIECE_COUNT - 1 ? PAYLOAD_BYTES - (PIECE_COUNT - 1) * PIECE_LENGTH : PIECE_LENGTH;
}

function verifiedBytes(states: number[]): number {
    assert(states.length === PIECE_COUNT, `Expected ${PIECE_COUNT} piece states, got ${states.length}`);
    return states.reduce((sum, state, index) => sum + (state === 2 ? pieceBytes(index) : 0), 0);
}

function orderForRound(modes: Mode[], round: number): Mode[] {
    const offset = (round - 1) % modes.length;
    return [...modes.slice(offset), ...modes.slice(0, offset)];
}

async function freePort(): Promise<number> {
    const server = createServer();
    await new Promise<void>((accept, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", accept);
    });
    const address = server.address();
    assert(address && typeof address !== "string", "No local port assigned");
    await new Promise<void>((accept, reject) => server.close(error => error ? reject(error) : accept()));
    return address.port;
}

async function assertPeerPortAvailable(port: number): Promise<void> {
    const tcp = createServer();
    const udp = createSocket("udp4");
    let tcpBound = false;
    let udpBound = false;
    try {
        await new Promise<void>((accept, reject) => {
            tcp.once("error", reject);
            tcp.listen(port, "0.0.0.0", accept);
        });
        tcpBound = true;
        await new Promise<void>((accept, reject) => {
            udp.once("error", reject);
            udp.bind(port, "0.0.0.0", accept);
        });
        udpBound = true;
    }
    catch (error) {
        throw new Error(`Stable peer port ${port} is unavailable for TCP and UDP`, { cause: error });
    }
    finally {
        if (udpBound) await new Promise<void>(accept => udp.close(() => accept()));
        if (tcpBound) await new Promise<void>(accept => tcp.close(() => accept()));
    }
}

async function waitFor<T>(label: string, read: () => Promise<T>, accepts: (value: T) => boolean,
    timeoutMilliseconds: number): Promise<T> {
    const deadline = Date.now() + timeoutMilliseconds;
    let last!: T;
    do {
        last = await read();
        if (accepts(last)) return last;
        await Bun.sleep(500);
    } while (Date.now() < deadline);
    throw new Error(`${label} timed out; last observation: ${JSON.stringify(last)}`);
}

async function verifyCompletedPieces(payloadPath: string, states: number[], hashes: string[]): Promise<number> {
    assert(hashes.length === PIECE_COUNT, `Expected ${PIECE_COUNT} piece hashes, got ${hashes.length}`);
    const handle = await open(payloadPath, "r");
    let bytes = 0;
    try {
        for (let index = 0; index < states.length; ++index) {
            if (states[index] !== 2) continue;
            const length = pieceBytes(index);
            const buffer = Buffer.allocUnsafe(length);
            let bytesRead = 0;
            while (bytesRead < length) {
                const read = await handle.read(buffer, bytesRead, length - bytesRead, index * PIECE_LENGTH + bytesRead);
                assert(read.bytesRead > 0, `Piece ${index}: file ended after ${bytesRead} of ${length} bytes`);
                bytesRead += read.bytesRead;
            }
            assert(createHash("sha1").update(buffer).digest("hex") === hashes[index], `Piece ${index}: SHA-1 mismatch`);
            bytes += length;
        }
    }
    finally { await handle.close(); }
    return bytes;
}

async function run(mode: Mode, round: number, ordinal: number, attempt: number, torrentPath: string, reportRoot: string,
    peerPort: number, executableHashes: Map<string, string>): Promise<RunResult> {
    const executable = mode === "upstream-native" ? controlExecutable : qbuttExecutable!;
    const appName = mode === "upstream-native" ? "qBittorrent" : "qbutt";
    const root = join(reportRoot, `run-${round}-${ordinal}-${mode}-attempt-${attempt}`);
    const profile = join(root, "profile");
    const config = join(profile, appName, "config");
    const destination = join(root, "payload");
    const webPort = await freePort();
    const password = randomBytes(32).toString("hex");
    const salt = randomBytes(16);
    const passwordHash = `${salt.toString("base64")}:`
        + pbkdf2Sync(password, salt, 100000, 64, "sha512").toString("base64");
    await mkdir(config, { recursive: true });
    await mkdir(destination, { recursive: true });
    await writeFile(join(config, `${appName}.ini`), [
        "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=true",
        "Session\\BTProtocol=0", "Session\\Interface=", "Session\\InterfaceAddress=", `Session\\Port=${peerPort}`,
        "Session\\QueueingSystemEnabled=false", "Session\\IgnoreLimitsOnLAN=false",
        "Session\\AddExtensionToIncompleteFiles=false", "Session\\UseUnwantedFolder=false",
        "[Network]", "PortForwardingEnabled=false",
        "[Preferences]", "Advanced\\updateCheck=false", "Connection\\ResolvePeerCountries=false",
        "Connection\\ResolvePeerHostNames=false", "General\\ExitConfirm=false", "General\\CloseToTray=false",
        "General\\MinimizeToTray=false", "WebUI\\Enabled=true", "WebUI\\Address=127.0.0.1", `WebUI\\Port=${webPort}`,
        "WebUI\\Username=lab", `WebUI\\Password_PBKDF2=@ByteArray(${passwordHash})`, "WebUI\\LocalHostAuth=true",
        "WebUI\\UseUPnP=false", "WebUI\\ServerDomains=127.0.0.1", "WebUI\\HostHeaderValidation=true",
        "WebUI\\CSRFProtection=true", ...labAppearanceSettings(), "",
    ].join("\n"));

    const origin = `http://127.0.0.1:${webPort}`;
    let cookie = "";
    const child = Bun.spawn([executable, `--profile=${profile}`, `--webui-port=${webPort}`, "--no-splash",
        "--confirm-legal-notice"], {
        env: { ...process.env, QT_QPA_PLATFORM: "offscreen" },
        stdout: Bun.file(join(root, "app.stdout.log")),
        stderr: Bun.file(join(root, "app.stderr.log")), windowsHide: true,
    });
    let clean = false;
    async function request(path: string, body?: Record<string, string> | FormData): Promise<Response> {
        const response = await fetch(`${origin}/api/v2/${path}`, {
            method: body ? "POST" : "GET", headers: { Origin: origin, Referer: `${origin}/`, Cookie: cookie },
            body: body instanceof FormData ? body : body ? new URLSearchParams(body) : undefined,
            signal: AbortSignal.timeout(10000),
        });
        if (!response.ok) throw new Error(`WebUI ${path.split("?")[0]} returned HTTP ${response.status}`);
        return response;
    }
    async function json<T>(path: string): Promise<T> { return (await request(path)).json() as Promise<T>; }
    async function info(): Promise<TorrentInfo> {
        const torrents = await json<TorrentInfo[]>(`torrents/info?hashes=${INFO_HASH}`);
        assert(torrents.length === 1, "Pinned public torrent disappeared");
        return torrents[0]!;
    }
    async function shutdown() {
        if (child.exitCode === null) {
            try { await request("app/shutdown", {}); }
            catch {}
        }
        const exit = await Promise.race([child.exited, Bun.sleep(20000).then(() => undefined)]);
        if (exit === undefined && child.exitCode === null) child.kill();
        const finalExit = await child.exited;
        assert(finalExit === 0, `Native app exited ${finalExit}; inspect ${root}`);
    }

    try {
        await waitFor("WebUI readiness", async () => {
            assert(child.exitCode === null, `Native app exited ${child.exitCode}; inspect ${root}`);
            try {
                const response = await request("auth/login", { username: "lab", password });
                await response.text();
                cookie = response.headers.get("set-cookie")?.split(";")[0] ?? "";
                return cookie;
            }
            catch { return ""; }
        }, value => value.length > 0, 30000);
        const appVersion = await (await request("app/version")).text();
        const interfaces = await json<{ name: string; value: string }[]>("app/networkInterfaceList");
        const matchingInterfaces = interfaces
            .filter(item => item.name === nativeInterface || item.value === nativeInterface);
        assert(matchingInterfaces.length === 1,
            `Native interface ${nativeInterface} has no unique qBittorrent mapping`);
        const interfaceValue = matchingInterfaces[0]!.value;
        const interfaceAddresses = await json<string[]>(
            `app/networkInterfaceAddressList?iface=${encodeURIComponent(interfaceValue)}`);
        assert(interfaceAddresses.includes(nativeAddress),
            `Native address ${nativeAddress} is absent from ${nativeInterface}`);
        await request("app/setPreferences", { json: JSON.stringify({ current_network_interface: interfaceValue,
            current_interface_address: nativeAddress, bittorrent_protocol: 0, dht: true }) });
        await waitFor("physical Native binding", () => json<Record<string, unknown>>("app/preferences"), preferences =>
            preferences.current_network_interface === interfaceValue
                && preferences.current_interface_address === nativeAddress
                && preferences.bittorrent_protocol === 0, 30000);

        if (mode === "qbutt-one-tunnel" || mode === "qbutt-mixed") {
            const selectedNames = mode === "qbutt-one-tunnel" ? proxyNames.slice(0, 1) : proxyNames;
            for (const [index, proxyName] of selectedNames.entries()) {
                await request("qbuttPaths/open", { configPath: proxyConfig!, proxyName,
                    edgeId: `public-swarm-${index + 1}`, interfaceName: nativeInterface });
                await waitFor("public path open", () => json<PathsStatus>("qbuttPaths/status"),
                    status => !status.busy && status.paths.filter(path => path.open).length === index + 1, 60000);
            }
            await request("qbuttPaths/policy", mode === "qbutt-mixed"
                ? { mode: "mixed", nativeInterface: interfaceValue } : { mode: "pinned" });
        }

        const data = new FormData();
        data.set("torrents", Bun.file(torrentPath));
        data.set("savepath", destination);
        data.set("stopped", "true");
        data.set("autoTMM", "false");
        data.set("contentLayout", "Original");
        data.set("dlLimit", String(warmupRate));
        await request("torrents/add", data);
        await waitFor("pinned torrent add", () => json<TorrentInfo[]>(`torrents/info?hashes=${INFO_HASH}`),
            torrents => torrents.length === 1 && torrents[0]!.hash === INFO_HASH, 30000);
        const properties = await json<Record<string, number>>("torrents/properties?hash=" + INFO_HASH);
        assert(properties.total_size === PAYLOAD_BYTES && properties.piece_size === PIECE_LENGTH
            && properties.pieces_num === PIECE_COUNT, "Public torrent metadata differs from its pinned identity");
        const hashes = await json<string[]>("torrents/pieceHashes?hash=" + INFO_HASH);
        assert(hashes.length === PIECE_COUNT && hashes.every(hash => /^[0-9a-f]{40}$/.test(hash)),
            "Public torrent returned an invalid piece-hash set");

        const setupStarted = performance.now();
        await request("torrents/start", { hashes: INFO_HASH });
        const warmup = await waitFor("public swarm warmup", async () => {
            try {
                const [states, torrent, transfer] = await Promise.all([
                    json<number[]>("torrents/pieceStates?hash=" + INFO_HASH),
                    info(), json<TransferInfo>("transfer/info"),
                ]);
                return { completePieces: states.filter(state => state === 2).length, state: torrent.state,
                    connectedSeeds: torrent.num_seeds, connectedLeeches: torrent.num_leechs,
                    seedsReported: torrent.num_complete, leechesReported: torrent.num_incomplete,
                    dhtNodes: transfer.dht_nodes, connectionStatus: transfer.connection_status, apiError: "" };
            }
            catch (error) {
                assert(child.exitCode === null, `Native app exited ${child.exitCode}; inspect ${root}`);
                return { completePieces: 0, state: "webui-unavailable", connectedSeeds: 0, connectedLeeches: 0,
                    seedsReported: 0, leechesReported: 0, dhtNodes: 0, connectionStatus: "unknown",
                    apiError: error instanceof Error ? error.message : String(error) };
            }
        }, observation => observation.completePieces >= minimumWarmupPieces
            && (observation.connectedSeeds + observation.connectedLeeches) > 0, 120000);
        const connectionSetupMilliseconds = performance.now() - setupStarted;
        const beforeStates = await json<number[]>("torrents/pieceStates?hash=" + INFO_HASH);
        const verifiedBytesBeforeWindow = verifiedBytes(beforeStates);
        const beforeInfo = await info();
        const beforeTransfer = await json<TransferInfo>("transfer/info");
        const managedPaths = mode === "qbutt-one-tunnel" || mode === "qbutt-mixed";
        const beforePaths = managedPaths ? (await json<PathsStatus>("qbuttPaths/status")).diagnostics.routes : [];
        await request("torrents/setDownloadLimit", { hashes: INFO_HASH, limit: String(measuredRate) });

        const uiLatencies: number[] = [];
        const measurementStarted = performance.now();
        const deadline = performance.now() + measurementMilliseconds;
        while (performance.now() < deadline) {
            const probeStarted = performance.now();
            await info();
            uiLatencies.push(performance.now() - probeStarted);
            await Bun.sleep(Math.min(750, Math.max(0, deadline - performance.now())));
        }
        await request("torrents/stop", { hashes: INFO_HASH });
        const afterInfo = await waitFor("public torrent stop", info,
            torrent => torrent.state.startsWith("stopped") && torrent.dlspeed === 0, 30000);
        let afterStates = await json<number[]>("torrents/pieceStates?hash=" + INFO_HASH);
        await Bun.sleep(750);
        const stableStates = await json<number[]>("torrents/pieceStates?hash=" + INFO_HASH);
        assert(JSON.stringify(stableStates) === JSON.stringify(afterStates),
            "Piece state changed after the stopped boundary");
        afterStates = stableStates;
        const measuredMilliseconds = performance.now() - measurementStarted;
        const afterTransfer = await json<TransferInfo>("transfer/info");
        let pathMeasurements: RunResult["pathMeasurements"];
        if (managedPaths) {
            const status = await json<PathsStatus>("qbuttPaths/status");
            pathMeasurements = status.diagnostics.routes.map(route => {
                const before = beforePaths.find(item => item.pathId === route.pathId && item.generation === route.generation);
                const payloadDownload = route.payloadDownload - (before?.payloadDownload ?? 0);
                const verifiedDownload = route.verifiedDownload - (before?.verifiedDownload ?? 0);
                assert(payloadDownload >= 0 && verifiedDownload >= 0, "Path counters regressed within a generation");
                return { pathId: route.pathId, generation: route.generation, payloadDownload, verifiedDownload,
                    verifiedBytesPerSecond: verifiedDownload / (measuredMilliseconds / 1000) };
            });
            await request("qbuttPaths/stop", {});
            await waitFor("public path stop", () => json<PathsStatus>("qbuttPaths/status"),
                status => !status.busy && status.paths.every(path => !path.open), 30000);
        }
        await shutdown();
        clean = true;

        const verifiedBytesAfterWindow = await verifyCompletedPieces(
            join(destination, PAYLOAD_NAME), afterStates, hashes);
        assert(verifiedBytesAfterWindow === verifiedBytes(afterStates),
            "Disk piece verification disagrees with engine state");
        assert(beforeStates.every((state, index) => state !== 2 || afterStates[index] === 2),
            "A verified warmup piece disappeared during the measurement window");
        const measuredVerifiedBytes = verifiedBytesAfterWindow - verifiedBytesBeforeWindow;
        // A stalled window is a valid zero-speed observation. Excluding it would
        // bias the public-swarm comparison toward successful transfer periods.
        assert(measuredVerifiedBytes >= 0, "Verified data regressed during the window");
        const result: RunResult = {
            mode, round, ordinal, attempt, peerPort, executableSha256: executableHashes.get(executable)!, appVersion,
            connectionSetupMilliseconds, measurementMilliseconds: measuredMilliseconds,
            verifiedBytesBeforeWindow, verifiedBytesAfterWindow, measuredVerifiedBytes,
            measuredVerifiedBytesPerSecond: measuredVerifiedBytes / (measuredMilliseconds / 1000),
            verifiedPiecesAfterWindow: afterStates.filter(state => state === 2).length,
            clientTransferDataDelta: afterTransfer.dl_info_data - beforeTransfer.dl_info_data,
            torrentDownloadedSessionDelta: afterInfo.downloaded_session - beforeInfo.downloaded_session,
            uiProbeMilliseconds: summarizeLatencies(uiLatencies),
            publicPeers: {
                connectedAtWarmup: warmup.connectedSeeds + warmup.connectedLeeches,
                seedsReported: warmup.seedsReported, leechesReported: warmup.leechesReported,
                dhtNodes: warmup.dhtNodes,
            },
            ...(pathMeasurements === undefined ? {} : { pathMeasurements }), evidenceRoot: root,
        };
        return result;
    }
    finally {
        if (!clean && child.exitCode === null) {
            child.kill();
            await child.exited;
        }
        const resolvedDestination = await realpath(destination);
        assert(dirname(resolvedDestination) === await realpath(root), "Refusing to clean payload outside its run root");
        await rm(destination, { recursive: true, force: true });
    }
}

if (!qbuttOnly) assert((await stat(controlExecutable)).isFile(), "Upstream control executable is missing");
if (qbuttExecutable) assert((await stat(qbuttExecutable)).isFile(), "qbutt executable is missing");
if (proxyConfig) assert((await stat(proxyConfig)).isFile(), "Mihomo proxy config is missing");
const executables = [...(qbuttOnly ? [] : [controlExecutable]), ...(qbuttExecutable ? [qbuttExecutable] : [])];
const executableHashes = new Map<string, string>();
for (const executable of executables) executableHashes.set(executable, sha256(await readFile(executable)));
if (!qbuttOnly) assert(executableHashes.get(controlExecutable) === CONTROL_SHA256,
    `The upstream control must be ${CONTROL_REVISION} (${CONTROL_SHA256})`);
if (qbuttExecutable) assert(executableHashes.get(qbuttExecutable) !== CONTROL_SHA256,
    "qbutt executable must be distinct from the upstream control");
await allowLabNetwork([process.execPath, ...executables,
    ...executables.map(executable => join(dirname(executable), "qbutt-net.exe")).filter(existsSync)]);

const reportRoot = await mkdtemp(join(tmpdir(), "qbutt-public-swarm-"));
const reportPath = join(reportRoot, "evidence.json");
const torrentPath = join(reportRoot, "source.torrent");
const response = await fetch(SOURCE_URL, { redirect: "follow", signal: AbortSignal.timeout(30000) });
assert(response.ok && response.url === SOURCE_URL,
    `Official torrent fetch returned ${response.status} from ${response.url}`);
const torrentBytes = new Uint8Array(await response.arrayBuffer());
assert(sha256(torrentBytes) === SOURCE_SHA256, "Official torrent file changed from the pinned SHA-256");
await writeFile(torrentPath, torrentBytes);
const modes: Mode[] = [...(qbuttOnly ? [] : ["upstream-native" as const]), ...(qbuttExecutable ? ["qbutt-native" as const] : []),
    ...(proxyConfig ? ["qbutt-one-tunnel" as const, "qbutt-mixed" as const] : [])];
assert(peerPort + rounds * modes.length * attemptsPerWindow <= 49151,
    "The deterministic peer-port range exceeds the non-ephemeral boundary");
const evidence: Record<string, unknown> = {
    schema: 1, suite: "public-swarm-benchmark", status: "running", startedAt: new Date().toISOString(),
    source: { url: SOURCE_URL, checksumUrl: CHECKSUM_URL, torrentSha256: SOURCE_SHA256,
        infoHashV1: INFO_HASH, payloadName: PAYLOAD_NAME,
        payloadBytes: PAYLOAD_BYTES, fullPayloadSha256Expected: PAYLOAD_SHA256, fullPayloadVerified: false,
        pieceLength: PIECE_LENGTH, pieceCount: PIECE_COUNT },
    control: qbuttOnly ? null : { revision: CONTROL_REVISION, executable: controlExecutable,
        executableSha256: executableHashes.get(controlExecutable) },
    qbutt: qbuttExecutable
        ? { executable: qbuttExecutable, executableSha256: executableHashes.get(qbuttExecutable) } : null,
    topology: { rounds, modes,
        orderByRound: Array.from({ length: rounds }, (_, index) => orderForRound(modes, index + 1)),
        warmupRateBytesPerSecond: warmupRate, measurementRateBytesPerSecond: measuredRate,
        requestedMeasurementMilliseconds: measurementMilliseconds, minimumWarmupPieces,
        nativeInterface, nativeAddress, configuredTunnelCount: proxyConfig ? proxyNames.length : 0,
        peerPortBase: peerPort, attemptsPerWindow },
    limits: [
        ...(qbuttOnly ? ["qbutt route comparison only; no unchanged-upstream performance claim"] : []),
        "Live public swarm membership, peer capacity and Internet path conditions change during the run",
        "Completed pieces are read from disk and checked against the torrent SHA-1 list;"
            + " the full ISO SHA-256 is not claimed",
        "Client transfer counters are recorded separately and are not packet-level wire-byte measurements",
        "Path payload and hash-verified download deltas are reported separately; neither is carrier wire traffic",
        "Zero-throughput measurement windows are retained after successful warmup, not retried or discarded",
        "Measured verified bytes are pieces that reached verified state during the window;"
            + " pre-window partial blocks are not separable",
        "Public windows complement the deterministic lab benchmark and do not replace its release thresholds",
    ], runs: [], rejectedAttempts: [],
};
await writeFile(reportPath, JSON.stringify(evidence, null, 2) + "\n");
try {
    const runs: RunResult[] = [];
    for (let round = 1; round <= rounds; ++round) {
        const order = orderForRound(modes, round);
        for (const [ordinal, mode] of order.entries()) {
            let result: RunResult | undefined;
            for (let attempt = 1; attempt <= attemptsPerWindow && !result; ++attempt) {
                const slot = ((round - 1) * modes.length + ordinal) * attemptsPerWindow + attempt - 1;
                const runPort = peerPort + slot;
                await assertPeerPortAvailable(runPort);
                try {
                    result = await run(mode, round, ordinal + 1, attempt,
                        torrentPath, reportRoot, runPort, executableHashes);
                }
                catch (error) {
                    const rejected = { mode, round, ordinal: ordinal + 1, attempt, peerPort: runPort,
                        error: error instanceof Error ? error.message : String(error),
                        evidenceRoot: join(reportRoot, `run-${round}-${ordinal + 1}-${mode}-attempt-${attempt}`) };
                    (evidence.rejectedAttempts as unknown[]).push(rejected);
                    await writeFile(reportPath, JSON.stringify(evidence, null, 2) + "\n");
                    console.error(JSON.stringify({ check: "public-swarm-attempt-rejected", ...rejected }));
                    if (attempt === attemptsPerWindow) throw error;
                }
            }
            assert(result, "Public swarm window did not produce a result");
            runs.push(result);
            evidence.runs = runs;
            await writeFile(reportPath, JSON.stringify(evidence, null, 2) + "\n");
            console.log(JSON.stringify({ check: "public-swarm-window", ...result, evidence: reportPath }));
        }
    }
    evidence.status = "passed";
    evidence.finishedAt = new Date().toISOString();
    const rejectedAttempts = evidence.rejectedAttempts as { mode: Mode }[];
    evidence.summary = Object.fromEntries(modes.map(mode => {
        const samples = runs.filter(run => run.mode === mode).map(run => run.measuredVerifiedBytesPerSecond);
        const rejectedCount = rejectedAttempts.filter(attempt => attempt.mode === mode).length;
        return [mode, { windows: samples.length, medianVerifiedBytesPerSecond: median(samples),
            minimumVerifiedBytesPerSecond: Math.min(...samples), maximumVerifiedBytesPerSecond: Math.max(...samples),
            rejectedAttempts: rejectedCount, acceptedAttemptRatio: samples.length / (samples.length + rejectedCount) }];
    }));
}
catch (error) {
    evidence.status = "failed";
    evidence.finishedAt = new Date().toISOString();
    evidence.error = error instanceof Error ? error.stack ?? error.message : String(error);
    throw error;
}
finally {
    await writeFile(reportPath, JSON.stringify(evidence, null, 2) + "\n");
    console.log(JSON.stringify({ status: evidence.status, evidence: reportPath }));
}
