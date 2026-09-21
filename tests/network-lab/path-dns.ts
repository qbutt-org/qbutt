import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { join } from "node:path";
import { createLab, waitFor } from "../lab";
import { startProxy } from "./proxy";
import { startDns } from "./dns-fixture";

interface DnsPolicy { server: string; bootstrapServer: string; family: string }
interface Path {
    pathId: string; generation: number; edgeId: string; open: boolean; proxyName: string;
    localAddress?: string; dns?: DnsPolicy;
    gateway?: { state: string; tcp: boolean; udp: boolean };
}
interface Status {
    busy: boolean; pinned: boolean; processId: number; mode: string; dns: DnsPolicy; paths: Path[];
    resolution: { requestId: number; pathId: string; generation: number; state: string;
        addresses?: string[]; errorCode?: string };
}

const nativeMode = process.argv.includes("--native");
const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE;
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS;
if (nativeMode)
    assert(nativeInterface && nativeAddress && networkInterfaces()[nativeInterface]?.some(address =>
        address.address === nativeAddress && address.family === "IPv4" && !address.internal),
    "Set a physical QBUTT_LAB_NATIVE_INTERFACE and its QBUTT_LAB_NATIVE_ADDRESS");
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
    await lab.request("qbuttPaths/open", { configPath, proxyName: `node-${side}`, interfaceName });
    const status = await idle();
    const path = status.paths.find(path => path.proxyName === `node-${side}` && path.open);
    assert(path, `Real transport path did not open: ${JSON.stringify(status)}`);
    assert.deepEqual(path.gateway, { state: "outgoing-only", tcp: false, udp: false },
        "DNS-only fixture unexpectedly acquired a public gateway lease");
    return path;
};
let failure: unknown;
try {
    for (const side of [0, 1]) {
        const dns = await startDns(side);
        dnsServers.push(dns);
        proxies.push(await startProxy({ ...credentials[side]!, listenAddress: `127.0.0.${side + 20}`, targets: [{ host: `127.0.0.${side + 8}`,
            port: 53, connectHost: "127.0.0.1", connectPort: dns.port }] }));
    }
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
        name: `node-${side}`, type: "socks5", server: proxy.host, port: proxy.port, ...credentials[side], udp: true,
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

    if (nativeMode) {
        first = await open(0);
        const nativeDns = await startDns(2, nativeAddress!);
        dnsServers.push(nativeDns);
        const server = `${nativeAddress}:${nativeDns.port}`;
        await lab.request("qbuttPaths/dns", { server, bootstrapServer: server, family: "ipv4" });
        await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface: nativeInterface! });
        const mixed = await idle();
        const native = mixed.paths.find(path => path.edgeId === "native" && path.localAddress === nativeAddress);
        assert(native?.open && mixed.mode === "mixed" && mixed.processId === processId
            && [first, second].every(proxy => mixed.paths.some(path => path.pathId === proxy.pathId && path.open)),
        "Mixed Native DNS setup did not retain both SOCKS paths and the child");
        await resolve(native, "native.test");
        const nativeResult = await idle();
        assert.deepEqual(nativeResult.resolution.addresses, ["127.0.0.4"]);
        assert(nativeResult.resolution.pathId === native.pathId
            && nativeResult.resolution.generation === native.generation);
        assert(nativeDns.queries.some(query => query.family === 1 && query.source === nativeAddress),
            "Native DNS did not bind the physical source interface");
        assert(nativeResult.processId === processId && nativeResult.paths.every(path => path.open),
            "Native DNS reply retired healthy Mixed paths");

        await lab.request("qbuttPaths/policy", { mode: "tunnels" });
        await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface: nativeInterface! });
        const rolled = await idle();
        const current = rolled.paths.find(path => path.edgeId === "native" && path.localAddress === nativeAddress);
        assert(current?.open && current.pathId === native.pathId && current.generation > native.generation);
        const queryCount = nativeDns.queries.length;
        await assert.rejects(lab.request("qbuttPaths/resolve", { pathId: native.pathId,
            generation: String(native.generation), host: "stale.test", family: "ipv4" }), /HTTP 400/);
        assert.equal(nativeDns.queries.length, queryCount, "Retired Native generation reached the DNS server");
        await resolve(current, "current.test");
        const currentResult = await idle();
        assert.deepEqual(currentResult.resolution.addresses, ["127.0.0.4"]);
        assert(currentResult.resolution.pathId === current.pathId
            && currentResult.resolution.generation === current.generation);
        assert(currentResult.processId === processId && currentResult.paths.every(path => path.open));
        await lab.checkpoint({ check: "mixed-physical-native-dns-generation", nativeInterface, nativeAddress,
            retiredGeneration: native.generation, currentGeneration: current.generation,
            pathId: current.pathId, addresses: currentResult.resolution.addresses,
            queryCount: nativeDns.queries.length, querySources: [...new Set(nativeDns.queries.map(query => query.source))],
            processRetained: true,
            activePaths: currentResult.paths.filter(path => path.open).length });
        await lab.request("qbuttPaths/policy", { mode: "pinned" });
        await lab.request("qbuttPaths/dns", { ...policies[0]! });
    }

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
    const cleanup = await Promise.allSettled([
        ...proxies.map(proxy => proxy.close()),
        ...dnsServers.map(dns => dns.stop()),
    ]);
    const cleanupErrors = cleanup.flatMap(result => result.status === "rejected" ? [result.reason] : []);
    if (!failure && cleanupErrors.length > 0)
        failure = new AggregateError(cleanupErrors, "Path DNS fixture cleanup failed");
}
await lab.finish(failure);
if (failure) throw failure;
