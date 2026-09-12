import assert from "node:assert/strict";
import { cp, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, verifyPayload, waitFor } from "../lab";

interface Status {
    id: string;
    state: string;
    error?: string;
    staging?: { payload_path: string; finalized: boolean };
}

async function snapshot(root: string, prefix = ""): Promise<Record<string, string>> {
    const result: Record<string, string> = {};
    for (const entry of await readdir(join(root, prefix), { withFileTypes: true })) {
        if (entry.name.startsWith(".qbutt-staging-"))
            continue;
        const path = join(prefix, entry.name);
        if (entry.isDirectory())
            Object.assign(result, await snapshot(root, path));
        else
            result[path] = sha256(await readFile(join(root, path)));
    }
    return result;
}

const lab = await createLab("staging-faults");
const fixture = lab.manifest.torrents.find(torrent => torrent.name === "v1")!;
const files = fixture.files.filter(file => !file.pad);
const points = ["preparing", "downloading", "ready_to_commit", "committing", "committed", "journal_retired"];
for (let index = 0; index < files.length; ++index) {
    for (const step of ["backed_up", "installed"])
        for (const boundary of ["before", "renamed", "after"])
            points.push(`committing:${index}:${step}:${boundary}`);
    for (const step of ["uninstalled", "restored"])
        for (const boundary of ["before", "renamed", "after"])
            points.push(`rolling_back:${index}:${step}:${boundary}`);
}
points.push("rolling_back", "rolled_back");
const requested = process.env.QBUTT_STAGING_CASE;
const scenarios: { point: string; forward: boolean; name: string; format?: string; empty?: boolean }[]
    = points.map(point => ({ point, forward: false, name: point }));
for (const point of points.filter(point => point === "downloading" || point === "ready_to_commit" || point.startsWith("committ")))
    scenarios.push({ point, forward: true, name: `${point}@commit` });
for (const forward of [false, true])
    scenarios.push({ point: "committing:2:installed:renamed", forward, empty: true, name: `empty-target@${forward ? "commit" : "rollback"}` });
for (const format of ["v2", "hybrid"])
    scenarios.push({ point: "committed", forward: true, format, name: `${format}@commit` });
const cases = requested ? scenarios.filter(item => item.name === requested) : scenarios;
assert(cases.length, "Requested staging fault checkpoint does not exist");
let failure: unknown;

async function status(expected: string[]) {
    const result = await waitFor("staging transition", () => lab.json<Status>("qbuttRepair/status"),
        value => expected.includes(value.state) || value.state === "failed", 60000);
    assert(result.state !== "failed", result.error);
    return result;
}

async function crashed() {
    await waitFor("deliberate native process termination", async () => lab.exitCode, code => code !== null && code !== undefined, 15000);
    assert(lab.exitCode === 197, `Fault build must exit 197; observed ${lab.exitCode}. Build with QBUTT_STAGING_FAULTS=ON.`);
    try { await lab.shutdown(); }
    catch (error) { assert(String(error).includes("Native app exited 197"), String(error)); }
}

async function recover(hash: string, fault?: string) {
    if (fault)
        process.env.QBUTT_STAGING_FAULT = fault;
    else
        delete process.env.QBUTT_STAGING_FAULT;
    await lab.start();
    await waitFor("recovery torrent suspended", () => lab.info(hash), torrent => torrent.state.startsWith("stopped") || torrent.state === "missingFiles");
    const response = await lab.request("qbuttRepair/analyze", { hash, mode: "recover" });
    const operation = await response.json() as Status;
    await status(["preparing", "downloading", "ready_to_commit", "committing", "committed", "rolling_back", "rolled_back"]);
    return operation;
}

try {
    for (let caseIndex = 0; caseIndex < cases.length; ++caseIndex) {
        const { point, forward, name, empty, format = "v1" } = cases[caseIndex]!;
        const rollback = point.startsWith("rolling_back") || point === "rolled_back";
        const firstFault = rollback ? `committing:${files.length - 1}:installed:after` : point;
        process.env.QBUTT_STAGING_FAULT = firstFault;
        await lab.start();
        const destination = join(lab.root, `target-${caseIndex}`);
        if (empty)
            await mkdir(destination);
        else
            await cp(join(lab.fixtures, "variants", "grow"), destination, { recursive: true });
        await writeFile(join(destination, "unknown-save.dat"), `unknown-${caseIndex}`);
        const hash = await lab.add(format, destination);
        await waitFor("stopped fault target", () => lab.info(hash), torrent => torrent.state.startsWith("stopped"));
        const before = await snapshot(destination);
        const target = lab.manifest.torrents.find(torrent => torrent.name === format)!;
        const mappings = Object.fromEntries(target.files.filter(file => !file.pad).map(file => [file.index, join(lab.fixtures, "seed", file.path)]));
        let operation = await (await lab.request("qbuttRepair/analyze", {
            hash, mode: "staged", mappings: JSON.stringify(mappings),
        })).json() as Status;
        await status(["planned"]);
        await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
        if (!["preparing", "downloading", "ready_to_commit"].includes(firstFault)) {
            await status(["ready_to_commit"]);
            await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
        }
        await crashed();
        if (point === "journal_retired") {
            delete process.env.QBUTT_STAGING_FAULT;
            await lab.start();
            await waitFor("retired operation restarted stopped", () => lab.info(hash), torrent => torrent.state.startsWith("stopped"));
            const info = await lab.json<{ save_path: string }[]>(`torrents/info?hashes=${hash}`);
            assert(join(info[0]!.save_path) === destination, "Retired journal left a stale native staging location");
            await verifyPayload(destination, lab.manifest.payload);
        }
        else {
            operation = await recover(hash, rollback ? point : undefined);
            await lab.request("torrents/start", { hashes: hash });
            const suspended = await lab.info(hash);
            assert(suspended.state.startsWith("stopped") || suspended.state === "missingFiles", "Pending recovery admitted a normal writer");
            if (forward) {
                if (point === "downloading") {
                    await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
                    await status(["ready_to_commit"]);
                }
                await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
                const finalized = await waitFor("recovered commit persisted and retired", () => lab.json<Status>("qbuttRepair/status"),
                    value => value.staging?.finalized === true || value.state === "failed");
                assert(finalized.state === "committed", finalized.error);
                await verifyPayload(destination, lab.manifest.payload);
                await lab.request("qbuttRepair/cancel", { id: operation.id });
                await lab.shutdown();
                await lab.start();
                await waitFor("recovered committed torrent stopped", () => lab.info(hash), torrent => torrent.state.startsWith("stopped"));
                await verifyPayload(destination, lab.manifest.payload);
            }
            else {
                await lab.request("qbuttRepair/rollback", { id: operation.id, consent: "true" });
                if (rollback) {
                    await crashed();
                    operation = await recover(hash);
                    await lab.request("qbuttRepair/rollback", { id: operation.id, consent: "true" });
                }
                const restored = await waitFor("rollback persisted and journal retired", () => lab.json<Status>("qbuttRepair/status"),
                    value => value.staging?.finalized === true || value.state === "failed");
                assert(restored.state === "rolled_back", restored.error);
                assert.deepEqual(await snapshot(destination), before, `Original target changed after ${point} recovery`);
                assert((await lab.info(hash)).progress < 1, "Rollback retained staged completion bits");
                await lab.request("qbuttRepair/cancel", { id: operation.id });
                await lab.shutdown();
                delete process.env.QBUTT_STAGING_FAULT;
                await lab.start();
                await waitFor("rolled back torrent restarted stopped", () => lab.info(hash),
                    torrent => torrent.state.startsWith("stopped") || torrent.state === "missingFiles");
                assert.deepEqual(await snapshot(destination), before, "Restart after rollback changed original payload");
            }
        }
        await verifyPayload(join(lab.fixtures, "seed"), lab.manifest.payload);
        assert((await readFile(join(destination, "unknown-save.dat"), "utf8")) === `unknown-${caseIndex}`);
        await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
        await lab.shutdown();
        await lab.checkpoint({ point: name, check: "crash-restart-idempotent-recovery-preserves-original-source-unknown" });
    }
}
catch (error) { failure = error; }
finally {
    delete process.env.QBUTT_STAGING_FAULT;
    try { await lab.shutdown(); }
    catch (error) { failure ??= error; }
}
await lab.finish(failure);
if (failure)
    throw failure;
