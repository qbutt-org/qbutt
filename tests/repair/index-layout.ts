import assert from "node:assert/strict";
import { mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join, resolve, sep } from "node:path";
import type { TorrentFixture } from "../fixtures/generate";
import { createLab, verifyPayload, waitFor } from "../lab";
import { snapshot } from "./staging-checks";

interface StagedStatus {
    id: string;
    state: string;
    error?: string;
    analysis?: { verified_bytes: number; valid_pieces: number; unverified_pieces: number };
    staging?: { payload_path: string; finalized: boolean; files: { path: string; source: string }[] };
}

const lab = await createLab("repair-index-layout");
const source = join(lab.root, "source");
const destination = join(lab.root, "destination");
const hashInput = join(lab.root, "hash-input");
let failure: unknown;
try {
    // These complete files came from the old 16 KiB layout. Every source name
    // and directory changes, while target order and piece boundaries also move.
    const oldTorrent = lab.manifest.torrents.find(item => item.name === "v1")!;
    const order = [2, 3, 0, 4, 1];
    const targetPayload = order.map((index, position) => ({ ...lab.manifest.payload[index]!,
        path: `updated/${position}/target-${index}.bin` }));
    const expectedSources = new Map<string, string>();
    for (const [position, index] of order.entries()) {
        const bytes = await readFile(join(lab.fixtures, "seed", lab.manifest.payload[index]!.path));
        const renamedSource = join(source, `old-${index}`, `renamed-${index}.dat`);
        const targetPath = targetPayload[position]!.path;
        const hashPath = join(hashInput, targetPath);
        await mkdir(dirname(renamedSource), { recursive: true });
        await mkdir(dirname(hashPath), { recursive: true });
        await writeFile(renamedSource, bytes);
        await writeFile(hashPath, bytes);
        expectedSources.set(targetPath, resolve(renamedSource));
    }
    const generator = Bun.spawn([lab.python, "-c", `
import json, libtorrent as lt, pathlib, sys
assert lt.__version__ == "2.0.14.0"
payload = json.loads(sys.argv[1])
files = lt.file_storage()
for item in payload:
    files.add_file(item["path"], item["size"])
creator = lt.create_torrent(files, 65536, lt.create_torrent.v1_only)
creator.set_creator("qbutt changed-layout repair fixture")
creator.set_priv(True)
lt.set_piece_hashes(creator, sys.argv[2])
encoded = lt.bencode(creator.generate())
pathlib.Path(sys.argv[3]).write_bytes(encoded)
info = lt.torrent_info(encoded)
storage = info.files()
print(json.dumps({"name": "reordered", "file": "reordered.torrent",
    "pieceLength": info.piece_length(), "pieceCount": info.num_pieces(),
    "infoHashV1": str(info.info_hashes().v1), "infoHashV2": None,
    "files": [{"index": i, "path": storage.file_path(i).replace("\\\\", "/"),
        "size": storage.file_size(i), "offset": storage.file_offset(i), "pad": False}
        for i in range(storage.num_files())]}))
`, JSON.stringify(targetPayload), hashInput, join(lab.fixtures, "reordered.torrent")],
    { stdout: "pipe", stderr: "pipe", timeout: 30000 });
    const [code, stdout, stderr] = await Promise.all([generator.exited,
        new Response(generator.stdout).text(), new Response(generator.stderr).text()]);
    assert.equal(code, 0, stderr);
    const target = JSON.parse(stdout) as TorrentFixture;
    assert.equal(oldTorrent.pieceLength, 16384);
    assert.equal(target.pieceLength, 65536);
    assert.notEqual(target.infoHashV1, oldTorrent.infoHashV1);
    assert.notDeepEqual(target.files.map(file => file.size), oldTorrent.files.map(file => file.size));
    assert.deepEqual(target.files.map(file => file.path), targetPayload.map(file => file.path));
    lab.manifest.torrents.push(target);
    // No target-shaped copy or seed remains available to the application.
    await rm(hashInput, { recursive: true });
    await mkdir(destination);
    await writeFile(join(destination, "user-notes.txt"), "Preserve unknown user data.\n");
    const sourceBefore = await snapshot(source);
    const expectedBytes = targetPayload.reduce((sum, file) => sum + file.size, 0);

    await lab.start();
    const hash = await lab.add(target.name, destination);
    await waitFor("reordered target stopped", () => lab.info(hash), status => status.state.startsWith("stopped"));
    // Torrent initialization may create empty files before repair owns it.
    const targetBefore = await snapshot(destination);
    const operation = await (await lab.request("qbuttRepair/analyze", {
        hash, mode: "staged", sources: JSON.stringify([source]),
    })).json() as StagedStatus;
    const planned = await waitFor("renamed source index and target layout hashes", () => lab.json<StagedStatus>("qbuttRepair/status"),
        status => status.state === "planned" || status.state === "failed");
    assert.equal(planned.state, "planned", planned.error);
    assert.equal(planned.analysis!.verified_bytes, expectedBytes);
    assert.equal(planned.analysis!.valid_pieces, target.pieceCount);
    assert.equal(planned.analysis!.unverified_pieces, 0);
    assert.equal(planned.staging!.files.length, targetPayload.length);
    for (const file of planned.staging!.files)
        assert.equal(resolve(file.source), expectedSources.get(file.path), "Index mapped a different physical source");
    assert.deepEqual(await snapshot(source), sourceBefore, "Planning changed renamed sources");
    assert.deepEqual(await snapshot(destination), targetBefore, "Planning changed destination");
    await assert.rejects(stat(planned.staging!.payload_path), { code: "ENOENT" }, "Planning created staging before consent");
    await lab.checkpoint({ check: "renamed-index-verifies-changed-target-layout", sourcePieceLength: oldTorrent.pieceLength,
        targetPieceLength: target.pieceLength, targetPieceCount: target.pieceCount, verifiedBytes: expectedBytes,
        mappedFiles: planned.staging!.files.length, sourceInfoHash: oldTorrent.infoHashV1, targetInfoHash: target.infoHashV1 });

    await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
    const ready = await waitFor("target reconstructed without peer or webseed", () => lab.json<StagedStatus>("qbuttRepair/status"),
        status => status.state === "ready_to_commit" || status.state === "failed");
    assert.equal(ready.state, "ready_to_commit", ready.error);
    assert.equal(await verifyPayload(ready.staging!.payload_path, targetPayload), expectedBytes);
    assert.deepEqual(await snapshot(destination, dirname(ready.staging!.payload_path)), targetBefore,
        "Preparation wrote the destination");
    await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
    const committed = await waitFor("changed layout commit", () => lab.json<StagedStatus>("qbuttRepair/status"),
        status => status.staging?.finalized === true || status.state === "failed");
    assert.equal(committed.state, "committed", committed.error);
    await lab.request("qbuttRepair/cancel", { id: operation.id });
    await lab.shutdown();
    await lab.start();
    await waitFor("completed changed layout restored", () => lab.info(hash),
        status => status.state === "stoppedUP" && status.progress === 1);
    const properties = await lab.json<{ total_downloaded: number; total_downloaded_session: number; pieces_have: number }>(
        `torrents/properties?hash=${hash}`);
    assert.equal(properties.total_downloaded, 0, "Reconstruction unexpectedly downloaded network payload");
    assert.equal(properties.total_downloaded_session, 0);
    assert.equal(properties.pieces_have, target.pieceCount);
    assert.equal(await verifyPayload(destination, targetPayload), expectedBytes);
    assert.deepEqual(await snapshot(source), sourceBefore, "Preparation/commit/restart changed renamed sources");
    assert.equal(await readFile(join(destination, "user-notes.txt"), "utf8"), "Preserve unknown user data.\n");
    await lab.checkpoint({ check: "changed-layout-stage-commit-restart-without-network", verifiedBytes: expectedBytes,
        downloadedPayloadBytes: properties.total_downloaded, nativePieces: properties.pieces_have, exactSizes: true });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); }
    catch (error) { failure ??= error; }
    await lab.finish(failure);
    for (const name of ["fixtures", "profile", "source", "destination", "hash-input"]) {
        const target = resolve(lab.root, name);
        assert(target.startsWith(`${resolve(lab.root)}${sep}`), "Cleanup escaped the owned fixture");
        await rm(target, { recursive: true, force: true });
    }
}
if (failure)
    throw failure;
