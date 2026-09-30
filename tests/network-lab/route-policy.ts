import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createSocket } from "node:dgram";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { networkInterfaces, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { startProxy } from "./proxy";

const executable = resolve(process.argv[2] ?? process.env.QBUTT_POLICY_EXE ?? "route-policy-integration.exe");
const publicAddress = process.argv[3] ?? process.env.QBUTT_PUBLIC_IPV4;
assert(publicAddress && /^\d{1,3}(?:\.\d{1,3}){3}$/.test(publicAddress),
    "Pass a synthetic IPv4 identity as the second argument or QBUTT_PUBLIC_IPV4");
const physicalAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS
    ?? Object.values(networkInterfaces()).flatMap(addresses => addresses ?? [])
        .find(address => address.family === "IPv4" && !address.internal)?.address;
assert(physicalAddress, "A non-loopback local IPv4 address is required for the DHT listener control");
const root = await mkdtemp(join(tmpdir(), "qbutt-route-policy-"));
const markers = join(root, "markers");
await mkdir(markers);
const mark = (name: string, data = "") => writeFile(join(markers, name), data);
const phase = () => {
    if (existsSync(join(markers, "ordinary-dht-restored")))
        return "ordinary";
    if (existsSync(join(markers, "phase-anonymous")))
        return existsSync(join(markers, "phase-anonymous-end")) ? "after-anonymous" : "anonymous";
    if (existsSync(join(markers, "phase-rebind")))
        return "rebind";
    if (existsSync(join(markers, "phase-rebind-start")))
        return "rebind-transition";
    return existsSync(join(markers, "phase-b")) ? "b" : "a";
};
function queryBytes(url: string, name: string) {
    const query = url.slice(url.indexOf("?") + 1);
    const encoded = query.split("&").find(item => item.startsWith(`${name}=`))?.slice(name.length + 1);
    if (encoded === undefined)
        return null;
    const bytes: number[] = [];
    for (let index = 0; index < encoded.length;) {
        if (encoded[index] === "%" && /^[0-9a-f]{2}$/i.test(encoded.slice(index + 1, index + 3))) {
            bytes.push(Number.parseInt(encoded.slice(index + 1, index + 3), 16));
            index += 3;
        }
        else {
            bytes.push(encoded.charCodeAt(index) & 255);
            index++;
        }
    }
    return Buffer.from(bytes);
}
const httpSources: { source: string; phase: string }[] = [];
const httpAnnounces: { source: string; phase: string; port: number; ip: string | null;
    ipv4: string | null; ipv6: string | null; peerId: string | null; key: string | null;
    host: string | null; infoHash: string | null; event: string | null }[] = [];
const webRequests: { path: string; range: string | null }[] = [];
let releaseOldTracker!: () => void;
const oldTrackerReleased = new Promise<void>(resolve => { releaseOldTracker = resolve; });
const tracker = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request, server) {
        const source = server.requestIP(request)?.address ?? "";
        const requestUrl = request.url;
        const url = new URL(requestUrl);
        const announce = { source, phase: phase(), port: Number(url.searchParams.get("port")),
            ip: url.searchParams.get("ip"), ipv4: url.searchParams.get("ipv4"), ipv6: url.searchParams.get("ipv6"),
            peerId: queryBytes(requestUrl, "peer_id")?.toString("hex") ?? null,
            key: url.searchParams.get("key"), host: request.headers.get("host"),
            infoHash: queryBytes(requestUrl, "info_hash")?.toString("hex") ?? null,
            event: url.searchParams.get("event") };
        httpSources.push({ source, phase: announce.phase });
        if (source === "127.0.0.2") {
            httpAnnounces.push(announce);
            await mark("http-a");
            await oldTrackerReleased;
        }
        else if (source === "127.0.0.3") {
            httpAnnounces.push(announce);
            await mark(announce.phase === "anonymous" ? "http-anonymous" : "http-b");
            releaseOldTracker();
        }
        else if (source === "127.0.0.1" && announce.host?.startsWith("tracker.invalid:")) {
            httpAnnounces.push(announce);
            if (announce.event === "stopped" && [1, 41004].includes(announce.port))
                await mark(`http-socks-stopped-${announce.port}`);
        }
        else if (source === "127.0.0.4") {
            await mark("http-default");
        }
        else if (source === "127.0.0.6" && announce.phase === "ordinary") {
            httpAnnounces.push(announce);
        }
        else {
            return new Response("unexpected source", { status: 403 });
        }
        return new Response("d8:intervali60e5:peers0:e", {
            headers: { "content-type": "text/plain" },
        });
    },
});

const payload = Buffer.alloc(1024 * 1024);
for (let index = 0; index < payload.length; index++)
    payload[index] = (index * 29 + Math.floor(index / 97)) & 255;
const webseed = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request) {
        const url = new URL(request.url);
        const range = request.headers.get("range");
        webRequests.push({ path: url.pathname, range });
        if (!url.pathname.endsWith("/payload.bin"))
            return new Response("not found", { status: 404 });
        if (!range)
            return new Response(payload);
        const match = /^bytes=(\d+)-(\d+)$/.exec(range);
        if (!match)
            return new Response("invalid range", { status: 416 });
        const first = Number(match[1]);
        const last = Math.min(Number(match[2]), payload.length - 1);
        if (first > last || first >= payload.length)
            return new Response("range outside payload", { status: 416 });
        return new Response(payload.subarray(first, last + 1), { status: 206, headers: {
            "accept-ranges": "bytes",
            "content-range": `bytes ${first}-${last}/${payload.length}`,
            "content-length": String(last - first + 1),
        } });
    },
});

const username = "route-user";
const password = "route-password";
// Deterministic hostname-to-IPv4 SOCKS mapping, not an A-only DNS wire test.
const proxy = await startProxy({ username, password, targets: [
    { host: "webseed.invalid", port: webseed.port!, connectHost: "127.0.0.1", connectPort: webseed.port! },
    { host: "tracker.invalid", port: tracker.port!, connectHost: "127.0.0.1", connectPort: tracker.port! },
] });

const udpSources: { source: string; phase: string }[] = [];
const udpAnnounces: { source: string; sourcePort: number; phase: string; port: number; ipv4: string;
    peerId: string; key: number }[] = [];
const udpTracker = createSocket("udp4");
udpTracker.on("message", (packet, source) => {
    const packetPhase = phase();
    udpSources.push({ source: source.address, phase: packetPhase });
    if (packet.length < 16)
        return;
    const action = packet.readUInt32BE(8);
    const transaction = packet.subarray(12, 16);
    if (action === 0) {
        const response = Buffer.alloc(16);
        transaction.copy(response, 4);
        response.writeBigUInt64BE(0x123456789abcdef0n, 8);
        udpTracker.send(response, source.port, source.address);
    }
    else if (action === 1) {
        if (packet.length < 98)
            return;
        const address = packet.readUInt32BE(84);
        const announce = { source: source.address, sourcePort: source.port,
            phase: packetPhase, port: packet.readUInt16BE(96),
            ipv4: `${address >>> 24}.${address >>> 16 & 255}.${address >>> 8 & 255}.${address & 255}`,
            peerId: packet.subarray(36, 56).toString("hex"), key: packet.readUInt32BE(88) };
        udpAnnounces.push(announce);
        if (announce.phase !== "ordinary")
            void mark(announce.phase === "anonymous" ? "udp-anonymous"
                : announce.phase === "rebind" ? "udp-rebind"
                    : source.address === "127.0.0.2" ? "udp-a" : "udp-b");
        if (source.address !== "127.0.0.3"
            && !(source.address === "127.0.0.6" && announce.phase === "ordinary"))
            return;
        const response = Buffer.alloc(20);
        response.writeUInt32BE(1, 0);
        transaction.copy(response, 4);
        response.writeUInt32BE(60, 8);
        udpTracker.send(response, source.port, source.address);
    }
});
await new Promise<void>(resolve => udpTracker.bind(0, "127.0.0.1", resolve));
const udpTrackerPort = (udpTracker.address() as { port: number }).port;

const dhtQueries: { source: string; sourcePort: number; phase: string; query: string; id: string }[] = [];
const dhtAnnounces: { source: string; sourcePort: number; phase: string;
    port: number; impliedPort: number | null }[] = [];
const dht = createSocket("udp4");
function field(packet: Buffer, name: string) {
    const prefix = Buffer.from(`${name.length}:${name}`);
    const start = packet.indexOf(prefix);
    if (start < 0)
        return Buffer.alloc(0);
    let cursor = start + prefix.length;
    let separator = cursor;
    while (separator < packet.length && packet[separator] >= 48 && packet[separator] <= 57)
        separator++;
    if (separator === cursor || packet[separator] !== 58)
        return Buffer.alloc(0);
    const length = Number(packet.subarray(cursor, separator).toString());
    cursor = separator + 1;
    return packet.subarray(cursor, cursor + length);
}
function integerField(packet: Buffer, name: string) {
    const prefix = Buffer.from(`${name.length}:${name}i`);
    const start = packet.indexOf(prefix);
    if (start < 0)
        return null;
    const first = start + prefix.length;
    const end = packet.indexOf(101, first);
    if (end < 0)
        return null;
    const value = Number(packet.subarray(first, end).toString());
    return Number.isSafeInteger(value) ? value : null;
}
dht.on("message", (packet, source) => {
    const packetPhase = phase();
    const id = field(packet, "id");
    const transaction = field(packet, "t");
    if (!transaction.length)
        return;
    const query = field(packet, "q").toString();
    dhtQueries.push({ source: source.address, sourcePort: source.port, phase: packetPhase, query,
        id: id.length === 20 ? id.toString("hex") : "" });
    let response: Buffer;
    if (query === "announce_peer") {
        const port = integerField(packet, "port");
        if (port === null)
            return;
        dhtAnnounces.push({ source: source.address, sourcePort: source.port, phase: packetPhase, port,
            impliedPort: integerField(packet, "implied_port") });
        if (packetPhase !== "ordinary")
            void mark(packetPhase === "rebind" ? "dht-rebind"
                : source.address === "127.0.0.2" ? "dht-a" : "dht-b");
        response = Buffer.concat([Buffer.from("d1:rd2:id20:"), Buffer.alloc(20, 0x42),
            Buffer.from("e1:t"), Buffer.from(String(transaction.length)), Buffer.from(":"), transaction,
            Buffer.from("1:y1:re")]);
    }
    else if (query === "get_peers") {
        response = Buffer.concat([Buffer.from("d1:rd2:id20:"), Buffer.alloc(20, 0x42),
            Buffer.from("5:nodes0:5:token11:route-tokene1:t"),
            Buffer.from(String(transaction.length)), Buffer.from(":"), transaction,
            Buffer.from("1:y1:re")]);
    }
    else {
        return;
    }
    dht.send(response, source.port, source.address);
});
await new Promise<void>(resolve => dht.bind(0, "127.0.0.1", resolve));
const dhtPort = (dht.address() as { port: number }).port;

let failure: unknown;
try {
    const child = spawn(executable, [root + "-client", String(tracker.port), String(udpTrackerPort),
        String(dhtPort), String(webseed.port), String(proxy.port), username, password,
        publicAddress, markers, physicalAddress],
    { stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    const standardDhtProbe = (async () => {
        const marker = join(markers, "native-standard-udp");
        for (let attempt = 0; attempt < 1000 && !existsSync(marker); attempt++)
            await Bun.sleep(10);
        assert(existsSync(marker), "Physical UDP listener was not reported");
        const port = Number(await readFile(marker, "utf8"));
        assert(Number.isInteger(port) && port > 0);
        const probe = createSocket("udp4");
        let physicalPort = port;
        let preManagedReply = false;
        let managedReply = false;
        let routedReply = false;
        let oldBReply = false;
        let reboundReply = false;
        let ordinaryReply = false;
        const ordinaryResponses: { address: string; port: number }[] = [];
        let ordinaryBReply = false;
        let routePort = 0;
        let oldBPort = 0;
        let reboundPort = 0;
        probe.on("message", (packet, source) => {
            const transaction = field(packet, "t").toString();
            if (transaction === "xx" && source.address === physicalAddress && source.port === port)
                preManagedReply = true;
            if (transaction === "yy") managedReply = true;
            if (transaction === "rr") {
                ordinaryResponses.push({ address: source.address, port: source.port });
                if (source.address === physicalAddress && source.port === physicalPort)
                    ordinaryReply = true;
            }
            if (transaction === "zz" && field(packet, "y").toString() === "r"
                && source.address === "127.0.0.2" && source.port === routePort)
                routedReply = true;
            if (transaction === "oo") oldBReply = true;
            if (transaction === "nn" && field(packet, "y").toString() === "r"
                && source.address === "127.0.0.3" && source.port === reboundPort)
                reboundReply = true;
            if (transaction === "dd" && field(packet, "y").toString() === "r"
                && source.address === "127.0.0.3" && source.port === reboundPort)
                ordinaryBReply = true;
        });
        try {
            const ping = async (transaction: string) => {
                const packet = Buffer.concat([Buffer.from("d1:ad2:id20:"), Buffer.alloc(20, 0x61),
                    Buffer.from(`e1:q4:ping1:t2:${transaction}1:y1:qe`)]);
                await new Promise<void>((resolve, reject) => probe.send(packet, physicalPort, physicalAddress,
                    error => error ? reject(error) : resolve()));
            };
            for (let attempt = 0; attempt < 20 && !preManagedReply; attempt++) {
                await ping("xx");
                await Bun.sleep(150);
            }
            assert(preManagedReply, "Physical DHT socket did not answer before managed policy");
            await mark("standard-dht-positive");
            const managed = join(markers, "managed-dht-active");
            for (let attempt = 0; attempt < 1000 && !existsSync(managed); attempt++)
                await Bun.sleep(10);
            assert(existsSync(managed), "Managed policy was not applied");
            await ping("yy");
            await Bun.sleep(600);
            assert(!managedReply, "Managed physical uTP listener leaked a context-0 DHT response");
            routePort = Number(await readFile(join(markers, "native-route-udp-a"), "utf8"));
            assert(Number.isInteger(routePort) && routePort > 0);
            const lookup = (transaction: string) => Buffer.concat([Buffer.from("d1:ad2:id20:"), Buffer.alloc(20, 0x61),
                Buffer.from("9:info_hash20:"), Buffer.alloc(20, 0x62),
                Buffer.from(`e1:q9:get_peers1:t2:${transaction}1:y1:qe`)]);
            for (let attempt = 0; attempt < 20 && !routedReply; attempt++) {
                await new Promise<void>((resolve, reject) => probe.send(lookup("zz"), routePort, "127.0.0.2",
                    error => error ? reject(error) : resolve()));
                await Bun.sleep(150);
            }
            assert(routedReply, "Managed Native DHT did not answer get_peers on its physical UDP socket");
            await mark("managed-dht-replied");
            const rebound = join(markers, "native-rebound-ports");
            for (let attempt = 0; attempt < 6000 && !existsSync(rebound); attempt++)
                await Bun.sleep(10);
            assert(existsSync(rebound), "Active Native route did not rebind with the preferred peer port");
            oldBPort = Number(await readFile(join(markers, "native-route-udp-b"), "utf8"));
            let reboundPorts: number[] = [];
            for (let attempt = 0; attempt < 50; attempt++) {
                reboundPorts = (await readFile(rebound, "utf8")).split(",").map(Number);
                if (reboundPorts.length === 2 && reboundPorts.every(Number.isInteger)) break;
                await Bun.sleep(10);
            }
            reboundPort = reboundPorts[1]!;
            assert(Number.isInteger(reboundPorts[0]) && reboundPorts[0]! > 0
                && Number.isInteger(reboundPort) && reboundPort > 0 && reboundPort !== oldBPort);
            const released = createSocket("udp4");
            try {
                await new Promise<void>((resolve, reject) => {
                    released.once("error", reject);
                    released.bind(oldBPort, "127.0.0.3", () => {
                        released.removeListener("error", reject);
                        resolve();
                    });
                });
            }
            finally { released.close(); }
            for (let attempt = 0; attempt < 3; attempt++) {
                await new Promise<void>((resolve, reject) => probe.send(lookup("oo"), oldBPort, "127.0.0.3",
                    error => error ? reject(error) : resolve()));
                await Bun.sleep(150);
            }
            await Bun.sleep(600);
            assert(!oldBReply, "Retired Native UDP listener still answered DHT get_peers");
            for (let attempt = 0; attempt < 20 && !reboundReply; attempt++) {
                await new Promise<void>((resolve, reject) => probe.send(lookup("nn"), reboundPort, "127.0.0.3",
                    error => error ? reject(error) : resolve()));
                await Bun.sleep(150);
            }
            assert(reboundReply, "Rebound Native DHT did not answer on the new physical UDP port");
            await mark("managed-rebind-dht-replied");
            const restored = join(markers, "ordinary-dht-restored");
            for (let attempt = 0; attempt < 12000 && !existsSync(restored)
                && child.exitCode === null; attempt++)
                await Bun.sleep(10);
            assert(existsSync(restored), "Managed Native did not return to ordinary Native policy");
            physicalPort = Number(await readFile(join(markers, "native-physical-udp-current"), "utf8"));
            assert(Number.isInteger(physicalPort) && physicalPort > 0);
            for (let attempt = 0; attempt < 20 && !ordinaryReply; attempt++) {
                await ping("rr");
                await Bun.sleep(150);
            }
            assert(ordinaryReply, "Ordinary physical DHT did not recover after managed retirement: "
                + JSON.stringify({ physicalPort, ordinaryResponses }));
            for (let attempt = 0; attempt < 20 && !ordinaryBReply; ++attempt) {
                await new Promise<void>((resolve, reject) => probe.send(lookup("dd"), reboundPort, "127.0.0.3",
                    error => error ? reject(error) : resolve()));
                await Bun.sleep(150);
            }
            assert(ordinaryBReply, "Physical Native listener did not return to ordinary DHT after managed retirement");
            await mark("ordinary-dht-replied");
        }
        finally {
            probe.close();
        }
    })();
    const [exitCode, stdout, stderr, probeError] = await Promise.all([
        new Promise<number | null>(resolve => child.once("exit", resolve)),
        new Response(child.stdout!).text(), new Response(child.stderr!).text(),
        standardDhtProbe.then(() => null, error => error),
    ]);
    assert.equal(exitCode, 0, `route policy client failed (${exitCode}): ${stderr}\n`
        + JSON.stringify({ httpSources, httpAnnounces, udpSources, udpAnnounces, dhtQueries, dhtAnnounces })
        + (probeError ? `\nprobe: ${probeError}` : ""));
    if (probeError)
        throw probeError;
    const clientEvidence = JSON.parse(stdout);
    assert(clientEvidence.passed && clientEvidence.webSeedVerifiedBytes === payload.length);
    assert.equal(clientEvidence.alternateRouteRetryVerifiedBytes, 256 * 1024,
        "The alternate route did not verify the generated torrent payload");
    assert(clientEvidence.alternateRouteRetryMs < 25_000 && proxy.stats.deniedConnections >= 1,
        "The failed SOCKS route did not switch promptly to the healthy Native route");
    assert.deepEqual(clientEvidence.hostnameTrackerEndpoints.sort((a: { pathId: number }, b: { pathId: number }) =>
        a.pathId - b.pathId), [
        { pathId: 2, generation: 1, listenerFamily: "ipv4" },
        { pathId: 5, generation: 7, listenerFamily: "ipv4" },
    ], "Hostname tracker endpoints lost a context or retained a duplicate family");
    const hostnameAnnounces = httpAnnounces.filter(item => item.host === `tracker.invalid:${tracker.port}`);
    for (const event of ["started", "stopped"]) {
        const announces = hostnameAnnounces.filter(item => item.event === event).sort((a, b) => a.port - b.port);
        assert.deepEqual(announces.map(item => ({ port: item.port, ip: item.ip, ipv4: item.ipv4, ipv6: item.ipv6 })), [
            { port: 1, ip: null, ipv4: null, ipv6: null },
            { port: 41004, ip: null, ipv4: publicAddress, ipv6: null },
        ], `Expected one ${event} announce per SOCKS context with the leased listener preserved`);
    }
    assert(hostnameAnnounces.every(item => item.source === "127.0.0.1"
        && item.ip === null && item.ipv6 === null
        && (item.port === 41004 ? item.ipv4 === publicAddress : item.port === 1 && item.ipv4 === null)));
    for (const field of ["infoHash", "peerId", "key"] as const)
        assert(hostnameAnnounces[0]![field] && new Set(hostnameAnnounces.map(item => item[field])).size === 1,
            `SOCKS tracker contexts used different ${field}`);
    assert.deepEqual(new Set(httpSources.filter(item => item.phase !== "ordinary").map(item => item.source)),
        new Set(["127.0.0.1", "127.0.0.2", "127.0.0.3", "127.0.0.4"]));
    assert.deepEqual(new Set(udpSources.filter(item => item.phase !== "ordinary").map(item => item.source)),
        new Set(["127.0.0.2", "127.0.0.3"]));
    assert.deepEqual(new Set(dhtQueries.filter(item => item.phase !== "ordinary").map(item => item.source)),
        new Set(["127.0.0.2", "127.0.0.3"]));
    assert(httpSources.filter(item => item.source === "127.0.0.6")
        .every(item => item.phase === "ordinary"), "uTP listener made HTTP tracker requests while managed");
    assert(udpSources.filter(item => item.source === "127.0.0.6")
        .every(item => item.phase === "ordinary"), "uTP listener made UDP tracker requests while managed");
    assert(dhtQueries.filter(item => item.source === "127.0.0.6")
        .every(item => item.phase === "ordinary"), "uTP listener made DHT requests while managed");
    const dhtAId = dhtQueries.find(item => item.source === "127.0.0.2" && item.phase === "a")?.id;
    const dhtBId = dhtQueries.find(item => item.source === "127.0.0.3" && item.phase === "b")?.id;
    assert(dhtAId && dhtBId && dhtAId !== dhtBId,
        "Distinct Native route generations reused one DHT node identity");
    const nativePorts = clientEvidence.nativeListeners as {
        tcpA: number; udpA: number; tcpB: number; udpB: number;
        reboundTcpB: number; reboundUdpB: number; udpUtp: number;
    };
    assert(Object.values(nativePorts).every(port => Number.isInteger(port) && port > 0));
    assert(nativePorts.tcpA !== 41001 && nativePorts.tcpB !== 41002,
        "Synthetic public endpoint accidentally matched the physical listener");
    const httpA = httpAnnounces.filter(item => item.source === "127.0.0.2" && item.phase === "a");
    const httpB = httpAnnounces.filter(item => item.source === "127.0.0.3" && item.phase === "b");
    const httpRebound = httpAnnounces.filter(item => item.source === "127.0.0.3" && item.phase === "rebind");
    const udpA = udpAnnounces.filter(item => item.source === "127.0.0.2" && item.phase === "a");
    const udpB = udpAnnounces.filter(item => item.source === "127.0.0.3" && item.phase === "b");
    const udpRebound = udpAnnounces.filter(item => item.source === "127.0.0.3" && item.phase === "rebind");
    assert(httpA.length > 0 && httpA.every(item => item.phase === "a"
        && item.port === nativePorts.tcpA && item.ip === null && item.ipv4 === null));
    assert(httpB.length > 0 && httpB.every(item => item.port === nativePorts.tcpB
        && item.ip === null && item.ipv4 === null));
    assert(httpRebound.length > 0 && httpRebound.every(item => item.port === nativePorts.reboundTcpB));
    assert(udpA.length > 0 && udpA.every(item => item.phase === "a"
        && item.sourcePort === nativePorts.udpA
        && item.port === nativePorts.tcpA && item.ipv4 === "0.0.0.0"));
    assert(udpB.length > 0 && udpB.every(item => item.port === nativePorts.tcpB
        && item.sourcePort === nativePorts.udpB && item.ipv4 === "0.0.0.0"));
    assert(udpRebound.length > 0 && udpRebound.every(item => item.sourcePort === nativePorts.reboundUdpB
        && item.port === nativePorts.reboundTcpB && item.ipv4 === "0.0.0.0"));
    const dhtA = dhtAnnounces.filter(item => item.source === "127.0.0.2" && item.phase === "a");
    const dhtB = dhtAnnounces.filter(item => item.source === "127.0.0.3" && item.phase === "b");
    const dhtRebound = dhtAnnounces.filter(item => item.source === "127.0.0.3" && item.phase === "rebind");
    assert(dhtA.length > 0 && dhtA.every(item => item.phase === "a"
        && item.sourcePort === nativePorts.udpA
        && item.port === nativePorts.tcpA && item.impliedPort !== 1));
    assert(dhtB.length > 0 && dhtB.every(item => item.sourcePort === nativePorts.udpB
        && item.port === nativePorts.tcpB && item.impliedPort !== 1));
    assert(dhtRebound.length > 0 && dhtRebound.every(item => item.sourcePort === nativePorts.reboundUdpB
        && item.port === nativePorts.reboundTcpB && item.impliedPort !== 1));
    assert(dhtQueries.filter(item => item.source === "127.0.0.2" && item.phase !== "ordinary")
        .every(item => item.phase === "a"), "Retired Native A continued DHT lookups");
    assert(dhtQueries.some(item => item.source === "127.0.0.2" && item.query === "get_peers"
        && item.sourcePort === nativePorts.udpA));
    assert(dhtQueries.some(item => item.source === "127.0.0.3" && item.query === "get_peers"
        && item.phase === "b" && item.sourcePort === nativePorts.udpB));
    assert(dhtQueries.some(item => item.source === "127.0.0.3" && item.query === "get_peers"
        && item.phase === "rebind" && item.sourcePort === nativePorts.reboundUdpB));
    assert(dhtQueries.filter(item => item.source === "127.0.0.3" && item.phase === "rebind")
        .every(item => item.sourcePort === nativePorts.reboundUdpB),
    "Retired Native B socket continued DHT lookups after rebind");
    assert(clientEvidence.retiredTrackerReplyRejected, "Retired generation accepted an HTTP tracker reply");
    const anonymousHttp = httpAnnounces.filter(item => item.source === "127.0.0.3"
        && item.phase === "anonymous");
    const anonymousUdp = udpAnnounces.filter(item => item.source === "127.0.0.3"
        && item.phase === "anonymous");
    assert(anonymousHttp.length > 0 && anonymousHttp.every(item => item.port === nativePorts.reboundTcpB
        && item.ip === null && item.ipv4 === null));
    assert(anonymousUdp.length > 0 && anonymousUdp.every(item => item.port === nativePorts.reboundTcpB
        && item.sourcePort === nativePorts.reboundUdpB && item.ipv4 === "0.0.0.0"));
    assert.equal(httpB[0].peerId, udpB[0].peerId, "HTTP and UDP trackers used different peer IDs");
    assert.equal(Number.parseInt(httpB[0].key!, 16) >>> 0, udpB[0].key,
        "HTTP and UDP trackers used different keys");
    assert(proxy.stats.authenticatedConnections >= 2 && webRequests.length > 0,
        "Tracker and web seed did not use the authenticated route SOCKS listener");
    assert(webRequests.every(request => request.path.endsWith("/payload.bin")));
    const evidence = { ...clientEvidence, publicIdentityAddress: publicAddress,
        hostnameTrackerResolution: "deterministic-socks-mapping-to-ipv4",
        httpSources: [...new Set(httpSources.map(item => item.source))],
        udpSources: [...new Set(udpSources.map(item => item.source))],
        dhtSources: [...new Set(dhtQueries.map(item => item.source))],
        dhtNodeIds: [...new Set(dhtQueries.map(item => item.id).filter(Boolean))],
        httpAnnounces, udpAnnounces, dhtQueries, dhtAnnounces,
        standardPhysicalDhtPreManagedResponded: true,
        standardPhysicalDhtSuppressed: true, managedNativeDhtResponded: true,
        reboundNativeDhtResponded: true, ordinaryPhysicalDhtRestored: true,
        proxyAuthenticatedConnections: proxy.stats.authenticatedConnections,
        proxyRelayDownloadBytes: proxy.stats.downloadStreamBytes, webSeedRequests: webRequests.length };
    await writeFile(join(root, "evidence.json"), JSON.stringify(evidence, null, 2));
    console.log(JSON.stringify({ passed: true, evidencePath: join(root, "evidence.json"), ...evidence }));
}
catch (error) {
    failure = error;
    throw error;
}
finally {
    releaseOldTracker();
    tracker.stop(true);
    webseed.stop(true);
    udpTracker.close();
    dht.close();
    await proxy.close();
    if (failure)
        console.error(`Preserved failed fixture: ${root}`);
}
