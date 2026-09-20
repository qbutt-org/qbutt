import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { cp, mkdtemp, mkdir, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, verifyPayload, waitFor } from "../lab";
import { allowLabNetwork } from "../windows-firewall";
import { startProxy } from "./proxy";

const PROTOCOL = 4;
const GATEWAY_PROTOCOL = 2;
const PUBLIC_FIXTURE_ADDRESS = "1.0.0.2";
const LOOPBACK_INTERFACE = 1;
const useUtp = process.argv.includes("--utp");

interface GatewayState {
    state: "leased" | "outgoing-only";
    publicEndpoint?: string;
    family?: "ipv4" | "ipv6";
    tcp: boolean;
    udp: boolean;
    expiresUnixMilli?: number;
}

interface PathState {
    pathId: string;
    generation: number;
    open: boolean;
    gateway: GatewayState;
    wire?: WireState;
}

interface WireState {
    relayDownloadBytes: number;
    relayUploadBytes: number;
    carrierDownloadBytes: number;
    carrierUploadBytes: number;
    carrierDownloadPackets: number;
    carrierUploadPackets: number;
    relayDownloadCopies: number;
}

interface PeerState {
    pathId: string;
    generation: number;
    infoHash: string;
    peer: string;
    port: number;
    payloadDownload: number;
}

interface PathStatus {
    busy: boolean;
    open: boolean;
    pinned: boolean;
    processId: number;
    paths: PathState[];
    peers: PeerState[];
    diagnostics: { routes: { pathId: string; generation: number; verifiedDownload: number }[] };
}

interface TraceEntry {
    direction: "request" | "response" | "event";
    keys: string[];
    method?: string;
    version?: number;
    pathId?: string;
    generation?: number;
    gatewayKeys?: string[];
    resultKeys?: string[];
    errorKeys?: string[];
    code?: string;
    messageMatchesCode?: boolean;
    publicEndpoint?: string;
    remote?: string;
    reason?: string;
    expiresUnixMilli?: number;
    tcp?: boolean;
    udp?: boolean;
}

async function run(command: string[], options: { cwd?: string; env?: Record<string, string>; timeout?: number } = {}) {
    const child = Bun.spawn(command, {
        cwd: options.cwd,
        env: { ...process.env, ...options.env },
        stdout: "pipe", stderr: "pipe", windowsHide: true,
        timeout: options.timeout ?? 120000,
    });
    const [exitCode, stdout, stderr] = await Promise.all([
        child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ]);
    assert.equal(exitCode, 0, `${command[0]} exited ${exitCode}: ${stderr || stdout}`);
    return stdout.trim();
}

async function addLoopbackAddress(add: boolean): Promise<void> {
    const powershell = Bun.which("pwsh.exe");
    assert(powershell, "PowerShell 7 is required for the gateway lab");
    const script = add ? String.raw`
$ErrorActionPreference = 'Stop'
$interface = Get-NetIPInterface -InterfaceIndex $env:QBUTT_GATEWAY_LAB_INTERFACE -AddressFamily IPv4 -ErrorAction Stop
if ($interface.InterfaceAlias -notlike '*Loopback*') { throw 'Controlled public fixture interface is not the Windows loopback.' }
$existing = Get-NetIPAddress -AddressFamily IPv4 -IPAddress $env:QBUTT_GATEWAY_LAB_ADDRESS -ErrorAction SilentlyContinue
if ($existing) { throw 'The controlled public fixture address is already assigned; refusing to borrow it.' }
try {
New-NetIPAddress -InterfaceIndex $env:QBUTT_GATEWAY_LAB_INTERFACE -IPAddress $env:QBUTT_GATEWAY_LAB_ADDRESS ` +
        String.raw`-PrefixLength 32 -AddressFamily IPv4 -PolicyStore ActiveStore -SkipAsSource $true | Out-Null
$active = Get-NetIPAddress -InterfaceIndex $env:QBUTT_GATEWAY_LAB_INTERFACE -IPAddress $env:QBUTT_GATEWAY_LAB_ADDRESS -ErrorAction Stop
if ($active.PrefixLength -ne 32) { throw 'Controlled public fixture address validation failed.' }
}
catch {
    Get-NetIPAddress -InterfaceIndex $env:QBUTT_GATEWAY_LAB_INTERFACE -IPAddress $env:QBUTT_GATEWAY_LAB_ADDRESS -ErrorAction SilentlyContinue |
        Remove-NetIPAddress -Confirm:$false
    throw
}
` : String.raw`
$ErrorActionPreference = 'Stop'
Get-NetIPAddress -InterfaceIndex $env:QBUTT_GATEWAY_LAB_INTERFACE -IPAddress $env:QBUTT_GATEWAY_LAB_ADDRESS -ErrorAction SilentlyContinue |
    Remove-NetIPAddress -Confirm:$false
`;
    await run([powershell, "-NoProfile", "-NonInteractive", "-Command", script], { env: {
        QBUTT_GATEWAY_LAB_ADDRESS: PUBLIC_FIXTURE_ADDRESS,
        QBUTT_GATEWAY_LAB_INTERFACE: String(LOOPBACK_INTERFACE),
    } });
}

async function probeTcpUdpPort(host: string): Promise<number> {
    for (let attempt = 0; attempt < 128; ++attempt) {
        const port = 49152 + (randomBytes(2).readUInt16BE(0) % 16384);
        const tcp = createServer();
        const udp = createSocket("udp4");
        try {
            await new Promise<void>((accept, reject) => {
                tcp.once("error", reject);
                tcp.listen(port, host, accept);
            });
            await new Promise<void>((accept, reject) => {
                udp.once("error", reject);
                udp.bind(port, host, accept);
            });
            await new Promise<void>((accept, reject) => tcp.close(error => error ? reject(error) : accept()));
            await new Promise<void>(accept => udp.close(accept));
            return port;
        }
        catch {
            if (tcp.listening)
                await new Promise<void>(accept => tcp.close(() => accept()));
            try { udp.close(); }
            catch {}
        }
    }
    throw new Error("Could not find an unused controlled TCP/UDP gateway port");
}

const wrapperSource = String.raw`
import { appendFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const real = process.env.QBUTT_REAL_NET;
const trace = process.env.QBUTT_GATEWAY_TRACE;
if (!real || !trace) process.exit(64);
const pending = new Map();
function keys(value) { return Object.keys(value).sort(); }
function record(line, direction) {
    let value;
    try { value = JSON.parse(line); } catch { return; }
    const entry = { direction, keys: keys(value), version: value.v };
    if (direction === "request") {
        entry.method = value.method;
        entry.pathId = value.pathId;
        entry.generation = value.generation;
        if (value.gateway) entry.gatewayKeys = keys(value.gateway);
        if (Number.isSafeInteger(value.id)) pending.set(value.id, {
            method: value.method, pathId: value.pathId, generation: value.generation,
        });
    }
    else if (value.id === 0) {
        entry.direction = "event";
        entry.method = value.event;
        entry.pathId = value.pathId;
        entry.generation = value.generation;
        entry.publicEndpoint = value.publicEndpoint;
        entry.remote = value.remote;
        entry.reason = value.reason;
    }
    else {
        const request = pending.get(value.id) || {};
        pending.delete(value.id);
        entry.method = request.method;
        entry.pathId = request.pathId;
        entry.generation = request.generation;
        if (value.result && typeof value.result === "object") {
            entry.resultKeys = keys(value.result);
            for (const name of ["publicEndpoint", "expiresUnixMilli", "tcp", "udp"])
                if (Object.hasOwn(value.result, name)) entry[name] = value.result[name];
        }
        if (value.error && typeof value.error === "object") {
            entry.errorKeys = keys(value.error);
            entry.code = value.error.code;
            entry.messageMatchesCode = value.error.message === value.error.code;
        }
    }
    if ((entry.method || "").startsWith("gateway.") || entry.method === "incomingTcp" || entry.method === "gatewayClosed")
        appendFileSync(trace, JSON.stringify(entry) + "\n");
}
const child = spawn(real, process.argv.slice(2), { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
const requests = createInterface({ input: process.stdin });
requests.on("line", line => { record(line, "request"); child.stdin.write(line + "\n"); });
requests.on("close", () => child.stdin.end());
const responses = createInterface({ input: child.stdout });
responses.on("line", line => { record(line, "response"); process.stdout.write(line + "\n"); });
child.stderr.pipe(process.stderr);
child.on("exit", code => process.exit(code === null ? 1 : code));
`;

const inboundSeedSource = String.raw`
import json, pathlib, sys, threading, time
import libtorrent as lt
if lt.__version__ != "2.0.14.0": raise RuntimeError("Unexpected libtorrent fixture binding")
torrent, save_path, host, port, listen_port = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3], int(sys.argv[4]), int(sys.argv[5])
utp = sys.argv[6] == "utp"
session = lt.session({"listen_interfaces":f"127.0.0.1:{listen_port}", "outgoing_interfaces":"127.0.0.1",
 "enable_dht":False,"enable_lsd":False,"enable_upnp":False,"enable_natpmp":False,
 "enable_incoming_utp":utp,"enable_outgoing_utp":utp,"enable_incoming_tcp":False,"enable_outgoing_tcp":not utp,
 "alert_mask":lt.alert.category_t.error_notification | lt.alert.category_t.peer_notification | lt.alert.category_t.connect_notification,
 "dht_bootstrap_nodes":"","upload_rate_limit":65536,"ignore_limits_on_local_network":False,"connections_limit":4})
params=lt.add_torrent_params(); params.ti=lt.torrent_info(str(torrent)); params.save_path=str(save_path)
params.flags &= ~lt.torrent_flags.auto_managed; params.flags &= ~lt.torrent_flags.paused
handle=session.add_torrent(params)
deadline=time.monotonic()+30
while not handle.status().is_seeding:
 if time.monotonic()>deadline: raise RuntimeError("Inbound seed did not verify generated payload")
 for alert in session.pop_alerts():
  if isinstance(alert,(lt.torrent_error_alert,lt.listen_failed_alert)): raise RuntimeError(alert.message())
 time.sleep(.05)
handle.connect_peer((host,port))
print(json.dumps({"ready":True,"target":f"{host}:{port}","verifiedPayloadBytes":handle.status().total_done}),flush=True)
done=threading.Event(); threading.Thread(target=lambda:(sys.stdin.read(),done.set()),daemon=True).start()
peers=set(); local_endpoints=set(); peer_errors=[]
while not done.wait(.05):
 info=handle.get_peer_info()
 peers.update(peer.ip[0] for peer in info)
 local_endpoints.update(f"{peer.local_endpoint[0]}:{peer.local_endpoint[1]}" for peer in info)
 for alert in session.pop_alerts():
  if isinstance(alert,lt.torrent_error_alert): raise RuntimeError(alert.message())
  if isinstance(alert,(lt.peer_error_alert,lt.peer_disconnected_alert)) and len(peer_errors)<32: peer_errors.append(alert.message())
status=handle.status()
print(json.dumps({"uploadPayloadBytes":status.total_payload_upload,"downloadPayloadBytes":status.total_payload_download,
 "peerAddresses":sorted(peers),"localEndpoints":sorted(local_endpoints),"peerErrors":peer_errors}),flush=True)
del handle; del session
`;

async function firstLine(reader: ReadableStreamDefaultReader<Uint8Array>, label: string): Promise<{ line: string; rest: string }> {
    let text = "";
    const deadline = Date.now() + 35000;
    while (!text.includes("\n")) {
        assert(Date.now() < deadline, `${label} readiness timed out`);
        let timer: ReturnType<typeof setTimeout>;
        let result: ReadableStreamReadResult<Uint8Array>;
        try {
            result = await Promise.race([reader.read(), new Promise<never>((_, reject) => {
                timer = setTimeout(() => reject(new Error(`${label} readiness timed out`)), deadline - Date.now());
            })]);
        }
        finally { clearTimeout(timer!); }
        assert(!result.done, `${label} ended before readiness`);
        text += new TextDecoder().decode(result.value);
    }
    const newline = text.indexOf("\n");
    return { line: text.slice(0, newline), rest: text.slice(newline + 1) };
}

async function startInboundSeed(python: string, root: string, fixtures: string, endpoint: string) {
    const separator = endpoint.lastIndexOf(":");
    assert(separator > 0, "Gateway returned an invalid public endpoint");
    const source = join(root, "inbound-seed.py");
    await writeFile(source, inboundSeedSource);
    const listenPort = await probeTcpUdpPort("127.0.0.1");
    const child = Bun.spawn([python, source, join(fixtures, "v1.torrent"), join(fixtures, "seed"),
        endpoint.slice(0, separator), endpoint.slice(separator + 1), String(listenPort), useUtp ? "utp" : "tcp"], {
        stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(root, "inbound-seed.stderr.log")), windowsHide: true,
    });
    const reader = child.stdout.getReader();
    let readyLine: { line: string; rest: string };
    let ready: { ready: boolean; target: string; verifiedPayloadBytes: number };
    try {
        readyLine = await firstLine(reader, "Inbound seed");
        ready = JSON.parse(readyLine.line);
        assert(ready.ready && ready.target === endpoint && ready.verifiedPayloadBytes > 0,
            "Inbound seed readiness mismatch");
    }
    catch (error) {
        try { await reader.cancel(); } catch {}
        try { reader.releaseLock(); } catch {}
        try { child.stdin.end(); } catch {}
        child.kill();
        await child.exited;
        throw error;
    }
    return {
        ready,
        async stop() {
            child.stdin.end();
            const timeout = setTimeout(() => child.kill(), 15000);
            try { assert.equal(await child.exited, 0, "Inbound seed exited unsuccessfully"); }
            finally { clearTimeout(timeout); }
            let text = readyLine.rest;
            for (;;) {
                const result = await reader.read();
                if (result.done) break;
                text += new TextDecoder().decode(result.value);
            }
            reader.releaseLock();
            await writeFile(join(root, "inbound-seed.jsonl"), readyLine.line + "\n" + text);
            return JSON.parse(text.trimEnd().split("\n").at(-1)!) as {
                uploadPayloadBytes: number; downloadPayloadBytes: number; peerAddresses: string[]; localEndpoints: string[];
            };
        },
    };
}

function expectKeys(actual: string[] | undefined, expected: string[], label: string) {
    assert.deepEqual(actual, [...expected].sort(), `${label} schema mismatch`);
}

const originalExecutable = process.env.QBUTT_LAB_EXE;
const gatewaySource = process.env.QBUTT_LAB_GATEWAY_SOURCE;
assert(originalExecutable && gatewaySource, "Set QBUTT_LAB_EXE and QBUTT_LAB_GATEWAY_SOURCE");
assert.equal(process.env.QBUTT_LAB_APP_NAME ?? "qbutt", "qbutt", "Gateway integration requires the qbutt application");
assert((await stat(originalExecutable)).isFile(), "qbutt executable is missing");
assert((await stat(join(dirname(originalExecutable), "qbutt-net.exe"))).isFile(), "Portable bundle has no qbutt-net.exe");

const sourceLock = JSON.parse(await readFile(resolve(import.meta.dir, "../..", "upstream-lock.json"), "utf8")) as {
    qbuttNet: { repository: string; commit: string };
};
const gatewayRevision = await run(["git", "rev-parse", "HEAD"], { cwd: gatewaySource });
assert.equal(gatewayRevision, sourceLock.qbuttNet.commit, "Gateway source checkout differs from pinned qbutt-net revision");
assert.equal(await run(["git", "status", "--porcelain"], { cwd: gatewaySource }), "",
    "Gateway source checkout is not fully clean");

const bundle = await mkdtemp(join(tmpdir(), "qbutt-gateway-app-"));
await cp(dirname(originalExecutable), bundle, { recursive: true, filter: path => {
    if (["profile", ".git"].includes(basename(path))) return false;
    return basename(path) === basename(originalExecutable)
        || !extname(path) || [".dll", ".qm", ".json"].includes(extname(path).toLowerCase());
} });
const wrapper = join(bundle, "qbutt-net.exe");
const realNet = join(bundle, "qbutt-net-real.exe");
await run(["go", "build", "-mod=readonly", "-trimpath", "-o", realNet, "./cmd/qbutt-net"],
    { cwd: gatewaySource, timeout: 180000 });
const wrapperFile = join(bundle, "qbutt-net-trace.ts");
await writeFile(wrapperFile, wrapperSource);
await allowLabNetwork([process.execPath]);
await run([process.execPath, "build", "--compile", wrapperFile, "--outfile", wrapper]);
process.env.QBUTT_LAB_EXE = join(bundle, basename(originalExecutable));

const lab = await createLab(useUtp ? "gateway-utp" : "gateway");
const tracePath = join(lab.root, "gateway-v4-trace.jsonl");
process.env.QBUTT_REAL_NET = realNet;
process.env.QBUTT_GATEWAY_TRACE = tracePath;
const gatewayExecutable = join(lab.root, "qbutt-gateway.exe");
await run(["go", "build", "-mod=readonly", "-trimpath", "-o", gatewayExecutable, "./cmd/qbutt-gateway"],
    { cwd: gatewaySource, timeout: 180000 });
await allowLabNetwork([realNet, gatewayExecutable]);
await lab.checkpoint({
    check: "pinned-gateway-binaries", qbuttNetCommit: sourceLock.qbuttNet.commit,
    qbuttNetSha256: sha256(await readFile(realNet)), gatewaySha256: sha256(await readFile(gatewayExecutable)),
    recorderSha256: sha256(await readFile(wrapper)),
});

let failure: unknown;
let aliasAdded = false;
let gateway: ReturnType<typeof Bun.spawn> | undefined;
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let seed: Awaited<ReturnType<typeof startInboundSeed>> | undefined;
try {
    const certificates = join(lab.root, "gateway-certificates");
    await mkdir(certificates);
    const certificateResult = JSON.parse(await run(["go", "run", join(import.meta.dir, "gateway-certificates.go"),
        certificates, "127.0.0.1"])) as { fingerprint: string };
    assert.match(certificateResult.fingerprint, /^[0-9a-f]{64}$/);

    await addLoopbackAddress(true);
    aliasAdded = true;
    const publicPort = await probeTcpUdpPort(PUBLIC_FIXTURE_ADDRESS);
    const gatewayConfig = join(lab.root, "gateway.json");
    await writeFile(gatewayConfig, JSON.stringify({
        controlAddress: "127.0.0.1:0", datagramAddress: "127.0.0.1:0",
        listenerIP: PUBLIC_FIXTURE_ADDRESS, advertiseIP: PUBLIC_FIXTURE_ADDRESS, allowedPorts: [publicPort],
        maxClients: 1, maxLeases: 4, maxTCPPerLease: 8, maxTCP: 16, maxTTLSeconds: 120,
        maxUDPPacketsPerSecond: 128, maxUDPBytesPerSecond: 8 * 1024 * 1024,
        clientCertificateSHA256: certificateResult.fingerprint,
        certificate: join(certificates, "server.pem"), privateKey: join(certificates, "server-key.pem"),
        clientCA: join(certificates, "ca.pem"),
    }));
    gateway = Bun.spawn([gatewayExecutable, "--config", gatewayConfig, "--stdio"], {
        stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(lab.root, "gateway.stderr.log")), windowsHide: true,
    });
    const gatewayReader = gateway.stdout.getReader();
    const readyLine = await firstLine(gatewayReader, "Gateway");
    gatewayReader.releaseLock();
    const ready = JSON.parse(readyLine.line) as { ready: boolean; control: string; datagrams: string; protocol: number };
    assert(ready.ready && ready.protocol === GATEWAY_PROTOCOL, "Gateway carrier protocol readiness mismatch");
    const separator = ready.control.lastIndexOf(":");
    assert(separator > 0);
    const controlHost = ready.control.slice(0, separator);
    const controlPort = Number(ready.control.slice(separator + 1));
    const credentials = { username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") };
    proxy = await startProxy({ ...credentials, udp: useUtp, targets: [{
        host: controlHost, port: controlPort, connectHost: controlHost, connectPort: controlPort,
    }, ...(useUtp ? [{ host: "127.0.0.1", port: Number(ready.datagrams.split(":").at(-1)) }] : [])] });
    const nodeConfig = join(lab.root, "controlled-gateway-node.json");
    await writeFile(nodeConfig, JSON.stringify({ proxies: [{
        name: "gateway-fixture", type: "socks5", server: proxy.host, port: proxy.port,
        username: credentials.username, password: credentials.password, udp: useUtp,
    }] }));
    await lab.start();
    await lab.request("qbuttPaths/gateway", {
        controlAddress: ready.control, datagramAddress: useUtp ? ready.datagrams : "", serverName: "127.0.0.1",
        caPath: join(certificates, "ca.pem"), certificatePath: join(certificates, "client.pem"),
        privateKeyPath: join(certificates, "client-key.pem"), port: String(publicPort),
        tcp: String(!useUtp), udp: String(useUtp),
    });
    if (useUtp)
        await lab.request("app/setPreferences", { json: JSON.stringify({ bittorrent_protocol: 2 }) });
    const pathRequest = { configPath: nodeConfig, proxyName: "gateway-fixture", interfaceName: "Loopback Pseudo-Interface 1" };
    const readStatus = () => lab.json<PathStatus>("qbuttPaths/status");
    await lab.request("qbuttPaths/open", pathRequest);
    const opened = await waitFor("application gateway open", readStatus, status => !status.busy
        && status.paths.length === 1 && status.paths[0]!.gateway.state === "leased");
    const firstPath = opened.paths[0]!;
    assert(opened.open && opened.pinned && firstPath.open
        && firstPath.gateway.tcp === !useUtp && firstPath.gateway.udp === useUtp
        && firstPath.gateway.family === "ipv4");
    const firstEndpoint = firstPath.gateway.publicEndpoint!;
    const firstExpiry = firstPath.gateway.expiresUnixMilli!;
    assert(firstEndpoint === `${PUBLIC_FIXTURE_ADDRESS}:${publicPort}` && firstExpiry > Date.now());

    const destination = join(lab.root, "downloads");
    const hash = await lab.add("v1", destination);
    await lab.request("torrents/start", { hashes: hash });
    seed = await startInboundSeed(lab.python, lab.root, lab.fixtures, firstEndpoint);
    const ingress = await waitFor("trusted libtorrent ingress", readStatus, status => status.peers.some(peer =>
        peer.pathId === firstPath.pathId && peer.generation === firstPath.generation
        && peer.infoHash === hash && peer.payloadDownload > 0), 45000);
    const ingressPeer = ingress.peers.find(peer => peer.infoHash === hash)!;
    const renewed = await waitFor("gateway lease renewal", readStatus, status => {
        const path = status.paths.find(item => item.pathId === firstPath.pathId);
        return !!path && !status.busy && path.generation === firstPath.generation
            && path.gateway.state === "leased" && path.gateway.expiresUnixMilli! > firstExpiry;
    }, 50000);
    const renewedPath = renewed.paths.find(path => path.pathId === firstPath.pathId)!;
    assert.equal(renewedPath.gateway.publicEndpoint, firstEndpoint, "Renewal changed public endpoint");
    await waitFor("inbound-only payload completion", () => lab.info(hash), info => info.progress === 1, 90000);
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("verified torrent stop", () => lab.info(hash), info => info.state === "stoppedUP");
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    const seedFinal = await seed.stop();
    seed = undefined;
    assert(seedFinal.uploadPayloadBytes >= verifiedBytes && seedFinal.downloadPayloadBytes === 0,
        "Independent inbound seed did not account for the verified payload");
    const originalPeer = `${ingressPeer.peer}:${ingressPeer.port}`;
    assert(seedFinal.localEndpoints.includes(originalPeer),
        "qbutt peer telemetry did not preserve the independent seed's exact source endpoint");
    const metered = await waitFor("real gateway wire counters", readStatus, status => {
        const wire = status.paths.find(path => path.pathId === firstPath.pathId)?.wire;
        return Boolean(wire && wire.relayDownloadBytes > 0 && wire.relayUploadBytes > 0
            && wire.carrierDownloadBytes > 0 && wire.carrierUploadBytes > 0);
    });
    const wire = metered.paths.find(path => path.pathId === firstPath.pathId)!.wire!;
    const credited = metered.diagnostics.routes.find(route => route.pathId === firstPath.pathId
        && route.generation === firstPath.generation);
    assert.equal(credited?.verifiedDownload, verifiedBytes, "Ingress verified credit differs from exact file bytes");
    assert(metered.diagnostics.routes.every(route => route.pathId === firstPath.pathId || route.verifiedDownload === 0),
        "Another path received credit for gateway ingress");
    if (useUtp) {
        assert(wire.carrierDownloadPackets > 0 && wire.carrierUploadPackets > 0,
            "uTP ingress did not traverse the gateway datagram carrier");
        assert(proxy.stats.uploadDatagramBytes > 0 && proxy.stats.downloadDatagramBytes > 0,
            "Gateway datagrams did not traverse the selected SOCKS adapter");
    }
    else
        assert(wire.carrierDownloadPackets === 0 && wire.carrierUploadPackets === 0
            && wire.relayDownloadCopies === 0, "TCP-only gateway reported UDP packet or fanout counters");
    await lab.checkpoint({
        check: "real-gateway-trusted-ingress-and-renewal", protocol: PROTOCOL,
        qbuttNetCommit: sourceLock.qbuttNet.commit, pathId: firstPath.pathId, generation: firstPath.generation,
        publicEndpoint: firstEndpoint, firstExpiry, renewedExpiry: renewedPath.gateway.expiresUnixMilli,
        trustedPeer: { address: ingressPeer.peer, port: ingressPeer.port, payloadDownload: ingressPeer.payloadDownload },
        verifiedBytes, engineVerifiedBytes: credited!.verifiedDownload, seedUploadPayloadBytes: seedFinal.uploadPayloadBytes, wire,
        controlledGatewayInboundProven: true, publicInternetInboundProven: false,
        peerProtocol: useUtp ? "utp" : "tcp",
    });

    await lab.request("qbuttPaths/stop", { pathId: firstPath.pathId });
    const explicitlyStopped = await waitFor("explicit gateway path stop", readStatus,
        status => !status.busy && status.pinned && status.paths.some(path => path.pathId === firstPath.pathId
            && !path.open && path.gateway.state === "outgoing-only"));
    assert(!explicitlyStopped.open && explicitlyStopped.processId === opened.processId && explicitlyStopped.processId > 0,
        "Selected-edge stop changed ownership of the reusable transport child");
    await lab.request("qbuttPaths/open", pathRequest);
    const reopened = await waitFor("gateway reopen", readStatus, status => !status.busy
        && status.paths.length === 1 && status.paths[0]!.gateway.state === "leased");
    const secondPath = reopened.paths[0]!;
    assert(secondPath.generation > firstPath.generation, "Explicit reopen reused a retired generation");

    gateway.stdin.end();
    assert.equal(await gateway.exited, 0, "Controlled gateway did not stop cleanly");
    gateway = undefined;
    const rolled = await waitFor("terminal gateway event rollover", readStatus, status => !status.busy
        && status.paths.length === 1 && status.paths[0]!.generation > secondPath.generation
        && status.paths[0]!.gateway.state === "outgoing-only", 20000);
    const thirdPath = rolled.paths[0]!;
    assert(rolled.open && rolled.pinned && thirdPath.open, "Terminal gateway event did not retain a managed outgoing path");
    await lab.checkpoint({
        check: "explicit-close-and-terminal-event-rollover",
        stoppedGeneration: firstPath.generation, reopenedGeneration: secondPath.generation,
        rolledGeneration: thirdPath.generation, terminalGatewayState: thirdPath.gateway.state,
    });

    await lab.request("qbuttPaths/stop", { pathId: thirdPath.pathId });
    await waitFor("final path stop", readStatus, status => !status.busy && status.paths.some(path =>
        path.pathId === thirdPath.pathId && !path.open && path.gateway.state === "outgoing-only"));
    await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
    await waitFor("final torrent removal", () => lab.json<unknown[]>("torrents/info"), torrents => torrents.length === 0);
    await lab.request("qbuttPaths/native", {});
    assert(!(await readStatus()).pinned, "Empty session did not return to Native");
    await lab.shutdown();
    assert.equal(await verifyPayload(destination, lab.manifest.payload), verifiedBytes, "Shutdown changed verified ingress payload");

    const trace = (await readFile(tracePath, "utf8")).trimEnd().split("\n").map(line => JSON.parse(line) as TraceEntry);
    assert(trace.length > 0 && trace.every(entry => entry.version === PROTOCOL), "Gateway trace contains a non-v4 frame");
    const openRequest = trace.find(entry => entry.direction === "request" && entry.method === "gateway.open"
        && entry.generation === firstPath.generation)!;
    expectKeys(openRequest.keys, ["v", "id", "method", "pathId", "generation", "gateway"], "gateway.open request");
    expectKeys(openRequest.gatewayKeys, ["controlAddress", "datagramAddress", "serverName", "caPath", "certificatePath",
        "privateKeyPath", "port", "tcp", "udp", "ttlSeconds"], "gateway.open payload");
    assert.equal(openRequest.version, PROTOCOL);
    const openResponse = trace.find(entry => entry.direction === "response" && entry.method === "gateway.open"
        && entry.generation === firstPath.generation && entry.resultKeys)!;
    expectKeys(openResponse.keys, ["v", "id", "result"], "gateway.open response");
    expectKeys(openResponse.resultKeys, ["pathId", "generation", "publicEndpoint", "tcp", "udp", "expiresUnixMilli",
        "relayHost", "relayPort"], "gateway.open result");
    assert(openResponse.publicEndpoint === firstEndpoint
        && openResponse.tcp === !useUtp && openResponse.udp === useUtp,
        "gateway.open result changed the requested transport capability");
    const renewalRequest = trace.find(entry => entry.direction === "request" && entry.method === "gateway.renew"
        && entry.generation === firstPath.generation)!;
    expectKeys(renewalRequest.keys, ["v", "id", "method", "pathId", "generation"], "gateway.renew request");
    const renewal = trace.find(entry => entry.direction === "response" && entry.method === "gateway.renew"
        && entry.generation === firstPath.generation && entry.resultKeys)!;
    expectKeys(renewal.keys, ["v", "id", "result"], "gateway.renew response");
    expectKeys(renewal.resultKeys, openResponse.resultKeys!, "gateway.renew result");
    assert.equal(renewal.publicEndpoint, firstEndpoint);
    assert(renewal.expiresUnixMilli! > firstExpiry);
    const incoming = trace.filter(entry => entry.direction === "event" && entry.method === "incomingTcp"
        && entry.generation === firstPath.generation);
    if (useUtp)
        assert.equal(incoming.length, 0, "UDP-only gateway accepted a TCP peer");
    else {
        assert(incoming.length > 0, "No trusted TCP ingress event");
        for (const event of incoming)
            expectKeys(event.keys, ["v", "id", "event", "pathId", "generation", "remote", "publicEndpoint",
                "relayHost", "relayPort", "relayToken"], "incomingTcp event");
        assert(incoming.some(event => event.remote === originalPeer && event.publicEndpoint === firstEndpoint),
            "No incomingTcp event preserved the payload seed's source and public endpoint");
    }
    const closeRequest = trace.find(entry => entry.direction === "request" && entry.method === "gateway.close"
        && entry.generation === firstPath.generation)!;
    expectKeys(closeRequest.keys, ["v", "id", "method", "pathId", "generation"], "gateway.close request");
    const closed = trace.find(entry => entry.direction === "response" && entry.method === "gateway.close"
        && entry.generation === firstPath.generation && entry.resultKeys)!;
    expectKeys(closed.keys, ["v", "id", "result"], "gateway.close response");
    expectKeys(closed.resultKeys, [], "gateway.close result");
    const terminalEvents = trace.filter(entry => entry.direction === "event" && entry.method === "gatewayClosed");
    assert.equal(terminalEvents.length, 1, "Expected exactly one unexpected gateway retirement event");
    assert(!terminalEvents.some(entry => entry.generation === firstPath.generation),
        "Explicit gateway path close emitted a terminal event");
    const terminalEvent = terminalEvents[0]!;
    assert.equal(terminalEvent.generation, secondPath.generation,
        "Terminal event did not identify the unexpectedly retired generation");
    expectKeys(terminalEvent.keys, ["v", "id", "event", "pathId", "generation", "reason"], "gatewayClosed event");
    assert.equal(terminalEvent.reason, "gateway_closed", "Terminal gateway event exposed an unstable reason");
    const terminalIndex = trace.indexOf(terminalEvent);
    const rolloverOpenIndex = trace.findIndex(entry => entry.direction === "request" && entry.method === "gateway.open"
        && entry.generation === thirdPath.generation);
    assert(rolloverOpenIndex > terminalIndex, "Rollover gateway.open did not follow the terminal event");
    const failedRollover = trace.find(entry => entry.direction === "response" && entry.method === "gateway.open"
        && entry.generation === thirdPath.generation && entry.errorKeys)!;
    expectKeys(failedRollover.keys, ["v", "id", "error"], "failed rollover response");
    expectKeys(failedRollover.errorKeys, ["code", "message"], "failed rollover error");
    // Shutdown may race the next connection before or during its TLS handshake.
    assert(["gateway_connect_failed", "gateway_authentication_failed"].includes(failedRollover.code!)
        && failedRollover.messageMatchesCode, "Failed rollover did not return a canonical gateway startup error");
    await lab.checkpoint({ check: "exact-gateway-v4-frames", traceEntries: trace.length,
        openGeneration: firstPath.generation, renewalGeneration: firstPath.generation,
        terminalEventGeneration: secondPath.generation, rolloverOpenGeneration: thirdPath.generation,
        rolloverError: failedRollover.code });
}
catch (error) {
    failure = error;
    try { await lab.shutdown(); }
    catch (shutdownError) { console.error(String(shutdownError)); }
}
finally {
    try { if (seed) await seed.stop(); }
    catch (error) { if (!failure) failure = error; }
    try { await proxy?.close(); }
    catch (error) { if (!failure) failure = error; }
    try {
        if (gateway && gateway.exitCode === null) {
            gateway.stdin.end();
            const timeout = setTimeout(() => gateway?.kill(), 5000);
            try { await gateway.exited; }
            finally { clearTimeout(timeout); }
        }
    }
    catch (error) { if (!failure) failure = error; }
    if (aliasAdded) {
        try { await addLoopbackAddress(false); }
        catch (error) { if (!failure) failure = error; }
    }
    try {
        assert(lab.exitCode !== null, "App must stop before fixture cleanup");
        const resolvedRoot = await realpath(lab.root);
        for (const path of [lab.fixtures, join(lab.root, "profile"), join(lab.root, "downloads"),
            join(lab.root, "gateway-certificates"), join(lab.root, "controlled-gateway-node.json"),
            join(lab.root, "gateway.json"), gatewayExecutable]) {
            try {
                assert.equal(dirname(await realpath(path)), resolvedRoot, "Fixture cleanup escaped its lab root");
                await rm(path, { recursive: true, force: true });
            }
            catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
        }
        assert.equal(dirname(await realpath(bundle)), await realpath(tmpdir()), "Bundle cleanup escaped temp");
        await rm(bundle, { recursive: true, force: true });
    }
    catch (error) { if (!failure) failure = error; }
}
await lab.finish(failure);
if (failure) throw failure;
