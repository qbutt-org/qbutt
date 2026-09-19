// Controlled remote-host ingress through a public gateway lease. Never mutates observer routing or firewall.
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
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
    preparedLab = await createLab("gateway-wan");
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
let failure: unknown;
try {
    remoteRoot = await run([...ssh, "mktemp -d /tmp/qbutt-gateway-XXXXXXXX"]);
    assert(/^\/tmp\/qbutt-gateway-[a-zA-Z0-9]{8}$/.test(remoteRoot), "Unexpected observer temporary path");
    assert.equal(await run([...ssh, "uname -m"]), "x86_64", "Observer architecture is not the pinned Linux x64 target");
    const certificates = join(lab.root, "certificates");
    await mkdir(certificates);
    const certificate = JSON.parse(await run(["go", "run", join(import.meta.dir, "gateway-certificates.go"),
        certificates, observerIP])) as { fingerprint: string };
    assert.match(certificate.fingerprint, /^[0-9a-f]{64}$/);
    const files = [gatewayLinux, join(import.meta.dir, "gateway-wan-ports.py"),
        join(import.meta.dir, "wan-seed.py"), ...["ca.pem", "server.pem", "server-key.pem", "client.pem", "client-key.pem"]
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
        maxTTLSeconds: 120, maxUDPPacketsPerSecond: 64, maxUDPBytesPerSecond: 1024 * 1024,
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
    proxy = await startProxy({ ...credentials, remoteAddress: observerIP, targets: [{ host: observerIP, port: ports.control,
        connectHost: observerIP, connectPort: ports.control }] });
    const nodeConfig = join(lab.root, "node.json");
    await writeFile(nodeConfig, JSON.stringify({ proxies: [{ name: "wan-gateway", type: "socks5",
        server: proxy.host, port: proxy.port, username: credentials.username, password: credentials.password,
        udp: false }] }));
    await lab.start();
    const interfaces = await lab.json<{ name: string; value: string }[]>("app/networkInterfaceList");
    const physical = interfaces.filter(item => item.name === nativeInterface || item.value === nativeInterface);
    assert.equal(physical.length, 1, "Physical adapter has no unique application mapping");
    await lab.request("qbuttPaths/gateway", { controlAddress: `${observerIP}:${ports.control}`,
        datagramAddress: "", serverName: observerIP, caPath: join(certificates, "ca.pem"),
        certificatePath: join(certificates, "client.pem"), privateKeyPath: join(certificates, "client-key.pem"),
        port: String(ports.listener), tcp: "true", udp: "false" });
    await lab.request("qbuttPaths/open", { configPath: nodeConfig, proxyName: "wan-gateway",
        edgeId: "wan-gateway", interfaceName: physical[0]!.value });
    const readStatus = () => lab.json<Status>("qbuttPaths/status");
    const leased = await waitFor("remote public gateway lease", readStatus, status => !status.busy
        && status.pinned && status.paths.length === 1 && status.paths[0]!.gateway.state === "leased", 30000);
    const path = leased.paths[0]!;
    assert(path.open && path.gateway.family === "ipv4" && path.gateway.tcp && !path.gateway.udp
        && path.gateway.publicEndpoint === `${observerIP}:${ports.listener}`);

    const payload = randomBytes(512 * 1024);
    const source = join(lab.root, "source");
    const destination = join(lab.root, "download");
    await mkdir(source);
    await writeFile(join(source, "wan.bin"), payload);
    const torrent = join(lab.root, "wan.torrent");
    const hash = await run([lab.python, "-c", `
import libtorrent as lt, pathlib, sys
assert lt.__version__ == "2.0.14.0"
files = lt.file_storage()
files.add_file("wan.bin", ${payload.length})
creator = lt.create_torrent(files, 65536, lt.create_torrent.v1_only)
creator.set_comment("Generated legal qbutt gateway WAN fixture; no discovery")
lt.set_piece_hashes(creator, sys.argv[1])
encoded = lt.bencode(creator.generate())
pathlib.Path(sys.argv[2]).write_bytes(encoded)
print(lt.torrent_info(encoded).info_hashes().v1)
`, source, torrent]);
    assert.match(hash, /^[0-9a-f]{40}$/);
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

    peer = Bun.spawn([...ssh, `timeout --signal=TERM --kill-after=5s 240s python3 -u ${remoteRoot}/wan-seed.py`], {
        stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(lab.root, "observer-peer.stderr.log")), windowsHide: true });
    peerReplies = responses(peer);
    peer.stdin.write(JSON.stringify({ infoHash: hash, payload: payload.toString("base64"), pieceLength: 65536,
        count: 1, rate: 128 * 1024, duration: 240,
        connectTarget: { host: observerIP, port: ports.listener } }) + "\n");
    await peer.stdin.flush();
    const peerReady = await peerReplies.next("remote first-peer readiness", 30000);
    assert(peerReady.ready && Array.isArray(peerReady.ports) && peerReady.ports.length === 0
        && peerReady.pieceCount === payload.length / 65536);
    peer.stdin.write('{"command":"start","rate":131072}\n');
    await peer.stdin.flush();
    assert((await peerReplies.next("remote peer start")).started);
    const inbound = await waitFor("observer-first trusted ingress", readStatus, status => status.peers.some(candidate =>
        candidate.pathId === path.pathId && candidate.generation === path.generation
        && candidate.infoHash === hash && candidate.payloadDownload > 0), 45000);
    const inboundPeer = inbound.peers.find(candidate => candidate.infoHash === hash)!;
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
        errors: unknown[] };
    assert(observed.stopped && observed.errors.length === 0 && observed.connections.length === 1);
    const connection = observed.connections[0]!;
    assert(connection.side === 0 && connection.payloadBytes >= downloaded.length
        && connection.requestedPieces.length === payload.length / 65536,
    "Observer did not upload the complete generated torrent");
    assert.equal(`${inboundPeer.peer}:${inboundPeer.port}`, connection.localEndpoint,
        "Home libtorrent lost the original remote peer endpoint");
    assert(inboundPeer.pathId === path.pathId && inboundPeer.generation === path.generation,
        "Trusted ingress was assigned to the wrong path generation");
    const metered = await waitFor("WAN gateway transport counters", readStatus, status => {
        const wire = status.paths.find(item => item.pathId === path.pathId)?.wire;
        return Boolean(wire && wire.relayDownloadBytes > 0 && wire.relayUploadBytes > 0
            && wire.carrierDownloadBytes > 0 && wire.carrierUploadBytes > 0);
    });
    const wire = metered.paths.find(item => item.pathId === path.pathId)!.wire!;
    assert(wire.carrierDownloadPackets === 0 && wire.carrierUploadPackets === 0
        && wire.relayDownloadCopies === 0, "TCP-only WAN fixture reported UDP traffic");
    await lab.checkpoint({ check: "observer-first-public-lease-to-home-libtorrent", observer,
        gatewayRevision: sourceLock.qbuttNet.commit, publicEndpoint: path.gateway.publicEndpoint,
        originalRemoteEndpoint: connection.localEndpoint, pathId: path.pathId, generation: path.generation,
        verifiedBytes: downloaded.length, sha256: sha256(downloaded), wire,
        thirdPartyReachabilityProven: false, udpProven: false });
    peer.stdin.end();
    assert.equal(await peer.exited, 0, "Remote peer did not exit cleanly");
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
    peerReplies?.close();
    gatewayReplies?.close();
    try { await proxy?.close(); } catch (error) { failure ??= error; }
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    if (/^\/tmp\/qbutt-gateway-[a-zA-Z0-9]{8}$/.test(remoteRoot)) {
        const names = ["qbutt-gateway-linux", "gateway-wan-ports.py", "wan-seed.py", "ca.pem", "server.pem",
            "server-key.pem", "client.pem", "client-key.pem", "gateway.json"];
        try { await run([...ssh, `rm -f -- ${names.map(name => `${remoteRoot}/${name}`).join(" ")} && rmdir -- ${remoteRoot}`]); }
        catch (error) { failure ??= error; }
    }
    try {
        const root = await realpath(lab.root);
        for (const name of ["source", "download", "fixtures", "certificates",
            "qbutt-gateway-linux", "gateway.json", "node.json", "wan.torrent"]) {
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
