import assert from "node:assert/strict";
import { cp, link, readFile, readdir, stat, unlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, startSeed, verifyPayload, waitFor, type TorrentFile } from "../lab";

interface StagedStatus {
    id: string;
    state: string;
    error?: string;
    staging?: { payload_path: string; required_bytes: string; payload_bytes: string; finalized: boolean;
        files: { path: string; source: string; verified: { mtime: string } }[] };
}

async function snapshot(root: string, prefix = ""): Promise<Record<string, string>> {
    const result: Record<string, string> = {};
    for (const item of await readdir(join(root, prefix), { withFileTypes: true })) {
        if (item.name.startsWith(".qbutt-staging-"))
            continue;
        const path = join(prefix, item.name);
        if (item.isDirectory())
            Object.assign(result, await snapshot(root, path));
        else
            result[path] = sha256(await readFile(join(root, path)));
    }
    return result;
}

const lab = await createLab("staging");
let failure: unknown;
try {
    await lab.start();
    for (const scenario of ["v1", "v2", "hybrid", "v1-selected", "v2-selected", "hybrid-selected"]) {
        const format = scenario.split("-")[0]!;
        const selective = scenario.endsWith("-selected");
        const seed = await startSeed(lab.python, lab.fixtures, format, lab.root);
        try {
            const destination = join(lab.root, `destination-${scenario}`);
            const source = join(lab.root, `source-${scenario}`);
            await cp(join(lab.fixtures, "variants", selective ? "renamed" : "corrupt"), source, { recursive: true });
            await cp(join(lab.fixtures, "variants", "grow"), destination, { recursive: true });
            await writeFile(join(destination, "unknown-save.dat"), "preserve this saved game");
            if (selective)
                await writeFile(join(destination, "bundle", "skip.bin"), "Unselected original must remain unchanged");
            const original = await snapshot(destination);
            const sourceBefore = await snapshot(source);
            const hash = await lab.add(format, destination);
            await waitFor("stopped staged target", () => lab.info(hash), status => status.state.startsWith("stopped"));
            const fileList = await lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`);
            const skipped = fileList.find(file => file.name.endsWith("/skip.bin"))!;
            if (selective)
                await lab.request("torrents/filePrio", { hash, id: String(skipped.index), priority: "0" });
            const mappings = Object.fromEntries(lab.manifest.torrents.find(torrent => torrent.name === format)!.files
                .filter(file => !file.pad).map(file => [file.index, join(source, file.path)]));
            const operation = await (await lab.request("qbuttRepair/analyze", {
                hash, mode: "staged", sources: JSON.stringify([source]), mappings: JSON.stringify(selective ? {} : mappings),
            })).json() as StagedStatus;
            const planned = await waitFor("staging plan", () => lab.json<StagedStatus>("qbuttRepair/status"),
                status => status.state === "planned" || status.state === "failed");
            assert(planned.state === "planned", planned.error);
            assert.deepEqual(await snapshot(destination), original, "Planning wrote target data");
            assert.deepEqual(await snapshot(source), sourceBefore, "Planning wrote source data");
            await assert.rejects(stat(planned.staging!.payload_path), "Read-only planning created staging");
            const heldSource = join(source, "bundle", selective ? "renamed.bin" : "alpha.bin");
            await assert.rejects(writeFile(heldSource, await readFile(heldSource)), "Planning did not exclude source writers");
            assert(Number(planned.staging!.payload_bytes) === lab.manifest.payload.reduce((sum, file) => sum + file.size, 0));
            assert(Number(planned.staging!.required_bytes) >= Number(planned.staging!.payload_bytes));
            if (selective)
                assert(planned.staging!.files.find(file => file.path.endsWith("/alpha.bin"))!.source.endsWith("/renamed.bin"),
                    "Metadata index did not map the uniquely sized renamed source");
            await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
            await waitFor("staging native downloader", () => lab.info(hash), status => !status.state.startsWith("checking"));
            await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.1:${seed.port}` });
            const ready = await waitFor("verified staged target", () => lab.json<StagedStatus>("qbuttRepair/status"),
                status => status.state === "ready_to_commit" || status.state === "failed", 90000);
            assert(ready.state === "ready_to_commit", ready.error);
            await verifyPayload(ready.staging!.payload_path, lab.manifest.payload);
            assert.deepEqual(await snapshot(destination), original, "Staged download wrote the original target");
            assert.deepEqual(await snapshot(source), sourceBefore, "Staged download wrote its source");
            await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
            const committed = await waitFor("journalled commit", () => lab.json<StagedStatus>("qbuttRepair/status"),
                status => status.staging?.finalized === true || status.state === "failed");
            assert(committed.state === "committed", committed.error);
            const committedPayload = lab.manifest.payload.filter(file => !selective || !file.path.endsWith("/skip.bin"));
            const verifiedBytes = await verifyPayload(destination, committedPayload);
            if (selective)
                assert((await readFile(join(destination, "bundle", "skip.bin"), "utf8")) === "Unselected original must remain unchanged");
            assert((await readFile(join(destination, "unknown-save.dat"), "utf8")) === "preserve this saved game");
            assert.deepEqual(await snapshot(source), sourceBefore, "Commit modified an external source");
            await lab.request("qbuttRepair/cancel", { id: operation.id });
            await lab.shutdown();
            await lab.start();
            await waitFor("committed torrent restarted stopped", () => lab.info(hash), status => status.state.startsWith("stopped"));
            await verifyPayload(destination, committedPayload);
            if (selective) {
                assert((await lab.info(hash)).progress < 1, "Partial commit retained native pieces for uncommitted files");
                await lab.request("torrents/filePrio", { hash, id: String(skipped.index), priority: "1" });
                await lab.request("torrents/start", { hashes: hash });
                await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.1:${seed.port}` });
                await waitFor("normal download after selective commit", () => lab.info(hash), status => status.progress === 1);
                await lab.request("torrents/stop", { hashes: hash });
                await waitFor("normal completion stopped", () => lab.info(hash), status => status.state.startsWith("stopped"));
                await verifyPayload(destination, lab.manifest.payload);
            }
            await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
            await lab.checkpoint({ scenario, check: "independent-staging-native-download-verified-commit-restart", verifiedBytes });
        }
        finally { await seed.stop(); }
    }
    for (const kind of ["hardlink", "reparse"] as const) {
        assert(lab.manifest.filesystemNegatives[kind]?.status === "ready", `Required Windows ${kind} fixture is unavailable`);
        const destination = join(lab.root, `rejected-${kind}`);
        await cp(join(lab.fixtures, "seed"), destination, { recursive: true });
        const before = await snapshot(destination);
        const hash = await lab.add("v1", destination);
        await waitFor("stopped alias target", () => lab.info(hash), status => status.state.startsWith("stopped"));
        await assert.rejects(lab.request("qbuttRepair/analyze", { hash, mode: "stage-typo" }), /HTTP 400/);
        const mappings = Object.fromEntries(lab.manifest.torrents.find(torrent => torrent.name === "v1")!.files
            .filter(file => !file.pad).map(file => [file.index, join(lab.fixtures, "variants", kind, file.path)]));
        const operation = await (await lab.request("qbuttRepair/analyze", {
            hash, mode: "staged", mappings: JSON.stringify(mappings),
        })).json() as StagedStatus;
        const rejected = await waitFor("unsafe source rejected", () => lab.json<StagedStatus>("qbuttRepair/status"),
            status => status.state === "failed");
        assert(/hardlink|reparse|link/i.test(rejected.error!), rejected.error);
        assert.deepEqual(await snapshot(destination), before);
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
        await lab.checkpoint({ check: "unsafe-source-rejected-without-writes", kind });
    }

    const destination = join(lab.root, "mutated-stage");
    await cp(join(lab.fixtures, "variants", "grow"), destination, { recursive: true });
    const original = await snapshot(destination);
    const hash = await lab.add("v1", destination);
    await waitFor("stopped mutation target", () => lab.info(hash), status => status.state.startsWith("stopped"));
    let operation = await (await lab.request("qbuttRepair/analyze", {
        hash, mode: "staged", sources: JSON.stringify([join(lab.fixtures, "seed")]),
    })).json() as StagedStatus;
    await waitFor("mutation plan", () => lab.json<StagedStatus>("qbuttRepair/status"), status => status.state === "planned");
    await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
    const prepared = await waitFor("mutation stage ready", () => lab.json<StagedStatus>("qbuttRepair/status"),
        status => status.state === "ready_to_commit");
    const stagedFile = join(prepared.staging!.payload_path, "bundle", "alpha.bin");
    const alias = join(lab.root, "stage-alias.bin");
    const valid = await readFile(stagedFile);
    await assert.rejects(link(stagedFile, alias), "Active staging directory ownership allowed a new hardlink");
    await lab.request("qbuttRepair/cancel", { id: operation.id });
    await link(stagedFile, alias);
    operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as StagedStatus;
    await waitFor("recover hardlinked staging", () => lab.json<StagedStatus>("qbuttRepair/status"), status => status.state === "ready_to_commit");
    await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
    await waitFor("hardlinked stage rejected", () => lab.json<StagedStatus>("qbuttRepair/status"), status => status.state === "failed");
    assert.deepEqual(await readFile(alias), valid, "Commit modified a hardlink alias");
    assert.deepEqual(await snapshot(destination), original);
    await unlink(alias);
    await lab.request("qbuttRepair/cancel", { id: operation.id });

    const mtime = prepared.staging!.files.find(file => file.path.endsWith("/alpha.bin"))!.verified.mtime;
    assert(/^\d+$/.test(mtime));
    async function restoreTimestamp() {
        const command = `[IO.File]::SetLastWriteTimeUtc('${stagedFile.replaceAll("'", "''")}', [DateTime]::FromFileTimeUtc([Int64]'${mtime}'))`;
        const process = Bun.spawn(["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(command, "utf16le").toString("base64")],
            { stdout: "pipe", stderr: "pipe", timeout: 15000 });
        assert(await process.exited === 0, await new Response(process.stderr).text());
    }
    const corrupt = Buffer.from(valid);
    corrupt[0] = corrupt[0]! ^ 0xff;
    await writeFile(stagedFile, corrupt);
    await restoreTimestamp();
    operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as StagedStatus;
    await waitFor("recover altered staging", () => lab.json<StagedStatus>("qbuttRepair/status"), status => status.state === "ready_to_commit");
    await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
    const rejected = await waitFor("same-identity corruption rejected by hashes", () => lab.json<StagedStatus>("qbuttRepair/status"), status => status.state === "failed");
    assert(/hashes.*changed/.test(rejected.error!), rejected.error);
    assert.deepEqual(await snapshot(destination), original);
    await verifyPayload(join(lab.fixtures, "seed"), lab.manifest.payload);
    await lab.request("qbuttRepair/cancel", { id: operation.id });
    await writeFile(stagedFile, valid);
    await restoreTimestamp();
    operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as StagedStatus;
    await waitFor("recover restored staging", () => lab.json<StagedStatus>("qbuttRepair/status"), status => status.state === "ready_to_commit");
    await lab.request("qbuttRepair/rollback", { id: operation.id, consent: "true" });
    await waitFor("mutation scenario safely rolled back", () => lab.json<StagedStatus>("qbuttRepair/status"), status => status.staging?.finalized === true);
    assert.deepEqual(await snapshot(destination), original);
    await lab.checkpoint({ check: "commit-rejects-new-hardlinks-and-content-mutation-with-restored-mtime" });
}
catch (error) { failure = error; }
finally { await lab.shutdown(); }
await lab.finish(failure);
if (failure)
    throw failure;
