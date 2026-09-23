import assert from "node:assert/strict";
import { mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor, type TorrentStatus } from "./lab";

interface Claim { hash: string; rule: string; status: string; actions: string[] }
interface Preview { hash: string; ready: boolean }

const lab = await createLab("auto-remove");
const appName = process.env.QBUTT_LAB_APP_NAME ?? "qbutt";
assert.equal(appName, "qbutt", "Auto-remove acceptance requires qbutt");
const configPath = join(lab.root, "profile", appName, "config", `${appName}.ini`);
const destinations = ["default-on", "disabled", "explicit-stop"].map(name => join(lab.root, name));
const unknownPath = join(destinations[0]!, "user-notes.txt");
const unknownBytes = Buffer.from("Keep this unrelated file after torrent removal.\n");
let failure: unknown;

async function download(name: string, destination: string, mode: "keep" | "remove" | "stop"): Promise<string> {
    const seed = await startSeed(lab.python, lab.fixtures, name, lab.root, { uploadRate: 1024 * 1024 });
    try {
        const hash = await lab.add(name, destination);
        if (mode === "stop")
            await lab.request("torrents/setCategory", { hashes: hash, category: "keep" });
        await lab.request("torrents/start", { hashes: hash });
        await lab.request("torrents/addPeers", { hashes: hash, peers: `${seed.host}:${seed.port}` });
        if (mode === "remove") {
            await waitFor("completed download automatically removed", () => lab.json<TorrentStatus[]>(`torrents/info?hashes=${hash}`),
                items => items.length === 0);
        }
        else {
            await waitFor("download complete", () => lab.info(hash), item => item.progress === 1);
            await waitFor("completion disk and resume barrier", () => lab.json<Preview[]>("qbuttPolicies/preview"),
                items => items.some(item => item.hash === hash && item.ready));
        }
        if (mode !== "keep") {
            const entries = await waitFor("completion receipt dispatched", () => lab.json<Claim[]>("qbuttPolicies/journal"),
                items => items.some(item => item.hash === hash && item.status === "dispatched"));
            const claims = entries.filter(item => item.hash === hash);
            assert.equal(claims.length, 1, "The same completion dispatched multiple actions");
            assert.deepEqual(claims[0]!.actions, [mode === "remove" ? "remove_torrent" : "stop"]);
            if (mode === "stop")
                await waitFor("explicit stop rule retains the torrent", () => lab.info(hash), item => item.state === "stoppedUP");
        }
        const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
        const traffic = await seed.stop();
        assert(traffic.uploadPayloadBytes >= verifiedBytes, "The local peer did not supply the complete payload");
        assert.equal(traffic.downloadPayloadBytes, 0, "The controlled seed downloaded payload");
        await lab.checkpoint({ check: mode, hash, verifiedBytes, exactSizes: true, filesPreserved: true });
        return hash;
    }
    finally {
        await seed.stop();
    }
}

try {
    const fixtureConfig = (await readFile(configPath, "utf8")).replaceAll("\r\n", "\n");
    assert(fixtureConfig.includes("Downloads\\AutoRemoveCompletedTorrents=false\n"),
        "The shared lab must pin its original keep-torrent behavior");
    await writeFile(configPath, fixtureConfig.replace("Downloads\\AutoRemoveCompletedTorrents=false\n", ""));
    await mkdir(destinations[0]!, { recursive: true });
    await writeFile(unknownPath, unknownBytes);
    await lab.start();
    const removedHash = await download("v1", destinations[0]!, "remove");
    assert.deepEqual(await readFile(unknownPath), unknownBytes, "Automatic removal changed an unrelated file");
    await lab.shutdown();

    // An explicit disabled setting preserves the user's earlier behavior.
    const config = (await readFile(configPath, "utf8")).replaceAll("\r\n", "\n")
        .replace(/^Downloads\\AutoRemoveCompletedTorrents=.*\n/gm, "");
    assert(config.includes("[Preferences]\n"), "The isolated profile has no Preferences section");
    await writeFile(configPath, config.replace("[Preferences]\n",
        "[Preferences]\nDownloads\\AutoRemoveCompletedTorrents=false\n"));
    await lab.start();
    const retainedHash = await download("v1-64k", destinations[1]!, "keep");
    assert.equal((await lab.info(retainedHash)).progress, 1, "Explicit disabled setting removed a completed torrent");
    await lab.shutdown();

    const disabledConfig = (await readFile(configPath, "utf8")).replaceAll("\r\n", "\n");
    await writeFile(configPath, disabledConfig.replace("Downloads\\AutoRemoveCompletedTorrents=false\n",
        "Downloads\\AutoRemoveCompletedTorrents=true\n"));
    await lab.start();
    await waitFor("previous completed torrent retained after enabling", () => lab.info(retainedHash),
        item => item.progress === 1);

    await lab.request("torrents/createCategory", { category: "keep" });
    await lab.request("qbuttPolicies/configure", { configuration: JSON.stringify({ enabled: true, allow_delete_data: false,
        rules: [{ id: "explicit-stop", enabled: true, match: { category: "keep" }, actions: ["stop"] }] }) });
    const stoppedHash = await download("v2", destinations[2]!, "stop");
    await lab.shutdown();
    await lab.start();
    const restored = await waitFor("retained completed torrents restored", () => lab.json<TorrentStatus[]>("torrents/info"),
        items => items.length === 2 && items.every(item => item.progress === 1));
    assert.deepEqual(restored.map(item => item.hash).sort(), [retainedHash, stoppedHash].sort(),
        "Restart lost a retained torrent or restored the automatically removed one");
    const receipts = await lab.json<Claim[]>("qbuttPolicies/journal");
    assert.equal(receipts.length, 2, "Restart replayed a completion action");
    assert(receipts.some(item => item.hash === removedHash && item.actions.includes("remove_torrent")));
    await lab.shutdown();
    for (const destination of destinations)
        await verifyPayload(destination, lab.manifest.payload);
    assert.deepEqual(await readFile(unknownPath), unknownBytes, "Restart changed an unrelated file after automatic removal");
    await lab.checkpoint({ check: "restart-and-final-payload", explicitlyDisabledRetained: true,
        explicitStopRetained: true, removedTorrentAbsent: true, allPayloadsPreserved: true, unknownFilePreserved: true });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    const root = await realpath(lab.root);
    for (const path of [lab.fixtures, join(lab.root, "profile"), ...destinations]) {
        const target = await realpath(path).catch(() => undefined);
        if (!target) continue;
        assert.equal(dirname(target), root, "Cleanup escaped the isolated fixture");
        try { await rm(target, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
