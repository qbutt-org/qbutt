import { appendFile, readFile, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createLab, requireCondition, startSeed, verifyPayload, waitFor, type TorrentFile } from "./lab";

const lab = await createLab("native");
let failure: unknown;
try {
    await lab.start();
    for (const name of ["v1", "v2", "hybrid"]) {
        const destination = join(lab.root, "downloads", name);
        const seed = await startSeed(lab.python, lab.fixtures, name, lab.root);
        try {
            const hash = await lab.add(name, destination);
            const peer = `${seed.host}:${seed.port}`;
            const files = await lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`);
            const skipped = files.find(file => file.name.endsWith("/skip.bin"));
            requireCondition(skipped, "Selective fixture must contain skip.bin");
            await lab.request("torrents/filePrio", { hash, id: String(skipped.index), priority: "0" });
            await lab.request("torrents/start", { hashes: hash });
            await lab.request("torrents/addPeers", { hashes: hash, peers: peer });
            await waitFor("selective download", () => lab.info(hash), info => info.progress === 1);
            const selection = await lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`);
            const skippedStatus = selection.find(file => file.index === skipped.index)!;
            requireCondition(skippedStatus.priority === 0 && skippedStatus.progress < 1, "Skipped file was not excluded from download");
            await lab.request("torrents/stop", { hashes: hash });
            await waitFor("selective stop", () => lab.info(hash), info => info.state.startsWith("stopped"));
            const selectedBytes = await verifyPayload(destination, lab.manifest.payload.filter(file => !file.path.endsWith("/skip.bin")));
            await lab.checkpoint({ name, check: "selective-download", verifiedBytes: selectedBytes, skippedIncomplete: true });

            // A completed selective job disconnects the only peer. A clean restart
            // avoids waiting for libtorrent's normal per-endpoint retry cooldown.
            await lab.shutdown();
            await lab.start();

            await lab.request("torrents/filePrio", { hash, id: String(skipped.index), priority: "1" });
            await lab.request("torrents/start", { hashes: hash });
            await lab.request("torrents/addPeers", { hashes: hash, peers: peer });
            await waitFor("partial download", () => lab.info(hash), info => info.completed > selectedBytes + 32768 && info.progress < 1);
            await lab.request("torrents/stop", { hashes: hash });
            await waitFor("partial stop", () => lab.info(hash), info => info.state === "stoppedDL");
            const stopped = await lab.info(hash);
            await Bun.sleep(400);
            requireCondition((await lab.info(hash)).completed === stopped.completed, "Stopped job continued completing data");
            await lab.shutdown();
            await lab.start();
            const restored = await waitFor("partial torrent restoration", () => lab.info(hash), info => info.state === "stoppedDL");
            requireCondition(restored.state === "stoppedDL" && restored.progress > 0 && restored.progress < 1,
                "Restart did not restore stopped partial torrent");
            await lab.request("torrents/start", { hashes: hash });
            await lab.request("torrents/addPeers", { hashes: hash, peers: peer });
            await waitFor("full download", () => lab.info(hash), info => info.progress === 1);
            await lab.request("torrents/stop", { hashes: hash });
            await waitFor("full stop", () => lab.info(hash), info => info.state === "stoppedUP");
            const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
            await lab.request("torrents/recheck", { hashes: hash });
            // The API exposes a cached libtorrent status (default refresh 1.5s).
            // Do not accept the pre-request stoppedUP snapshot as completion.
            await Bun.sleep(2000);
            await waitFor("recheck completion", () => lab.info(hash), info => info.progress === 1 && info.state === "stoppedUP");
            requireCondition(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes, "Recheck changed verified data");
            await lab.checkpoint({ name, check: "download-stop-restart-resume-recheck", verifiedBytes, exactSizes: true });

            if (name === "v1") {
                // Ordinary recheck verifies torrent ranges, not an overlong file's tail.
                const alpha = join(destination, "bundle", "alpha.bin");
                const before = await readFile(alpha);
                const corrupted = Buffer.from(before);
                corrupted[0] = corrupted[0]! ^ 0xff;
                await writeFile(alpha, corrupted);
                await lab.request("torrents/recheck", { hashes: hash });
                await waitFor("corruption recheck", () => lab.info(hash), info => info.progress < 1 && info.state === "stoppedDL");
                await writeFile(alpha, before);
                await appendFile(alpha, Buffer.alloc(8193, 0xa5));
                await lab.request("torrents/recheck", { hashes: hash });
                await Bun.sleep(2000);
                await waitFor("overlong recheck", () => lab.info(hash), info => info.progress === 1 && info.state === "stoppedUP");
                const tailPreserved = (await stat(alpha)).size === before.length + 8193;
                requireCondition(tailPreserved, "Baseline assumption changed: ordinary recheck now truncates extra tails");
                await lab.checkpoint({ name, check: "ordinary-recheck-does-not-repair-tail", hashComplete: true, extraTailBytes: 8193 });
                // Restore only this lab-owned generated payload before final verification.
                await writeFile(alpha, before);
            }
            await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
            requireCondition(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes, "Remove torrent deleted payload");
        }
        finally {
            await seed.stop();
        }
    }
    await lab.shutdown();
}
catch (error) {
    failure = error;
    try { await lab.shutdown(); }
    catch (shutdownError) { console.error(String(shutdownError)); }
}
await lab.finish(failure);
if (failure)
    throw failure;
