import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface Peer { infoHash: string; peer: string; port: number; pathId: string }
interface Status { busy: boolean; paths: { open: boolean }[]; peers: Peer[] }
const lab = await createLab("connection-budget");
const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
const proxies: Awaited<ReturnType<typeof startProxy>>[] = [];
const torrents = ["v1-public", "v1-64k"];
const hashes: string[] = [];
let failure: unknown;
try {
    for (const [torrent, name] of torrents.entries()) {
        for (const side of [0, 1]) seeds.push(await startSeed(lab.python, lab.fixtures, name, lab.root,
            { label: `${torrent}-${side}`, uploadRate: 8192 }));
    }
    const credentials = { username: randomBytes(12).toString("hex"), password: randomBytes(16).toString("hex") };
    const targets = seeds.map((seed, index) => ({ host: `127.0.0.${index + 2}`, port: seed.port,
        connectHost: seed.host, connectPort: seed.port }));
    for (const side of [0, 1]) proxies.push(await startProxy({ ...credentials,
        listenAddress: `127.0.0.${side + 20}`, targets }));
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: proxies.map((proxy, side) => ({
        name: `budget-${side}`, type: "socks5", server: proxy.host, port: proxy.port, ...credentials,
    })) }));
    await lab.start();
    const status = () => lab.json<Status>("qbuttPaths/status");
    for (const side of [0, 1]) {
        await lab.request("qbuttPaths/open", { configPath, proxyName: `budget-${side}`,
            interfaceName: "Loopback Pseudo-Interface 1" });
        await waitFor("budget path ready", status,
            value => !value.busy && value.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const limits = async (session: number, torrent: number) => {
        await lab.request("app/setPreferences", { json: JSON.stringify({ max_connec: session,
            max_connec_per_torrent: torrent, bittorrent_protocol: 1 }) });
        const actual = await lab.json<{ max_connec: number; max_connec_per_torrent: number }>("app/preferences");
        assert.equal(actual.max_connec, session);
        assert.equal(actual.max_connec_per_torrent, torrent);
    };
    await limits(3, 2);
    for (const [index, name] of torrents.entries()) {
        const hash = await lab.add(name, join(lab.root, `download-${index}`));
        hashes.push(hash);
        await lab.request("torrents/start", { hashes: hash });
        const peers = targets.slice(index * 2, index * 2 + 2).map(target => `${target.host}:${target.port}`);
        // Repeated candidates must not create another connection through a second path.
        await lab.request("torrents/addPeers", { hashes: hash, peers: [...peers, ...peers].join("|") });
    }
    const check = (peers: Peer[], sessionLimit: number, torrentLimit: number) => {
        assert(peers.length <= sessionLimit, "Managed routes bypassed the session connection limit");
        assert.equal(new Set(peers.map(peer => `${peer.infoHash}:${peer.peer}:${peer.port}`)).size, peers.length,
            "One original endpoint acquired simultaneous connections through multiple paths");
        for (const hash of hashes) assert(peers.filter(peer => peer.infoHash === hash).length <= torrentLimit,
            "Managed routes bypassed the torrent connection limit");
    };
    await waitFor("shared session budget occupied", async () => {
        const current = await status(); check(current.peers, 3, 2); return current;
    }, value => value.peers.length === 3 && hashes.every(hash => value.peers.some(peer => peer.infoHash === hash))
        && new Set(value.peers.map(peer => peer.pathId)).size === 2, 60000);
    const observe = async (sessionLimit: number, torrentLimit: number, expected: number) => {
        let samples = 0, maximum = 0;
        const paths = new Set<string>();
        const deadline = Date.now() + 3000;
        do {
            const current = await status();
            check(current.peers, sessionLimit, torrentLimit);
            maximum = Math.max(maximum, current.peers.length);
            for (const peer of current.peers) paths.add(peer.pathId);
            samples++;
            await Bun.sleep(100);
        } while (Date.now() < deadline);
        assert.equal(maximum, expected, "Fixture did not occupy the intended connection budget");
        await lab.checkpoint({ check: "shared-peer-connection-budget", sessionLimit, torrentLimit,
            activeTorrents: hashes.length, maximum, samples, activePaths: paths.size,
            duplicateOriginalEndpoints: 0, scope: "Outgoing TCP peers; explicit repeated candidates, no incoming slack or discovery traffic" });
    };
    await observe(3, 2, 3);
    await limits(4, 1);
    await waitFor("per-torrent budget drained", status, value => value.peers.length === 2
        && hashes.every(hash => value.peers.filter(peer => peer.infoHash === hash).length === 1), 15000);
    await observe(4, 1, 2);
    for (const seed of seeds) await seed.setUploadRate(1024 * 1024);
    for (const [index, hash] of hashes.entries()) {
        await waitFor("budgeted torrent complete", () => lab.info(hash), value => value.progress === 1, 60000);
        await lab.request("torrents/stop", { hashes: hash });
        await waitFor("budgeted torrent stopped", () => lab.info(hash), value => value.state === "stoppedUP");
        const verifiedBytes = await verifyPayload(join(lab.root, `download-${index}`), lab.manifest.payload);
        await lab.checkpoint({ check: "budgeted-payload", hash, verifiedBytes, exactSizesAndHashes: true });
    }
}
catch (error) {
    failure = error;
    await lab.checkpoint({ check: "failure-context", appExitCode: lab.exitCode,
        error: error instanceof Error ? error.stack : String(error) });
}
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    for (const proxy of proxies) { try { await proxy.close(); } catch (error) { failure ??= error; } }
    for (const seed of seeds) {
        try { const result = await seed.stop(); assert.equal(result.downloadPayloadBytes, 0); }
        catch (error) { failure ??= error; }
    }
    for (const name of ["fixtures", "profile", "download-0", "download-1", "nodes.json"]) {
        const path = resolve(lab.root, name); assert.equal(dirname(path), resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
