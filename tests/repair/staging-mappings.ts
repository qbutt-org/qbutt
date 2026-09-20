import assert from "node:assert/strict";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve, sep } from "node:path";
import { createLab, verifyPayload, waitFor } from "../lab";
import { assertRecoverySuspended, snapshot } from "./staging-checks";

interface Status {
    id: string;
    state: string;
    error?: string;
    analysis?: { files: { native_index: number; path: string }[] };
    staging?: { version: number; destination: string; payload_path: string; finalized: boolean;
        files: { index: number; path: string }[] };
}

assert(!process.env.QBUTT_LAB_RESUME_BACKEND || process.env.QBUTT_LAB_RESUME_BACKEND === "Legacy",
    "This fixture includes a legacy bencoded resume migration; use the Legacy resume backend.");
const lab = await createLab("staging-receipt-mappings");
const scenarios = ["v1-manual", "v2-autotmm-recovery", "hybrid-autotmm-rollback", "v1-legacy"];
let lock: Bun.Subprocess<"pipe", "pipe", Bun.BunFile> | undefined;
let failure: unknown;

async function status(states: string[]): Promise<Status> {
    const result = await waitFor("staging mapping transition", () => lab.json<Status>("qbuttRepair/status"),
        value => states.includes(value.state) || value.state === "failed");
    assert.notEqual(result.state, "failed", result.error);
    return result;
}

async function analyze(hash: string, mode: string, mappings?: Record<number, string>): Promise<Status> {
    return waitFor("staging mapping admission", async () => {
        await lab.request("qbuttRepair/analyze", { hash, mode, mappings: JSON.stringify(mappings ?? {}) });
        const result = await waitFor("mapping analysis", () => lab.json<Status>("qbuttRepair/status"),
            value => ["analyzed", "planned", "ready_to_commit", "committed", "failed"].includes(value.state));
        if (result.state === "failed") {
            await lab.request("qbuttRepair/cancel", { id: result.id });
            assert(result.error?.startsWith("Wait for"), result.error);
        }
        return result;
    }, result => result.state !== "failed", 15000);
}

try {
    await lab.start();
    await lab.request("app/setPreferences", { json: JSON.stringify({ locale: "en" }) });
    await lab.shutdown();
    await lab.start();
    for (const scenario of scenarios) {
        const format = scenario.split("-")[0]!;
        const autoTMM = scenario.includes("autotmm");
        const legacy = scenario.endsWith("legacy");
        const rollback = scenario.endsWith("rollback");
        const directory = join(lab.root, scenario);
        const savePath = join(directory, "completed");
        const downloadPath = legacy ? "" : join(directory, "incomplete");
        const destination = downloadPath || savePath;
        const category = `staging-${format}`;
        const data = join(lab.root, "profile", process.env.QBUTT_LAB_APP_NAME ?? "qbutt", "data");
        await mkdir(savePath, { recursive: true });
        await mkdir(destination, { recursive: true });
        await lab.request("app/setPreferences", { json: JSON.stringify({ incomplete_files_ext: !legacy,
            save_path: savePath, temp_path: destination, temp_path_enabled: !legacy }) });
        if (autoTMM)
            await lab.request("torrents/createCategory", { category, savePath,
                downloadPathEnabled: "true", downloadPath });
        const fixture = lab.manifest.torrents.find(item => item.name === format)!;
        const hash = fixture.infoHashV2?.slice(0, 40) ?? fixture.infoHashV1!;
        const add = new FormData();
        add.set("torrents", Bun.file(join(lab.fixtures, fixture.file)));
        add.set("autoTMM", String(autoTMM));
        add.set("stopped", "true");
        add.set("contentLayout", "Original");
        if (autoTMM)
            add.set("category", category);
        else {
            add.set("savepath", savePath);
            add.set("downloadPath", downloadPath);
        }
        await lab.request("torrents/add", add);
        await waitFor("staged mapping torrent admitted", () => lab.json<unknown[]>(`torrents/info?hashes=${hash}`),
            items => items.length === 1);

        async function assertSettings() {
            const properties = await lab.json<{ save_path: string; download_path: string }>(`torrents/properties?hash=${hash}`);
            assert.equal(resolve(properties.save_path), savePath, "Temporary storage replaced the logical save path");
            assert.equal(properties.download_path ? resolve(properties.download_path) : "", downloadPath,
                "Temporary storage replaced the logical download path");
            const [info] = await lab.json<{ auto_tmm: boolean; category: string }[]>(`torrents/info?hashes=${hash}`);
            assert.equal(info?.auto_tmm, autoTMM, "Staging changed AutoTMM");
            assert.equal(info?.category, autoTMM ? category : "");
        }

        const mapping = await analyze(hash, "inplace");
        await lab.request("qbuttRepair/cancel", { id: mapping.id });
        for (const file of mapping.analysis!.files) {
            const logical = fixture.files.find(item => item.index === file.native_index)!;
            assert(resolve(file.path).startsWith(`${resolve(destination)}${sep}`));
            assert.equal(file.path.endsWith(".!qB"), !legacy && logical.size !== 0);
            await mkdir(dirname(file.path), { recursive: true });
            await writeFile(file.path, await readFile(join(lab.fixtures, "variants", "grow", logical.path)));
        }
        await writeFile(join(destination, "unknown-save.dat"), "Unknown data must survive staging and native relocation.");
        const original = await snapshot(directory);
        const source = join(lab.fixtures, "seed");
        const sourceBefore = await snapshot(source);
        const mappings = Object.fromEntries(fixture.files.filter(file => !file.pad)
            .map(file => [file.index, join(source, file.path)]));
        let operation = await analyze(hash, "staged", mappings);
        assert.equal(operation.staging!.version, 3);
        assert.equal(resolve(operation.staging!.destination), destination);
        assert.deepEqual(await snapshot(directory), original, "Planning wrote original data");
        await assertSettings();
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await lab.shutdown();
        await lab.start();
        await waitFor("cancelled staged layout restored", () => lab.info(hash), info => info.state.startsWith("stopped"));
        await assertSettings();
        assert.deepEqual(await snapshot(directory), original, "Preview cancel/restart changed original data");

        operation = await analyze(hash, "staged", mappings);
        const transaction = dirname(operation.staging!.payload_path);
        await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
        const ready = await status(["ready_to_commit"]);
        await assertSettings();
        assert.deepEqual(await snapshot(directory, transaction), original, "Independent preparation modified originals");
        assert.deepEqual(await snapshot(source), sourceBefore, "Independent preparation modified its source");
        const stagedPayload = lab.manifest.payload.map(file => ({ ...file,
            path: ready.staging!.files.find(entry => fixture.files.find(item => item.index === entry.index)!.path === file.path)!.path }));
        await verifyPayload(ready.staging!.payload_path, stagedPayload);

        if (autoTMM || legacy) {
            await lab.request("qbuttRepair/cancel", { id: operation.id });
            await lab.shutdown();
            if (legacy) {
                const journalPath = join(data, "staging", `${hash}.json`);
                const journal = JSON.parse(await readFile(journalPath, "utf8"));
                journal.version = 2;
                await writeFile(journalPath, JSON.stringify(journal));
                const migrate = Bun.spawn([lab.python, "-c", `
import libtorrent as lt, pathlib, sys
path = pathlib.Path(sys.argv[1])
resume = lt.bdecode(path.read_bytes())
resume[b"qBt-savePath"] = sys.argv[2].encode()
resume[b"qBt-downloadPath"] = b""
path.write_bytes(lt.bencode(resume))
`, join(data, "BT_backup", `${hash}.fastresume`), ready.staging!.payload_path], {
                    stdout: "pipe", stderr: "pipe", windowsHide: true });
                assert.equal(await migrate.exited, 0, await new Response(migrate.stderr).text());
            }
            await lab.start();
            await waitFor("pending staged layout suspended", () => lab.info(hash),
                info => info.state.startsWith("stopped") || info.state === "missingFiles");
            if (autoTMM)
                await assertSettings();
            await assertRecoverySuspended(lab, hash);
            assert.deepEqual(await snapshot(directory, transaction), original, "Hold startup wrote original data");
            operation = await analyze(hash, "recover");
        }

        if (rollback) {
            await lab.request("qbuttRepair/rollback", { id: operation.id, consent: "true" });
            const rolledBack = await waitFor("mapped rollback persisted", () => lab.json<Status>("qbuttRepair/status"),
                value => value.staging?.finalized === true || value.state === "failed");
            assert.equal(rolledBack.state, "rolled_back", rolledBack.error);
            await assertSettings();
            assert.deepEqual(await snapshot(directory, transaction), original, "Rollback changed original data");
            await lab.request("qbuttRepair/cancel", { id: operation.id });
            await lab.shutdown();
            await lab.start();
            await waitFor("rolled-back layout restarted", () => lab.info(hash), info => info.state.startsWith("stopped"));
            await assertSettings();
            assert.deepEqual(await snapshot(directory, transaction), original, "Rollback restart changed original data");
            await lab.checkpoint({ scenario, check: "staged-layout-rollback-preserves-original-and-logical-settings" });
            await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
            await waitFor("rolled-back torrent removed", () => lab.json<unknown[]>("torrents/info"), items => items.length === 0);
            continue;
        }
        if (autoTMM) {
            lock = Bun.spawn([lab.python, join(import.meta.dir, "resume-lock.py"), "Legacy", lab.root,
                join(data, "BT_backup", `${hash}.fastresume`)],
            { stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(lab.root, "lock.stderr.log")),
                timeout: 90000, windowsHide: true });
            const reader = lock.stdout.getReader();
            let output = "";
            try {
                while (!output.includes("\n")) {
                    const chunk = await reader.read();
                    assert(!chunk.done, "Resume storage lock ended before readiness");
                    output += new TextDecoder().decode(chunk.value);
                }
                assert(JSON.parse(output.trim()).ready, "Resume storage lock not acquired");
            }
            finally { reader.releaseLock(); }
        }
        await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
        if (autoTMM) {
            const failed = await waitFor("mapped commit resume persistence failure", () => lab.json<Status>("qbuttRepair/status"),
                value => value.state === "failed" || value.staging?.finalized === true);
            assert.equal(failed.state, "failed");
            assert.match(failed.error ?? "", /location.*saved|saved.*location/i);
            assert(!failed.staging?.finalized, "Failed persistence retired the recovery journal");
            assert.equal(JSON.parse(await readFile(join(data, "staging", `${hash}.json`), "utf8")).state, "committed");
            await assertSettings();
            await verifyPayload(destination, stagedPayload);
            await assertRecoverySuspended(lab, hash);
            lock!.stdin.end();
            assert.equal(await lock!.exited, 0, "Resume storage lock did not release cleanly");
            lock = undefined;
            await lab.request("qbuttRepair/cancel", { id: operation.id });
            await lab.shutdown();
            await lab.start();
            await waitFor("interrupted commit suspended", () => lab.info(hash),
                info => info.state.startsWith("stopped") || info.state === "missingFiles");
            await assertSettings();
            await assertRecoverySuspended(lab, hash);
            operation = await analyze(hash, "recover");
            assert.equal(operation.state, "committed");
            await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
        }
        const committed = await waitFor("mapped commit persisted", () => lab.json<Status>("qbuttRepair/status"),
            value => value.staging?.finalized === true || value.state === "failed");
        assert.equal(committed.state, "committed", committed.error);
        await assertSettings();
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        if (autoTMM || legacy)
            await lab.request("torrents/start", { hashes: hash });
        await waitFor("committed data checked by the native engine", () => lab.info(hash), info => info.progress === 1);
        await waitFor("ordinary final layout", async () => Promise.all(lab.manifest.payload
            .map(file => Bun.file(join(savePath, file.path)).exists())), files => files.every(Boolean));
        const verifiedBytes = await verifyPayload(savePath, lab.manifest.payload);
        await lab.request("torrents/stop", { hashes: hash });
        await waitFor("committed torrent stopped", () => lab.info(hash), info => info.state === "stoppedUP");
        assert.equal(await readFile(join(destination, "unknown-save.dat"), "utf8"),
            "Unknown data must survive staging and native relocation.");
        assert.deepEqual(await snapshot(source), sourceBefore);
        await lab.shutdown();
        await lab.start();
        await waitFor("final layout restarted", () => lab.info(hash), info => info.state === "stoppedUP");
        await assertSettings();
        assert.equal(await verifyPayload(savePath, lab.manifest.payload), verifiedBytes);
        await lab.checkpoint({ scenario, check: "staged-native-layout-and-persisted-logical-settings", verifiedBytes,
            exactSizes: true, originalUnchangedUntilCommit: true, cancelRestart: true,
            failedResumeReceiptRecovery: autoTMM, legacyVersion2Recovery: legacy, completedNamesAndDirectory: true });
        await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
        await waitFor("mapped staged torrent removed", () => lab.json<unknown[]>("torrents/info"), items => items.length === 0);
    }
}
catch (error) { failure = error; }
finally {
    if (lock && lock.exitCode === null) {
        lock.stdin.end();
        if (await lock.exited !== 0)
            failure ??= new Error("Resume storage lock failed during cleanup");
    }
    try { await lab.shutdown(); }
    catch (error) { failure ??= error; }
    for (const name of ["fixtures", "profile", ...scenarios]) {
        try {
            const target = resolve(lab.root, name);
            assert(target.startsWith(`${resolve(lab.root)}${sep}`), "Cleanup escaped owned fixture");
            await rm(target, { recursive: true, force: true });
        }
        catch (error) { failure ??= error; }
    }
    await lab.finish(failure);
}
if (failure)
    throw failure;
