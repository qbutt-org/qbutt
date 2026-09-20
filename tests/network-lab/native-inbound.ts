import assert from "node:assert/strict";
import { rm } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";

interface PathStatus {
    open: boolean; pinned: boolean; processId: number; paths: unknown[];
    torrent: { knownPeers: number; connections: number };
}
interface Peer { ip: string; port: number; flags: string; connection: string; downloaded: number }

const baseline = process.argv.includes("--baseline");
assert.equal(process.env.QBUTT_LAB_APP_NAME ?? "qbutt", baseline ? "qBittorrent" : "qbutt");
const lab = await createLab("native-inbound-utp", { protocol: "UTP" });
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let failure: unknown;
try {
    await lab.start();
    const preferences = await lab.json<Record<string, unknown>>("app/preferences");
    assert.equal(preferences.bittorrent_protocol, 2, "Native fixture did not start uTP-only");
    const listenPort = Number(preferences.listen_port);
    assert(Number.isInteger(listenPort) && listenPort > 0 && listenPort < 65536);
    const probe = Bun.spawn(["pwsh.exe", "-NoProfile", "-NonInteractive", "-Command", String.raw`
$ErrorActionPreference = 'Stop'
$owned = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $env:QBUTT_LAB_EXE -and $_.CommandLine.Contains($env:QBUTT_INBOUND_PROFILE) })
if ($owned.Count -ne 1) { throw 'Could not identify the single isolated app process' }
@(Get-NetUDPEndpoint -OwningProcess $owned[0].ProcessId | Select-Object LocalAddress,LocalPort) | ConvertTo-Json -Compress
`], { env: { ...process.env, QBUTT_INBOUND_PROFILE: join(lab.root, "profile") },
        stdout: "pipe", stderr: "pipe", timeout: 10000, windowsHide: true });
    const [exit, output, error] = await Promise.all([probe.exited, new Response(probe.stdout).text(), new Response(probe.stderr).text()]);
    assert.equal(exit, 0, error);
    const udp = [JSON.parse(output)].flat() as { LocalAddress: string; LocalPort: number }[];
    await lab.checkpoint({ check: "native-listener", baseline, listenPort, protocol: preferences.bittorrent_protocol, udp });
    assert(udp.some(endpoint => endpoint.LocalAddress === "127.0.0.1" && endpoint.LocalPort === listenPort),
        "Configured qbutt listening port is not actually bound for UDP");
    const destination = join(lab.root, "downloads");
    const hash = await lab.add("v1", destination);
    await lab.request("torrents/start", { hashes: hash });
    if (!baseline) {
        const before = await lab.json<PathStatus>(`qbuttPaths/status?hash=${hash}`);
        assert(!before.open && !before.pinned && before.processId === 0 && before.paths.length === 0,
            "Native incoming regression must have no managed paths or transport child");
        assert(before.torrent.knownPeers === 0 && before.torrent.connections === 0,
            "The Native downloader already knows a peer before the independent seed initiates");
    }
    seed = await startSeed(lab.python, lab.fixtures, "v1", lab.root, {
        listenAddress: "127.0.0.2", uploadRate: 128 * 1024, transport: "utp",
        neighbor: { host: "127.0.0.1", port: listenPort },
    });
    const peers = await waitFor("ordinary Native incoming uTP payload", async () => Object.values(
        (await lab.json<{ peers: Record<string, Peer> }>(`sync/torrentPeers?hash=${hash}`)).peers),
    values => values.some(peer => peer.downloaded > 0), 30000);
    assert(peers.length === 1 && peers[0]!.ip === "127.0.0.2"
        && peers[0]!.flags.includes("I") && peers[0]!.flags.includes("P"),
    "The only Native peer must be an incoming uTP connection");
    await waitFor("Native incoming uTP completion", () => lab.info(hash), info => info.progress === 1, 60000);
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("Native incoming payload stop", () => lab.info(hash), info => info.state === "stoppedUP");
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    const final = await seed.stop();
    seed = undefined;
    assert.equal(final.downloadPayloadBytes, 0, "Seed became a second payload writer");
    assert(final.uploadPayloadBytes >= verifiedBytes, "Independent seed did not supply the verified data");
    assert.deepEqual(final.peerAddresses, ["127.0.0.1"]);
    assert.deepEqual(final.outgoingPeerAddresses, ["127.0.0.1"]);
    assert.equal(await verifyPayload(join(lab.fixtures, "seed"), lab.manifest.payload), verifiedBytes);
    if (!baseline) {
        const after = await lab.json<PathStatus>(`qbuttPaths/status?hash=${hash}`);
        assert(!after.open && !after.pinned && after.processId === 0 && after.paths.length === 0);
    }
    await lab.checkpoint({ check: "native-incoming-utp", listenPort, peer: peers[0], verifiedBytes,
        exactSizes: true, hashesVerified: true, noManagedPaths: true, noTCP: true, seed: final });
}
catch (error) {
    failure = error;
    try { await lab.checkpoint({ check: "native-incoming-failure", appLog: (await lab.json<unknown[]>(
        "log/main?normal=true&info=true&warning=true&critical=true&last_known_id=-1")).slice(-30) }); } catch {}
}
finally {
    try { if (seed) await seed.stop(); } catch (error) { failure ??= error; }
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    for (const name of ["fixtures", "profile", "downloads"]) {
        const path = resolve(lab.root, name);
        assert.equal(dirname(path), resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
