import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { networkInterfaces, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { sha256, type TorrentFixture } from "../fixtures/generate";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy, type ProxyStats } from "./proxy";
import { prepareResourceSampler, type ResourceWindow } from "./process-resources";
import { createTcpBottleneck } from "./tcp-bottleneck";

type Mode = "upstream-native" | "qbutt-native" | "qbutt-one-tunnel" | "qbutt-mixed" | "qbutt-static";

interface PathsStatus {
    busy: boolean;
    processId: number;
    paths: { pathId: string; generation: number; edgeId: string; proxyName: string; open: boolean; localAddress?: string;
        closedPayloadDownload?: number }[];
    peers: { pathId: string; generation: number; peer: string; port: number; localPort: number; payloadDownload: number }[];
    diagnostics: { v: number; scope: string; blockedSelections: number; eventsTruncated: boolean;
        routes: { pathId: string; generation: number; type: string; attempts: number; connected: number;
            closed: number; connectionFailures: number; timeouts: number; payloadDownload: number;
            payloadUpload: number; verifiedDownload: number; demandMilliseconds: number;
            chokedMilliseconds: number }[];
        events: { ageMilliseconds: number; pathId: string; generation: number; event: string;
            decision: string }[] };
}

interface RouteResult {
    kind: "native" | "tunnel";
    pathId?: string;
    generation?: number;
    sourcePayloadUploadBytes?: number;
    enginePayloadDownload?: number;
    relay?: ProxyStats;
}

interface RunResult {
    mode: Mode;
    round: number;
    executableSha256: string;
    appVersion: string;
    exactPayloadBytes: number;
    warmupVerifiedBytes: number;
    measuredVerifiedBytes: number;
    preparationMilliseconds: number;
    connectionSetupMilliseconds: number;
    measurementMilliseconds: number;
    endToEndCompletionMilliseconds: number;
    verifiedBytesPerSecond: number;
    endToEndVerifiedBytesPerSecond: number;
    uiProbeMilliseconds: { count: number; median: number; p95: number; maximum: number };
    resources: ResourceWindow;
    routes: RouteResult[];
    sourcePayloadBytes: number;
    redundantPayloadBytes: number;
    bottleneck?: { capBytesPerSecond: number } & ReturnType<typeof createTcpBottleneck>["stats"];
    sharedPeers?: { port: number; initialPathId: string; initialGeneration: number; staticBucket?: number;
        sourcePayloadUploadBytes: number }[];
    recovery?: { failedPathId: string; healthyPathId: string; milliseconds: number; nativeConnectionRetained: true };
    unequalPaths?: {
        training: { pathId: string; generation: number; capBytesPerSecond: number; verifiedDownload: number;
            demandMilliseconds: number; verifiedBytesPerDemandSecond: number; limiterStreamBytes: number }[];
        measured: { pathId: string; generation: number; capBytesPerSecond: number; assignedPeers: number;
            limiterStreamBytes: number }[];
        assignmentCeilingBytesPerSecond: number;
        observedAssignmentStable: true;
    };
    evidence: string;
}

type SeedHandle = Awaited<ReturnType<typeof startSeed>>;
type Bottleneck = ReturnType<typeof createTcpBottleneck>;
type BottleneckSnapshot = ReturnType<Bottleneck["snapshot"]>;

interface StaticPeerFixture {
    seed: SeedHandle;
    endpointPort: number;
    bucket: number;
    phase: "training" | "measurement";
}

const CONTROL_SHA256 = "9393e0c523b35a437fb9b356b4c7c7402dbbd9d97b9c1ae519fd01f1219c471e";
const CONTROL_REVISION = "0b63c3d17373f6132ea211c9dcd4241284ccdfaf";
const WARMUP_RATE = 1024;
const STATIC_PEER_RATE = 8 * 1024;
const UNEQUAL_ROUTE_RATES = [48 * 1024, 16 * 1024, 8 * 1024] as const;
const UNEQUAL_TRAINING_RATE = 96 * 1024;
const UNEQUAL_TRAINING_SAMPLE = 64 * 1024;
const UNEQUAL_MEASUREMENT_PEERS = 9;
const TRANSFER_RATE = Number(process.env.QBUTT_BENCH_ROUTE_RATE ?? 96 * 1024);
const scenario = process.env.QBUTT_BENCH_SCENARIO ?? "capacity";
assert(["capacity", "shared-cap", "shared-network-cap", "failed-path", "static-comparison", "static-unequal"].includes(scenario), "Unknown benchmark scenario");
const unequalStatic = scenario === "static-unequal";
const comparingStatic = scenario === "static-comparison" || unequalStatic;
const sharedNetworkCap = scenario === "shared-network-cap";
const sharedCap = scenario === "shared-cap" || sharedNetworkCap;
const SOURCE_RATE = comparingStatic ? STATIC_PEER_RATE
    : sharedCap ? Math.min(TRANSFER_RATE * 4, 1024 * 1024) : TRANSFER_RATE;
const ROUNDS = Number(process.env.QBUTT_BENCH_ROUNDS ?? (scenario === "failed-path" ? 1 : 4));
const MODES: Mode[] = comparingStatic ? ["qbutt-static", "qbutt-mixed"]
    : sharedCap ? ["qbutt-native", "qbutt-mixed"]
    : scenario === "failed-path" ? ["qbutt-mixed"]
    : ["upstream-native", "qbutt-native", "qbutt-one-tunnel", "qbutt-mixed"];
const baselineExecutable = resolve(process.env.QBUTT_BENCH_BASELINE_EXE ?? "");
const qbuttExecutable = resolve(process.env.QBUTT_BENCH_QBUTT_EXE ?? "");
const staticExecutable = resolve(process.env.QBUTT_BENCH_STATIC_EXE ?? "");
const staticReceiptPath = resolve(process.env.QBUTT_BENCH_STATIC_RECEIPT ?? "");
const staticNetworkChild = comparingStatic ? join(dirname(staticExecutable), "qbutt-net.exe") : "";
const qbuttNetworkChild = comparingStatic ? join(dirname(qbuttExecutable), "qbutt-net.exe") : "";
const python = resolve(process.env.QBUTT_LAB_PYTHON ?? "");
const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE ?? "";
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS ?? "";

assert(process.env.QBUTT_BENCH_QBUTT_EXE && process.env.QBUTT_LAB_PYTHON
    && (comparingStatic ? process.env.QBUTT_BENCH_STATIC_EXE && process.env.QBUTT_BENCH_STATIC_RECEIPT
        : process.env.QBUTT_BENCH_BASELINE_EXE),
"Set QBUTT_BENCH_QBUTT_EXE, QBUTT_LAB_PYTHON, and the scenario's baseline executable and receipt");
assert(nativeInterface && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
assert(Number.isInteger(ROUNDS) && (scenario === "failed-path" ? ROUNDS === 1 : ROUNDS >= 3 && ROUNDS <= 9),
    "QBUTT_BENCH_ROUNDS must be 1 for failed-path, otherwise between 3 and 9");
assert(Number.isInteger(TRANSFER_RATE) && TRANSFER_RATE >= 32 * 1024 && TRANSFER_RATE <= 512 * 1024,
    "QBUTT_BENCH_ROUTE_RATE must be between 32 and 512 KiB/s");
assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal),
"Native benchmark address must belong to the selected non-loopback IPv4 interface");
assert((await stat(comparingStatic ? staticExecutable : baselineExecutable)).isFile()
    && (await stat(qbuttExecutable)).isFile() && (await stat(python)).isFile()
    && (!comparingStatic || (await stat(staticNetworkChild)).isFile()
        && (await stat(qbuttNetworkChild)).isFile()),
"A benchmark executable is missing");
const [baselineHash, qbuttHash] = await Promise.all([
    readFile(comparingStatic ? staticExecutable : baselineExecutable).then(sha256),
    readFile(qbuttExecutable).then(sha256),
]);
const [staticNetworkChildHash, qbuttNetworkChildHash] = comparingStatic ? await Promise.all([
    readFile(staticNetworkChild).then(sha256), readFile(qbuttNetworkChild).then(sha256),
]) : [undefined, undefined];
const staticReceipt = comparingStatic ? JSON.parse(await readFile(staticReceiptPath, "utf8")) as {
    source: string; normalSource: string; patchSha256: string; staticSha256: string; normalSha256: string;
    qbuttNetSha256: string;
} : undefined;
assert(comparingStatic ? staticReceipt && /^[0-9a-f]{64}$/.test(staticReceipt.staticSha256)
    && /^[0-9a-f]{64}$/.test(staticReceipt.normalSha256)
    && /^[0-9a-f]{64}$/.test(staticReceipt.qbuttNetSha256)
    && baselineHash === staticReceipt.staticSha256 && qbuttHash === staticReceipt.normalSha256
    && staticNetworkChildHash === staticReceipt.qbuttNetSha256
    && qbuttNetworkChildHash === staticReceipt.qbuttNetSha256
    : baselineHash === CONTROL_SHA256,
`A benchmark binary differs from the pinned ${comparingStatic ? "static/normal pair" : CONTROL_REVISION} control`);
assert(baselineHash !== qbuttHash, "The qbutt executable must be distinct from the baseline");

function quantile(values: number[], fraction: number): number {
    assert(values.length > 0, "Cannot summarize an empty sample");
    return [...values].sort((left, right) => left - right)[Math.ceil(values.length * fraction) - 1]!;
}

function summarizeLatencies(values: number[]) {
    return {
        count: values.length,
        median: quantile(values, 0.5),
        p95: quantile(values, 0.95),
        maximum: Math.max(...values),
    };
}

function median(values: number[]): number {
    assert(values.length > 0, "Cannot summarize an empty sample");
    const sorted = [...values].sort((left, right) => left - right);
    const middle = Math.floor(sorted.length / 2);
    return sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}

function staticBucket(torrent: TorrentFixture, peerAddress: string, port: number): number {
    assert(torrent.infoHashV1 && !torrent.infoHashV2, "The static comparator requires a v1-only public torrent");
    const infoHash = Buffer.from(torrent.infoHashV1, "hex");
    assert(infoHash.length === 20 && Number.isInteger(port) && port > 0 && port <= 65535,
        "The static comparator requires a valid v1 infohash and peer port");
    const address = peerAddress.split(".").map(Number);
    assert(address.length === 4 && address.every(byte => Number.isInteger(byte) && byte >= 0 && byte <= 255),
        "The static comparator requires a numeric IPv4 peer");
    const bytes = [...infoHash, ...Buffer.alloc(32), 4,
        ...address, port >> 8, port & 255];
    let hash = 14695981039346656037n;
    for (const byte of bytes)
        hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 1099511628211n);
    return Number(hash % 3n);
}

function bottleneckDelta(before: BottleneckSnapshot, after: BottleneckSnapshot) {
    assert(after.downstreamStreamBytes >= before.downstreamStreamBytes
        && after.acceptedConnections >= before.acceptedConnections, "Bottleneck counters regressed");
    return { acceptedConnections: after.acceptedConnections - before.acceptedConnections,
        downstreamStreamBytes: after.downstreamStreamBytes - before.downstreamStreamBytes };
}

function orderForRound(round: number): Mode[] {
    const offset = (round - 1) % MODES.length;
    return [...MODES.slice(offset), ...MODES.slice(0, offset)];
}

async function makePartialSeed(torrent: TorrentFixture, fixtureRoot: string, destination: string,
    pieces: number[]): Promise<number> {
    let bytes = 0;
    for (const file of torrent.files) {
        assert(!file.pad, "The benchmark fixture must not contain padding");
        const source = await readFile(join(fixtureRoot, "seed", file.path));
        const partial = Buffer.alloc(file.size);
        for (const piece of pieces) {
            const from = Math.max(file.offset, piece * torrent.pieceLength);
            const to = Math.min(file.offset + file.size, (piece + 1) * torrent.pieceLength);
            if (to > from) {
                source.copy(partial, from - file.offset, from - file.offset, to - file.offset);
                bytes += to - from;
            }
        }
        await mkdir(dirname(join(destination, file.path)), { recursive: true });
        await writeFile(join(destination, file.path), partial);
    }
    return bytes;
}

async function awaitCompletion(lab: Awaited<ReturnType<typeof createLab>>, hash: string,
    observe?: () => Promise<void>) {
    const latencies: number[] = [];
    const deadline = Date.now() + 120000;
    for (;;) {
        const started = performance.now();
        const info = await lab.info(hash);
        latencies.push(performance.now() - started);
        await observe?.();
        if (info.progress === 1)
            return latencies;
        if (Date.now() >= deadline)
            throw new Error(`Timed transfer did not complete; last verified count ${info.completed}`);
        await Bun.sleep(100);
    }
}

async function run(mode: Mode, round: number): Promise<RunResult> {
    const executable = mode === "upstream-native" ? baselineExecutable
        : mode === "qbutt-static" ? staticExecutable : qbuttExecutable;
    assert(sha256(await readFile(executable)) === (mode === "upstream-native" || mode === "qbutt-static"
        ? baselineHash : qbuttHash),
        "Benchmark executable changed between windows");
    if (comparingStatic)
        assert(sha256(await readFile(join(dirname(executable), "qbutt-net.exe"))) === staticReceipt!.qbuttNetSha256,
            "Benchmark qbutt-net executable changed between windows");
    process.env.QBUTT_LAB_EXE = executable;
    process.env.QBUTT_LAB_PYTHON = python;
    process.env.QBUTT_LAB_APP_NAME = mode === "upstream-native" ? "qBittorrent" : "qbutt";
    const lab = await createLab(`benchmark-${mode}-${round}`);
    const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
    const exactPayloadBytes = lab.manifest.payload.reduce((sum, file) => sum + file.size, 0);
    const tunnelCount = (mode === "qbutt-mixed" || mode === "qbutt-static") ? 2
        : mode === "qbutt-one-tunnel" ? 1 : 0;
    const routeCount = tunnelCount === 2 ? 3 : 1;
    const seeds: SeedHandle[] = [];
    const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
    const bottleneck = sharedNetworkCap ? createTcpBottleneck(TRANSFER_RATE) : undefined;
    const unequalBottlenecks = unequalStatic ? UNEQUAL_ROUTE_RATES.map(rate => createTcpBottleneck(rate)) : [];
    const staticPeers: StaticPeerFixture[] = [];
    const peerPorts: number[] = [];
    const subsetBytes: number[] = [];
    const credentials = Array.from({ length: tunnelCount }, () => ({
        username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex"),
    }));
    const assignedRoutes: { pathId: string; generation: number; edgeId: string; native: boolean }[] = [];
    let sharedPeerAssignments: { port: number; initialPathId: string; initialGeneration: number;
        staticBucket?: number }[] = [];
    let unequalTraining: NonNullable<RunResult["unequalPaths"]>["training"] = [];
    let unequalMeasurement: { peer: StaticPeerFixture; pathId: string; generation: number }[] = [];
    let unequalMeasurementStart: BottleneckSnapshot[] = [];
    let failure: unknown;
    let recovery: RunResult["recovery"];
    let unequalPaths: RunResult["unequalPaths"];
    let resourceSampler: Awaited<ReturnType<typeof prepareResourceSampler>> | undefined;
    let cleanupPromise: Promise<void> | undefined;
    const cleanup = () => cleanupPromise ??= (async () => {
        const results = await Promise.allSettled([
            resourceSampler?.close(),
            lab.shutdown(), bottleneck?.close(), ...unequalBottlenecks.map(item => item.close()),
            ...proxies.map(proxy => proxy.close()), ...seeds.map(seed => seed.stop()),
        ]);
        const failures = results.filter((result): result is PromiseRejectedResult => result.status === "rejected");
        if (failures.length)
            throw new AggregateError(failures.map(result => result.reason), "Benchmark process cleanup failed");
    })();
    try {
        if (unequalStatic) {
            const addPeer = async (phase: StaticPeerFixture["phase"], bucket: number,
                pieces?: number[], savePath?: string) => {
                const seed = await startSeed(lab.python, lab.fixtures, torrent.name, lab.root, {
                    label: `${mode}-${phase}-${bucket}`, listenAddress: nativeAddress,
                    uploadRate: WARMUP_RATE, pieces, savePath,
                });
                seeds.push(seed);
                const nativeListener = await (async () => {
                    const firstPort = 20000 + (seed.port % 20000);
                    for (let candidate = 0; candidate < 96; ++candidate) {
                        const endpointPort = 20000 + ((firstPort - 20000 + candidate) % 20000);
                        if (staticBucket(torrent, nativeAddress, endpointPort) !== bucket)
                            continue;
                        try {
                            return await unequalBottlenecks[2]!.listen(nativeAddress, seed.host, seed.port,
                                { port: endpointPort });
                        }
                        catch (error) {
                            const code = (error as NodeJS.ErrnoException).code;
                            if (code !== "EADDRINUSE" && code !== "EACCES")
                                throw error;
                        }
                    }
                    throw new Error(`Could not bind a ${phase} endpoint in static bucket ${bucket}`);
                })();
                const endpointPort = nativeListener.port;
                for (let side = 0; side < 2; ++side) {
                    await unequalBottlenecks[side]!.listen(`127.0.0.${side + 40}`, seed.host, seed.port, {
                        port: endpointPort, clientAddress: "127.0.0.1", upstreamLocalAddress: nativeAddress,
                    });
                }
                staticPeers.push({ seed, endpointPort, bucket, phase });
            };

            for (let bucket = 0; bucket < 3; ++bucket) {
                const pieces = Array.from({ length: 8 }, (_, offset) => bucket * 8 + offset);
                assert(pieces.at(-1)! < torrent.pieceCount, "The training corpus needs 24 distinct pieces");
                const savePath = join(lab.root, `partial-${bucket}`);
                const verifiedPayloadBytes = await makePartialSeed(torrent, lab.fixtures, savePath, pieces);
                assert(verifiedPayloadBytes >= UNEQUAL_TRAINING_SAMPLE * 2,
                    "The training peer needs enough distinct payload after route admission warmup");
                await addPeer("training", bucket, pieces, savePath);
            }
            for (let bucket = 0; bucket < 3; ++bucket)
                for (let peer = 0; peer < UNEQUAL_MEASUREMENT_PEERS / 3; ++peer)
                    await addPeer("measurement", bucket);

            for (let side = 0; side < tunnelCount; ++side) {
                proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`,
                    targets: staticPeers.map(peer => ({ host: nativeAddress, port: peer.endpointPort,
                        connectHost: `127.0.0.${side + 40}`, connectPort: peer.endpointPort })),
                }));
            }
        }
        else if (comparingStatic) {
            const accepted = [0, 0, 0];
            for (let candidate = 0; accepted.some(count => count < 2) && candidate < 24; ++candidate) {
                const seed = await startSeed(lab.python, lab.fixtures, torrent.name, lab.root, {
                    label: `${mode}-candidate-${candidate}`, listenAddress: nativeAddress,
                    uploadRate: WARMUP_RATE,
                });
                const bucket = staticBucket(torrent, nativeAddress, seed.port);
                if (accepted[bucket]! >= 2) {
                    await seed.stop();
                    continue;
                }
                seeds.push(seed);
                assert(seed.verifiedPayloadBytes === exactPayloadBytes,
                    "Shared comparator seed failed full payload verification");
                accepted[bucket] = accepted[bucket]! + 1;
            }
            assert(seeds.length === 6, `Could not select two full peers per static hash bucket: ${accepted}`);
            for (let side = 0; side < tunnelCount; ++side) {
                proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`,
                    remoteAddresses: [nativeAddress],
                    targets: seeds.map(seed => ({ host: nativeAddress, port: seed.port })),
                }));
            }
        }
        else {
            for (let side = 0; side < routeCount; ++side) {
                const native = mode !== "qbutt-one-tunnel" && (mode !== "qbutt-mixed" || side === routeCount - 1);
                const pieces = routeCount === 1 ? undefined
                    : Array.from({ length: torrent.pieceCount }, (_, piece) => piece).filter(piece => piece % routeCount === side);
                const savePath = pieces ? join(lab.root, `partial-${side}`) : undefined;
                subsetBytes.push(pieces ? await makePartialSeed(torrent, lab.fixtures, savePath!, pieces) : exactPayloadBytes);
                const seed = await startSeed(lab.python, lab.fixtures, torrent.name, lab.root, {
                    savePath, pieces, label: `${mode}-${side}`,
                    listenAddress: native ? nativeAddress : undefined, uploadRate: WARMUP_RATE,
                });
                seeds.push(seed);
                assert(seed.verifiedPayloadBytes === subsetBytes[side], "Seed verified-byte count differs from its physical data");
                const peerPort = bottleneck
                    ? (await bottleneck.listen(native ? nativeAddress : "127.0.0.1", seed.host, seed.port)).port
                    : seed.port;
                peerPorts.push(peerPort);
                if (!native) {
                    const syntheticHost = `127.0.0.${side + 2}`;
                    const targets = [{
                        host: syntheticHost, port: peerPort, connectHost: seed.host, connectPort: peerPort,
                    }];
                    if (scenario === "failed-path" && side === 1)
                        targets.push({ host: "127.0.0.2", port: seeds[0]!.port,
                            connectHost: seeds[0]!.host, connectPort: seeds[0]!.port });
                    proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`, targets }));
                }
            }
        }
        if (!comparingStatic)
            assert(subsetBytes.reduce((sum, bytes) => sum + bytes, 0) === exactPayloadBytes,
                "Complementary benchmark seeds do not cover the exact payload once");

        await lab.start();
        const appVersion = await (await lab.request("app/version")).text();
        if (comparingStatic) {
            await lab.request("app/setPreferences", { json: JSON.stringify({
                enable_multi_connections_from_same_ip: true,
            }) });
            await waitFor("benchmark peers on one fixture address are permitted",
                () => lab.json<Record<string, unknown>>("app/preferences"),
                preferences => preferences.enable_multi_connections_from_same_ip === true);
        }
        if (mode === "upstream-native" || mode === "qbutt-native") {
            await lab.request("app/setPreferences", { json: JSON.stringify({
                current_network_interface: "", current_interface_address: nativeAddress, bittorrent_protocol: 1,
            }) });
            await waitFor("direct Native address binding", () => lab.json<Record<string, unknown>>("app/preferences"),
                preferences => preferences.current_interface_address === nativeAddress && preferences.bittorrent_protocol === 1);
        }
        else {
            const configPath = join(lab.root, "benchmark-nodes.json");
            await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
                name: `benchmark-${side}`, type: "socks5", server: proxy.host, port: proxy.port,
                ...credentials[side], udp: false,
            })) }));
            for (let side = 0; side < tunnelCount; ++side) {
                await lab.request("qbuttPaths/open", { configPath, proxyName: `benchmark-${side}`,
                    interfaceName: "Loopback Pseudo-Interface 1" });
                await waitFor("benchmark tunnel open", () => lab.json<PathsStatus>("qbuttPaths/status"),
                    status => !status.busy && status.paths.filter(path => path.open).length === side + 1);
            }
            await lab.request("qbuttPaths/policy", (mode === "qbutt-mixed" || mode === "qbutt-static")
                ? { mode: "mixed", nativeInterface } : { mode: "pinned" });
        }

        assert(lab.pid, "Benchmark app has no owned process ID");
        resourceSampler = await prepareResourceSampler(python, [
            { role: "app", pid: lab.pid, executable },
            ...(tunnelCount > 0 ? [{ role: "qbutt-net" as const,
                pid: (await lab.json<PathsStatus>("qbuttPaths/status")).processId,
                executable: join(dirname(executable), "qbutt-net.exe") }] : []),
        ], lab.root);

        const destination = join(lab.root, "target");
        const preparationStarted = performance.now();
        const hash = await lab.add(torrent.name, destination);
        const preparationMilliseconds = performance.now() - preparationStarted;
        const torrentStarted = performance.now();
        await lab.request("torrents/start", { hashes: hash });
        const setupStarted = performance.now();
        const endpoints = seeds.map((seed, side) => unequalStatic ? `${nativeAddress}:${staticPeers[side]!.endpointPort}`
            : comparingStatic ? `${nativeAddress}:${seed.port}`
            : mode === "qbutt-mixed" && side === routeCount - 1
            ? `${nativeAddress}:${peerPorts[side]}` : mode === "upstream-native" || mode === "qbutt-native"
                ? `${nativeAddress}:${peerPorts[side]}` : `127.0.0.${side + 2}:${peerPorts[side]}`);
        if (mode === "upstream-native" || mode === "qbutt-native") {
            await lab.request("torrents/addPeers", { hashes: hash, peers: endpoints[0]! });
            await waitFor("direct Native peer warmup", () => lab.json<{
                peers: Record<string, { downloaded: number; ip: string; port: number }>;
            }>(`sync/torrentPeers?hash=${hash}&rid=0`), status => Object.values(status.peers).some(peer =>
                peer.downloaded > 0 && peer.ip === nativeAddress && peer.port === peerPorts[0]), 60000);
        }
        else {
            const paths = await lab.json<PathsStatus>("qbuttPaths/status");
            if (comparingStatic) {
                for (let side = 0; side < routeCount; ++side) {
                    const path = paths.paths.find(candidate => side === routeCount - 1
                        ? candidate.edgeId === "native" && candidate.localAddress === nativeAddress
                        : candidate.proxyName === `benchmark-${side}`);
                    assert(path, `Static comparator route ${side} did not open`);
                    assignedRoutes.push({ pathId: path.pathId, generation: path.generation,
                        edgeId: path.edgeId, native: path.edgeId === "native" });
                }
                if (unequalStatic) {
                    const trainingPeers = staticPeers.filter(peer => peer.phase === "training");
                    const measurementPeers = staticPeers.filter(peer => peer.phase === "measurement");
                    const limiterBefore = unequalBottlenecks.map(item => item.snapshot());
                    for (const peer of trainingPeers)
                        await lab.request("torrents/addPeers", { hashes: hash,
                            peers: `${nativeAddress}:${peer.endpointPort}` });
                    const trainingConnected = await waitFor("one training peer on every route",
                        () => lab.json<PathsStatus>("qbuttPaths/status"), status => trainingPeers.every(peer =>
                            status.peers.some(candidate => candidate.peer === nativeAddress
                                && candidate.port === peer.endpointPort && candidate.payloadDownload > 0)), 120000);
                    const trainingAssignments = trainingPeers.map(peer => {
                        const connection = trainingConnected.peers.find(candidate => candidate.peer === nativeAddress
                            && candidate.port === peer.endpointPort)!;
                        return { peer, pathId: connection.pathId, generation: connection.generation };
                    });
                    assert(new Set(trainingAssignments.map(item => `${item.pathId}:${item.generation}`)).size === 3,
                        "Training did not cover each eligible route exactly once");
                    const sampleStart = trainingConnected.diagnostics.routes;
                    await Promise.all(trainingPeers.map(peer => peer.seed.setUploadRate(UNEQUAL_TRAINING_RATE)));
                    const trained = await waitFor("unequal route training sample", () =>
                        lab.json<PathsStatus>("qbuttPaths/status"), status => trainingAssignments.every(assignment => {
                            const before = sampleStart.find(route => route.pathId === assignment.pathId
                                && route.generation === assignment.generation);
                            const after = status.diagnostics.routes.find(route => route.pathId === assignment.pathId
                                && route.generation === assignment.generation);
                            return Boolean(before && after
                                && after.verifiedDownload - before.verifiedDownload >= UNEQUAL_TRAINING_SAMPLE);
                        }), 120000);
                    await Promise.all(trainingPeers.map(peer => peer.seed.stop()));
                    const drainedTraining = await waitFor("training peers disconnected before measured dials", () =>
                        lab.json<PathsStatus>("qbuttPaths/status"), status => trainingPeers.every(peer =>
                            !status.peers.some(candidate => candidate.peer === nativeAddress
                                && candidate.port === peer.endpointPort)), 30000);
                    const limiterAfter = unequalBottlenecks.map(item => item.snapshot());
                    unequalTraining = assignedRoutes.map((route, side) => {
                        const before = sampleStart.find(candidate => candidate.pathId === route.pathId
                            && candidate.generation === route.generation);
                        const after = trained.diagnostics.routes.find(candidate => candidate.pathId === route.pathId
                            && candidate.generation === route.generation);
                        assert(before && after, "Route diagnostics disappeared during training");
                        const verifiedDownload = after.verifiedDownload - before.verifiedDownload;
                        const demandMilliseconds = after.demandMilliseconds - before.demandMilliseconds;
                        assert(verifiedDownload >= UNEQUAL_TRAINING_SAMPLE && demandMilliseconds > 0,
                            "Route training did not produce the selector's verified-demand signal");
                        const limiterStreamBytes = bottleneckDelta(limiterBefore[side]!, limiterAfter[side]!)
                            .downstreamStreamBytes;
                        assert(limiterStreamBytes >= verifiedDownload,
                            "Training verified bytes bypassed its path limiter");
                        return { pathId: route.pathId, generation: route.generation,
                            capBytesPerSecond: UNEQUAL_ROUTE_RATES[side]!, verifiedDownload, demandMilliseconds,
                            verifiedBytesPerDemandSecond: verifiedDownload * 1000 / demandMilliseconds,
                            limiterStreamBytes };
                    });
                    assert(unequalTraining[0]!.verifiedBytesPerDemandSecond
                        >= unequalTraining[1]!.verifiedBytesPerDemandSecond * 1.8
                        && unequalTraining[1]!.verifiedBytesPerDemandSecond
                            >= unequalTraining[2]!.verifiedBytesPerDemandSecond * 1.4,
                    `Training did not establish ordered route quality: ${JSON.stringify(unequalTraining)}`);
                    await lab.checkpoint({ check: "unequal-route-training", mode,
                        phase: "after-training-stop-and-drain-before-measured-dials",
                        exactTargetsWhitelistedOnBothProxies: true, training: unequalTraining,
                        diagnostics: drainedTraining.diagnostics,
                        assignments: trainingAssignments.map(item => ({ endpointPort: item.peer.endpointPort,
                            staticBucket: item.peer.bucket, pathId: item.pathId, generation: item.generation })) });

                    for (const peer of measurementPeers)
                        await lab.request("torrents/addPeers", { hashes: hash,
                            peers: `${nativeAddress}:${peer.endpointPort}` });
                    const measuredConnected = await waitFor("fresh measured peers admitted after training",
                        () => lab.json<PathsStatus>("qbuttPaths/status"), status => measurementPeers.every(peer =>
                            status.peers.some(candidate => candidate.peer === nativeAddress
                                && candidate.port === peer.endpointPort && candidate.payloadDownload > 0)), 120000);
                    unequalMeasurement = measurementPeers.map(peer => {
                        const connection = measuredConnected.peers.find(candidate => candidate.peer === nativeAddress
                            && candidate.port === peer.endpointPort)!;
                        return { peer, pathId: connection.pathId, generation: connection.generation };
                    });
                    sharedPeerAssignments = unequalMeasurement.map(item => ({ port: item.peer.endpointPort,
                        initialPathId: item.pathId, initialGeneration: item.generation,
                        staticBucket: item.peer.bucket }));
                    if (mode === "qbutt-static") {
                        for (const route of assignedRoutes)
                            assert(unequalMeasurement.filter(peer => peer.pathId === route.pathId
                                && peer.generation === route.generation).length === UNEQUAL_MEASUREMENT_PEERS / 3,
                            `Static hash did not place three measured peers on path ${route.pathId}`);
                    }
                    await lab.checkpoint({ check: "unequal-fresh-dial-admission", mode,
                        trainingConnectionsClosed: true, sharedPeerAssignments, paths: assignedRoutes });
                }
                else {
                    for (const endpoint of endpoints)
                        await lab.request("torrents/addPeers", { hashes: hash, peers: endpoint });
                    const connected = await waitFor("six full peers transferring over shared reachable routes",
                        () => lab.json<PathsStatus>("qbuttPaths/status"), status =>
                            seeds.every(seed => status.peers.some(peer => peer.peer === nativeAddress
                                && peer.port === seed.port && peer.payloadDownload > 0
                                && assignedRoutes.some(route => route.pathId === peer.pathId
                                    && route.generation === peer.generation))), 120000);
                    sharedPeerAssignments = seeds.map(seed => {
                        const peer = connected.peers.find(candidate => candidate.peer === nativeAddress
                            && candidate.port === seed.port)!;
                        return { port: seed.port, initialPathId: peer.pathId, initialGeneration: peer.generation };
                    });
                    if (mode === "qbutt-static") {
                        for (const route of assignedRoutes)
                            assert(sharedPeerAssignments.filter(peer => peer.initialPathId === route.pathId
                                && peer.initialGeneration === route.generation).length === 2,
                            `Static hash did not place two peers on path ${route.pathId}`);
                    }
                    await lab.checkpoint({ check: "shared-peers-route-admission", mode,
                        exactTargetsWhitelistedOnBothProxies: true, sharedPeerAssignments, paths: assignedRoutes });
                }
            }
            else {
                const expected = endpoints.map((endpoint, side) => {
                    const [peer, port] = endpoint.split(":");
                    const path = paths.paths.find(candidate => mode === "qbutt-mixed" && side === routeCount - 1
                        ? candidate.edgeId === "native" && candidate.localAddress === nativeAddress
                        : candidate.proxyName === `benchmark-${side}`);
                    assert(path, `No route owns benchmark endpoint ${endpoint}`);
                    assignedRoutes.push({ pathId: path.pathId, generation: path.generation,
                        edgeId: path.edgeId, native: path.edgeId === "native" });
                    return { peer: peer!, port: Number(port), path };
                });
                for (const route of expected) {
                    await lab.request("torrents/addPeers", { hashes: hash, peers: `${route.peer}:${route.port}` });
                    await waitFor("exact benchmark peer route warmup", () => lab.json<PathsStatus>("qbuttPaths/status"), status =>
                        status.peers.some(peer => peer.peer === route.peer && peer.port === route.port
                            && peer.pathId === route.path.pathId && peer.generation === route.path.generation
                            && peer.payloadDownload > 0), 120000);
                }
            }
        }
        if (scenario === "failed-path") {
            const before = await lab.json<PathsStatus>("qbuttPaths/status");
            const nativePeer = before.peers.find(peer => peer.pathId === assignedRoutes[2]!.pathId)!;
            assert(nativePeer?.payloadDownload > 0, "Native must be active before the remote path fails");
            const failedAt = performance.now();
            await proxies[0]!.close();
            const recovered = await waitFor("failed peer automatically reconnects through the healthy path", () =>
                lab.json<PathsStatus>("qbuttPaths/status"), status => status.peers.some(peer =>
                    peer.peer === "127.0.0.2" && peer.port === seeds[0]!.port
                    && peer.pathId === assignedRoutes[1]!.pathId
                    && peer.generation === assignedRoutes[1]!.generation && peer.payloadDownload > 0), 120000);
            assert(recovered.peers.some(peer => peer.peer === nativePeer.peer && peer.port === nativePeer.port
                && peer.localPort === nativePeer.localPort && peer.pathId === nativePeer.pathId
                && peer.generation === nativePeer.generation), "Unrelated Native connection was replaced during recovery");
            recovery = { failedPathId: assignedRoutes[0]!.pathId, healthyPathId: assignedRoutes[1]!.pathId,
                milliseconds: performance.now() - failedAt, nativeConnectionRetained: true };
            await lab.checkpoint({ check: "bad-path-automatic-recovery", ...recovery,
                peer: "127.0.0.2", port: seeds[0]!.port, manualIntervention: false, peers: recovered.peers });
        }
        const connectionSetupMilliseconds = performance.now() - setupStarted;
        const warmupVerifiedBytes = (await lab.info(hash)).completed;
        assert(warmupVerifiedBytes < exactPayloadBytes, "Warmup completed the benchmark payload before measurement");
        if (scenario === "shared-cap") {
            await lab.request("torrents/setDownloadLimit", { hashes: hash, limit: String(TRANSFER_RATE) });
            await waitFor("shared torrent application limit", () => lab.json<{ dl_limit: number }[]>(
                `torrents/info?hashes=${hash}`), torrents => torrents[0]?.dl_limit === TRANSFER_RATE);
        }
        if (sharedNetworkCap || unequalStatic) {
            const preferences = await lab.json<{ dl_limit: number }>("app/preferences");
            const torrents = await lab.json<{ dl_limit: number }[]>(`torrents/info?hashes=${hash}`);
            assert(preferences.dl_limit <= 0 && torrents[0]!.dl_limit <= 0
                && await (await lab.request("transfer/speedLimitsMode")).text() === "0",
            "An external bottleneck requires application download limits to be disabled");
        }
        await Promise.all((unequalStatic ? staticPeers.filter(peer => peer.phase === "measurement").map(peer => peer.seed)
            : seeds).map(seed => seed.setUploadRate(SOURCE_RATE)));
        if (unequalStatic)
            unequalMeasurementStart = unequalBottlenecks.map(item => item.snapshot());
        await resourceSampler.start();
        const completionStarted = performance.now();
        let lastAssignmentCheck = 0;
        const checkAssignments = async () => {
            const current = await lab.json<PathsStatus>("qbuttPaths/status");
            for (const assignment of unequalMeasurement) {
                const peer = current.peers.find(candidate => candidate.peer === nativeAddress
                    && candidate.port === assignment.peer.endpointPort);
                assert(!peer || (peer.pathId === assignment.pathId && peer.generation === assignment.generation),
                    `Measured peer ${assignment.peer.endpointPort} moved between routes`);
            }
        };
        const uiLatencies = await awaitCompletion(lab, hash, unequalStatic ? async () => {
            if (performance.now() - lastAssignmentCheck < 500)
                return;
            lastAssignmentCheck = performance.now();
            await checkAssignments();
        } : undefined);
        if (unequalStatic)
            await checkAssignments();
        const completed = performance.now();
        const resources = await resourceSampler.stop();
        // Resource snapshots bracket this same transfer. Report the actual
        // command/acknowledgement bounds in milliseconds from its start.
        for (const boundary of Object.values(resources.boundaryBounds)) {
            boundary.sent -= completionStarted;
            boundary.acknowledged -= completionStarted;
        }
        const measurementMilliseconds = completed - completionStarted;
        const endToEndCompletionMilliseconds = completed - torrentStarted;
        const measuredVerifiedBytes = exactPayloadBytes - warmupVerifiedBytes;
        const verifiedBytesPerSecond = measuredVerifiedBytes / (measurementMilliseconds / 1000);
        const endToEndVerifiedBytesPerSecond = exactPayloadBytes / (endToEndCompletionMilliseconds / 1000);
        if (unequalStatic) {
            const limiterAfter = unequalBottlenecks.map(item => item.snapshot());
            const measured = assignedRoutes.map((route, side) => ({
                pathId: route.pathId,
                generation: route.generation,
                capBytesPerSecond: UNEQUAL_ROUTE_RATES[side]!,
                assignedPeers: unequalMeasurement.filter(peer => peer.pathId === route.pathId
                    && peer.generation === route.generation).length,
                limiterStreamBytes: bottleneckDelta(unequalMeasurementStart[side]!, limiterAfter[side]!)
                    .downstreamStreamBytes,
            }));
            for (const [side, route] of measured.entries()) {
                assert(route.assignedPeers === 0 || route.limiterStreamBytes > 0,
                    "An assigned measured route has no limiter traffic");
                const upperBound = route.capBytesPerSecond * measurementMilliseconds / 1000
                    + unequalBottlenecks[side]!.stats.burstAllowanceBytes + 1024;
                assert(route.limiterStreamBytes <= upperBound,
                    `Route limiter exceeded its byte budget: ${JSON.stringify({ route, upperBound })}`);
            }
            assert(measured.reduce((sum, route) => sum + route.limiterStreamBytes, 0) >= measuredVerifiedBytes,
                "Measured verified bytes bypassed the unequal path limiters");
            const assignmentCeilingBytesPerSecond = measured.reduce((sum, route) =>
                sum + Math.min(route.capBytesPerSecond, route.assignedPeers * STATIC_PEER_RATE), 0);
            unequalPaths = { training: unequalTraining, measured, assignmentCeilingBytesPerSecond,
                observedAssignmentStable: true };
        }
        await lab.request("torrents/stop", { hashes: hash });
        await waitFor("benchmark target stopped", () => lab.info(hash), info => info.state === "stoppedUP");
        assert(await verifyPayload(destination, lab.manifest.payload) === exactPayloadBytes,
            "Benchmark target failed exact size or SHA-256 verification");

        const status = tunnelCount > 0
            ? await waitFor("closed tunnel payload attribution", () => lab.json<PathsStatus>("qbuttPaths/status"),
                current => comparingStatic ? assignedRoutes.every(assigned => current.paths.some(path =>
                    path.pathId === assigned.pathId && path.generation === assigned.generation))
                    && (unequalStatic ? staticPeers.every(peer => !current.peers.some(candidate =>
                        candidate.peer === nativeAddress && candidate.port === peer.endpointPort))
                        : seeds.every(seed => !current.peers.some(peer => peer.peer === nativeAddress
                            && peer.port === seed.port)))
                    : scenario === "failed-path" ? assignedRoutes.filter(route => !route.native).reduce((sum, route) =>
                    sum + (current.paths.find(path => path.pathId === route.pathId
                        && path.generation === route.generation)?.closedPayloadDownload ?? 0), 0) >= subsetBytes[0]! + subsetBytes[1]!
                    : assignedRoutes.every((assigned, side) => assigned.native
                    || (current.paths.find(candidate => candidate.pathId === assigned.pathId
                        && candidate.generation === assigned.generation)?.closedPayloadDownload ?? 0) >= subsetBytes[side]!),
            10000) : undefined;
        if (status) {
            assert(assignedRoutes.length === routeCount, "Managed benchmark routes were not fully assigned");
            for (const [side, assigned] of assignedRoutes.entries()) {
                const path: PathsStatus["paths"][number] | undefined = status.paths.find(candidate => candidate.pathId === assigned.pathId
                    && candidate.generation === assigned.generation && candidate.edgeId === assigned.edgeId);
                assert(path, "Managed route telemetry disappeared before result capture");
                if (!assigned.native && scenario !== "failed-path" && !comparingStatic)
                    assert((path.closedPayloadDownload ?? 0) >= subsetBytes[side]!,
                        "Tunnel engine payload attribution omits part of its complementary subset");
            }
            if (recovery) await lab.checkpoint({ check: "bad-path-final-attribution", recovery,
                paths: status.paths, tunnelSeedBytes: subsetBytes[0]! + subsetBytes[1]! });
            await lab.request("qbuttPaths/stop", {});
            await waitFor("benchmark qbutt-net shutdown", () => lab.json<PathsStatus>("qbuttPaths/status"),
                stopped => !stopped.busy && stopped.paths.every(path => !path.open));
        }
        await lab.shutdown();
        const stoppedSeeds = await Promise.all(seeds.map(seed => seed.stop()));
        await Promise.all(proxies.map(proxy => proxy.close()));
        const routes: RouteResult[] = comparingStatic ? assignedRoutes.map((assigned, side) => {
            const path = status?.paths.find(candidate => candidate.pathId === assigned.pathId
                && candidate.generation === assigned.generation);
            return { kind: assigned.native ? "native" : "tunnel", pathId: assigned.pathId,
                generation: assigned.generation, enginePayloadDownload: path?.closedPayloadDownload,
                ...(!assigned.native ? { relay: { ...proxies[side]!.stats } } : {}) };
        }) : stoppedSeeds.map((seed, side) => {
            const native = mode !== "qbutt-one-tunnel" && (mode !== "qbutt-mixed" || side === routeCount - 1);
            const assigned = assignedRoutes[side];
            const path = status?.paths.find(candidate => candidate.pathId === assigned?.pathId
                && candidate.generation === assigned.generation);
            return {
                kind: native ? "native" : "tunnel", pathId: assigned?.pathId, generation: assigned?.generation,
                sourcePayloadUploadBytes: seed.uploadPayloadBytes,
                // A failed peer crosses paths; the recovery checkpoint preserves
                // connection identity, while these source totals stay per seed.
                enginePayloadDownload: scenario === "failed-path" ? undefined : path?.closedPayloadDownload,
                ...(!native ? { relay: { ...proxies[side]!.stats } } : {}),
            };
        });
        assert(stoppedSeeds.every((seed, side) => seed.peerAddresses.length === 1
            && seed.peerAddresses[0] === (comparingStatic || routes[side]!.kind === "native"
                ? nativeAddress : "127.0.0.1")),
        "A seed observed an unexpected source address");
        const sourcePayloadBytes = stoppedSeeds.reduce((sum, seed) => sum + seed.uploadPayloadBytes, 0);
        assert(sourcePayloadBytes >= exactPayloadBytes,
            "The only controlled sources uploaded fewer payload bytes than the exact verified target");
        const redundantPayloadBytes = sourcePayloadBytes - exactPayloadBytes;
        if (bottleneck) {
            await lab.checkpoint({ check: "shared-downstream-limiter", expectedListeners: routeCount,
                expectedPayloadBytes: exactPayloadBytes, subsetBytes, ...bottleneck.stats });
            // A peer may reconnect. Require every complementary source's data
            // to cross its limiter, rather than assuming one lifetime socket.
            assert(bottleneck.stats.listeners.length === routeCount
                && bottleneck.stats.listeners.every((listener, side) => listener.acceptedConnections >= 1
                    && listener.downstreamStreamBytes >= subsetBytes[side]!),
            `Shared downstream limiter proof failed: ${JSON.stringify(bottleneck.stats)}`);
        }
        const sharedPeers = comparingStatic ? sharedPeerAssignments.map((peer, side) => {
            const seedIndex = unequalStatic
                ? staticPeers.findIndex(candidate => candidate.endpointPort === peer.port) : side;
            assert(seedIndex >= 0, "Measured peer lost its controlled source");
            return { ...peer, sourcePayloadUploadBytes: stoppedSeeds[seedIndex]!.uploadPayloadBytes };
        }) : undefined;
        const result: RunResult = {
            mode, round, executableSha256: mode === "upstream-native" || mode === "qbutt-static"
                ? baselineHash : qbuttHash,
            appVersion, exactPayloadBytes, warmupVerifiedBytes, measuredVerifiedBytes,
            preparationMilliseconds, connectionSetupMilliseconds, measurementMilliseconds,
            endToEndCompletionMilliseconds, verifiedBytesPerSecond, endToEndVerifiedBytesPerSecond,
            uiProbeMilliseconds: summarizeLatencies(uiLatencies), resources, routes, sourcePayloadBytes,
            ...(sharedPeers ? { sharedPeers } : {}),
            redundantPayloadBytes, recovery, unequalPaths, evidence: join(lab.root, "evidence.json"),
            ...(bottleneck ? { bottleneck: { capBytesPerSecond: TRANSFER_RATE, ...bottleneck.stats } } : {}),
        };
        await lab.checkpoint({ check: "comparative-network-window", ...result });
        await lab.finish();
        return result;
    }
    catch (error) {
        failure = error;
        try { await lab.shutdown(); } catch (shutdownError) { console.error(String(shutdownError)); }
        throw error;
    }
    finally {
        let cleanupError: unknown;
        try { await cleanup(); }
        catch (error) { cleanupError = error; }
        if (!cleanupError) {
            for (const name of ["fixtures", "target", "partial-0", "partial-1", "partial-2", "benchmark-nodes.json", "profile"]) {
                if (name === "profile" && (failure || cleanupError)) continue;
                const target = resolve(lab.root, name);
                assert(dirname(target) === resolve(lab.root), "Cleanup escaped the newly created fixture");
                try { await rm(target, { recursive: true, force: true }); }
                catch (error) { cleanupError ??= error; }
            }
        }
        if (failure)
            await lab.finish(failure);
        if (cleanupError) {
            if (failure)
                console.error(String(cleanupError));
            else {
                await lab.finish(cleanupError);
                throw cleanupError;
            }
        }
    }
}

const reportRoot = await mkdtemp(join(tmpdir(), "qbutt-network-benchmark-"));
const reportPath = join(reportRoot, "evidence.json");
const evidence: Record<string, unknown> = {
    schema: 1,
    suite: "comparative-network-benchmark",
    status: "running",
    startedAt: new Date().toISOString(),
    control: comparingStatic
        ? { kind: "experimental-static-selector", executable: staticExecutable, executableSha256: baselineHash,
            pairedExecutableSha256: qbuttHash, source: staticReceipt!.source,
            pairedSource: staticReceipt!.normalSource, patchSha256: staticReceipt!.patchSha256,
            qbuttNetSha256: staticReceipt!.qbuttNetSha256 }
        : { revision: CONTROL_REVISION, executable: baselineExecutable, executableSha256: baselineHash },
    qbutt: { executable: qbuttExecutable, executableSha256: qbuttHash },
    topology: {
        scenario,
        rounds: ROUNDS,
        orderByRound: Array.from({ length: ROUNDS }, (_, round) => orderForRound(round + 1)),
        ...(comparingStatic ? { peerUploadLimitBytesPerSecond: SOURCE_RATE }
            : { routeUploadLimitBytesPerSecond: SOURCE_RATE }),
        ...(unequalStatic ? { routeDownstreamLimitsBytesPerSecond: UNEQUAL_ROUTE_RATES,
            trainingPeers: 3, trainingSampleBytesPerRoute: UNEQUAL_TRAINING_SAMPLE,
            trainingSourceLimitBytesPerSecond: UNEQUAL_TRAINING_RATE,
            measuredFullPeers: UNEQUAL_MEASUREMENT_PEERS, selectedStaticHashBuckets: [3, 3, 3],
            multiConnectionsPerIp: true }
            : comparingStatic ? { sixFullPeersWithCommonExactTargets: true,
                selectedStaticHashBuckets: [2, 2, 2], multiConnectionsPerIp: true } : {}),
        sharedApplicationDownloadLimit: scenario === "shared-cap" ? TRANSFER_RATE : undefined,
        sharedDownstreamRelayLimit: sharedNetworkCap ? TRANSFER_RATE : undefined,
        warmupUploadLimitBytesPerSecond: WARMUP_RATE,
        nativeInterface,
        nativeAddress,
        modes: MODES,
    },
    limits: [
        "Generated deterministic v1 payload and controlled TCP peers on one Windows host",
        unequalStatic
            ? "Three partial peers first train distinct paths above 64 KiB of verified-demand signal; nine fresh full peers are the only measured dials"
            : comparingStatic
            ? "Each timed window begins after all six peers deliver payload at a 1 KiB/s warmup cap, then all acknowledge the same measured per-peer cap"
            : "Each timed window begins after every required peer supplies payload at a 1 KiB/s warmup cap and acknowledges the measured cap",
        unequalStatic
            ? "Each path has its own 48/16/8 KiB/s downstream stream budget; measured sources have independent 8 KiB/s per-peer caps"
            : comparingStatic
            ? "Six full public TCP peers per run, each exact native endpoint whitelisted on both authenticated SOCKS routes; source cap is per peer, not per path"
            : sharedNetworkCap
            ? "Native and both SOCKS paths cross one shared downstream TCP stream limiter outside qbutt; application download limits are disabled. This emulates a shared network bottleneck, not a physical router"
            : scenario === "shared-cap"
            ? "One torrent-wide application download cap is shared by every path; sources can exceed it. This models an aggregate bottleneck, not a physical last-mile limiter"
            : "Each route has the same source payload cap; Mixed has additional complementary reachability and aggregate capacity",
        "Verified bytes are exact-size and SHA-256 checked; relay stream bytes include protocol data and are not wire bytes",
        "Resource counters cover the transfer window with explicit command/acknowledgement boundary bounds; CPU is per-core, memory peaks are sampled, process I/O is not disk-only",
        "Only the exact app and qbutt-net process handles are measured; runner, controlled peers and relays are excluded",
        ...(comparingStatic ? ["Static uses an unmerged source patch and separately pinned executable; peers, port hash buckets, path assignments and source payload are recorded per run",
            unequalStatic
                ? "All twelve exact endpoints exist behind each path-specific limiter; training connections close before nine fresh measured dials, whose path identity must remain stable"
                : "Both proxies whitelist all six exact peer targets; only selected peer/path connections are observed, not all 18 possible pairs",
            unequalStatic
                ? "The selector chooses routes only for new connections; this fixture does not claim migration of already-live peers"
                : "Equal reachable paths and per-peer caps do not model unequal route throughput or imply RouteSelector should outperform static distribution"] : []),
        "No public swarm, public egress, UDP/uTP/QUIC, inbound, packet capture, netem, disk throttle or physical last-mile claim",
    ],
    runs: [],
};
await writeFile(reportPath, JSON.stringify(evidence, null, 2) + "\n");
try {
    const runs: RunResult[] = [];
    // Rotating order gives every mode each ordinal position in the default four rounds.
    for (let round = 1; round <= ROUNDS; ++round) {
        for (const mode of orderForRound(round)) {
            const result = await run(mode, round);
            runs.push(result);
            evidence.runs = runs;
            await writeFile(reportPath, JSON.stringify(evidence, null, 2) + "\n");
        }
    }
    const medians = Object.fromEntries(MODES.map(mode => {
        const samples = runs.filter(run => run.mode === mode).map(run => run.verifiedBytesPerSecond);
        return [mode, median(samples)];
    })) as Record<Mode, number>;
    const medianCompletionMilliseconds = Object.fromEntries(MODES.map(mode => [mode, median(runs
        .filter(run => run.mode === mode).map(run => run.endToEndCompletionMilliseconds))])) as Record<Mode, number>;
    let comparison: Record<string, unknown>;
    if (scenario === "capacity") {
        const nativeRegressionPercent = 100 * (medians["upstream-native"] - medians["qbutt-native"])
            / medians["upstream-native"];
        const mixedGainPercent = 100 * (medians["qbutt-mixed"] - medians["qbutt-one-tunnel"])
            / medians["qbutt-one-tunnel"];
        assert(nativeRegressionPercent <= 5, `qbutt Native median regression is ${nativeRegressionPercent.toFixed(2)}%`);
        assert(mixedGainPercent > 0, `Mixed median did not exceed one tunnel (${mixedGainPercent.toFixed(2)}%)`);
        comparison = { nativeRegressionPercent, mixedGainPercent,
            gates: { nativeRegressionAtMostFivePercent: true, mixedExceedsOneTunnel: true } };
    }
    else if (scenario === "static-comparison") {
        const staticMedian = medians["qbutt-static"];
        const selectorMedian = medians["qbutt-mixed"];
        comparison = { selectorVersusStaticPercent: 100 * (selectorMedian / staticMedian - 1),
            gate: "neutral-comparison-only", unequalPathQuality: "not-tested" };
    }
    else if (unequalStatic) {
        const staticRuns = runs.filter(run => run.mode === "qbutt-static");
        const selectorRuns = runs.filter(run => run.mode === "qbutt-mixed");
        assert(staticRuns.every(run => run.unequalPaths?.measured.every(path => path.assignedPeers === 3)),
            "Static unequal windows did not retain their three-peers-per-route control");
        const paired = Array.from({ length: ROUNDS }, (_, offset) => {
            const round = offset + 1;
            const staticRun = staticRuns.find(run => run.round === round)!;
            const selectorRun = selectorRuns.find(run => run.round === round)!;
            const staticPaths = staticRun.unequalPaths;
            const selectorPaths = selectorRun.unequalPaths;
            assert(staticPaths && selectorPaths, "Unequal-path evidence is missing");
            return { round,
                staticVerifiedBytesPerSecond: staticRun.verifiedBytesPerSecond,
                selectorVerifiedBytesPerSecond: selectorRun.verifiedBytesPerSecond,
                selectorVersusStaticPercent: 100
                    * (selectorRun.verifiedBytesPerSecond / staticRun.verifiedBytesPerSecond - 1),
                staticAssignmentCeilingBytesPerSecond: staticPaths.assignmentCeilingBytesPerSecond,
                selectorAssignmentCeilingBytesPerSecond: selectorPaths.assignmentCeilingBytesPerSecond,
                assignmentCeilingRatio: selectorPaths.assignmentCeilingBytesPerSecond
                    / staticPaths.assignmentCeilingBytesPerSecond,
                staticCeilingUtilization: staticRun.verifiedBytesPerSecond
                    / staticPaths.assignmentCeilingBytesPerSecond,
                selectorCeilingUtilization: selectorRun.verifiedBytesPerSecond
                    / selectorPaths.assignmentCeilingBytesPerSecond,
                selectorFastPathPeers: selectorPaths.measured[0]!.assignedPeers,
            };
        });
        const staticMedian = medians["qbutt-static"];
        const selectorMedian = medians["qbutt-mixed"];
        const selectorVersusStaticPercent = 100 * (selectorMedian / staticMedian - 1);
        const requiredUsefulGainRounds = Math.ceil(paired.length * 0.75);
        const usefulGainRounds = paired.filter(item => item.selectorVersusStaticPercent >= 10).length;
        const adaptiveAssignmentObserved = paired.every(item => item.selectorFastPathPeers >= 6
            && item.assignmentCeilingRatio >= 1.2);
        const limiterUtilizationSane = paired.every(item => item.staticCeilingUtilization >= 0.7
            && item.staticCeilingUtilization <= 1.1 && item.selectorCeilingUtilization >= 0.7
            && item.selectorCeilingUtilization <= 1.1);
        const materialUsefulThroughputGain = selectorVersusStaticPercent >= 15
            && usefulGainRounds >= requiredUsefulGainRounds;
        comparison = {
            selectorVersusStaticPercent,
            adaptiveAssignmentObserved,
            limiterUtilizationSane,
            materialUsefulThroughputGain,
            usefulGainRounds,
            requiredUsefulGainRounds,
            adaptiveSpeedupProven: adaptiveAssignmentObserved && limiterUtilizationSane
                && materialUsefulThroughputGain,
            paired,
            interpretation: "New-dial adaptation under controlled unequal TCP path budgets; no live-peer migration, WAN or physical last-mile claim",
        };
    }
    else if (sharedCap) {
        const mixedGainPercent = 100 * (medians["qbutt-mixed"] / medians["qbutt-native"] - 1);
        for (const mode of MODES) assert(medians[mode] >= TRANSFER_RATE * 0.7 && medians[mode] <= TRANSFER_RATE * 1.1,
            `${mode} useful throughput did not reach 70–110% of the shared cap`);
        assert(mixedGainPercent <= 10, `Mixed exceeded the shared-cap Native rate by ${mixedGainPercent.toFixed(2)}%`);
        comparison = { mixedGainPercent, capBytesPerSecond: TRANSFER_RATE,
            gates: { bothUseSharedCapacity: true, noMaterialRemoteBenefit: true },
            limiter: sharedNetworkCap ? "shared-downstream-tcp-relay" : "torrent-application-limit",
            physicalLastMile: "not-tested" };
    }
    else {
        assert(runs.length === 1 && runs[0]!.recovery?.nativeConnectionRetained,
            "Bad-path window did not prove automatic recovery and retained Native connection");
        comparison = { recovery: runs[0]!.recovery, gates: { automaticRecovery: true, nativeConnectionRetained: true } };
    }
    evidence.status = "passed";
    evidence.finishedAt = new Date().toISOString();
    evidence.summary = { medianVerifiedBytesPerSecond: medians, medianEndToEndCompletionMilliseconds: medianCompletionMilliseconds,
        ...comparison };
}
catch (error) {
    evidence.status = "failed";
    evidence.finishedAt = new Date().toISOString();
    evidence.error = error instanceof Error ? error.message : String(error);
    throw error;
}
finally {
    await writeFile(reportPath, JSON.stringify(evidence, null, 2) + "\n");
    console.log(JSON.stringify({ status: evidence.status, evidence: reportPath }));
}
