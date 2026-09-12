import assert from "node:assert/strict";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { createLab, verifyPayload, waitFor, type TorrentFile } from "../lab";
import { assertRecoverySuspended, snapshot } from "./staging-checks";

interface Status {
    id: string;
    state: string;
    error?: string;
    staging?: { payload_path: string; finalized: boolean };
}
type Journal = Record<string, unknown> & { files: Record<string, unknown>[] };

const lab = await createLab("staging-journal");
let failure: unknown;
try {
    const destination = join(lab.root, "target");
    await cp(join(lab.fixtures, "variants", "grow"), destination, { recursive: true });
    await mkdir(join(destination, ".qbutt-staging-user-data"));
    await writeFile(join(destination, ".qbutt-staging-user-data", "save.bin"), "preserve unknown prefixed directory");
    const original = await snapshot(destination);
    await lab.start();
    const hash = await lab.add("v1", destination);
    await waitFor("journal target stopped", () => lab.info(hash), info => info.state.startsWith("stopped"));
    const files = await lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`);
    const skipped = files.find(file => file.name.endsWith("/skip.bin"));
    assert(skipped, "Expected selectable fixture file");
    await lab.request("torrents/filePrio", { hash, id: String(skipped.index), priority: "0" });
    let operation = await (await lab.request("qbuttRepair/analyze", {
        hash, mode: "staged", sources: JSON.stringify([join(lab.fixtures, "seed")]),
    })).json() as Status;
    const plan = await waitFor("journal plan", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "planned" || status.state === "failed");
    assert.equal(plan.state, "planned", plan.error);
    await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
    const prepared = await waitFor("journal staged payload verified", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "ready_to_commit" || status.state === "failed");
    assert.equal(prepared.state, "ready_to_commit", prepared.error);
    const transactionRoot = dirname(prepared.staging!.payload_path);
    await verifyPayload(prepared.staging!.payload_path, lab.manifest.payload);
    await lab.request("qbuttRepair/cancel", { id: operation.id });
    await lab.shutdown();
    const before = await snapshot(destination);
    const journalPath = join(lab.root, "profile", process.env.QBUTT_LAB_APP_NAME ?? "qbutt", "data", "staging", `${hash}.json`);
    const valid = await readFile(journalPath, "utf8");
    const persisted = JSON.parse(valid) as Journal;
    assert(persisted.files.some(file => file.selected === false && file.verified === undefined),
        "Valid recovery must exercise an unselected file without a verified identity");
    assert.equal(persisted.files[0]!.selected, true, "Corrupt variants require a selected first file");
    const changes: [string, (journal: Journal) => void][] = [
        ["missing-selection", journal => { delete journal.files[0]!.selected; }],
        ["wrong-selection-type", journal => { journal.files[0]!.selected = "true"; }],
        ["empty-selection", journal => {
            for (const file of journal.files) {
                file.selected = false;
                delete file.verified;
            }
        }],
        ["missing-original-identity", journal => { delete journal.files[0]!.original; }],
        ["partial-original-identity", journal => { delete (journal.files[0]!.original as Record<string, unknown>).mtime; }],
        ["noncanonical-original-identity", journal => { (journal.files[0]!.original as Record<string, unknown>).id = "01"; }],
        ["duplicate-index", journal => { journal.files[1]!.index = journal.files[0]!.index; }],
        ["fractional-index", journal => { journal.files[0]!.index = 0.5; }],
        ["wrong-file-size", journal => { journal.files[0]!.size = "-1"; }],
        ["missing-verified-identity", journal => { delete journal.files[0]!.verified; }],
        ["empty-verified-identity", journal => { journal.files[0]!.verified = {}; }],
        ["partial-verified-identity", journal => { delete (journal.files[0]!.verified as Record<string, unknown>).mtime; }],
        ["wrong-verified-size", journal => { (journal.files[0]!.verified as Record<string, unknown>).size = "0"; }],
        ["unknown-step", journal => { journal.files[0]!.step = "not-a-step"; }],
        ["unselected-file-step", journal => {
            journal.files[0]!.selected = false;
            journal.files[0]!.step = "installed";
            delete journal.files[0]!.verified;
        }],
        ["traversal-path", journal => { journal.files[0]!.path = "../outside.bin"; }],
        ["unknown-state", journal => { journal.state = "not-a-state"; }],
        ["unknown-version", journal => { journal.version = 999; }],
        ["invalid-operation-id", journal => { journal.operation = "../outside"; }],
    ];
    const cases: [string, string][] = changes.map(([name, mutate]) => {
        const journal = JSON.parse(valid) as Journal;
        mutate(journal);
        return [name, JSON.stringify(journal)];
    });
    cases.push(["truncated-json", valid.trimEnd().slice(0, -1)], ["wrong-root-type", "[]"]);
    for (const [name, invalid] of cases) {
        await writeFile(journalPath, invalid);
        await lab.start();
        await waitFor("malformed recovery torrent initialized", () => lab.info(hash),
            info => info.state.startsWith("stopped") || info.state === "missingFiles");
        operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as Status;
        const result = await waitFor("malformed journal rejected", () => lab.json<Status>("qbuttRepair/status"),
            status => status.state !== "analyzing");
        assert.equal(result.state, "failed", `${name}: malformed journal admitted as ${result.state}`);
        await assertRecoverySuspended(lab, hash);
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await lab.shutdown();
        assert.deepEqual(await snapshot(destination), before, `${name}: malformed recovery changed target/stage/backup/unknown data`);
        assert.equal(await readFile(journalPath, "utf8"), invalid, `${name}: malformed journal was rewritten or retired`);
        await verifyPayload(join(lab.fixtures, "seed"), lab.manifest.payload);
        await lab.checkpoint({ check: "malformed-journal-rejected-without-writes", name, afterShutdown: true });
        await writeFile(journalPath, valid);
    }
    await lab.start();
    await waitFor("valid recovery torrent initialized", () => lab.info(hash),
        info => info.state.startsWith("stopped") || info.state === "missingFiles");
    operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as Status;
    await waitFor("valid journal admitted after rejected variants", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "ready_to_commit");
    await lab.request("qbuttRepair/rollback", { id: operation.id, consent: "true" });
    const rolledBack = await waitFor("valid rollback finalized", () => lab.json<Status>("qbuttRepair/status"),
        status => status.staging?.finalized === true || status.state === "failed");
    assert.equal(rolledBack.state, "rolled_back", rolledBack.error);
    await assert.rejects(lab.request("qbuttRepair/rollback", { id: operation.id, consent: "true" }), /HTTP 409/);
    await lab.request("qbuttRepair/cancel", { id: operation.id });
    await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
    await lab.shutdown();
    assert.deepEqual(await snapshot(destination, transactionRoot), original, "Valid rollback or removal changed original data");
    await verifyPayload(join(lab.fixtures, "seed"), lab.manifest.payload);
    await lab.checkpoint({ check: "valid-journal-recovery-and-repeated-rollback-rejection", afterShutdown: true });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); }
    catch (error) { failure ??= error; }
}
await lab.finish(failure);
if (failure)
    throw failure;
