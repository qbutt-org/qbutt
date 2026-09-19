import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createSocket } from "node:dgram";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { startProxy } from "./proxy";

const executable = resolve(process.argv[2] ?? process.env.QBUTT_POLICY_EXE ?? "route-policy-integration.exe");
const publicAddress = process.argv[3] ?? process.env.QBUTT_PUBLIC_IPV4;
assert(publicAddress && /^\d{1,3}(?:\.\d{1,3}){3}$/.test(publicAddress),
    "Pass the currently observed public IPv4 address as the second argument or QBUTT_PUBLIC_IPV4");
const root = await mkdtemp(join(tmpdir(), "qbutt-route-policy-"));
const markers = join(root, "markers");
await mkdir(markers);
const mark = (name: string, data = "") => writeFile(join(markers, name), data);
const phase = () => existsSync(join(markers, "phase-anonymous"))
    && !existsSync(join(markers, "phase-anonymous-end"))
    ? "anonymous" : existsSync(join(markers, "phase-b")) ? "b" : "a";
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
const httpSources: string[] = [];
const httpAnnounces: { source: string; phase: string; port: number; ip: string | null;
    ipv4: string | null; peerId: string | null; key: string | null }[] = [];
const webRequests: { path: string; range: string | null }[] = [];
let releaseOldTracker!: () => void;
let oldTrackerRequestAborted = false;
const oldTrackerReleased = new Promise<void>(resolve => { releaseOldTracker = resolve; });
const tracker = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request, server) {
        const source = server.requestIP(request)?.address ?? "";
        const requestUrl = request.url;
        const url = new URL(requestUrl);
        const announce = { source, phase: phase(), port: Number(url.searchParams.get("port")),
            ip: url.searchParams.get("ip"), ipv4: url.searchParams.get("ipv4"),
            peerId: queryBytes(requestUrl, "peer_id")?.toString("hex") ?? null,
            key: url.searchParams.get("key") };
        httpSources.push(source);
        if (source === "127.0.0.2") {
            httpAnnounces.push(announce);
            await mark("http-a");
            await oldTrackerReleased;
            oldTrackerRequestAborted = request.signal.aborted;
        }
        else if (source === "127.0.0.3") {
            httpAnnounces.push(announce);
            await mark(announce.phase === "anonymous" ? "http-anonymous" : "http-b");
            releaseOldTracker();
        }
        else if (source === "127.0.0.1"
            && request.headers.get("host")?.startsWith("tracker.invalid:")) {
            httpAnnounces.push(announce);
            await mark("http-socks");
        }
        else if (source === "127.0.0.4") {
            await mark("http-default");
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
const proxy = await startProxy({ username, password, targets: [
    { host: "webseed.invalid", port: webseed.port!, connectHost: "127.0.0.1", connectPort: webseed.port! },
    { host: "tracker.invalid", port: tracker.port!, connectHost: "127.0.0.1", connectPort: tracker.port! },
] });

const udpSources: string[] = [];
const udpAnnounces: { source: string; phase: string; port: number; ipv4: string;
    peerId: string; key: number }[] = [];
const udpTracker = createSocket("udp4");
udpTracker.on("message", (packet, source) => {
    udpSources.push(source.address);
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
        const announce = { source: source.address, phase: phase(), port: packet.readUInt16BE(96),
            ipv4: `${address >>> 24}.${address >>> 16 & 255}.${address >>> 8 & 255}.${address & 255}`,
            peerId: packet.subarray(36, 56).toString("hex"), key: packet.readUInt32BE(88) };
        udpAnnounces.push(announce);
        void mark(announce.phase === "anonymous" ? "udp-anonymous"
            : source.address === "127.0.0.2" ? "udp-a" : "udp-b");
        if (source.address !== "127.0.0.3")
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

const dhtSources: string[] = [];
const dhtIds: string[] = [];
const dhtQueries: { source: string; phase: string; query: string }[] = [];
const dhtAnnounces: { source: string; phase: string; port: number; impliedPort: number | null }[] = [];
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
    dhtSources.push(source.address);
    const id = field(packet, "id");
    if (id.length === 20)
        dhtIds.push(id.toString("hex"));
    const transaction = field(packet, "t");
    if (!transaction.length)
        return;
    const query = field(packet, "q").toString();
    dhtQueries.push({ source: source.address, phase: phase(), query });
    let response: Buffer;
    if (query === "announce_peer") {
        const port = integerField(packet, "port");
        if (port === null)
            return;
        dhtAnnounces.push({ source: source.address, phase: phase(), port,
            impliedPort: integerField(packet, "implied_port") });
        void mark(source.address === "127.0.0.2" ? "dht-a" : "dht-b");
        response = Buffer.concat([Buffer.from("d1:rd2:id20:"), Buffer.alloc(20, 0x42),
            Buffer.from("e1:t"), Buffer.from(String(transaction.length)), Buffer.from(":"), transaction,
            Buffer.from("1:y1:re")]);
    }
    else if (query === "get_peers") {
        if (source.address === "127.0.0.7")
            void mark("dht-outgoing");
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
        String(dhtPort), String(webseed.port), String(proxy.port), username, password, publicAddress, markers],
    { stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    const [exitCode, stdout, stderr] = await Promise.all([
        new Promise<number | null>(resolve => child.once("exit", resolve)),
        new Response(child.stdout!).text(), new Response(child.stderr!).text(),
    ]);
    assert.equal(exitCode, 0, `route policy client failed (${exitCode}): ${stderr}\n`
        + JSON.stringify({ dhtQueries, dhtSources, dhtAnnounces }));
    const clientEvidence = JSON.parse(stdout);
    assert(clientEvidence.passed && clientEvidence.webSeedVerifiedBytes === payload.length);
    assert.deepEqual(new Set(httpSources), new Set(["127.0.0.1", "127.0.0.2", "127.0.0.3", "127.0.0.4"]));
    assert.deepEqual(new Set(udpSources), new Set(["127.0.0.2", "127.0.0.3"]));
    assert.deepEqual(new Set(dhtSources), new Set(["127.0.0.2", "127.0.0.3", "127.0.0.7"]));
    assert.equal(new Set(dhtIds).size, 3, "DHT generations reused one node identity");
    const httpA = httpAnnounces.filter(item => item.source === "127.0.0.2");
    const httpB = httpAnnounces.filter(item => item.source === "127.0.0.3" && item.phase === "b");
    const udpA = udpAnnounces.filter(item => item.source === "127.0.0.2");
    const udpB = udpAnnounces.filter(item => item.source === "127.0.0.3" && item.phase === "b");
    assert(httpA.length > 0 && httpA.every(item => item.phase === "a"
        && item.port === 41001 && item.ip === null && item.ipv4 === publicAddress));
    assert(httpB.length > 0 && httpB.every(item => item.port === 41002
        && item.ip === null && item.ipv4 === publicAddress));
    assert(httpAnnounces.some(item => item.source === "127.0.0.1"
        && item.port === 1 && item.ip === null && item.ipv4 === null),
        "Outgoing-only route claimed a public listener");
    assert(udpA.length > 0 && udpA.every(item => item.phase === "a"
        && item.port === 41001 && item.ipv4 === publicAddress));
    assert(udpB.length > 0 && udpB.every(item => item.port === 41002 && item.ipv4 === publicAddress));
    const dhtA = dhtAnnounces.filter(item => item.source === "127.0.0.2");
    const dhtB = dhtAnnounces.filter(item => item.source === "127.0.0.3");
    assert(dhtA.length > 0 && dhtA.every(item => item.phase === "a"
        && item.port === 42001 && item.impliedPort !== 1));
    assert(dhtB.length > 0 && dhtB.every(item => item.phase !== "a"
        && item.port === 42002 && item.impliedPort !== 1));
    assert(dhtQueries.some(item => item.source === "127.0.0.7" && item.query === "get_peers"));
    assert(!dhtAnnounces.some(item => item.source === "127.0.0.7"),
        "Outgoing-only DHT route published announce_peer");
    assert(oldTrackerRequestAborted, "Retired generation accepted its pending HTTP tracker reply");
    const anonymousHttp = httpAnnounces.filter(item => item.source === "127.0.0.3"
        && item.phase === "anonymous");
    const anonymousUdp = udpAnnounces.filter(item => item.source === "127.0.0.3"
        && item.phase === "anonymous");
    assert(anonymousHttp.length > 0 && anonymousHttp.every(item => item.port === 41002
        && item.ip === null && item.ipv4 === null));
    assert(anonymousUdp.length > 0 && anonymousUdp.every(item => item.port === 41002
        && item.ipv4 === "0.0.0.0"));
    assert.equal(httpB[0].peerId, udpB[0].peerId, "HTTP and UDP trackers used different peer IDs");
    assert.equal(Number.parseInt(httpB[0].key!, 16) >>> 0, udpB[0].key,
        "HTTP and UDP trackers used different keys");
    assert(proxy.stats.authenticatedConnections >= 2 && webRequests.length > 0,
        "Tracker and web seed did not use the authenticated route SOCKS listener");
    assert(webRequests.every(request => request.path.endsWith("/payload.bin")));
    const evidence = { ...clientEvidence, publicIdentityAddress: publicAddress,
        httpSources: [...new Set(httpSources)], udpSources: [...new Set(udpSources)],
        dhtSources: [...new Set(dhtSources)], dhtNodeIds: [...new Set(dhtIds)],
        httpAnnounces, udpAnnounces, dhtAnnounces,
        oldTrackerRequestAborted,
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
