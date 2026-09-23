import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { mkdir, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { dirname, join } from "node:path";
import { createLab, startSeed, verifyPayload } from "../lab";
import { startProxy } from "../network-lab/proxy";
import { decode, encode, type Value } from "../network-lab/bencode";

const executable = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(executable, "Set QBUTT_QT_ACCEPTANCE_EXE to the stable Qt process driver beside qbutt-net");
process.env.QBUTT_LAB_EXE = executable;
const interfaceName = Object.entries(networkInterfaces()).find(([, entries]) =>
    entries?.some(entry => entry.internal && entry.address === "127.0.0.1"))?.[0];
assert(interfaceName, "No loopback interface for the isolated transport");
const lab = await createLab("native-address");
const torrent = lab.manifest.torrents.find(item => item.name === "v1-public")!;
const seeds: Awaited<ReturnType<typeof startSeed>>[] = [];
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let application: ReturnType<typeof Bun.spawn> | undefined;
let failure: unknown;
const dht = createSocket("udp4");
const dhtQueries: {source: string; query: string; nodeId: string; at: number}[] = [];
const dhtErrors: string[] = [];
const dhtEvidence = join(lab.root, "dht-observer.json");
let observationWrite = Promise.resolve();
try {
    dht.on("error", error => dhtErrors.push(String(error)));
    dht.on("message", (packet, remote) => {
        try {
            assert(dhtQueries.length < 512, "DHT observer request limit");
            const message = decode(packet);
            if (!Buffer.isBuffer(message.y) || message.y.toString() !== "q") return;
            assert(Buffer.isBuffer(message.q) && Buffer.isBuffer(message.t));
            const args = message.a as Record<string, Value>;
            assert(Buffer.isBuffer(args.id) && args.id.length === 20 && message.ro === 1);
            const query = message.q.toString();
            assert(["get_peers", "find_node", "ping"].includes(query));
            assert(["127.0.0.6", "127.0.0.7"].includes(remote.address), "Unexpected Native DHT source");
            dhtQueries.push({source: remote.address, query, nodeId: args.id.toString("hex"), at: Date.now()});
            const snapshot = JSON.stringify({queries: dhtQueries});
            observationWrite = observationWrite.then(async () => {
                await writeFile(`${dhtEvidence}.tmp`, snapshot);
                await rename(`${dhtEvidence}.tmp`, dhtEvidence);
            })
                .catch(error => { dhtErrors.push(String(error)); });
            const response: Record<string, Value> = {id: Buffer.alloc(20, 7)};
            if (query !== "ping") response.nodes = Buffer.alloc(0);
            if (query === "get_peers") response.token = Buffer.from("fixture");
            dht.send(encode({r: response, t: message.t, y: Buffer.from("r")}), remote.port, remote.address);
        }
        catch (error) { dhtErrors.push(String(error)); }
    });
    await new Promise<void>((resolve, reject) => {
        dht.once("error", reject);
        dht.bind(0, "127.0.0.12", resolve);
    });
    for (const side of [0, 1]) {
        const pieces = Array.from({length: torrent.pieceCount}, (_, index) => index).filter(index => index % 2 === side);
        const savePath = join(lab.root, `partial-${side}`);
        for (const file of torrent.files) {
            assert(!file.pad);
            const source = await readFile(join(lab.fixtures, "seed", file.path));
            const data = Buffer.alloc(file.size);
            for (const piece of pieces) {
                const from = Math.max(file.offset, piece * torrent.pieceLength);
                const to = Math.min(file.offset + file.size, (piece + 1) * torrent.pieceLength);
                if (to > from) source.copy(data, from - file.offset, from - file.offset, to - file.offset);
            }
            await mkdir(dirname(join(savePath, file.path)), {recursive: true});
            await writeFile(join(savePath, file.path), data);
        }
        // Keep remote's partial payload available through the Native reconnect window.
        seeds.push(await startSeed(lab.python, lab.fixtures, torrent.name, lab.root, {savePath, pieces,
            label: `address-${side}`, listenAddress: side === 0 ? "127.0.0.1" : "127.0.0.5",
            uploadRate: side === 0 ? 8 * 1024 : undefined,
            allowedPeerAddresses: side === 0 ? [] : ["127.0.0.6", "127.0.0.7"]}));
    }
    const credentials = {username: randomBytes(12).toString("hex"), password: randomBytes(24).toString("hex")};
    proxy = await startProxy({...credentials, listenAddress: "127.0.0.20", targets: [{host: "127.0.0.9",
        port: seeds[0]!.port, connectHost: seeds[0]!.host, connectPort: seeds[0]!.port}]});
    const configPath = join(lab.root, "nodes.yaml");
    await writeFile(configPath, Bun.YAML.stringify({proxies: [{name: "remote", type: "socks5",
        server: "127.0.0.20", port: proxy.port, ...credentials, udp: false}]}));
    const destination = join(lab.root, "download");
    await mkdir(destination);
    const specPath = join(lab.root, "spec.json");
    const evidencePath = join(lab.root, "qt-evidence.json");
    const profile = join(lab.root, "profile");
    await writeFile(specPath, JSON.stringify({mode: "native-address", evidencePath,
        configPath, interfaceName, torrentPath: join(lab.fixtures, torrent.file), destination,
        dhtPort: dht.address().port, dhtEvidence,
        remotePeer: `127.0.0.9:${seeds[0]!.port}`, nativePeer: `127.0.0.5:${seeds[1]!.port}`}));
    application = Bun.spawn([executable, `--profile=${profile}`], {
        cwd: dirname(executable), windowsHide: true,
        env: {...process.env, QT_QPA_PLATFORM: "offscreen", QBUTT_QT_ACCEPTANCE_SPEC: specPath},
        stdout: Bun.file(join(lab.root, "app.stdout.log")), stderr: Bun.file(join(lab.root, "app.stderr.log")),
    });
    const timeout = setTimeout(() => application?.kill(), 300000);
    const exit = await application.exited;
    clearTimeout(timeout);
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert(exit === 0 && evidence.status === "passed", `Qt Native address acceptance failed: ${JSON.stringify(evidence)}`);
    assert.deepEqual(dhtErrors, []);
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    await lab.checkpoint({check: "native-address-reconciliation", verifiedBytes, qtEvidence: evidencePath,
        scope: "Controlled in-process snapshots and real loopback peer/DHT sockets; no physical interface mutation"});
}
catch (error) { failure = error; }
finally {
    if (application?.exitCode === null) { application.kill(); await application.exited; }
    for (const [side, seed] of seeds.entries()) {
        const final = await seed.stop();
        await lab.checkpoint({check: `address-seed-${side}`, ...final});
        if (final.downloadPayloadBytes !== 0) failure ??= new Error("A seed downloaded payload");
        if (side === 1 && !failure) {
            try { assert.deepEqual([...new Set(final.peerAddresses)].sort(), ["127.0.0.6", "127.0.0.7"]); }
            catch (error) { failure = error; }
        }
    }
    await proxy?.close();
    await new Promise<void>(resolve => dht.close(resolve));
    await observationWrite;
    const root = await realpath(lab.root);
    for (const name of ["profile", "fixtures", "partial-0", "partial-1", "download", "nodes.yaml"]) {
        const path = join(root, name);
        assert(dirname(path) === root);
        await rm(path, {recursive: name !== "nodes.yaml", force: true});
    }
}
await lab.finish(failure);
if (failure) throw failure;
