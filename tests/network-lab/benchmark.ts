import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import { networkInterfaces, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { sha256, type TorrentFixture } from "../fixtures/generate";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy, type ProxyStats } from "./proxy";

type Mode = "upstream-native" | "qbutt-native" | "qbutt-one-tunnel" | "qbutt-mixed";

interface PathsStatus {
    busy: boolean;
    paths: { pathId: string; generation: number; edgeId: string; open: boolean; localAddress?: string;
        closedPayloadDownload?: number }[];
    peers: { pathId: string; generation: number; peer: string; port: number; payloadDownload: number }[];
}

interface RouteResult {
    kind: "native" | "tunnel";
    pathId?: string;
    generation?: number;
    sourcePayloadUploadBytes: number;
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
    routes: RouteResult[];
    redundantPayloadBytes: number;
    evidence: string;
}

const CONTROL_SHA256 = "70322489c36a613eec5788688355fca26268a520d74e3f41ebb2d90c1c8beb0f";
const CONTROL_REVISION = "0b63c3d17373f6132ea211c9dcd4241284ccdfaf";
const WARMUP_RATE = 1024;
const TRANSFER_RATE = Number(process.env.QBUTT_BENCH_ROUTE_RATE ?? 96 * 1024);
const ROUNDS = Number(process.env.QBUTT_BENCH_ROUNDS ?? 4);
const MODES: Mode[] = ["upstream-native", "qbutt-native", "qbutt-one-tunnel", "qbutt-mixed"];
const baselineExecutable = resolve(process.env.QBUTT_BENCH_BASELINE_EXE ?? "");
const qbuttExecutable = resolve(process.env.QBUTT_BENCH_QBUTT_EXE ?? "");
const python = resolve(process.env.QBUTT_LAB_PYTHON ?? "");
const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE ?? "";
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS ?? "";

assert(process.env.QBUTT_BENCH_BASELINE_EXE && process.env.QBUTT_BENCH_QBUTT_EXE && process.env.QBUTT_LAB_PYTHON,
    "Set QBUTT_BENCH_BASELINE_EXE, QBUTT_BENCH_QBUTT_EXE and QBUTT_LAB_PYTHON");
assert(nativeInterface && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
assert(Number.isInteger(ROUNDS) && ROUNDS >= 3 && ROUNDS <= 9, "QBUTT_BENCH_ROUNDS must be between 3 and 9");
assert(Number.isInteger(TRANSFER_RATE) && TRANSFER_RATE >= 32 * 1024 && TRANSFER_RATE <= 512 * 1024,
    "QBUTT_BENCH_ROUTE_RATE must be between 32 and 512 KiB/s");
assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal),
"Native benchmark address must belong to the selected non-loopback IPv4 interface");
assert((await stat(baselineExecutable)).isFile() && (await stat(qbuttExecutable)).isFile()
    && (await stat(python)).isFile(), "A benchmark executable is missing");
const [baselineHash, qbuttHash] = await Promise.all([
    readFile(baselineExecutable).then(sha256), readFile(qbuttExecutable).then(sha256),
]);
assert(baselineHash === CONTROL_SHA256,
    `The upstream control must be the validated ${CONTROL_REVISION} binary (${CONTROL_SHA256})`);
assert(baselineHash !== qbuttHash, "The qbutt executable must be distinct from the upstream control");

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

async function awaitCompletion(lab: Awaited<ReturnType<typeof createLab>>, hash: string) {
    const latencies: number[] = [];
    const deadline = Date.now() + 120000;
    for (;;) {
        const started = performance.now();
        const info = await lab.info(hash);
        latencies.push(performance.now() - started);
        if (info.progress === 1)
            return latencies;
        if (Date.now() >= deadline)
            throw new Error(`Timed transfer did not complete; last verified count ${info.completed}`);
        await Bun.sleep(100);
    }
}

async function run(mode: Mode, round: number): Promise<RunResult> {
    const executable = mode === "upstream-native" ? baselineExecutable : qbuttExecutable;
    process.env.QBUTT_LAB_EXE = executable;
    process.env.QBUTT_LAB_PYTHON = python;
    process.env.QBUTT_LAB_APP_NAME = mode === "upstream-native" ? "qBittorrent" : "qbutt";
    const lab = await createLab(`benchmark-${mode}-${round}`);
    const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
    const exactPayloadBytes = lab.manifest.payload.reduce((sum, file) => sum + file.size, 0);
    const tunnelCount = mode === "qbutt-mixed" ? 2 : mode === "qbutt-one-tunnel" ? 1 : 0;
    const routeCount = mode === "qbutt-mixed" ? 3 : 1;
    const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
    const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
    const subsetBytes: number[] = [];
    const credentials = Array.from({ length: tunnelCount }, () => ({
        username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex"),
    }));
    const assignedRoutes: { pathId: string; generation: number; edgeId: string; native: boolean }[] = [];
    let failure: unknown;
    let cleanupPromise: Promise<void> | undefined;
    const cleanup = () => cleanupPromise ??= (async () => {
        const results = await Promise.allSettled([
            lab.shutdown(), ...proxies.map(proxy => proxy.close()), ...seeds.map(seed => seed.stop()),
        ]);
        const failures = results.filter((result): result is PromiseRejectedResult => result.status === "rejected");
        if (failures.length)
            throw new AggregateError(failures.map(result => result.reason), "Benchmark process cleanup failed");
    })();
    try {
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
            if (!native) {
                const syntheticHost = `127.0.0.${side + 2}`;
                proxies.push(await startProxy({ ...credentials[side]!, targets: [{
                    host: syntheticHost, port: seed.port, connectHost: seed.host, connectPort: seed.port,
                }] }));
            }
        }
        assert(subsetBytes.reduce((sum, bytes) => sum + bytes, 0) === exactPayloadBytes,
            "Complementary benchmark seeds do not cover the exact payload once");

        await lab.start();
        const appVersion = await (await lab.request("app/version")).text();
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
                    edgeId: `benchmark-edge-${side}`, interfaceName: "Loopback Pseudo-Interface 1" });
                await waitFor("benchmark tunnel open", () => lab.json<PathsStatus>("qbuttPaths/status"),
                    status => !status.busy && status.paths.filter(path => path.open).length === side + 1);
            }
            await lab.request("qbuttPaths/policy", mode === "qbutt-mixed"
                ? { mode: "mixed", nativeInterface } : { mode: "pinned" });
        }

        const destination = join(lab.root, "target");
        const preparationStarted = performance.now();
        const hash = await lab.add(torrent.name, destination);
        const preparationMilliseconds = performance.now() - preparationStarted;
        const torrentStarted = performance.now();
        await lab.request("torrents/start", { hashes: hash });
        const setupStarted = performance.now();
        const endpoints = seeds.map((seed, side) => mode === "qbutt-mixed" && side === routeCount - 1
            ? `${nativeAddress}:${seed.port}` : mode === "upstream-native" || mode === "qbutt-native"
                ? `${nativeAddress}:${seed.port}` : `127.0.0.${side + 2}:${seed.port}`);
        if (mode === "upstream-native" || mode === "qbutt-native") {
            await lab.request("torrents/addPeers", { hashes: hash, peers: endpoints[0]! });
            await waitFor("direct Native peer warmup", () => lab.json<{
                peers: Record<string, { downloaded: number; ip: string; port: number }>;
            }>(`sync/torrentPeers?hash=${hash}&rid=0`), status => Object.values(status.peers).some(peer =>
                peer.downloaded > 0 && peer.ip === nativeAddress && peer.port === seeds[0]!.port), 60000);
        }
        else {
            const paths = await lab.json<PathsStatus>("qbuttPaths/status");
            const expected = endpoints.map((endpoint, side) => {
                const [peer, port] = endpoint.split(":");
                const path = paths.paths.find(candidate => mode === "qbutt-mixed" && side === routeCount - 1
                    ? candidate.edgeId === "native" && candidate.localAddress === nativeAddress
                    : candidate.edgeId === `benchmark-edge-${side}`);
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
        const connectionSetupMilliseconds = performance.now() - setupStarted;
        const warmupVerifiedBytes = (await lab.info(hash)).completed;
        assert(warmupVerifiedBytes < exactPayloadBytes, "Warmup completed the benchmark payload before measurement");
        await Promise.all(seeds.map(seed => seed.setUploadRate(TRANSFER_RATE)));
        const completionStarted = performance.now();
        const uiLatencies = await awaitCompletion(lab, hash);
        const completed = performance.now();
        const measurementMilliseconds = completed - completionStarted;
        const endToEndCompletionMilliseconds = completed - torrentStarted;
        const measuredVerifiedBytes = exactPayloadBytes - warmupVerifiedBytes;
        const verifiedBytesPerSecond = measuredVerifiedBytes / (measurementMilliseconds / 1000);
        const endToEndVerifiedBytesPerSecond = exactPayloadBytes / (endToEndCompletionMilliseconds / 1000);
        await lab.request("torrents/stop", { hashes: hash });
        await waitFor("benchmark target stopped", () => lab.info(hash), info => info.state === "stoppedUP");
        assert(await verifyPayload(destination, lab.manifest.payload) === exactPayloadBytes,
            "Benchmark target failed exact size or SHA-256 verification");

        const status = tunnelCount > 0
            ? await waitFor("closed tunnel payload attribution", () => lab.json<PathsStatus>("qbuttPaths/status"),
                current => assignedRoutes.every((assigned, side) => assigned.native
                    || (current.paths.find(candidate => candidate.pathId === assigned.pathId
                        && candidate.generation === assigned.generation)?.closedPayloadDownload ?? 0) >= subsetBytes[side]!),
            10000) : undefined;
        if (status) {
            assert(assignedRoutes.length === routeCount, "Managed benchmark routes were not fully assigned");
            for (const [side, assigned] of assignedRoutes.entries()) {
                const path = status.paths.find(candidate => candidate.pathId === assigned.pathId
                    && candidate.generation === assigned.generation && candidate.edgeId === assigned.edgeId);
                assert(path, "Managed route telemetry disappeared before result capture");
                if (!assigned.native)
                    assert((path.closedPayloadDownload ?? 0) >= subsetBytes[side]!,
                        "Tunnel engine payload attribution omits part of its complementary subset");
            }
            await lab.request("qbuttPaths/stop", {});
            await waitFor("benchmark qbutt-net shutdown", () => lab.json<PathsStatus>("qbuttPaths/status"),
                stopped => !stopped.busy && stopped.paths.every(path => !path.open));
        }
        await lab.shutdown();
        const stoppedSeeds = await Promise.all(seeds.map(seed => seed.stop()));
        await Promise.all(proxies.map(proxy => proxy.close()));
        const routes: RouteResult[] = stoppedSeeds.map((seed, side) => {
            const native = mode !== "qbutt-one-tunnel" && (mode !== "qbutt-mixed" || side === routeCount - 1);
            const assigned = assignedRoutes[side];
            const path = status?.paths.find(candidate => candidate.pathId === assigned?.pathId
                && candidate.generation === assigned.generation);
            return {
                kind: native ? "native" : "tunnel", pathId: assigned?.pathId, generation: assigned?.generation,
                sourcePayloadUploadBytes: seed.uploadPayloadBytes, enginePayloadDownload: path?.closedPayloadDownload,
                ...(!native ? { relay: { ...proxies[side]!.stats } } : {}),
            };
        });
        assert(stoppedSeeds.every((seed, side) => seed.peerAddresses.length === 1
            && seed.peerAddresses[0] === (routes[side]!.kind === "native" ? nativeAddress : "127.0.0.1")),
        "A seed observed an unexpected source address");
        const sourcePayloadBytes = routes.reduce((sum, route) => sum + route.sourcePayloadUploadBytes, 0);
        assert(sourcePayloadBytes >= exactPayloadBytes,
            "The only controlled sources uploaded fewer payload bytes than the exact verified target");
        const redundantPayloadBytes = sourcePayloadBytes - exactPayloadBytes;
        const result: RunResult = {
            mode, round, executableSha256: mode === "upstream-native" ? baselineHash : qbuttHash,
            appVersion, exactPayloadBytes, warmupVerifiedBytes, measuredVerifiedBytes,
            preparationMilliseconds, connectionSetupMilliseconds, measurementMilliseconds,
            endToEndCompletionMilliseconds, verifiedBytesPerSecond, endToEndVerifiedBytesPerSecond,
            uiProbeMilliseconds: summarizeLatencies(uiLatencies), routes,
            redundantPayloadBytes, evidence: join(lab.root, "evidence.json"),
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
    control: { revision: CONTROL_REVISION, executable: baselineExecutable, executableSha256: baselineHash },
    qbutt: { executable: qbuttExecutable, executableSha256: qbuttHash },
    topology: {
        rounds: ROUNDS,
        orderByRound: Array.from({ length: ROUNDS }, (_, round) => orderForRound(round + 1)),
        routeUploadLimitBytesPerSecond: TRANSFER_RATE,
        warmupUploadLimitBytesPerSecond: WARMUP_RATE,
        nativeInterface,
        nativeAddress,
        modes: MODES,
    },
    limits: [
        "Generated deterministic v1 payload and controlled TCP peers on one Windows host",
        "Each timed window begins after every required peer supplies payload at a 1 KiB/s warmup cap and acknowledges the measured cap",
        "Each route has the same application payload cap; Mixed has additional complementary reachability and aggregate capacity",
        "Verified bytes are exact-size and SHA-256 checked; relay stream bytes include protocol data and are not wire bytes",
        "No public swarm, public egress, UDP/uTP/QUIC, inbound, packet capture, netem, disk throttle, CPU/RAM/I/O or physical last-mile claim",
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
    const nativeRegressionPercent = 100 * (medians["upstream-native"] - medians["qbutt-native"])
        / medians["upstream-native"];
    const mixedGainPercent = 100 * (medians["qbutt-mixed"] - medians["qbutt-one-tunnel"])
        / medians["qbutt-one-tunnel"];
    assert(nativeRegressionPercent <= 5, `qbutt Native median regression is ${nativeRegressionPercent.toFixed(2)}%`);
    assert(mixedGainPercent > 0, `Mixed median did not exceed one tunnel (${mixedGainPercent.toFixed(2)}%)`);
    evidence.status = "passed";
    evidence.finishedAt = new Date().toISOString();
    evidence.summary = { medianVerifiedBytesPerSecond: medians, medianEndToEndCompletionMilliseconds: medianCompletionMilliseconds,
        nativeRegressionPercent, mixedGainPercent,
        gates: { nativeRegressionAtMostFivePercent: true, mixedExceedsOneTunnel: true } };
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
