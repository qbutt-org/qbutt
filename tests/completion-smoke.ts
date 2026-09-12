import assert from "node:assert/strict";
import { cp, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "./lab";

interface RepairStatus {
    id: string;
    state: string;
    error?: string;
    analysis?: { expected_bytes: number; verified_bytes: number };
}

interface LogMessage { message: string }

const completionMessage = 'Torrent download finished. Torrent: "completion-download"';

// Exercise the normal completion action with and without a held repair.
// This does not inject the queued-signal or nested-dialog race windows.
for (const holdRepair of [false, true]) {
    const lab = await createLab(holdRepair ? "completion-repair" : "completion-control");
    let failure: unknown;
    try {
        const appName = process.env.QBUTT_LAB_APP_NAME ?? "qbutt";
        const configPath = join(lab.root, "profile", appName, "config", `${appName}.ini`);
        const config = await readFile(configPath, "utf8");
        assert(config.includes("[Preferences]\n"), "Generated lab profile has no preferences section");
        await writeFile(configPath, config.replace("[Preferences]\n",
            "[Preferences]\nGeneral\\Locale=en\nDownloads\\AutoShutDownqBTOnCompletion=true\n")
            + "[ShutdownConfirmDlg]\nDontConfirmAutoExit=true\n");
        await lab.start();

        const destination = join(lab.root, "download");
        const hash = await lab.add("v1-64k", destination);
        await lab.request("torrents/rename", { hash, name: "completion-download" });
        await lab.request("torrents/start", { hashes: hash });
        // Keep an unfinished job active while the preseeded candidate is
        // checked, so its setup cannot trigger the configured exit action.
        await waitFor("unfinished completion trigger", () => lab.info(hash), info => info.state === "stalledDL");

        let operation: RepairStatus | undefined;
        const candidate = join(lab.root, "candidate");
        if (holdRepair) {
            await cp(join(lab.fixtures, "variants", "grow"), candidate, { recursive: true });
            const repairHash = await lab.add("v1", candidate);
            await lab.request("torrents/recheck", { hashes: repairHash });
            await waitFor("completed repair candidate", () => lab.info(repairHash),
                info => info.state === "stoppedUP" && info.progress === 1);
            await lab.request("qbuttRepair/analyze", { hash: repairHash });
            operation = await waitFor("held read-only repair", () => lab.json<RepairStatus>("qbuttRepair/status"),
                status => status.state === "analyzed" || status.state === "failed");
            assert(operation.state === "analyzed" && operation.analysis,
                `Repair analysis failed: ${operation.error}`);
            assert(operation.analysis.verified_bytes === operation.analysis.expected_bytes,
                "Grow fixture did not retain fully verified torrent content");
        }

        const seed = await startSeed(lab.python, lab.fixtures, "v1-64k", lab.root);
        try {
            await lab.request("torrents/addPeers", { hashes: hash, peers: `${seed.host}:${seed.port}` });
            if (operation) {
                await waitFor("download completed while repair held", () => lab.info(hash), info => info.progress === 1);
                await waitFor("native completion event", () => lab.json<LogMessage[]>("log/main"),
                    entries => entries.some(entry => entry.message === completionMessage));
                await lab.request("torrents/stop", { hashes: hash });
                await waitFor("completed download stopped", () => lab.info(hash), info => info.state === "stoppedUP");
                // The public status is cached; allow completion alerts and the
                // queued application action to run before the final observation.
                await Bun.sleep(2000);
                const retained = await lab.json<RepairStatus>("qbuttRepair/status");
                assert(retained.id === operation.id && retained.state === "analyzed",
                    "Completion released or changed the held repair operation");
                assert(lab.exitCode === null, "Completion exited the app while repair was held");
                assert.deepEqual(await readFile(join(candidate, "bundle", "alpha.bin")),
                    await readFile(join(lab.fixtures, "variants", "grow", "bundle", "alpha.bin")),
                    "Completion changed the repair candidate's oversized file");
                const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
                await lab.checkpoint({ check: "completion-autoexit-blocked-by-repair", verifiedBytes, exactSizes: true,
                    completionEvent: true, repairState: retained.state, appAlive: true });
                await lab.request("qbuttRepair/cancel", { id: operation.id });
            }
            else {
                const exitCode = await waitFor("completion autoexit", async () => lab.exitCode, code => code !== null);
                assert(exitCode === 0, `Completion action exited with code ${exitCode}`);
                const log = await readFile(join(lab.root, "profile", appName, "data", "logs", "qbutt.log"), "utf8");
                assert(log.includes(completionMessage) && log.includes("qbutt is now ready to exit"),
                    "Natural app exit has no native completion and orderly shutdown evidence");
                const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
                await lab.checkpoint({ check: "completion-autoexit-positive-control", verifiedBytes, exactSizes: true, exitCode });
            }
        }
        finally {
            await seed.stop();
        }
        await lab.shutdown();
        if (holdRepair)
            await verifyPayload(destination, lab.manifest.payload);
    }
    catch (error) {
        failure = error;
        try { await lab.shutdown(); }
        catch (shutdownError) { console.error(String(shutdownError)); }
    }
    await lab.finish(failure);
    if (failure)
        throw failure;
}
