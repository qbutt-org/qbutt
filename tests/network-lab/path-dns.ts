import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { createServer, type Socket } from "node:net";
import { networkInterfaces } from "node:os";
import { join } from "node:path";
import { createLab, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface DnsPolicy { server: string; bootstrapServer: string; family: string }
interface Path { pathId: string; generation: number; open: boolean; proxyName: string; dns: DnsPolicy }
interface Status {
    busy: boolean; pinned: boolean; processId: number; dns: DnsPolicy; paths: Path[];
    resolution: { requestId: number; pathId: string; generation: number; state: string;
        addresses?: string[]; errorCode?: string };
}

// One framed DNS request per TCP connection; every answer is generated locally.
async function startDns(side: number) {
    const sockets = new Set<Socket>();
    const queries: { family: number; slow: boolean }[] = [];
    const server = createServer(socket => {
        sockets.add(socket);
        socket.on("error", () => socket.destroy());
        socket.on("close", () => sockets.delete(socket));
        socket.setTimeout(3000, () => socket.destroy());
        let bytes = Buffer.alloc(0);
        socket.on("data", chunk => {
            bytes = Buffer.concat([bytes, chunk]);
            if (bytes.length > 2048) return socket.destroy();
            if (bytes.length < 2 || bytes.length < bytes.readUInt16BE(0) + 2) return;
            socket.removeAllListeners("data");
            const request = bytes.subarray(2);
            let offset = 12;
            const labels: string[] = [];
            while (offset < request.length && request[offset] !== 0) {
                const length = request[offset++]!;
                if (length > 63 || offset + length > request.length) return socket.destroy();
                labels.push(request.subarray(offset, offset + length).toString("ascii"));
                offset += length;
            }
            if (offset + 5 !== request.length) return socket.destroy();
            const family = request.readUInt16BE(offset + 1);
            if (![1, 28].includes(family)) return socket.destroy();
            const slow = labels[0] === "slow";
            queries.push({ family, slow });
            const header = Buffer.from(request.subarray(0, 12));
            header.writeUInt16BE(0x8180, 2);
            header.writeUInt16BE(1, 6);
            const answer = Buffer.alloc(family === 1 ? 16 : 28);
            answer.writeUInt16BE(0xc00c, 0);
            answer.writeUInt16BE(family, 2);
            answer.writeUInt16BE(1, 4);
            answer.writeUInt32BE(0, 6); // No cache: every assertion observes its own exchange.
            answer.writeUInt16BE(family === 1 ? 4 : 16, 10);
            if (family === 1) answer.set([127, 0, 0, side + 2], 12);
            else answer[27] = side + 2;
            const message = Buffer.concat([header, request.subarray(12), answer]);
            const frame = Buffer.alloc(message.length + 2);
            frame.writeUInt16BE(message.length);
            message.copy(frame, 2);
            const reply = () => { if (!socket.destroyed) socket.end(frame); };
            if (slow) setTimeout(reply, 600);
            else reply();
        });
    });
    await new Promise<void>((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", resolve);
    });
    const address = server.address();
    assert(address && typeof address !== "string");
    return { port: address.port, queries, async stop() {
        for (const socket of sockets) socket.destroy();
        await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    } };
}

const lab = await createLab("path-dns"); // Firewall preflight precedes all listeners.
const dnsServers: Awaited<ReturnType<typeof startDns>>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
const credentials = [0, 1].map(() => ({ username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") }));
const configPath = join(lab.root, "subscription.json");
const interfaceName = Object.entries(networkInterfaces()).find(([, addresses]) => addresses?.some(address => address.internal))?.[0];
assert(interfaceName, "No loopback interface for the owned DNS fixture");
const policies: DnsPolicy[] = [0, 1].map(side => ({ server: `127.0.0.${side + 8}:53`,
    bootstrapServer: "127.0.0.1:1", family: side ? "ipv4" : "dual" }));
const readStatus = () => lab.json<Status>("qbuttPaths/status");
const idle = () => waitFor("path control idle", readStatus, status => !status.busy, 12000);
const resolve = async (path: Path, host: string, family = "ipv4") => {
    const response = await lab.request("qbuttPaths/resolve", { pathId: path.pathId,
        generation: String(path.generation), host, family });
    const status = await response.json() as Status;
    assert(status.resolution.requestId > 0);
    assert(!JSON.stringify(status).includes(host), "Public status echoed the hostname");
    return status;
};
const open = async (side: number) => {
    await lab.request("qbuttPaths/dns", { ...policies[side]! });
    await lab.request("qbuttPaths/open", { configPath, proxyName: `node-${side}`, interfaceName, edgeId: `edge-${side}` });
    const status = await idle();
    const path = status.paths.find(path => path.proxyName === `node-${side}` && path.open);
    assert(path, "Real transport path did not open");
    return path;
};
let failure: unknown;
try {
    for (const side of [0, 1]) {
        const dns = await startDns(side);
        dnsServers.push(dns);
        proxies.push(await startProxy({ ...credentials[side]!, targets: [{ host: `127.0.0.${side + 8}`,
            port: 53, connectHost: "127.0.0.1", connectPort: dns.port }] }));
    }
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
        name: `node-${side}`, type: "socks5", server: "127.0.0.1", port: proxy.port, ...credentials[side], udp: true,
    })) }));
    await lab.start();
    await lab.request("qbuttPaths/list", { configPath });
    await idle();
    let first = await open(0);
    const second = await open(1);
    const opened = await readStatus();
    const processId = opened.processId;
    assert(processId > 0 && opened.paths.filter(path => path.open).length === 2);
    assert.deepEqual(opened.paths.find(path => path.pathId === first.pathId)!.dns, policies[0]);
    assert.deepEqual(opened.dns, policies[1]);
    await resolve(first, "same.test", "dual");
    assert.deepEqual((await idle()).resolution.addresses?.sort(), ["127.0.0.2", "::2"].sort());
    await resolve(second, "same.test");
    assert.deepEqual((await idle()).resolution.addresses, ["127.0.0.3"]);
    const queriesBefore = dnsServers.map(server => server.queries.length);
    await assert.rejects(lab.request("qbuttPaths/resolve", { pathId: first.pathId, generation: String(first.generation + 1),
        host: "same.test", family: "ipv4" }), /HTTP 400/);
    assert.deepEqual(dnsServers.map(server => server.queries.length), queriesBefore);
    await resolve(second, "same.test", "ipv6");
    const rejectedFamily = await idle();
    assert.equal(rejectedFamily.resolution.errorCode, "path_dns_failed");
    assert(rejectedFamily.paths.every(path => path.open) && rejectedFamily.processId === processId);
    await lab.checkpoint({ check: "two-real-paths-immutable-dns-and-family", paths: [first.pathId, second.pathId],
        firstAnswers: ["127.0.0.2", "::2"], secondAnswers: ["127.0.0.3"], queryCounts: queriesBefore });

    await resolve(first, "slow.test");
    await waitFor("first path DNS in flight", async () => dnsServers[0]!.queries, queries => queries.some(query => query.slow));
    await lab.request("qbuttPaths/stop", { pathId: first.pathId });
    const revoked = await readStatus();
    assert(!revoked.paths.find(path => path.pathId === first.pathId)!.open);
    assert(revoked.paths.find(path => path.pathId === second.pathId)!.open && revoked.processId === processId);
    assert.equal(revoked.resolution.errorCode, "path_stopped");
    assert.deepEqual((await idle()).resolution.addresses, []);
    await resolve(second, "same.test");
    assert.deepEqual((await idle()).resolution.addresses, ["127.0.0.3"]);
    const previous = first;
    first = await open(0);
    assert(first.pathId === previous.pathId && first.generation > previous.generation);
    await resolve(second, "slow.test");
    await waitFor("second path DNS in flight", async () => dnsServers[1]!.queries, queries => queries.some(query => query.slow));
    await lab.request("qbuttPaths/stop", { pathId: first.pathId });
    const unrelated = await idle();
    assert.equal(unrelated.resolution.state, "complete");
    assert.deepEqual(unrelated.resolution.addresses, ["127.0.0.3"]);
    assert(unrelated.paths.find(path => path.pathId === second.pathId)!.open && unrelated.processId === processId);
    await lab.checkpoint({ check: "selected-generation-revoked-without-killing-other-path", requestGeneration: previous.generation,
        retryGeneration: first.generation, processRetained: true });

    await resolve(second, "slow.test");
    await lab.request("qbuttPaths/stop", {});
    const stopped = await idle();
    assert(stopped.processId === 0 && stopped.pinned && stopped.paths.every(path => !path.open));
    assert.equal(stopped.resolution.errorCode, "path_stopped");
    assert.deepEqual(stopped.resolution.addresses, []);
    await lab.shutdown();
    await lab.start();
    const restored = await readStatus();
    assert.deepEqual(restored.dns, policies[0]);
    assert(restored.pinned && restored.processId === 0);
    await lab.checkpoint({ check: "global-stop-pending-lookup-and-profile-restart", queryCounts: dnsServers.map(server => server.queries.length),
        verifiedTorrentBytes: 0, applicationTransportDnsIsolation: "not-tested" });
    await lab.shutdown();
}
catch (error) {
    failure = error;
    try { await lab.shutdown(); } catch (shutdownError) { console.error(String(shutdownError)); }
}
finally {
    for (const proxy of proxies) await proxy.close();
    for (const dns of dnsServers) await dns.stop();
}
await lab.finish(failure);
if (failure) throw failure;
