import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { isIPv4 } from "node:net";
import { networkInterfaces } from "node:os";
import { join, sep } from "node:path";
import { createInterface } from "node:readline";
import { Readable } from "node:stream";
import { createLab, waitFor } from "../lab";
import { sha256 } from "../fixtures/generate";

interface Peer {
    pathId: string; generation: number; peer: string; port: number; infoHash: string;
    localAddress: string; payloadDownload: number;
}
interface Status {
    busy: boolean; mode: string; peers: Peer[];
    paths: { pathId: string; generation: number; edgeId: string; open: boolean; localAddress?: string }[];
}
interface Connection {
    side: number; remoteIP: string; peerId: string; requestedPieces: number[]; payloadBytes: number;
}

const observer = process.env.QBUTT_WAN_OBSERVER ?? "";
const observerIP = process.env.QBUTT_WAN_OBSERVER_IP ?? "";
const configPath = process.env.QBUTT_WAN_PROXY_CONFIG ?? "";
const proxyNames = (process.env.QBUTT_WAN_PROXY_NAMES ?? "").split("|").filter(Boolean);
const nativeInterface = process.env.QBUTT_LAB_NATIVE_INTERFACE ?? "";
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS ?? "";
assert(/^[a-zA-Z0-9][a-zA-Z0-9_.-]*$/.test(observer) && isIPv4(observerIP),
    "Set QBUTT_WAN_OBSERVER to an SSH host and QBUTT_WAN_OBSERVER_IP to its public IPv4");
assert(configPath && proxyNames.length === 3 && new Set(proxyNames).size === 3,
    "Select exactly three nodes with QBUTT_WAN_PROXY_CONFIG and pipe-separated QBUTT_WAN_PROXY_NAMES");
assert(networkInterfaces()[nativeInterface]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal), "Select the physical Native interface and its IPv4");

async function run(args: string[], timeout = 30000): Promise<string> {
    const child = Bun.spawn(args, { stdout: "pipe", stderr: "pipe", timeout, windowsHide: true });
    const [code, out, err] = await Promise.all([
        child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ]);
    assert(code === 0, `${args[0]} failed (${code}): ${err.slice(0, 1000)}`);
    return out.trim();
}

const lab = await createLab("wan");
const ssh = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", observer];
const payload = randomBytes(16 * 1024 * 1024);
const pieceLength = 65536;
const rate = 256 * 1024;
const source = join(lab.root, "source");
const destination = join(lab.root, "download");
const nodes = join(lab.root, "nodes.json");
let remoteRoot = "";
let server: ReturnType<typeof Bun.spawn> | undefined;
let lines: ReturnType<typeof createInterface> | undefined;
let failure: unknown;
try {
    let document: { proxies?: Record<string, unknown>[] };
    try { document = Bun.YAML.parse(await readFile(configPath, "utf8")) as typeof document; }
    catch { throw new Error("Cannot parse the selected subscription; contents are intentionally omitted"); }
    assert(Array.isArray(document.proxies), "Subscription has no proxies list");
    const selected = proxyNames.map((name, index) => {
        const matches = document.proxies!.filter(proxy => proxy.name === name);
        assert(matches.length === 1, "Selected node name must be unique");
        return { ...matches[0], name: `wan-${index}` };
    });
    assert(new Set(selected.map(proxy => proxy.server)).size === 3, "Use three separate edge servers");
    await writeFile(nodes, JSON.stringify({ proxies: selected }));
    await mkdir(source);
    await writeFile(join(source, "wan.bin"), payload);
    const torrentPath = join(lab.root, "wan.torrent");
    const hash = await run([lab.python, "-c", `
import libtorrent as lt, pathlib, sys
assert lt.__version__ == "2.0.14.0"
files = lt.file_storage()
files.add_file("wan.bin", ${payload.length})
creator = lt.create_torrent(files, ${pieceLength}, lt.create_torrent.v1_only)
creator.set_comment("Generated legal qbutt WAN fixture; no discovery")
lt.set_piece_hashes(creator, sys.argv[1])
encoded = lt.bencode(creator.generate())
pathlib.Path(sys.argv[2]).write_bytes(encoded)
print(lt.torrent_info(encoded).info_hashes().v1)
`, source, torrentPath]);
    assert(/^[0-9a-f]{40}$/.test(hash));
    remoteRoot = await run([...ssh, "mktemp -d /tmp/qbutt-wan-XXXXXXXX"]);
    assert(/^\/tmp\/qbutt-wan-[a-zA-Z0-9]{8}$/.test(remoteRoot), "Unexpected observer temporary path");
    await run(["scp", "-q", "-o", "BatchMode=yes", join(import.meta.dir, "wan-seed.py"),
        `${observer}:${remoteRoot}/seed.py`]);
    server = Bun.spawn([...ssh, `python3 -u ${remoteRoot}/seed.py`], {
        stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(lab.root, "observer.stderr.log")), windowsHide: true,
    });
    lines = createInterface({ input: Readable.fromWeb(server.stdout as never) });
    const replies = lines[Symbol.asyncIterator]();
    const reply = async (timeout = 20000): Promise<any> => {
        let timer: ReturnType<typeof setTimeout>;
        try {
            const result = await Promise.race([replies.next(), new Promise<never>((_, reject) => {
                timer = setTimeout(() => reject(new Error("Observer response timed out")), timeout);
            })]);
            assert(!result.done, "Observer closed without a response");
            return JSON.parse(result.value);
        }
        finally { clearTimeout(timer!); }
    };
    server.stdin.write(JSON.stringify({ infoHash: hash, payload: payload.toString("base64"),
        pieceLength, count: 4, rate, duration: 240 }) + "\n");
    await server.stdin.flush();
    const ready = await reply(45000) as { ready: boolean; ports: number[]; pieceCount: number };
    assert(ready.ready && ready.ports.length === 4 && new Set(ready.ports).size === 4
        && ready.pieceCount === payload.length / pieceLength);
    const readStatus = () => lab.json<Status>("qbuttPaths/status");
    await lab.start();
    const interfaces = await lab.json<{ name: string; value: string }[]>("app/networkInterfaceList");
    const physical = interfaces.filter(item => item.name === nativeInterface || item.value === nativeInterface);
    assert(physical.length === 1, "Physical adapter has no unique application mapping");
    await lab.request("app/setPreferences", { json: JSON.stringify({ current_network_interface: physical[0]!.value,
        current_interface_address: nativeAddress, bittorrent_protocol: 1, encryption: 2,
        enable_multi_connections_from_same_ip: true }) });
    for (let side = 0; side < 3; ++side) {
        await lab.request("qbuttPaths/open", { configPath: nodes, proxyName: `wan-${side}`,
            edgeId: `wan-${side}`, interfaceName: nativeInterface });
        await waitFor("WAN path open", readStatus,
            status => !status.busy && status.paths.filter(path => path.open).length === side + 1);
    }
    await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface: physical[0]!.value });
    const status = await readStatus();
    const paths = [0, 1, 2, 3].map(side => status.paths.find(path => side === 3
        ? path.edgeId === "native" && path.localAddress === nativeAddress : path.edgeId === `wan-${side}`));
    assert(status.mode === "mixed" && paths.every(path => path?.open));
    const data = new FormData();
    data.set("torrents", Bun.file(torrentPath));
    data.set("savepath", destination);
    data.set("stopped", "true");
    data.set("autoTMM", "false");
    data.set("contentLayout", "Original");
    await lab.request("torrents/add", data);
    await waitFor("WAN torrent add", () => lab.json<{ hash: string }[]>("torrents/info"),
        torrents => torrents.length === 1 && torrents[0]!.hash === hash);
    await lab.request("torrents/start", { hashes: hash });
    for (const [side, port] of ready.ports.entries()) {
        await lab.request("torrents/addPeers", { hashes: hash, peers: `${observerIP}:${port}` });
        await waitFor("WAN peer connects through its assigned path", readStatus, current =>
            current.peers.some(peer => peer.port === port && peer.pathId === paths[side]!.pathId), 30000);
    }
    server.stdin.write(JSON.stringify({ command: "start", rate }) + "\n");
    await server.stdin.flush();
    assert((await reply()).started);
    const started = performance.now();
    const before = await waitFor("four WAN peers deliver payload", readStatus, current =>
        paths.every((path, side) => current.peers.some(peer => peer.pathId === path!.pathId
            && peer.port === ready.ports[side] && peer.payloadDownload >= pieceLength)));
    const intervalStarted = performance.now();
    await Bun.sleep(1500);
    const after = await readStatus();
    const flows = paths.map((path, side) => {
        const first = before.peers.find(peer => peer.port === ready.ports[side])!;
        const last = after.peers.find(peer => peer.port === ready.ports[side]);
        assert(last && last.pathId === path!.pathId && last.generation === path!.generation
            && last.infoHash === hash && last.payloadDownload > first.payloadDownload,
        "Each path must advance in the same interval for the same torrent");
        if (side === 3) assert(last.localAddress === nativeAddress, "Native peer left the physical interface");
        return { side, pathId: last.pathId, generation: last.generation, port: last.port,
            payloadDelta: last.payloadDownload - first.payloadDownload };
    });
    await lab.checkpoint({ check: "four-simultaneous-wan-paths", infoHash: hash,
        intervalMilliseconds: performance.now() - intervalStarted, flows });
    await waitFor("complete WAN fixture", () => lab.info(hash), info => info.progress === 1, 90000);
    const transferMilliseconds = performance.now() - started;
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("WAN torrent stopped", () => lab.info(hash), info => info.state === "stoppedUP");
    const downloaded = await readFile(join(destination, "wan.bin"));
    assert(downloaded.length === payload.length && sha256(downloaded) === sha256(payload), "WAN payload hash/size mismatch");
    server.stdin.write('{"command":"stop"}\n');
    await server.stdin.flush();
    const observed = await reply() as { stopped: boolean; connections: Connection[]; errors: string[] };
    assert(observed.stopped && observed.errors.length === 0, "Observer reported protocol errors");
    const remoteFlows = paths.map((_, side) => {
        const connections = observed.connections.filter(connection => connection.side === side && connection.payloadBytes > 0);
        assert(connections.length === 1, "WAN peer must have exactly one payload connection");
        const connection = connections[0]!;
        assert(connection.payloadBytes === payload.length / 4 && connection.requestedPieces.length === ready.pieceCount / 4
            && connection.requestedPieces.every(piece => piece % 4 === side), "Remote peer did not send its exact exclusive piece set");
        return connection;
    });
    assert(new Set(remoteFlows.map(connection => connection.remoteIP)).size === 4,
        "Native and three VPN paths must reach the observer through four distinct public IPs");
    await lab.checkpoint({ check: "remote-egress-and-verified-payload", infoHash: hash, verifiedBytes: downloaded.length,
        sha256: sha256(downloaded), exactSize: true, transferMilliseconds, remoteFlows,
        limits: "Controlled TCP fixture with manual peers and per-peer caps; no public-swarm, discovery, UDP or speedup claim" });
    server.stdin.end();
    assert(await server.exited === 0, "Observer failed to exit cleanly");
}
catch (error) { failure = error; }
finally {
    if (server?.exitCode === null) {
        server.stdin.end();
        const exited = await Promise.race([server.exited.then(() => true), Bun.sleep(5000).then(() => false)]);
        if (!exited) { server.kill(); await server.exited; }
    }
    lines?.close();
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    if (/^\/tmp\/qbutt-wan-[a-zA-Z0-9]{8}$/.test(remoteRoot)) {
        try { await run([...ssh, `rm -f -- ${remoteRoot}/seed.py && rmdir -- ${remoteRoot}`]); }
        catch (error) { failure ??= error; }
    }
    // Only generated payload and this run's private node copy are disposable.
    // Keep torrent metadata, logs and compact evidence for reproducibility.
    const ownedRoot = await realpath(lab.root);
    for (const name of ["source", "download", "fixtures", "nodes.json"]) {
        const path = join(lab.root, name);
        try {
            assert((await realpath(path)).startsWith(ownedRoot + sep), "Cleanup escaped the owned lab directory");
            await rm(path, { recursive: true, force: true });
        }
        catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") failure ??= error; }
    }
    await lab.finish(failure);
}
if (failure) throw failure;
