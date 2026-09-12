import assert from "node:assert/strict";
import { cp, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createLab, verifyPayload, waitFor } from "./lab";

interface Rule { id: string; enabled: boolean; match: Record<string, unknown>; actions: string[] }
interface Configuration { enabled: boolean; allow_delete_data: boolean; rules: Rule[] }
interface Claim { key: string; rule: string; status: string; reason: { destination: string }; actions: string[] }
interface Preview { hash: string; preview_required: boolean; ready: boolean; rules: Claim[] }
interface Repair { id: string; state: string; error?: string; staging?: { finalized: boolean; can_commit: boolean } }

const lab = await createLab("policies");
let failure: unknown;
const rule = (id: string, actions: string[], match: Record<string, unknown> = {}): Rule => ({ id, enabled: true, match, actions });
async function configure(rules: Rule[], allowDelete = false, enabled = true) {
    const configuration: Configuration = { enabled, allow_delete_data: allowDelete, rules };
    await lab.request("qbuttPolicies/configure", { configuration: JSON.stringify(configuration) });
    assert.deepEqual(await lab.json<Configuration>("qbuttPolicies/configuration"), configuration);
}
async function checkNative(hash: string) {
    await lab.request("torrents/recheck", { hashes: hash });
    await waitFor("native wanted files verified", () => lab.info(hash), info => info.progress === 1 && info.state === "stoppedUP");
    await waitFor("native completion barrier finished", () => lab.json<Preview[]>("qbuttPolicies/preview"), items => items.some(item => item.hash === hash && item.ready));
}
async function removed(hash: string) {
    await waitFor("policy removed native task", () => lab.json<{ hash: string }[]>(`torrents/info?hashes=${hash}`), torrents => !torrents.length);
}
async function claims(expected: number) {
    return waitFor("durable completion claims", () => lab.json<Claim[]>("qbuttPolicies/journal"), entries => entries.length === expected && entries.every(entry => entry.status === "dispatched"));
}

try {
    await lab.start();
    const destination = join(lab.root, "target");
    await cp(join(lab.fixtures, "seed"), destination, { recursive: true });
    await writeFile(join(destination, "unknown-save.dat"), "Unknown files survive all policy actions.\n");
    const hash = await lab.add("v1", destination);
    await checkNative(hash);
    await lab.request("torrents/createCategory", { category: "games" });
    await lab.request("torrents/setCategory", { hashes: hash, category: "games" });
    await lab.request("torrents/addTags", { hashes: hash, tags: "ready,local" });
    const before = await lab.json<Configuration>("qbuttPolicies/configuration");
    for (const invalid of [
        { enabled: true, allow_delete_data: false, rules: [rule("denied", ["delete_data"])] },
        { enabled: true, allow_delete_data: true, rules: [rule("conflict", ["remove_torrent", "delete_data"])] },
        { enabled: true, allow_delete_data: false, rules: [rule("unknown", ["shutdown"])] },
        { enabled: true, allow_delete_data: false, rules: [rule("bad-threshold", ["stop"], { min_ratio: -1 })] },
    ]) {
        await assert.rejects(lab.request("qbuttPolicies/configure", { configuration: JSON.stringify(invalid) }), /HTTP 400/);
        assert.deepEqual(await lab.json<Configuration>("qbuttPolicies/configuration"), before);
    }
    await configure([
        rule("wrong-category", ["remove_torrent"], { category: "other" }),
        rule("wrong-tag", ["remove_torrent"], { tags: ["missing"] }),
        rule("too-early", ["remove_torrent"], { min_seeding_seconds: 100000 }),
        rule("notice", ["notify"], { category: "games", tags: ["ready", "local"], min_ratio: 0, min_seeding_seconds: 0 }),
        rule("first-terminal", ["stop"]), rule("unreachable", ["remove_torrent"]),
    ]);
    const first = await claims(2);
    assert.deepEqual(first.map(entry => entry.rule), ["notice", "first-terminal"]);
    assert((await lab.info(hash)).state === "stoppedUP");
    await lab.shutdown();
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    await lab.start();
    await Bun.sleep(1800);
    assert.deepEqual(await lab.json<Claim[]>("qbuttPolicies/journal"), first, "Restart replayed already claimed rules");
    await lab.checkpoint({ check: "first-terminal-and-restart-idempotency", claimedRules: first.map(entry => entry.rule), verifiedBytes });

    // The imported marker is part of the real native resume record, not an
    // independent policy list. D3 exercises the actual migration writer.
    await configure([rule("import-removal", ["remove_torrent"])], false, false);
    await lab.shutdown();
    const resume = join(lab.root, "profile", "qbutt", "data", "BT_backup", `${hash}.fastresume`);
    const marker = Bun.spawn([lab.python, "-c", "import libtorrent as lt,pathlib,sys; p=pathlib.Path(sys.argv[1]); d=lt.bdecode(p.read_bytes()); d[b'qbutt-completion-policy-preview']=1; p.write_bytes(lt.bencode(d))", resume], { stdout: "pipe", stderr: "pipe" });
    assert(await marker.exited === 0, await new Response(marker.stderr).text());
    await lab.start();
    await configure([rule("import-removal", ["remove_torrent"])]);
    const preview = (await lab.json<Preview[]>("qbuttPolicies/preview")).find(item => item.hash === hash)!;
    assert(preview.preview_required && preview.rules[0]?.rule === "import-removal");
    await lab.shutdown();
    await lab.start();
    await Bun.sleep(1600);
    assert((await lab.info(hash)).progress === 1, "Import preview allowed automatic removal");
    await lab.request("qbuttPolicies/acknowledge", { hash, consent: "true" });
    await removed(hash);
    await claims(3);
    await lab.shutdown();
    assert(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes);
    assert((await readFile(join(destination, "unknown-save.dat"), "utf8")).startsWith("Unknown"));
    await lab.checkpoint({ check: "native-import-preview-restart-acknowledgement-remove-keeps-payload", verifiedBytes, unknownPreserved: true });

    await lab.start();
    await configure([], false, false);
    const secondHash = await lab.add("v2", destination);
    await checkNative(secondHash);
    const operation = await (await lab.request("qbuttRepair/analyze", { hash: secondHash })).json() as Repair;
    await waitFor("held repair", () => lab.json<Repair>("qbuttRepair/status"), value => value.state === "analyzed");
    await configure([rule("held-removal", ["remove_torrent"])]);
    await Bun.sleep(1800);
    assert((await lab.info(secondHash)).progress === 1);
    assert((await lab.json<Claim[]>("qbuttPolicies/journal")).length === 3);
    await lab.request("qbuttRepair/cancel", { id: operation.id });
    await removed(secondHash);
    await claims(4);
    await lab.shutdown();
    assert(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes);
    await lab.checkpoint({ check: "repair-ownership-defers-terminal-policy", verifiedBytes });

    await lab.start();
    await configure([], false, false);
    const stagedDestination = join(lab.root, "staged-target");
    await cp(join(lab.fixtures, "variants", "corrupt"), stagedDestination, { recursive: true });
    await writeFile(join(stagedDestination, "unknown-save.dat"), "staged unknown");
    const stagedHash = await lab.add("hybrid", stagedDestination);
    await waitFor("staged target initialized and stopped", () => lab.info(stagedHash), value => value.state.startsWith("stopped"));
    const staged = await (await lab.request("qbuttRepair/analyze", { hash: stagedHash, mode: "staged", sources: JSON.stringify([join(lab.fixtures, "seed")]) })).json() as Repair;
    await waitFor("staged plan", () => lab.json<Repair>("qbuttRepair/status"), value => value.state === "planned");
    await configure([rule("committed-removal", ["remove_torrent"])]);
    await lab.request("qbuttRepair/prepare", { id: staged.id, consent: "true" });
    await waitFor("verified staging ready for commit", () => lab.json<Repair>("qbuttRepair/status"), value => value.staging?.can_commit === true, 60000);
    await Bun.sleep(1600);
    assert((await lab.json<Claim[]>("qbuttPolicies/journal")).length === 4);
    assert((await lab.json<{ hash: string }[]>(`torrents/info?hashes=${stagedHash}`)).length === 1);
    await lab.request("qbuttRepair/commit", { id: staged.id, consent: "true" });
    await removed(stagedHash);
    await claims(5);
    await lab.shutdown();
    assert(await verifyPayload(stagedDestination, lab.manifest.payload) === verifiedBytes);
    assert((await readFile(join(stagedDestination, "unknown-save.dat"), "utf8")) === "staged unknown");
    assert(await verifyPayload(join(lab.fixtures, "seed"), lab.manifest.payload) === verifiedBytes);
    await lab.checkpoint({ check: "staging-verified-is-held-until-commit-and-resume-receipt", verifiedBytes, sourcePreserved: true, unknownPreserved: true });

    await lab.start();
    await configure([], false, false);
    const finalHash = await lab.add("v1-64k", destination);
    await checkNative(finalHash);
    const moved = join(lab.root, "moved");
    await lab.request("torrents/setLocation", { hashes: finalHash, location: moved });
    await configure([rule("moved-removal", ["remove_torrent"])]);
    await removed(finalHash);
    const movedClaims = await claims(6);
    assert(movedClaims[5]!.reason.destination.replaceAll("\\", "/").toLowerCase() === moved.replaceAll("\\", "/").toLowerCase());
    await lab.shutdown();
    assert(await verifyPayload(moved, lab.manifest.payload) === verifiedBytes);
    assert((await readFile(join(destination, "unknown-save.dat"), "utf8")).startsWith("Unknown"));
    await lab.checkpoint({ check: "move-completes-before-policy-receipt-and-removal", verifiedBytes });

    await lab.start();
    await configure([], false, false);
    const deleteHash = await lab.add("v1", moved);
    await checkNative(deleteHash);
    await writeFile(join(moved, "keep-me.txt"), "outside torrent metadata");
    await configure([rule("explicit-delete", ["delete_data"])], true);
    await removed(deleteHash);
    await claims(7);
    await waitFor("explicit payload deletion", async () => Bun.file(join(moved, lab.manifest.payload[0]!.path)).exists(), exists => !exists);
    await lab.shutdown();
    for (const file of lab.manifest.payload)
        assert(!await Bun.file(join(moved, file.path)).exists(), `Explicit delete left ${file.path}`);
    assert((await readFile(join(moved, "keep-me.txt"), "utf8")) === "outside torrent metadata");
    await lab.checkpoint({ check: "separately-enabled-delete-data-preserves-unknown-files", unknownPreserved: true });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); }
    catch (error) { failure ??= error; }
    await lab.finish(failure);
}
if (failure)
    throw failure;


const thresholdLab = await createLab("policies-thresholds");
let thresholdFailure: unknown;
try {
    await thresholdLab.start();
    const destination = join(thresholdLab.root, "seed");
    await cp(join(thresholdLab.fixtures, "seed"), destination, { recursive: true });
    const hash = await thresholdLab.add("v1", destination);
    await thresholdLab.request("torrents/recheck", { hashes: hash });
    await waitFor("threshold seed verified", () => thresholdLab.info(hash), info => info.progress === 1 && info.state === "stoppedUP");
    await waitFor("threshold seed completion persisted", () => thresholdLab.json<Preview[]>("qbuttPolicies/preview"), items => items.some(item => item.hash === hash && item.ready));
    await thresholdLab.request("torrents/start", { hashes: hash });
    const preferences = await thresholdLab.json<{ listen_port: number }>("app/preferences");
    const download = join(thresholdLab.root, "download");
    const peer = Bun.spawn([thresholdLab.python, join(import.meta.dir, "fixtures", "download.py"),
        join(thresholdLab.fixtures, "v1.torrent"), download, String(preferences.listen_port)],
        { stdout: "pipe", stderr: "pipe", timeout: 70000 });
    const [code, output, error] = await Promise.all([peer.exited, new Response(peer.stdout).text(), new Response(peer.stderr).text()]);
    assert(code === 0, error);
    const peerEvidence = JSON.parse(output) as { finished: boolean; verified_bytes: number };
    const verifiedBytes = await verifyPayload(download, thresholdLab.manifest.payload);
    assert(peerEvidence.finished && peerEvidence.verified_bytes === verifiedBytes);
    const uploaded = await waitFor("actual native ratio increases after upload", () => thresholdLab.json<{ ratio: number; seeding_time: number }[]>(`torrents/info?hashes=${hash}`), items => items[0]!.ratio > 0);
    const threshold = uploaded[0]!.seeding_time + 4;
    await thresholdLab.request("qbuttPolicies/configure", { configuration: JSON.stringify({ enabled: true, allow_delete_data: false,
        rules: [rule("after-upload-and-seeding", ["stop"], { min_ratio: 0.001, min_seeding_seconds: threshold })] }) });
    assert((await thresholdLab.json<Claim[]>("qbuttPolicies/journal")).length === 0, "Time condition fired before native threshold");
    const journal = await waitFor("native ratio and seeding time rule", () => thresholdLab.json<(Claim & { reason: { ratio: number; seeding_seconds: number } })[]>("qbuttPolicies/journal"), items => items.length === 1);
    assert(journal[0]!.reason.ratio >= 0.001 && journal[0]!.reason.seeding_seconds >= threshold);
    await waitFor("threshold action stopped native seed", () => thresholdLab.info(hash), item => item.state === "stoppedUP");
    await thresholdLab.shutdown();
    assert(await verifyPayload(destination, thresholdLab.manifest.payload) === verifiedBytes);
    await thresholdLab.checkpoint({ check: "native-upload-ratio-and-elapsed-seeding-time", verifiedBytes, reason: journal[0]!.reason });
}
catch (error) { thresholdFailure = error; }
finally {
    try { await thresholdLab.shutdown(); }
    catch (error) { thresholdFailure ??= error; }
    await thresholdLab.finish(thresholdFailure);
}
if (thresholdFailure)
    throw thresholdFailure;
