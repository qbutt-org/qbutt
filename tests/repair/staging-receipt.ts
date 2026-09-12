import assert from "node:assert/strict";
import { cp, readFile, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createLab, verifyPayload, waitFor } from "../lab";
import { assertRecoverySuspended, snapshot } from "./staging-checks";

interface Status {
    id: string;
    state: string;
    error?: string;
    staging?: { finalized: boolean };
}

const lab = await createLab("staging-receipt");
const backend = process.env.QBUTT_LAB_RESUME_BACKEND ?? "Legacy";
let lock: Bun.Subprocess<"pipe", "pipe", Bun.BunFile> | undefined;
let failure: unknown;
try {
    const destination = join(lab.root, "target");
    await cp(join(lab.fixtures, "variants", "grow"), destination, { recursive: true });
    await writeFile(join(destination, "unknown-save.dat"), "preserve during failed resume persistence");
    await lab.start();
    const hash = await lab.add("v1", destination);
    await waitFor("receipt target stopped", () => lab.info(hash), info => info.state.startsWith("stopped"));
    let operation = await (await lab.request("qbuttRepair/analyze", {
        hash, mode: "staged", sources: JSON.stringify([join(lab.fixtures, "seed")]),
    })).json() as Status;
    const planned = await waitFor("receipt staging plan", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "planned" || status.state === "failed");
    assert.equal(planned.state, "planned", planned.error);
    await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
    const ready = await waitFor("receipt staging ready", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "ready_to_commit" || status.state === "failed");
    assert.equal(ready.state, "ready_to_commit", ready.error);
    const dataPath = join(lab.root, "profile", process.env.QBUTT_LAB_APP_NAME ?? "qbutt", "data");
    const storagePath = backend === "SQLite" ? join(dataPath, "torrents.db") : join(dataPath, "BT_backup", `${hash}.fastresume`);
    assert((await stat(storagePath)).isFile(), "Requested native backend did not create its storage file");
    lock = Bun.spawn([lab.python, join(import.meta.dir, "resume-lock.py"), backend, lab.root, storagePath],
        { stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(lab.root, "lock.stderr.log")), timeout: 90000 });
    const reader = lock.stdout.getReader();
    let output = "";
    try {
        while (!output.includes("\n")) {
            const chunk = await reader.read();
            assert(!chunk.done, "Native storage lock ended before readiness");
            output += new TextDecoder().decode(chunk.value);
        }
        const locked = JSON.parse(output.trim()) as { ready: boolean; backend: string };
        assert(locked.ready && locked.backend === backend, "Native storage lock not acquired");
    }
    finally { reader.releaseLock(); }
    await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
    const failed = await waitFor("final resume storage failure", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "failed" || status.staging?.finalized === true, 45000);
    assert.equal(failed.state, "failed", "Commit retired the journal without a persisted resume receipt");
    assert.match(failed.error ?? "", /location.*saved|saved.*location/i, "Failure did not reach final resume persistence");
    assert(!failed.staging?.finalized, "Failed persistence was published as finalized");
    const journalPath = join(dataPath, "staging", `${hash}.json`);
    assert.equal(JSON.parse(await readFile(journalPath, "utf8")).state, "committed", "Recoverable committed journal was not retained");
    await verifyPayload(destination, lab.manifest.payload);
    await assertRecoverySuspended(lab, hash);
    lock.stdin.end();
    assert.equal(await lock.exited, 0, "Storage lock did not release cleanly");
    await lab.request("qbuttRepair/cancel", { id: operation.id });
    await lab.shutdown();
    await lab.start();
    await waitFor("receipt recovery torrent initialized", () => lab.info(hash),
        info => info.state.startsWith("stopped") || info.state === "missingFiles");
    operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as Status;
    const recovered = await waitFor("failed receipt recovered", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "committed" || status.state === "failed");
    assert.equal(recovered.state, "committed", recovered.error);
    await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
    const finalized = await waitFor("retry persisted and retired", () => lab.json<Status>("qbuttRepair/status"),
        status => status.staging?.finalized === true || status.state === "failed");
    assert.equal(finalized.state, "committed", finalized.error);
    await assert.rejects(stat(journalPath), { code: "ENOENT" }, "Finalized journal still active");
    await lab.request("qbuttRepair/cancel", { id: operation.id });
    const finalFiles = await snapshot(destination);
    await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
    await lab.shutdown();
    assert.deepEqual(await snapshot(destination), finalFiles, "Removal/shutdown changed committed target or backups");
    await verifyPayload(destination, lab.manifest.payload);
    await verifyPayload(join(lab.fixtures, "seed"), lab.manifest.payload);
    assert.equal(await readFile(join(destination, "unknown-save.dat"), "utf8"), "preserve during failed resume persistence");
    await lab.checkpoint({ check: "failed-resume-receipt-retains-journal-until-recovered", backend, afterShutdown: true });
}
catch (error) { failure = error; }
finally {
    if (lock && lock.exitCode === null) {
        lock.stdin.end();
        if (await lock.exited !== 0)
            failure ??= new Error("Storage lock failed during cleanup");
    }
    try { await lab.shutdown(); }
    catch (error) { failure ??= error; }
}
await lab.finish(failure);
if (failure)
    throw failure;
