import assert from "node:assert/strict";
import { createConnection } from "node:net";
import { networkInterfaces } from "node:os";
import { join } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";

interface Status {
    mode: string;
    paths: { pathId: string; generation: number; edgeId: string; open: boolean; localAddress?: string }[];
    peers: { infoHash: string; pathId: string; generation: number; peer: string;
        payloadDownload: number }[];
}
interface Peer { flags: string; downloaded: number }
interface Listeners { tcp: number[]; udp: number[] }
interface LogEntry { message: string }

const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE;
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS;
assert(nativeInterface && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal), "Select a local physical IPv4 address");
const transport = process.argv.includes("--utp") ? "utp" : "tcp";
const lab = await createLab(`managed-native-inbound-${transport}`);
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let failure: unknown;

async function listeners(): Promise<Listeners> {
    const probe = Bun.spawn(["pwsh.exe", "-NoProfile", "-NonInteractive", "-Command", String.raw`
$ErrorActionPreference = 'Stop'
$owned = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $env:QBUTT_LAB_EXE -and $_.CommandLine.Contains($env:QBUTT_INBOUND_PROFILE) })
if ($owned.Count -ne 1) { throw 'Could not identify the single isolated app process' }
$tcp = @(Get-NetTCPConnection -State Listen -OwningProcess $owned[0].ProcessId -ErrorAction SilentlyContinue | Where-Object LocalAddress -eq $env:QBUTT_INBOUND_ADDRESS | Select-Object -ExpandProperty LocalPort)
$udp = @(Get-NetUDPEndpoint -OwningProcess $owned[0].ProcessId -ErrorAction SilentlyContinue | Where-Object LocalAddress -eq $env:QBUTT_INBOUND_ADDRESS | Select-Object -ExpandProperty LocalPort)
@{ tcp = $tcp; udp = $udp } | ConvertTo-Json -Compress
`], { env: { ...process.env, QBUTT_INBOUND_PROFILE: join(lab.root, "profile"),
        QBUTT_INBOUND_ADDRESS: nativeAddress }, stdout: "pipe", stderr: "pipe",
        timeout: 10000, windowsHide: true });
    const [exit, output, error] = await Promise.all([probe.exited,
        new Response(probe.stdout).text(), new Response(probe.stderr).text()]);
    assert.equal(exit, 0, error);
    const result = JSON.parse(output) as Listeners;
    return { tcp: result.tcp.map(Number), udp: result.udp.map(Number) };
}

async function physicalListeners() {
    const result = await waitFor("actual managed physical TCP and UDP listeners", async () => {
        const bound = await listeners();
        const entries = await lab.json<LogEntry[]>(
            "log/main?normal=true&info=true&warning=true&critical=true&last_known_id=-1");
        const actualPort = (protocol: "TCP" | "UTP") => {
            const matches = entries.flatMap(entry => [...entry.message.matchAll(
                new RegExp(`IP: "${nativeAddress.replaceAll(".", "\\.")}"\\. Port: "${protocol}/(\\d+)"`, "g"))]);
            return Number(matches.at(-1)?.[1]);
        };
        return { bound, tcpPort: actualPort("TCP"), udpPort: actualPort("UTP") };
    }, value => value.bound.tcp.includes(value.tcpPort)
        && value.bound.udp.includes(value.udpPort), 30000);
    assert.equal(result.bound.tcp.length, 1, "Native TCP listener is ambiguous");
    return result;
}

function handshake(hash: string) {
    return Buffer.concat([Buffer.from([19]), Buffer.from("BitTorrent protocol"),
        Buffer.alloc(8), Buffer.from(hash, "hex"), Buffer.from("-QB0001-012345678901")]);
}

async function rejectedHandshake(port: number, hash: string) {
    const socket = createConnection({ host: nativeAddress, port, localAddress: nativeAddress });
    const closed = new Promise<void>(resolve => socket.once("close", resolve));
    try {
        await new Promise<void>((resolve, reject) => {
            socket.once("connect", resolve);
            socket.once("error", reject);
        });
        socket.on("error", () => {});
        socket.write(handshake(hash));
        assert(await Promise.race([closed.then(() => true), Bun.sleep(3000).then(() => false)]),
            "The unauthorized incoming handshake stayed open");
    }
    finally { socket.destroy(); }
}

try {
    await lab.start();
    await lab.request("app/setPreferences", { json: JSON.stringify({
        bittorrent_protocol: 0, dht: false, pex: false, lsd: false,
    }) });
    await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface });
    const mixed = await lab.json<Status>("qbuttPaths/status");
    let native = mixed.paths.find(path => path.edgeId === "native" && path.localAddress === nativeAddress);
    assert(mixed.mode === "mixed" && native?.open, "Mixed did not admit the physical Native route");
    let { bound, tcpPort, udpPort } = await physicalListeners();
    const destination = join(lab.root, "download");
    const hash = await lab.add("v1-public", destination);
    await lab.request("torrents/start", { hashes: hash });
    if (transport === "tcp") {
        const privateHash = await lab.add("v1", join(lab.root, "private-download"));
        await lab.request("torrents/start", { hashes: privateHash });
        await rejectedHandshake(tcpPort, privateHash);
        const privatePeers = (await lab.json<{ peers: Record<string, Peer> }>(
            `sync/torrentPeers?hash=${privateHash}`)).peers;
        assert.equal(Object.keys(privatePeers).length, 0,
            "A physical Native inbound peer attached to a private pinned torrent");
        const held = createConnection({ host: nativeAddress, port: tcpPort, localAddress: nativeAddress });
        const heldClosed = new Promise<void>(resolve => held.once("close", resolve));
        try {
            await new Promise<void>((resolve, reject) => {
                held.once("connect", resolve);
                held.once("error", reject);
            });
            held.on("error", () => {});
            await lab.request("qbuttPaths/policy", { mode: "tunnels" });
            await waitFor("old physical listeners retired", listeners,
                value => value.tcp.length === 0 && value.udp.length === 0, 30000);
            await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface });
            const replaced = await lab.json<Status>("qbuttPaths/status");
            const replacement = replaced.paths.find(path => path.edgeId === "native"
                && path.localAddress === nativeAddress);
            assert(replacement?.open && replacement.generation !== native.generation,
                "A replacement Native path reused the retired generation");
            native = replacement;
            ({ bound, tcpPort, udpPort } = await physicalListeners());
            if (!held.destroyed) held.write(handshake(hash));
            assert(await Promise.race([heldClosed.then(() => true), Bun.sleep(3000).then(() => false)]),
                "A pre-transition incoming socket inherited the replacement Native route");
            const oldPeers = (await lab.json<{ peers: Record<string, Peer> }>(
                `sync/torrentPeers?hash=${hash}`)).peers;
            assert.equal(Object.keys(oldPeers).length, 0,
                "The pre-transition socket attached after generation replacement");
        }
        finally { held.destroy(); }
    }
    const before = await lab.json<Status>("qbuttPaths/status");
    assert(!before.peers.length, "The downloader had peers before the independent inbound seed");
    seed = await startSeed(lab.python, lab.fixtures, "v1-public", lab.root, {
        label: `managed-inbound-${transport}`, listenAddress: nativeAddress, transport,
        neighbor: { host: nativeAddress, port: transport === "tcp" ? tcpPort : udpPort },
    });
    const route = await waitFor("managed incoming Native payload", () => lab.json<Status>("qbuttPaths/status"),
        status => status.peers.some(peer => peer.infoHash === hash && peer.pathId === native.pathId
            && peer.generation === native.generation && peer.peer === nativeAddress
            && peer.payloadDownload > 0), 30000);
    const peers = Object.values((await lab.json<{ peers: Record<string, Peer> }>(
        `sync/torrentPeers?hash=${hash}`)).peers);
    assert.equal(route.peers.length, 1, "An unattributed or duplicate managed peer appeared");
    assert.equal(peers.length, 1, "The torrent admitted an unexpected peer");
    assert(peers.some(peer => peer.flags.includes("I")
        && (transport === "tcp" || peer.flags.includes("P"))),
    "The Native peer was not admitted as incoming with the requested transport");
    await waitFor("managed incoming Native completion", () => lab.info(hash),
        info => info.progress === 1, 60000);
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    await waitFor("physical listeners retired in Tunnels Only", listeners,
        value => value.tcp.length === 0 && value.udp.length === 0, 30000);
    const tunnels = await lab.json<Status>("qbuttPaths/status");
    assert(tunnels.mode === "tunnels" && !tunnels.paths.some(path => path.edgeId === "native")
        && !tunnels.peers.some(peer => peer.pathId === native.pathId && peer.generation === native.generation),
    "Tunnels Only retained a Native listener or peer");
    const final = await seed.stop();
    seed = undefined;
    assert(final.uploadPayloadBytes >= verifiedBytes && final.downloadPayloadBytes === 0,
        "Independent seed did not supply the verified payload");
    assert.deepEqual(final.outgoingPeerAddresses, [nativeAddress]);
    await lab.checkpoint({ check: "managed-native-inbound", transport, bound,
        actualTcpPort: tcpPort, actualUdpPort: udpPort, verifiedBytes,
        route: { pathId: native.pathId, generation: native.generation },
        incoming: true, noContextZero: true,
        tunnelsRetired: true });
}
catch (error) {
    failure = error;
    try { await lab.checkpoint({ check: "managed-native-inbound-failure", transport,
        appLog: (await lab.json<unknown[]>("log/main?normal=true&info=true&warning=true&critical=true&last_known_id=-1")).slice(-30) }); } catch {}
}
finally {
    try { if (seed) await seed.stop(); } catch (error) { failure ??= error; }
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
}
await lab.finish(failure);
if (failure) throw failure;
