import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createSocket } from "node:dgram";
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
const httpSources: string[] = [];
const webRequests: { path: string; range: string | null }[] = [];
let releaseOldTracker!: () => void;
const oldTrackerReleased = new Promise<void>(resolve => { releaseOldTracker = resolve; });
const tracker = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request, server) {
        const source = server.requestIP(request)?.address ?? "";
        httpSources.push(source);
        if (source === "127.0.0.2") {
            await mark("http-a");
            await oldTrackerReleased;
        }
        else if (source === "127.0.0.3") {
            await mark("http-b");
            releaseOldTracker();
        }
        else if (source === "127.0.0.1"
            && request.headers.get("host")?.startsWith("tracker.invalid:")) {
            await mark("http-socks");
        }
        else if (source === "127.0.0.4") {
            await mark("http-default");
        }
        else {
            return new Response("unexpected source", { status: 403 });
        }
        return new Response("d8:intervali60e5:peers0:e", {
            headers: { "content-type": "text/plain", "content-length": "26" },
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
const udpTracker = createSocket("udp4");
udpTracker.on("message", (packet, source) => {
    udpSources.push(source.address);
    void mark(source.address === "127.0.0.2" ? "udp-a" : "udp-b");
    if (source.address !== "127.0.0.3" || packet.length < 16)
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
dht.on("message", (packet, source) => {
    dhtSources.push(source.address);
    const id = field(packet, "id");
    if (id.length === 20)
        dhtIds.push(id.toString("hex"));
    void mark(source.address === "127.0.0.2" ? "dht-a" : "dht-b");
    if (source.address !== "127.0.0.3")
        return;
    const transaction = field(packet, "t");
    if (!transaction.length)
        return;
    const response = Buffer.concat([Buffer.from("d1:rd2:id20:"), Buffer.alloc(20, 0x42),
        Buffer.from("5:nodes0:e1:t"), Buffer.from(String(transaction.length)), Buffer.from(":"), transaction,
        Buffer.from("1:y1:re")]);
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
    assert.equal(exitCode, 0, `route policy client failed (${exitCode}): ${stderr}`);
    const clientEvidence = JSON.parse(stdout);
    assert(clientEvidence.passed && clientEvidence.webSeedVerifiedBytes === payload.length);
    assert.deepEqual(new Set(httpSources), new Set(["127.0.0.1", "127.0.0.2", "127.0.0.3", "127.0.0.4"]));
    assert.deepEqual(new Set(udpSources), new Set(["127.0.0.2", "127.0.0.3"]));
    assert.deepEqual(new Set(dhtSources), new Set(["127.0.0.2", "127.0.0.3"]));
    assert.equal(new Set(dhtIds).size, 2, "DHT generations reused one node identity");
    assert(proxy.stats.authenticatedConnections >= 2 && webRequests.length > 0,
        "Tracker and web seed did not use the authenticated route SOCKS listener");
    assert(webRequests.every(request => request.path.endsWith("/payload.bin")));
    const evidence = { ...clientEvidence, publicIdentityAddress: publicAddress,
        httpSources: [...new Set(httpSources)], udpSources: [...new Set(udpSources)],
        dhtSources: [...new Set(dhtSources)], dhtNodeIds: [...new Set(dhtIds)],
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
