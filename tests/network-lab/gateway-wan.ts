// Controlled remote-host ingress through a public gateway lease. Never mutates observer routing or firewall.
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { cp, mkdir, mkdtemp, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { isIPv4 } from "node:net";
import { networkInterfaces, tmpdir } from "node:os";
import { basename, dirname, join, sep } from "node:path";
import { createInterface } from "node:readline";
import { Readable } from "node:stream";
import { sha256 } from "../fixtures/generate";
import { createLab, waitFor } from "../lab";
import { allowLabNetwork } from "../windows-firewall";
import { startProxy } from "./proxy";

interface Status {
    busy: boolean;
    pinned: boolean;
    paths: { pathId: string; generation: number; open: boolean; gateway: {
        state: string; publicEndpoint?: string; family?: string; tcp: boolean; udp: boolean };
        wire?: { relayDownloadBytes: number; relayUploadBytes: number;
            carrierDownloadBytes: number; carrierUploadBytes: number;
            carrierDownloadPackets: number; carrierUploadPackets: number; relayDownloadCopies: number } }[];
    peers: { pathId: string; generation: number; infoHash: string; peer: string; port: number;
        payloadDownload: number }[];
}

const observer = process.env.QBUTT_WAN_OBSERVER ?? "";
const observerIP = process.env.QBUTT_WAN_OBSERVER_IP ?? "";
const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE ?? "";
const gatewaySource = process.env.QBUTT_LAB_GATEWAY_SOURCE ?? "";
const executable = process.env.QBUTT_LAB_EXE ?? "";
const sourceConfig = process.env.QBUTT_GATEWAY_WAN_PROXY_CONFIG ?? "";
const sourceName = process.env.QBUTT_GATEWAY_WAN_PROXY_NAME ?? "";
const independentSource = sourceConfig !== "" || sourceName !== "";
const useUtp = process.argv.includes("--utp");
const useDht = process.argv.includes("--dht");
if (process.argv.includes("--source-preflight")) {
    await sourcePreflight();
    process.exit(0);
}
assert(!useDht || useUtp, "Inbound DHT requires the UDP lease (--utp)");
assert(!useUtp || independentSource, "WAN UDP acceptance requires an independent selected VPN source");
assert(!independentSource || (sourceConfig !== "" && sourceName !== ""),
    "Set both QBUTT_GATEWAY_WAN_PROXY_CONFIG and QBUTT_GATEWAY_WAN_PROXY_NAME for one selected source node");
assert(/^[a-zA-Z0-9][a-zA-Z0-9_.-]*$/.test(observer) && isIPv4(observerIP),
    "Set QBUTT_WAN_OBSERVER and its numeric QBUTT_WAN_OBSERVER_IP");
assert(networkInterfaces()[nativeInterface]?.some(address => address.family === "IPv4" && !address.internal),
    "Set QBUTT_LAB_NATIVE_INTERFACE to an active physical IPv4 interface");
assert(gatewaySource && executable && (await stat(executable)).isFile(),
    "Set QBUTT_LAB_GATEWAY_SOURCE and QBUTT_LAB_EXE to the pinned source and portable application");
assert.equal(process.env.QBUTT_LAB_APP_NAME ?? "qbutt", "qbutt");

async function run(args: string[], options: { cwd?: string; env?: Record<string, string>; timeout?: number } = {}) {
    const child = Bun.spawn(args, { cwd: options.cwd, env: { ...process.env, ...options.env },
        stdout: "pipe", stderr: "pipe", timeout: options.timeout ?? 120000, windowsHide: true });
    const [code, out, err] = await Promise.all([child.exited, new Response(child.stdout).text(),
        new Response(child.stderr).text()]);
    assert.equal(code, 0, `${args[0]} failed (${code}): ${err.slice(0, 1000)}`);
    return out.trim();
}

function responses(child: ReturnType<typeof Bun.spawn>) {
    const lines = createInterface({ input: Readable.fromWeb(child.stdout as never) });
    const iterator = lines[Symbol.asyncIterator]();
    return {
        close: () => lines.close(),
        async next(label: string, timeout = 30000): Promise<any> {
            let timer: ReturnType<typeof setTimeout>;
            try {
                const result = await Promise.race([iterator.next(), new Promise<never>((_, reject) => {
                    timer = setTimeout(() => reject(new Error(`${label} timed out`)), timeout);
                })]);
                assert(!result.done, `${label} process closed before response`);
                return JSON.parse(result.value);
            }
            finally { clearTimeout(timer!); }
        },
    };
}

async function generatePayload(python: string, root: string) {
    const payload = randomBytes(512 * 1024);
    const source = join(root, "source");
    const torrent = join(root, "wan.torrent");
    await mkdir(source);
    await writeFile(join(source, "wan.bin"), payload);
    const hash = await run([python, "-c", `
import libtorrent as lt, pathlib, sys
assert lt.__version__ == "2.0.14.0"
files = lt.file_storage()
files.add_file("wan.bin", 524288)
creator = lt.create_torrent(files, 65536, lt.create_torrent.v1_only)
creator.set_comment("Generated legal qbutt gateway WAN fixture; no discovery")
lt.set_piece_hashes(creator, sys.argv[1])
encoded = lt.bencode(creator.generate())
pathlib.Path(sys.argv[2]).write_bytes(encoded)
print(lt.torrent_info(encoded).info_hashes().v1)
`, source, torrent]);
    assert.match(hash, /^[0-9a-f]{40}$/);
    return { payload, source, torrent, hash };
}

async function sourcePreflight() {
    const python = process.env.QBUTT_LAB_PYTHON;
    assert(python, "Set QBUTT_LAB_PYTHON to the pinned fixture interpreter");
    await allowLabNetwork([process.execPath, python]);
    const root = await mkdtemp(join(tmpdir(), "qbutt-wan-source-preflight-"));
    const children: ReturnType<typeof Bun.spawn>[] = [];
    const readers: ReturnType<typeof responses>[] = [];
    let relay: Awaited<ReturnType<typeof startProxy>> | undefined;
    let failure: unknown;
    let result: Record<string, unknown> = {};
    try {
        const { payload, source, torrent, hash } = await generatePayload(python, root);
        const spawnPeer = (receiver = false) => {
            const child = Bun.spawn([python, "-u", join(import.meta.dir, "wan-udp-peer.py"),
                ...(receiver ? ["--receive"] : [])], { stdin: "pipe", stdout: "pipe",
                stderr: Bun.file(join(root, receiver ? "receiver.stderr.log" : "source.stderr.log")), windowsHide: true });
            children.push(child);
            const replies = responses(child);
            readers.push(replies);
            return { child, replies };
        };
        const receiver = spawnPeer(true);
        receiver.child.stdin.write(JSON.stringify({ torrent, infoHash: hash, savePath: join(root, "download") }) + "\n");
        await receiver.child.stdin.flush();
        const ready = await receiver.replies.next("Local uTP receiver readiness");
        assert(ready.ready && ready.protocol === "utp" && Number.isInteger(ready.port), JSON.stringify(ready));
        // This synthetic destination is reachable only through the exact SOCKS
        // mapping. The receiver and authenticated SOCKS listener use loopback;
        // the source requires SOCKS for all peer traffic, with discovery disabled.
        const target = { host: "198.18.0.1", port: ready.port };
        const credentials = { username: "wan-preflight", password: randomBytes(24).toString("hex") };
        relay = await startProxy({ ...credentials, udp: true,
            targets: [{ ...target, connectHost: "127.0.0.1" }] });
        const seed = spawnPeer();
        seed.child.stdin.write(JSON.stringify({ torrent, infoHash: hash, savePath: source,
            connectTarget: target, connectProxy: { port: relay.port, ...credentials } }) + "\n");
        await seed.child.stdin.flush();
        const seedReady = await seed.replies.next("Local uTP source readiness");
        assert(seedReady.ready && seedReady.protocol === "utp" && seedReady.verifiedPayloadBytes === payload.length,
            JSON.stringify(seedReady));
        seed.child.stdin.write('{"command":"start"}\n');
        await seed.child.stdin.flush();
        assert((await seed.replies.next("Local uTP source start")).started);
        const received = await receiver.replies.next("Local proxied uTP payload", 45000);
        assert(received.complete && received.protocol === "utp" && received.verifiedPayloadBytes === payload.length
            && received.sha256 === sha256(payload) && received.incomingUtpPeerSeen, JSON.stringify(received));
        seed.child.stdin.write('{"command":"stop"}\n');
        await seed.child.stdin.flush();
        const sent = await seed.replies.next("Local uTP source final report");
        assert(sent.stopped && sent.protocol === "utp" && sent.uploadPayloadBytes >= payload.length
            && sent.downloadPayloadBytes === 0, JSON.stringify(sent));
        assert.deepEqual(sent.remoteEndpoints, [`${target.host}:${target.port}`]);
        assert(relay.stats.authenticatedConnections > 0 && relay.stats.deniedConnections === 0);
        for (const child of children) {
            child.stdin.end();
            assert.equal(await child.exited, 0, "Local uTP helper did not exit cleanly");
        }
        result = { verifiedBytes: payload.length, sha256: received.sha256, source: sent, receiver: received, target };
    }
    catch (error) { failure = error; }
    finally {
        for (const child of children) {
            if (child.exitCode === null) {
                child.stdin.end();
                const timer = setTimeout(() => child.kill(), 3000);
                try { await child.exited; } catch (error) { failure ??= error; }
                finally { clearTimeout(timer); }
            }
        }
        for (const reader of readers) reader.close();
        if (relay) result.proxy = { ...relay.stats };
        try { await relay?.close(); } catch (error) { failure ??= error; }
        try {
            const owned = await realpath(root);
            assert(dirname(owned) === await realpath(tmpdir()) && basename(owned).startsWith("qbutt-wan-source-preflight-"));
            for (const name of ["source", "download", "wan.torrent"])
                await rm(join(owned, name), { recursive: true, force: true });
        }
        catch (error) { failure ??= error; }
    }
    await writeFile(join(root, "evidence.json"), JSON.stringify({ suite: "wan-source-preflight",
        status: failure ? "failed" : "passed", ...result,
        error: failure ? String(failure) : undefined }, null, 2) + "\n");
    console.log(`WAN source preflight ${failure ? "failed" : "passed"}: ${root}`);
    if (failure) throw failure;
}

function observedLeasePeer(line: string, port: number, udp = false): string | undefined {
    const syn = (udp
        ? /\bIn\s+IP\s+([0-9.]+)\.([0-9]+)\s+>\s+([0-9.]+)\.([0-9]+): UDP, length/
        : /\bIn\s+IP\s+([0-9.]+)\.([0-9]+)\s+>\s+([0-9.]+)\.([0-9]+): Flags \[S\]/).exec(line);
    if (!syn || !isIPv4(syn[1]!) || (syn[3] !== observerIP) || (Number(syn[4]) !== port))
        return undefined;
    const sourcePort = Number(syn[2]);
    return (sourcePort > 0 && sourcePort <= 65535) ? `${syn[1]}:${sourcePort}` : undefined;
}

async function captureOwnedPort(port: number, protocol: "tcp" | "utp" | "udp" = "tcp") {
    assert(Number.isInteger(port) && port >= 49152 && port <= 65535);
    const lines: string[] = [];
    // Fixed SLL2 (20) + IPv4 without options (20) + UDP (8) + uTP (20).
    // The snapshot cannot contain any BitTorrent payload beyond the uTP header.
    const filter = `host ${observerIP} and ${protocol === "tcp"
        ? `tcp port ${port} and (tcp[13] & 7 != 0)`
        : `udp port ${port}${protocol === "utp"
            ? " and ip[0] = 0x45 and ((ip[6:2] & 16383) = 0) and ((udp[8] & 15) = 1) and ((udp[8] & 240) <= 64)" : ""}`}`;
    const captureOptions = protocol === "utp" ? "-y LINUX_SLL2 -s 68 -xx -c 32" : "-s 96 -c 16";
    const child = Bun.spawn([...ssh, `timeout --signal=TERM --kill-after=2s 35s sudo -n tcpdump -i any -nn -tt -l ${captureOptions} '${filter}'`], {
        stdin: "ignore", stdout: "pipe", stderr: "pipe", windowsHide: true });
    const pump = (async () => {
        let summary = "";
        let hex = "";
        for await (const line of createInterface({ input: Readable.fromWeb(child.stdout as never) })) {
            if (protocol !== "utp") {
                if (lines.length < 16) lines.push(line);
                continue;
            }
            const bytes = /^\s+0x[0-9a-f]+:\s+([0-9a-f ]+)$/i.exec(line);
            if (!bytes) {
                summary = line;
                hex = "";
                continue;
            }
            hex += bytes[1]!.replaceAll(" ", "");
            if (hex.length !== 136 || lines.length >= 32) continue;
            const header = Buffer.from(hex, "hex").subarray(48);
            lines.push(`${summary} uTP type=${["DATA", "FIN", "STATE", "RESET", "SYN"][header[0]! >> 4]}`
                + ` connectionId=${header.readUInt16BE(2)} seq=${header.readUInt16BE(16)} ack=${header.readUInt16BE(18)}`);
        }
    })();
    const diagnostics: string[] = [];
    const ready = (async () => {
        for await (const line of createInterface({ input: Readable.fromWeb(child.stderr as never) })) {
            if (line.includes("listening on")) return;
            if (diagnostics.length < 4) diagnostics.push(line);
        }
        throw new Error(`Scoped packet observer exited before readiness: ${diagnostics.join(" | ")}`);
    })();
    let timer: ReturnType<typeof setTimeout>;
    try {
        await Promise.race([ready, new Promise<never>((_, reject) => {
            timer = setTimeout(() => reject(new Error("Scoped packet observer readiness timed out")), 7000);
        })]);
    }
    catch (error) {
        if (child.exitCode === null) child.kill();
        await child.exited;
        await pump;
        throw error;
    }
    finally { clearTimeout(timer!); }
    return { port, protocol, child, lines, pump };
}

async function stopCapture(capture: Awaited<ReturnType<typeof captureOwnedPort>>) {
    if (capture.child.exitCode === null) capture.child.kill();
    await capture.child.exited;
    await capture.pump;
    return capture.lines;
}

const sourceLock = JSON.parse(await readFile(join(import.meta.dir, "../..", "upstream-lock.json"), "utf8")) as {
    qbuttNet: { commit: string } };
assert.equal(await run(["git", "rev-parse", "HEAD"], { cwd: gatewaySource }), sourceLock.qbuttNet.commit,
    "Gateway source does not match the pinned qbutt-net revision");
assert.equal(await run(["git", "status", "--porcelain"], { cwd: gatewaySource }), "",
    "Gateway source checkout must be fully clean");

const bundle = await mkdtemp(join(tmpdir(), "qbutt-gateway-wan-bundle-"));
async function removeBundle() {
    const owned = await realpath(bundle);
    const temp = await realpath(tmpdir());
    assert(owned.startsWith(temp + sep) && basename(owned).startsWith("qbutt-gateway-wan-bundle-"));
    await rm(owned, { recursive: true, force: true });
}
const netExecutable = join(bundle, "qbutt-net.exe");
let preparedLab: Awaited<ReturnType<typeof createLab>> | undefined;
try {
    await cp(dirname(executable), bundle, { recursive: true,
        filter: path => !["profile", ".git"].includes(basename(path)) });
    await run(["go", "build", "-mod=readonly", "-trimpath", "-o", netExecutable, "./cmd/qbutt-net"],
        { cwd: gatewaySource, timeout: 180000 });
    process.env.QBUTT_LAB_EXE = join(bundle, basename(executable));
    await allowLabNetwork([process.execPath, netExecutable]);
    preparedLab = await createLab(`gateway-wan${useUtp ? "-utp" : ""}${useDht ? "-dht" : ""}`,
        { protocol: useUtp ? "UTP" : "TCP" });
    await run(["go", "build", "-mod=readonly", "-trimpath", "-o", join(preparedLab.root, "qbutt-gateway-linux"),
        "./cmd/qbutt-gateway"], { cwd: gatewaySource,
        env: { GOOS: "linux", GOARCH: "amd64", CGO_ENABLED: "0" }, timeout: 180000 });
}
catch (error) {
    try { if (preparedLab) await preparedLab.finish(error); }
    finally { await removeBundle(); }
    throw error;
}
const lab = preparedLab;
const gatewayLinux = join(lab.root, "qbutt-gateway-linux");
const ssh = ["ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10",
    "-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=3", observer];
const scp = ["scp", "-q", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"];
let remoteRoot = "";
let gateway: ReturnType<typeof Bun.spawn> | undefined;
let gatewayReplies: ReturnType<typeof responses> | undefined;
let peer: ReturnType<typeof Bun.spawn> | undefined;
let peerReplies: ReturnType<typeof responses> | undefined;
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let bootstrap: ReturnType<typeof createSocket> | undefined;
let sourceChild: ReturnType<typeof Bun.spawn> | undefined;
let sourceReplies: ReturnType<typeof responses> | undefined;
let sourceEndpoint: { host: string; port: number; configuredServerId: string;
    socksUsername: string; socksPassword: string } | undefined;
let sourceCheck: ReturnType<typeof Bun.spawn> | undefined;
let sourceCheckReplies: ReturnType<typeof responses> | undefined;
let packetCapture: Awaited<ReturnType<typeof captureOwnedPort>> | undefined;
let sourcePacketCapture: Awaited<ReturnType<typeof captureOwnedPort>> | undefined;
let failure: unknown;
try {
    remoteRoot = await run([...ssh, "mktemp -d /tmp/qbutt-gateway-XXXXXXXX"]);
    assert(/^\/tmp\/qbutt-gateway-[a-zA-Z0-9]{8}$/.test(remoteRoot), "Unexpected observer temporary path");
    assert.equal(await run([...ssh, "uname -m"]), "x86_64", "Observer architecture is not the pinned Linux x64 target");
    let homeSSHOrigin: string | undefined;
    if (independentSource) {
        let document: { proxies?: Record<string, unknown>[] };
        try { document = Bun.YAML.parse(await readFile(sourceConfig, "utf8")) as typeof document; }
        catch { throw new Error("Cannot parse selected source configuration; contents are intentionally omitted"); }
        assert(Array.isArray(document.proxies), "Selected source configuration has no proxies");
        const selected = document.proxies.filter(item => item.name === sourceName);
        assert.equal(selected.length, 1, "Select exactly one named source adapter");
        const selectedFile = join(lab.root, "source-node.json");
        await writeFile(selectedFile, JSON.stringify({ proxies: [{ ...selected[0], name: "wan-source" }] }));
        sourceChild = Bun.spawn([netExecutable, "--stdio"], { stdin: "pipe", stdout: "pipe",
            stderr: Bun.file(join(lab.root, "source-child.stderr.log")), windowsHide: true });
        sourceReplies = responses(sourceChild);
        const requestSource = async (id: number, method: string, fields: object = {}) => {
            sourceChild!.stdin.write(JSON.stringify({ v: 6, id, method, ...fields }) + "\n");
            await sourceChild!.stdin.flush();
            const reply = await sourceReplies!.next(`selected source ${method}`);
            assert.equal(reply.id, id);
            assert.equal(reply.v, 6);
            assert(!reply.error, `Selected source ${method} failed: ${reply.error?.code}`);
            return reply.result;
        };
        const hello = await requestSource(1, "hello");
        assert.equal(hello.protocol, 6);
        const listed = await requestSource(2, "list", { configPath: selectedFile, proxyName: "wan-source" });
        assert(Array.isArray(listed?.proxies) && listed.proxies.length === 1
            && listed.proxies[0]?.name === "wan-source"
            && /^[0-9a-f]{64}$/.test(listed.proxies[0]?.configuredServerId),
            "Selected WAN source did not expose one configured server identity");
        const configuredServerId = listed.proxies[0].configuredServerId;
        sourceEndpoint = await requestSource(3, "open", { configPath: selectedFile,
            proxyName: "wan-source", configuredServerId, pathId: "wan-source", generation: 1,
            interfaceName: nativeInterface, dns: { server: "1.1.1.1:53",
                bootstrapServer: "1.1.1.1:53", family: "ipv4" } });
        assert(sourceEndpoint && sourceEndpoint.configuredServerId === configuredServerId
            && sourceEndpoint.host === "127.0.0.1" && Number.isInteger(sourceEndpoint.port)
            && sourceEndpoint.port > 0 && sourceEndpoint.port <= 65535
            && typeof sourceEndpoint.socksUsername === "string"
            && typeof sourceEndpoint.socksPassword === "string");

        await run([...scp, join(import.meta.dir, "wan-source-check.py"), `${observer}:${remoteRoot}/`]);
        sourceCheck = Bun.spawn([...ssh, `timeout --signal=TERM --kill-after=2s 45s python3 -u ${remoteRoot}/wan-source-check.py server`], {
            stdin: "ignore", stdout: "pipe", stderr: Bun.file(join(lab.root, "source-check.stderr.log")), windowsHide: true });
        sourceCheckReplies = responses(sourceCheck);
        const checkReady = await sourceCheckReplies.next("observer source-check readiness", 15000) as {
            ready: boolean; port: number };
        assert(checkReady.ready && Number.isInteger(checkReady.port)
            && checkReady.port >= 49152 && checkReady.port <= 65535);
        sourcePacketCapture = await captureOwnedPort(checkReady.port);
        const checkClient = Bun.spawn([lab.python, "-u", join(import.meta.dir, "wan-source-check.py"), "client"], {
            stdin: "pipe", stdout: "pipe", stderr: "pipe", windowsHide: true });
        checkClient.stdin.write(JSON.stringify({ port: sourceEndpoint.port,
            username: sourceEndpoint.socksUsername, password: sourceEndpoint.socksPassword,
            targetIP: observerIP, targetPort: checkReady.port }) + "\n");
        checkClient.stdin.end();
        const [clientExit, clientOut, clientErr] = await Promise.all([checkClient.exited,
            new Response(checkClient.stdout).text(), new Response(checkClient.stderr).text()]);
        const flags = await stopCapture(sourcePacketCapture);
        sourcePacketCapture = undefined;
        const sourceStatus = await requestSource(4, "status") as { paths: { pathId: string;
            generation: number; wire: Record<string, number> }[] };
        assert(sourceStatus.paths.length === 1 && sourceStatus.paths[0]!.pathId === "wan-source"
            && sourceStatus.paths[0]!.generation === 1);
        await lab.checkpoint({ check: "selected-source-probe-transport", publicEndpoint: `${observerIP}:${checkReady.port}`,
            packets: flags, wire: sourceStatus.paths[0]!.wire });
        const accepted = await sourceCheckReplies.next("observer source-check accept", 1000)
            .catch(() => undefined) as { accepted: boolean; source: string } | undefined;
        const served = accepted?.accepted ? await sourceCheckReplies.next("observer source-check serve", 1000)
            .catch(() => undefined) as { served: boolean; source: string } | undefined : undefined;
        assert.equal(clientExit, 0, `Selected source check failed (observer accepted: ${Boolean(accepted)}, served: ${Boolean(served)}): ${clientErr.slice(0, 500)}`);
        assert(accepted?.accepted && served?.served && accepted.source === served.source,
            "Observer did not serve selected source check");
        assert.equal(await sourceCheck.exited, 0, "Observer source check did not exit cleanly");
        sourceCheck = undefined;
        sourceCheckReplies.close();
        sourceCheckReplies = undefined;
        const check = JSON.parse(clientOut) as { source: string };
        assert.equal(check.source, served.source, "Selected adapter changed the source-check endpoint");
        const sourceIP = /^([0-9.]+):[1-9][0-9]{0,4}$/.exec(check.source)?.[1];
        homeSSHOrigin = (await run([...ssh, "printf '%s' \"$SSH_CONNECTION\""])).split(/\s+/)[0]!;
        assert(sourceIP && isIPv4(sourceIP) && isIPv4(homeSSHOrigin));
        assert(sourceIP !== homeSSHOrigin && sourceIP !== observerIP,
            "Selected source adapter routed directly; independent VPN exit was not observed");
        await lab.checkpoint({ check: "selected-source-exit", sourceIP, homeSSHOrigin });
    }
    const certificates = join(lab.root, "certificates");
    await mkdir(certificates);
    const certificate = JSON.parse(await run(["go", "run", join(import.meta.dir, "gateway-certificates.go"),
        certificates, observerIP])) as { fingerprint: string };
    assert.match(certificate.fingerprint, /^[0-9a-f]{64}$/);
    const files = [gatewayLinux, join(import.meta.dir, "gateway-wan-ports.py"),
        ...(!independentSource ? [join(import.meta.dir, "wan-seed.py")] : []),
        ...["ca.pem", "server.pem", "server-key.pem", "client.pem", "client-key.pem"]
            .map(name => join(certificates, name))];
    await run([...scp, ...files, `${observer}:${remoteRoot}/`]);
    await run([...ssh, `chmod 700 ${remoteRoot}/qbutt-gateway-linux && chmod 600 ${remoteRoot}/*.pem`]);
    const ports = JSON.parse(await run([...ssh, `python3 ${remoteRoot}/gateway-wan-ports.py`])) as {
        control: number; datagrams: number; listener: number };
    assert(Object.values(ports).length === 3 && new Set(Object.values(ports)).size === 3
        && Object.values(ports).every(port => Number.isInteger(port) && port >= 49152 && port <= 65535));
    const remoteConfig = join(lab.root, "gateway.json");
    await writeFile(remoteConfig, JSON.stringify({ controlAddress: `0.0.0.0:${ports.control}`,
        datagramAddress: `0.0.0.0:${ports.datagrams}`, listenerIP: "0.0.0.0", advertiseIP: observerIP,
        allowedPorts: [ports.listener], maxClients: 1, maxLeases: 2, maxTCPPerLease: 4, maxTCP: 8,
        maxTTLSeconds: 120, maxUDPPacketsPerSecond: useUtp ? 256 : 64, maxUDPBytesPerSecond: 1024 * 1024,
        clientCertificateSHA256: certificate.fingerprint, certificate: `${remoteRoot}/server.pem`,
        privateKey: `${remoteRoot}/server-key.pem`, clientCA: `${remoteRoot}/ca.pem` }));
    await run([...scp, remoteConfig, `${observer}:${remoteRoot}/gateway.json`]);
    gateway = Bun.spawn([...ssh, `timeout --signal=TERM --kill-after=5s 240s ${remoteRoot}/qbutt-gateway-linux --config ${remoteRoot}/gateway.json --stdio`], {
        stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(lab.root, "observer-gateway.stderr.log")), windowsHide: true });
    gatewayReplies = responses(gateway);
    const ready = await gatewayReplies.next("remote gateway readiness") as {
        ready: boolean; control: string; datagrams: string; protocol: number };
    assert(ready.ready && ready.protocol === 2
        && ["0.0.0.0", "[::]"].some(host => ready.control === `${host}:${ports.control}`)
        && ["0.0.0.0", "[::]"].some(host => ready.datagrams === `${host}:${ports.datagrams}`),
    "Remote gateway did not bind the selected high control and datagram ports");
    const credentials = { username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") };
    if (useDht) {
        bootstrap = createSocket("udp4");
        await new Promise<void>((accept, reject) => {
            bootstrap!.once("error", reject);
            bootstrap!.bind(0, "127.0.0.1", accept);
        });
    }
    const bootstrapEndpoint = bootstrap ? `127.0.0.1:${bootstrap.address().port}` : "";
    proxy = await startProxy({ ...credentials, remoteAddress: observerIP, udp: useUtp,
        targets: [{ host: observerIP, port: ports.control, connectHost: observerIP, connectPort: ports.control },
            ...(useUtp ? [{ host: observerIP, port: ports.datagrams }] : []),
            ...(bootstrap ? [{ host: "127.0.0.1", port: bootstrap.address().port }] : [])] });
    const nodeConfig = join(lab.root, "node.json");
    await writeFile(nodeConfig, JSON.stringify({ proxies: [{ name: "wan-gateway", type: "socks5",
        server: proxy.host, port: proxy.port, username: credentials.username, password: credentials.password,
        udp: useUtp }] }));
    await lab.start();
    const interfaces = await lab.json<{ name: string; value: string }[]>("app/networkInterfaceList");
    const physical = interfaces.filter(item => item.name === nativeInterface || item.value === nativeInterface);
    assert.equal(physical.length, 1, "Physical adapter has no unique application mapping");
    await lab.request("qbuttPaths/gateway", { controlAddress: `${observerIP}:${ports.control}`,
        datagramAddress: useUtp ? `${observerIP}:${ports.datagrams}` : "", serverName: observerIP, caPath: join(certificates, "ca.pem"),
        certificatePath: join(certificates, "client.pem"), privateKeyPath: join(certificates, "client-key.pem"),
        port: String(ports.listener), tcp: String(!useUtp), udp: String(useUtp) });
    if (useDht)
        await lab.request("app/setPreferences", { json: JSON.stringify({ dht_bootstrap_nodes: bootstrapEndpoint, dht: false }) });
    await lab.request("qbuttPaths/open", { configPath: nodeConfig, proxyName: "wan-gateway",
        interfaceName: nativeInterface });
    const readStatus = () => lab.json<Status>("qbuttPaths/status");
    const leased = await waitFor("remote public gateway lease", readStatus, status => !status.busy
        && status.pinned && status.paths.length === 1 && status.paths[0]!.gateway.state === "leased", 30000);
    const path = leased.paths[0]!;
    assert(path.open && path.gateway.family === "ipv4" && path.gateway.tcp === !useUtp && path.gateway.udp === useUtp
        && path.gateway.publicEndpoint === `${observerIP}:${ports.listener}`);
    await lab.checkpoint({ check: "public-gateway-lease-ready", publicEndpoint: path.gateway.publicEndpoint,
        pathId: path.pathId, generation: path.generation });
    if (useDht) {
        await lab.request("app/setPreferences", { json: JSON.stringify({ dht: true }) });
        const preferences = await lab.json<{ dht: boolean; dht_bootstrap_nodes: string }>("app/preferences");
        assert(preferences.dht && preferences.dht_bootstrap_nodes === bootstrapEndpoint,
            "Inbound DHT fixture fell back to public bootstrap routers");
    }

    const { payload, source, torrent, hash } = await generatePayload(lab.python, lab.root);
    const destination = join(lab.root, "download");
    const form = new FormData();
    form.set("torrents", Bun.file(torrent));
    form.set("savepath", destination);
    form.set("stopped", "true");
    form.set("autoTMM", "false");
    form.set("contentLayout", "Original");
    await lab.request("torrents/add", form);
    await waitFor("gateway WAN torrent add", () => lab.json<{ hash: string }[]>("torrents/info"),
        torrents => torrents.length === 1 && torrents[0]!.hash === hash);
    await lab.request("torrents/start", { hashes: hash });

    if (independentSource && !useUtp) {
        packetCapture = await captureOwnedPort(ports.listener);
    }

    const peerCommand = independentSource
        ? [lab.python, "-u", join(import.meta.dir, useUtp ? "wan-udp-peer.py" : "wan-seed.py")]
        : [...ssh, `timeout --signal=TERM --kill-after=5s 240s python3 -u ${remoteRoot}/wan-seed.py`];
    peer = Bun.spawn(peerCommand, {
        stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(lab.root, "source-peer.stderr.log")), windowsHide: true });
    peerReplies = responses(peer);
    peer.stdin.write(JSON.stringify({ infoHash: hash,
        ...(useUtp ? { torrent, savePath: source } : { payload: payload.toString("base64"), pieceLength: 65536,
            count: 1, rate: 128 * 1024, duration: 240 }),
        connectTarget: { host: observerIP, port: ports.listener },
        ...(independentSource ? { connectProxy: { port: sourceEndpoint!.port,
            username: sourceEndpoint!.socksUsername, password: sourceEndpoint!.socksPassword } } : {}) }) + "\n");
    await peer.stdin.flush();
    const peerReady = await peerReplies.next("source peer readiness", 30000);
    assert(peerReady.ready && (useUtp
        ? peerReady.protocol === "utp" && peerReady.verifiedPayloadBytes === payload.length
        : Array.isArray(peerReady.ports) && peerReady.ports.length === 0 && peerReady.pieceCount === payload.length / 65536),
    `WAN source did not become ready: ${JSON.stringify(peerReady)}`);
    await lab.checkpoint({ check: "source-peer-ready", sourceProcess: independentSource ? "local" : "observer" });
    let observedSource = "";
    const observeSource = async (capture: Awaited<ReturnType<typeof captureOwnedPort>>) => {
        const endpoint = await waitFor("independent VPN source at public lease", async () => {
            const endpoints = [...new Set(capture.lines.filter(line => capture.protocol !== "utp" || line.includes("uTP type=SYN "))
                .map(line => observedLeasePeer(line, ports.listener,
                capture.protocol !== "tcp")).filter(Boolean))];
            assert(endpoints.length <= 1, "More than one source reached the owned public lease");
            return endpoints[0] ?? "";
        }, endpoint => endpoint !== "", 10000);
        const exitIP = endpoint.slice(0, endpoint.lastIndexOf(":"));
        assert(exitIP !== homeSSHOrigin && exitIP !== observerIP,
            "Source did not arrive from a VPN exit distinct from home and observer");
        return endpoint;
    };
    if (useDht) {
        sourcePacketCapture = await captureOwnedPort(ports.listener, "udp");
        peer.stdin.write('{"command":"dht"}\n');
        await peer.stdin.flush();
        const reply = await peerReplies.next("WAN inbound DHT replies", 30000) as {
            dht: { ping: { id: string; source: string }; get_peers: { id: string; source: string; token: boolean } } };
        assert(reply.dht && /^[0-9a-f]{40}$/.test(reply.dht.ping.id)
            && reply.dht.ping.id === reply.dht.get_peers.id && reply.dht.get_peers.token
            && reply.dht.ping.source === path.gateway.publicEndpoint && reply.dht.get_peers.source === path.gateway.publicEndpoint,
        "WAN DHT did not return correlated ping/get_peers replies from the public lease");
        const dhtSource = await observeSource(sourcePacketCapture);
        const packets = await stopCapture(sourcePacketCapture);
        sourcePacketCapture = undefined;
        await lab.checkpoint({ check: "wan-inbound-dht-through-independent-exit", publicEndpoint: path.gateway.publicEndpoint,
            pathId: path.pathId, generation: path.generation, dhtSource, homeSSHOrigin, packets, ...reply.dht });
    }
    if (useUtp)
        packetCapture = await captureOwnedPort(ports.listener, "utp");
    if (independentSource && !useUtp) {
        const connected = await peerReplies.next("VPN SOCKS connection", 25000);
        assert(connected.connected === true,
            `Source peer could not connect through Mihomo: ${JSON.stringify(connected.errors)}`);
        await lab.checkpoint({ check: "vpn-socks-connected", publicEndpoint: path.gateway.publicEndpoint });
        const handshake = await peerReplies.next("VPN BitTorrent handshake", 15000);
        assert(handshake.peerHandshake === true,
            `Source peer did not reach qbutt through the public lease: ${JSON.stringify(handshake.errors)}`);
        await lab.checkpoint({ check: "public-peer-handshake", pathId: path.pathId, generation: path.generation });
        observedSource = await observeSource(packetCapture!);
    }
    peer.stdin.write('{"command":"start","rate":131072}\n');
    await peer.stdin.flush();
    assert((await peerReplies.next("remote peer start")).started);
    await lab.checkpoint({ check: "source-peer-started" });
    const inbound = await waitFor("public gateway trusted ingress", readStatus, status => status.peers.some(candidate =>
        candidate.pathId === path.pathId && candidate.generation === path.generation
        && candidate.infoHash === hash && candidate.payloadDownload > 0), 45000);
    const inboundPeer = inbound.peers.find(candidate => candidate.infoHash === hash)!;
    if (useUtp)
        observedSource = await observeSource(packetCapture!);
    await waitFor("remote ingress payload completion", () => lab.info(hash), info => info.progress === 1, 90000);
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("remote ingress torrent stopped", () => lab.info(hash), info => info.state === "stoppedUP");
    const downloaded = await readFile(join(destination, "wan.bin"));
    assert.equal(downloaded.length, payload.length);
    assert.equal(sha256(downloaded), sha256(payload), "Remote ingress payload hash differs");
    peer.stdin.write('{"command":"stop"}\n');
    await peer.stdin.flush();
    const observed = await peerReplies.next("remote peer final report") as { stopped: boolean;
        connections: { side: number; localEndpoint: string; payloadBytes: number; requestedPieces: number[] }[];
        errors: unknown[]; protocol: string; uploadPayloadBytes: number; downloadPayloadBytes: number; remoteEndpoints: string[] };
    assert(observed.stopped);
    if (useUtp) {
        assert(observed.protocol === "utp" && observed.uploadPayloadBytes >= downloaded.length
            && observed.downloadPayloadBytes === 0, "Native uTP seed did not upload the verified payload");
        assert.deepEqual(observed.remoteEndpoints, [path.gateway.publicEndpoint]);
    }
    else {
        assert(observed.errors.length === 0 && observed.connections.length === 1);
        const connection = observed.connections[0]!;
        assert(connection.side === 0 && connection.payloadBytes >= downloaded.length
            && connection.requestedPieces.length === payload.length / 65536,
        "Source peer did not upload the complete generated torrent");
    }
    const originalRemoteEndpoint = independentSource ? observedSource : observed.connections[0]!.localEndpoint;
    assert.equal(`${inboundPeer.peer}:${inboundPeer.port}`, originalRemoteEndpoint,
        "Home libtorrent lost the original remote peer endpoint");
    assert(inboundPeer.pathId === path.pathId && inboundPeer.generation === path.generation,
        "Trusted ingress was assigned to the wrong path generation");
    const metered = await waitFor("WAN gateway transport counters", readStatus, status => {
        const wire = status.paths.find(item => item.pathId === path.pathId)?.wire;
        return Boolean(wire && wire.relayDownloadBytes > 0 && wire.relayUploadBytes > 0
            && wire.carrierDownloadBytes > 0 && wire.carrierUploadBytes > 0);
    });
    const wire = metered.paths.find(item => item.pathId === path.pathId)!.wire!;
    if (useUtp) {
        assert(wire.carrierDownloadPackets > 0 && wire.carrierUploadPackets > 0 && wire.relayDownloadCopies > 0,
            "WAN uTP did not traverse the gateway datagram carrier");
        assert(proxy.stats.uploadDatagramBytes > 0 && proxy.stats.downloadDatagramBytes > 0,
            "WAN gateway datagrams did not traverse the selected SOCKS adapter");
    }
    else
        assert(wire.carrierDownloadPackets === 0 && wire.carrierUploadPackets === 0
            && wire.relayDownloadCopies === 0, "TCP-only WAN fixture reported UDP traffic");
    await lab.checkpoint({ check: "public-gateway-ingress-to-home-libtorrent", observer,
        gatewayRevision: sourceLock.qbuttNet.commit, publicEndpoint: path.gateway.publicEndpoint,
        originalRemoteEndpoint, pathId: path.pathId, generation: path.generation,
        verifiedBytes: downloaded.length, sha256: sha256(downloaded), wire,
        sourceProcess: independentSource ? "local-through-vpn-exit" : "observer-local",
        independentVpnExitIngressProven: independentSource, homeSSHOrigin,
        thirdPartyReachabilityProven: false, peerProtocol: useUtp ? "utp" : "tcp", udpProven: useUtp,
        inboundDhtProven: useDht, sourcePeer: useUtp ? observed : undefined,
        carrierRoute: "Controlled relay uses the OS route; physical-interface bypass is not asserted." });
    peer.stdin.end();
    assert.equal(await peer.exited, 0, "Source peer did not exit cleanly");
    peer = undefined;
    peerReplies.close();
    peerReplies = undefined;
    gateway.stdin.end();
    assert.equal(await gateway.exited, 0, "Remote gateway did not stop cleanly");
    gateway = undefined;
    gatewayReplies.close();
    gatewayReplies = undefined;
    const rolled = await waitFor("remote gateway terminal retirement", readStatus, status => !status.busy
        && status.paths.length === 1 && status.paths[0]!.generation > path.generation
        && status.paths[0]!.gateway.state === "outgoing-only", 20000);
    assert(rolled.paths[0]!.open, "Terminal WAN rollover lost the managed outgoing path");
    await lab.request("qbuttPaths/stop", { pathId: path.pathId });
    await waitFor("WAN path stopped", readStatus, status => !status.busy
        && status.paths.some(item => item.pathId === path.pathId && !item.open));
    await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
    await waitFor("WAN torrent removed", () => lab.json<unknown[]>("torrents/info"), torrents => torrents.length === 0);
    await lab.request("qbuttPaths/native", {});
}
catch (error) { failure = error; }
finally {
    for (const child of [peer, gateway]) {
        if (child?.exitCode === null) {
            child.stdin.end();
            const timeout = setTimeout(() => child.kill(), 5000);
            try { await child.exited; } catch (error) { failure ??= error; }
            finally { clearTimeout(timeout); }
        }
    }
    if (packetCapture) {
        try {
            const packets = await stopCapture(packetCapture);
            await lab.checkpoint({ check: "owned-lease-peer-packets", publicEndpoint: `${observerIP}:${packetCapture.port}`,
                protocol: packetCapture.protocol, packets });
        }
        catch (error) { failure ??= error; }
    }
    if (sourcePacketCapture) {
        try { await stopCapture(sourcePacketCapture); } catch (error) { failure ??= error; }
    }
    if (sourceCheck) {
        if (sourceCheck.exitCode === null) sourceCheck.kill();
        try { await sourceCheck.exited; } catch (error) { failure ??= error; }
    }
    sourceCheckReplies?.close();
    if (sourceChild) {
        sourceChild.stdin.end();
        const timeout = setTimeout(() => sourceChild?.kill(), 5000);
        try { await sourceChild.exited; } catch (error) { failure ??= error; }
        finally { clearTimeout(timeout); }
    }
    sourceReplies?.close();
    peerReplies?.close();
    gatewayReplies?.close();
    try { await proxy?.close(); } catch (error) { failure ??= error; }
    try { if (bootstrap) await new Promise<void>(accept => bootstrap!.close(accept)); }
    catch (error) { failure ??= error; }
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    if (/^\/tmp\/qbutt-gateway-[a-zA-Z0-9]{8}$/.test(remoteRoot)) {
        const names = ["qbutt-gateway-linux", "gateway-wan-ports.py", "wan-seed.py", "wan-source-check.py", "ca.pem", "server.pem",
            "server-key.pem", "client.pem", "client-key.pem", "gateway.json"];
        try { await run([...ssh, `rm -f -- ${names.map(name => `${remoteRoot}/${name}`).join(" ")} && rmdir -- ${remoteRoot}`]); }
        catch (error) { failure ??= error; }
    }
    try {
        const root = await realpath(lab.root);
        for (const name of ["source", "download", "fixtures", "certificates",
            "qbutt-gateway-linux", "gateway.json", "node.json", "source-node.json", "wan.torrent"]) {
            const path = join(root, name);
            try {
                const owned = await realpath(path);
                assert(owned.startsWith(root + sep), "Generated payload cleanup escaped the lab root");
                await rm(owned, { recursive: true, force: true });
            }
            catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") failure ??= error; }
        }
        await removeBundle();
    }
    catch (error) { failure ??= error; }
    await lab.finish(failure);
}
if (failure) throw failure;
