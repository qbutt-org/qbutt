import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { networkInterfaces } from "node:os";
import { dirname, join } from "node:path";
import { createLab, startSeed, verifyPayload } from "../lab";
import { startProxy } from "../network-lab/proxy";

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
try {
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
        seeds.push(await startSeed(lab.python, lab.fixtures, torrent.name, lab.root, {savePath, pieces,
            label: `address-${side}`, listenAddress: side === 0 ? "127.0.0.1" : "127.0.0.5"}));
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
        remotePeer: `127.0.0.9:${seeds[0]!.port}`, nativePeer: `127.0.0.5:${seeds[1]!.port}`}));
    application = Bun.spawn([executable, `--profile=${profile}`, "--no-splash", "--confirm-legal-notice"], {
        cwd: dirname(executable), windowsHide: true,
        env: {...process.env, QT_QPA_PLATFORM: "offscreen", QBUTT_QT_ACCEPTANCE_SPEC: specPath},
        stdout: Bun.file(join(lab.root, "app.stdout.log")), stderr: Bun.file(join(lab.root, "app.stderr.log")),
    });
    const timeout = setTimeout(() => application?.kill(), 300000);
    const exit = await application.exited;
    clearTimeout(timeout);
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert(exit === 0 && evidence.status === "passed", `Qt Native address acceptance failed: ${JSON.stringify(evidence)}`);
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    await lab.checkpoint({check: "native-address-reconciliation", verifiedBytes, qtEvidence: evidencePath,
        scope: "Controlled in-process snapshots and real loopback peer sockets; no physical interface mutation"});
}
catch (error) { failure = error; }
finally {
    if (application?.exitCode === null) { application.kill(); await application.exited; }
    for (const [side, seed] of seeds.entries()) {
        const final = await seed.stop();
        await lab.checkpoint({check: `address-seed-${side}`, ...final});
        if (final.downloadPayloadBytes !== 0) failure ??= new Error("A seed downloaded payload");
        if (side === 1 && !failure) assert.deepEqual([...new Set(final.peerAddresses)].sort(), ["127.0.0.6", "127.0.0.7"]);
    }
    await proxy?.close();
    const root = await realpath(lab.root);
    for (const name of ["profile", "fixtures", "partial-0", "partial-1", "download", "nodes.yaml"]) {
        const path = join(root, name);
        assert(dirname(path) === root);
        await rm(path, {recursive: name !== "nodes.yaml", force: true});
    }
}
await lab.finish(failure);
if (failure) throw failure;
