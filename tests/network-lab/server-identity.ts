import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";
import { startDns } from "./dns-fixture";

interface Path { pathId: string; generation: number; edgeId: string; configuredServerId: string; proxyName: string; open: boolean }
interface Status {
    busy: boolean; processId: number; paths: Path[];
    nodes: { name: string; configuredServerId: string }[];
    serverGroups: Record<string, string>;
}

const lab = await createLab("server-identity");
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let dns: Awaited<ReturnType<typeof startDns>> | undefined;
let failure: unknown;
try {
    seed = await startSeed(lab.python, lab.fixtures, "v1", lab.root);
    const credentials = { username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") };
    for (const listenAddress of ["127.0.0.20", "127.0.0.20", "127.0.0.21"]) {
        proxies.push(await startProxy({ ...credentials, listenAddress,
            targets: [{ host: "127.0.0.2", port: seed.port, connectHost: seed.host, connectPort: seed.port }] }));
    }
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, index) => ({
        name: `node-${index}`, type: "socks5", server: proxy.host, port: proxy.port, ...credentials,
    })) }));
    await lab.start();
    const status = () => lab.json<Status>("qbuttPaths/status");
    await lab.request("qbuttPaths/list", { configPath });
    const listed = await waitFor("configured server list", status, value => !value.busy);
    assert.equal(listed.nodes.length, 3);
    const identities = listed.nodes.map(node => node.configuredServerId);
    assert(identities.every(value => /^[0-9a-f]{64}$/.test(value)));
    assert.equal(identities[0], identities[1], "Names and ports split one configured server");
    assert.notEqual(identities[0], identities[2]);
    await lab.request("qbuttPaths/groupServers", { proxyName: "node-0", sameAsProxyName: "node-1" });
    assert.deepEqual((await status()).serverGroups, {}, "An already identical server created redundant grouping state");
    const open = async (index: number) => {
        await lab.request("qbuttPaths/open", { configPath, proxyName: `node-${index}`,
            interfaceName: "Loopback Pseudo-Interface 1" });
        return await waitFor("identified open outcome", status, value => !value.busy);
    };
    const first = await open(0);
    assert.equal(first.paths.length, 1);
    const original = first.paths[0]!;
    assert(original.open && original.edgeId === identities[0]);
    const duplicate = await open(1);
    assert.equal(duplicate.processId, first.processId, "Duplicate selection interrupted the healthy child");
    assert.deepEqual(duplicate.paths, first.paths, "Alias opened a second path or replaced the active generation");
    await assert.rejects(lab.request("qbuttPaths/open", { configPath, proxyName: "node-1",
        interfaceName: "Loopback Pseudo-Interface 1", edgeId: "pretend-independent" }), /HTTP 400/);
    await lab.request("qbuttPaths/stop", { pathId: original.pathId });
    await waitFor("original transport stopped", status, value => !value.busy && value.paths.every(path => !path.open));
    const replaced = await open(1);
    assert.equal(replaced.paths.length, 1);
    const replacement = replaced.paths[0]!;
    assert(replacement.open && replacement.pathId === original.pathId
        && replacement.edgeId === original.edgeId && replacement.generation > original.generation
        && replacement.proxyName === "node-1", "Changing alias did not preserve the Edge and renew its transport generation");
    const distinct = await open(2);
    assert.equal(distinct.paths.length, 2);
    assert(distinct.paths.every(path => path.open));
    assert.equal(new Set(distinct.paths.map(path => path.edgeId)).size, 2);
    const destination = join(lab.root, "download");
    const hash = await lab.add("v1", destination);
    await lab.request("torrents/start", { hashes: hash });
    await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.2:${seed.port}` });
    await waitFor("replaced transport payload", () => lab.info(hash), value => value.progress === 1, 60000);
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("verified payload stopped", () => lab.info(hash), value => value.state === "stoppedUP");
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    assert.equal(proxies[0]!.stats.downloadStreamBytes, 0, "Retired original transport carried new payload");
    assert(proxies[1]!.stats.downloadStreamBytes >= verifiedBytes, "Replacement alias did not carry the torrent");
    await lab.checkpoint({ check: "same-server-alias-admission", duplicateRejected: true,
        arbitraryEdgeOverrideRejected: true, pathIdPreserved: true, originalGeneration: original.generation,
        replacementGeneration: replacement.generation, distinctServers: 2, verifiedBytes, exactSizesAndHashes: true,
        scope: "Configured server identity; different hostnames do not prove independent physical exits" });

    await lab.request("qbuttPaths/native", {});
    await waitFor("managed paths retired", status, value => !value.busy && value.paths.length === 0);
    await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
    await waitFor("original torrent removed", () => lab.json<unknown[]>("torrents/info"), value => value.length === 0);
    dns = await startDns(18); // All three distinct hostnames resolve to 127.0.0.20.
    proxies.push(await startProxy({ ...credentials, listenAddress: "127.0.0.20",
        targets: [{ host: "127.0.0.2", port: seed.port, connectHost: seed.host, connectPort: seed.port }] }));
    const aliasNames = ["alias-a", "alias-b", "unrelated-edge"];
    const aliasNodes = [proxies[0]!, proxies[1]!, proxies[3]!].map((proxy, index) => ({
        name: aliasNames[index]!, type: "socks5", server: `${aliasNames[index]}.test`, port: proxy.port, ...credentials,
    }));
    await writeFile(configPath, JSON.stringify({ proxies: aliasNodes }));
    await lab.request("qbuttPaths/list", { configPath });
    const aliases = await waitFor("distinct alias identities", status, value => !value.busy);
    const aliasIds = aliases.nodes.map(node => node.configuredServerId);
    assert.equal(new Set(aliasIds).size, 3);
    await lab.request("qbuttPaths/dns", { server: `127.0.0.1:${dns.port}`, bootstrapServer: `127.0.0.1:${dns.port}`, family: "ipv4" });
    for (const proxyName of aliasNames.slice(0, 2)) {
        await lab.request("qbuttPaths/open", { configPath, proxyName, interfaceName: "Loopback Pseudo-Interface 1" });
        const opened = await waitFor("unmerged alias open", status, value => !value.busy && value.paths.some(path => path.open));
        const path = opened.paths.find(candidate => candidate.open)!;
        await lab.request("qbuttPaths/stop", { pathId: path.pathId });
        await waitFor("unmerged alias closed", status, value => !value.busy && value.paths.every(candidate => !candidate.open));
    }
    const closedAliases = (await status()).paths;
    assert.equal(closedAliases.length, 2, "Closed records for both alias identities were not retained");
    await lab.request("qbuttPaths/open", { configPath, proxyName: aliasNames[0]!, reserveNames: JSON.stringify([aliasNames[1]]),
        interfaceName: "Loopback Pseudo-Interface 1" });
    const ungroupedReserve = await waitFor("ungrouped reserve rejected", status, value => !value.busy);
    assert.deepEqual(ungroupedReserve.paths, closedAliases, "An ungrouped reserve opened a path or changed closed records");
    await lab.request("qbuttPaths/groupServers", { proxyName: aliasNames[1]!, sameAsProxyName: aliasNames[0]! });
    const grouped = await status();
    assert.equal(grouped.serverGroups[aliasIds[1]!], aliasIds[0]);
    assert.equal(grouped.serverGroups[aliasIds[2]!], undefined, "An unselected shared-IP edge was grouped");
    assert(grouped.paths.every(path => !path.open && path.edgeId === aliasIds[0]), "Closed records kept stale edge identities");
    await lab.request("qbuttPaths/open", { configPath, proxyName: aliasNames[0]!, interfaceName: "Loopback Pseudo-Interface 1" });
    const reopened = await waitFor("merged closed aliases reopened", status, value => !value.busy && value.paths.some(path => path.open));
    assert.equal(reopened.paths.length, 2);
    assert.equal(reopened.paths.filter(path => path.open).length, 1);
    await lab.request("qbuttPaths/open", { configPath, proxyName: aliasNames[1]!, interfaceName: "Loopback Pseudo-Interface 1" });
    const denied = await waitFor("merged closed alias duplicate denied", status, value => !value.busy);
    assert.deepEqual(denied.paths, reopened.paths);
    await lab.request("qbuttPaths/native", {});
    await waitFor("merged records retired", status, value => !value.busy && value.paths.length === 0);
    aliasNodes[0]!.name = aliasNames[0] = "renamed-alias-a";
    await writeFile(configPath, JSON.stringify({ proxies: aliasNodes }));
    await lab.request("qbuttPaths/list", { configPath });
    await waitFor("subscription refresh", status, value => !value.busy);
    assert.deepEqual((await status()).serverGroups, grouped.serverGroups, "Node rename lost the server grouping");
    await lab.shutdown();
    await lab.start();
    assert.deepEqual((await status()).serverGroups, grouped.serverGroups, "Profile restart lost the explicit grouping");
    await lab.request("qbuttPaths/list", { configPath });
    await waitFor("restarted node list", status, value => !value.busy);
    await writeFile(configPath, JSON.stringify({ proxies: [...aliasNodes, ...Array.from({ length: 1021 }, (_, index) => ({
        name: `unselected-${index}-${"n".repeat(100)}`, type: "socks5", server: "unused.test", port: 1,
    }))] })); // Opening a selected subset must not require an oversized full-list IPC reply.
    await lab.request("qbuttPaths/dns", { server: `127.0.0.1:${dns.port}`, bootstrapServer: `127.0.0.1:${dns.port}`, family: "ipv4" });
    await lab.request("qbuttPaths/open", { configPath, proxyName: aliasNames[0]!, reserveNames: JSON.stringify([aliasNames[1]]),
        interfaceName: "Loopback Pseudo-Interface 1" });
    const active = await waitFor("grouped primary", status, value => !value.busy && value.paths.some(path => path.open));
    const primary = active.paths.find(path => path.open)!;
    assert.equal(primary.edgeId, aliasIds[0]);
    await lab.request("qbuttPaths/open", { configPath, proxyName: aliasNames[1]!, interfaceName: "Loopback Pseudo-Interface 1" });
    const duplicateAlias = await waitFor("duplicate alias refused", status, value => !value.busy);
    assert.deepEqual(duplicateAlias.paths, active.paths);
    await assert.rejects(lab.request("qbuttPaths/groupServers", { proxyName: aliasNames[2]!, sameAsProxyName: aliasNames[0]! }), /HTTP 400/);
    await assert.rejects(lab.request("qbuttPaths/resetServerGroups", {}), /HTTP 400/);
    await lab.request("qbuttPaths/open", { configPath, proxyName: aliasNames[2]!, interfaceName: "Loopback Pseudo-Interface 1" });
    const independent = await waitFor("ungrouped shared-IP edge", status, value => !value.busy && value.paths.filter(path => path.open).length === 2);
    const other = independent.paths.find(path => path.proxyName === aliasNames[2])!;
    assert.equal(other.edgeId, aliasIds[2]);
    await lab.request("qbuttPaths/transport", { pathId: primary.pathId, proxyName: aliasNames[1]! });
    const switched = await waitFor("explicit alias replacement", status, value => !value.busy
        && value.paths.some(path => path.open && path.proxyName === aliasNames[1]));
    const selected = switched.paths.find(path => path.pathId === primary.pathId)!;
    assert.equal(selected.edgeId, primary.edgeId);
    assert.equal(selected.configuredServerId, aliasIds[1]);
    assert(selected.generation > primary.generation);
    assert.deepEqual(switched.paths.find(path => path.pathId === other.pathId), other);
    const beforeAliasPayload = proxies[1]!.stats.downloadStreamBytes;
    const aliasDestination = join(lab.root, "alias-download");
    const aliasHash = await lab.add("v1", aliasDestination);
    await lab.request("torrents/start", { hashes: aliasHash });
    await lab.request("torrents/addPeers", { hashes: aliasHash, peers: `127.0.0.2:${seed.port}` });
    await waitFor("DNS alias payload", () => lab.info(aliasHash), value => value.progress === 1, 60000);
    await lab.request("torrents/stop", { hashes: aliasHash });
    await waitFor("DNS alias payload stopped", () => lab.info(aliasHash), value => value.state === "stoppedUP");
    const aliasVerifiedBytes = await verifyPayload(aliasDestination, lab.manifest.payload);
    assert(proxies[1]!.stats.downloadStreamBytes - beforeAliasPayload >= aliasVerifiedBytes);
    assert(dns.queries.length > 0, "DNS aliases were not used by the actual adapter");
    await lab.request("qbuttPaths/native", {});
    await waitFor("alias paths retired", status, value => !value.busy && value.paths.length === 0);
    await lab.request("qbuttPaths/resetServerGroups", {});
    assert.deepEqual((await status()).serverGroups, {});
    await lab.checkpoint({ check: "explicit-dns-alias-grouping", duplicateRejected: true, ungroupedReserveRejected: true,
        unselectedSharedIpIndependent: true, closedRecordsMerged: true, renameAndRestartPreserved: true, activeMutationRejected: true,
        originalGeneration: primary.generation, replacementGeneration: selected.generation,
        configuredIdentitiesDistinct: true, verifiedBytes: aliasVerifiedBytes, exactSizesAndHashes: true });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    const closed = await Promise.allSettled(proxies.map(proxy => proxy.close()));
    for (const result of closed) if (result.status === "rejected") failure ??= result.reason;
    try { if (seed) await seed.stop(); } catch (error) { failure ??= error; }
    try { if (dns) await dns.stop(); } catch (error) { failure ??= error; }
    for (const name of ["fixtures", "profile", "download", "alias-download", "nodes.json"]) {
        const path = resolve(lab.root, name);
        assert.equal(dirname(path), resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
