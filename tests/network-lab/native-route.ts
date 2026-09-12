import assert from "node:assert/strict";
import { networkInterfaces } from "node:os";
import { join } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";

const interfaceName = process.env.QBUTT_LAB_NATIVE_INTERFACE;
const nativeAddress = process.env.QBUTT_LAB_NATIVE_ADDRESS;
assert(interfaceName && nativeAddress, "Set QBUTT_LAB_NATIVE_INTERFACE and QBUTT_LAB_NATIVE_ADDRESS");
assert(networkInterfaces()[interfaceName]?.some(address => address.address === nativeAddress
    && address.family === "IPv4" && !address.internal), "Native fixture address must belong to the selected local adapter");

interface PathStatus {
    mode: string;
    paths: { pathId: string; edgeId: string; localAddress?: string; open: boolean }[];
    peers: { pathId: string; peer: string; port: number; localAddress: string; payloadDownload: number }[];
}

const lab = await createLab("native-route");
const seed = await startSeed(lab.python, lab.fixtures, "v1-public", lab.root,
    { listenAddress: nativeAddress, label: "native-route", uploadRate: 64 * 1024 });
let failure: unknown;
try {
    await lab.start();
    await lab.request("app/setPreferences", { json: JSON.stringify({ bittorrent_protocol: 1 }) });
    await waitFor("TCP-only peer protocol", () => lab.json<Record<string, unknown>>("app/preferences"),
        preferences => preferences.bittorrent_protocol === 1);
    await lab.request("qbuttPaths/policy", { mode: "mixed", nativeInterface: interfaceName });
    const selected = await lab.json<PathStatus>("qbuttPaths/status");
    const native = selected.paths.find(path => path.edgeId === "native");
    assert(selected.mode === "mixed" && native?.open && native.localAddress === nativeAddress,
        "Physical Native route was not admitted");
    const destination = join(lab.root, "download");
    const hash = await lab.add("v1-public", destination);
    await lab.request("torrents/start", { hashes: hash });
    await lab.request("torrents/addPeers", { hashes: hash, peers: `${nativeAddress}:${seed.port}` });
    const flowing = await waitFor("physical Native peer payload", () => lab.json<PathStatus>("qbuttPaths/status"),
        status => status.peers.some(peer => peer.pathId === native.pathId && peer.peer === nativeAddress
            && peer.port === seed.port && peer.localAddress === nativeAddress && peer.payloadDownload > 16384));
    assert(flowing.peers.length === 1, "Native-only fixture used an unexpected peer route");
    await waitFor("physical Native download", () => lab.info(hash), info => info.progress === 1, 60000);
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    await lab.checkpoint({ check: "physical-native-source-binding", verifiedBytes, localAddress: nativeAddress,
        routePathId: native.pathId, publicEgress: "not-tested" });
    await lab.shutdown();
}
catch (error) {
    failure = error;
    try { await lab.shutdown(); } catch (shutdownError) { console.error(String(shutdownError)); }
}
finally {
    const stopped = await seed.stop();
    await lab.checkpoint({ check: "physical-native-seed-shutdown", ...stopped });
    if (stopped.peerAddresses.length !== 1 || stopped.peerAddresses[0] !== nativeAddress)
        failure ??= new Error("Native seed did not observe the selected physical source address");
}
await lab.finish(failure);
if (failure) throw failure;
