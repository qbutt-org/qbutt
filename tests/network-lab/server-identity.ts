import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface Path { pathId: string; generation: number; edgeId: string; proxyName: string; open: boolean }
interface Status {
    busy: boolean; processId: number; paths: Path[];
    nodes: { name: string; configuredServerId: string }[];
}

const lab = await createLab("server-identity");
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
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
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    const closed = await Promise.allSettled(proxies.map(proxy => proxy.close()));
    for (const result of closed) if (result.status === "rejected") failure ??= result.reason;
    try { if (seed) await seed.stop(); } catch (error) { failure ??= error; }
    for (const name of ["fixtures", "profile", "download", "nodes.json"]) {
        const path = resolve(lab.root, name);
        assert.equal(dirname(path), resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
