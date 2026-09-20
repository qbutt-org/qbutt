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
    analysis?: { verified_bytes: number; valid_pieces: number; unverified_pieces: number; whole_file_v2_verification: boolean };
    staging?: { payload_path: string; finalized: boolean; files: { path: string; source: string }[] };
}

const format = process.argv.includes("--v2") ? "v2" : "v1";
const lab = await createLab(`repair-index-layout-${format}`);
const source = join(lab.root, "source");
const destination = join(lab.root, "destination");
const hashInput = join(lab.root, "hash-input");
let failure: unknown;
try {
    // Complete files are reused across changed names, order and piece lengths.
    // Unlike v1, v2 pieces stay file-aligned and each unchanged file root survives.
    const oldTorrent = lab.manifest.torrents.find(item => item.name === format)!;
    const order = [2, 3, 0, 4, 1];
    const targetPayload = order.map((index, position) => ({ ...lab.manifest.payload[index]!,
        originalPath: lab.manifest.payload[index]!.path,
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
flags = lt.create_torrent.v2_only if sys.argv[4] == "v2" else lt.create_torrent.v1_only
creator = lt.create_torrent(files, 65536, flags)
creator.set_creator("qbutt changed-layout repair fixture")
creator.set_priv(True)
lt.set_piece_hashes(creator, sys.argv[2])
encoded = lt.bencode(creator.generate())
pathlib.Path(sys.argv[3]).write_bytes(encoded)
info = lt.torrent_info(encoded)
storage = info.files()
file_roots = 0
piece_layers = []
if info.info_hashes().has_v2():
    original_encoded = pathlib.Path(sys.argv[5]).read_bytes()
    original_info = lt.torrent_info(original_encoded)
    original = original_info.files()
    original_roots = {original.file_path(i).replace("\\\\", "/"): original.root(i).to_bytes()
        for i in range(original.num_files()) if original.file_size(i) > 0
        and not original.file_flags(i) & lt.file_storage.flag_pad_file}
    original_paths = {item["path"]: item["originalPath"] for item in payload}
    old_layers = lt.bdecode(original_encoded)[b"piece layers"]
    new_layers = lt.bdecode(encoded)[b"piece layers"]
    for i in range(storage.num_files()):
        if storage.file_size(i) == 0 or storage.file_flags(i) & lt.file_storage.flag_pad_file:
            continue
        path = storage.file_path(i).replace("\\\\", "/")
        file_root = storage.root(i).to_bytes()
        assert file_root == original_roots[original_paths[path]], "Unchanged content has a different v2 file root"
        file_roots += 1
        if storage.file_size(i) > info.piece_length():
            assert old_layers[file_root] != new_layers[file_root], "Target reused the old v2 piece layer"
            piece_layers.append({"path": path, "sourcePieces": len(old_layers[file_root]) // 32,
                "targetPieces": len(new_layers[file_root]) // 32})
print(json.dumps({"name": "reordered", "file": "reordered.torrent",
    "pieceLength": info.piece_length(), "pieceCount": info.num_pieces(),
    "infoHashV1": str(info.info_hashes().v1) if info.info_hashes().has_v1() else None,
    "infoHashV2": str(info.info_hashes().v2) if info.info_hashes().has_v2() else None,
    "v2FileRoots": file_roots, "v2PieceLayers": piece_layers,
    "files": [{"index": i, "path": storage.file_path(i).replace("\\\\", "/"),
        "size": storage.file_size(i), "offset": storage.file_offset(i),
        "pad": bool(storage.file_flags(i) & lt.file_storage.flag_pad_file)}
        for i in range(storage.num_files())]}))
`, JSON.stringify(targetPayload), hashInput, join(lab.fixtures, "reordered.torrent"), format,
    join(lab.fixtures, oldTorrent.file)],
    { stdout: "pipe", stderr: "pipe", timeout: 30000, windowsHide: true });
    const [code, stdout, stderr] = await Promise.all([generator.exited,
        new Response(generator.stdout).text(), new Response(generator.stderr).text()]);
    assert.equal(code, 0, stderr);
    const target = JSON.parse(stdout) as TorrentFixture & {
        v2FileRoots: number; v2PieceLayers: { path: string; sourcePieces: number; targetPieces: number }[];
    };
    assert.equal(oldTorrent.pieceLength, 16384);
    assert.equal(target.pieceLength, 65536);
    assert.notEqual(target.infoHashV2 ?? target.infoHashV1, oldTorrent.infoHashV2 ?? oldTorrent.infoHashV1);
    const payloadFiles = target.files.filter(file => !file.pad);
    assert.notDeepEqual(payloadFiles.map(file => file.size), oldTorrent.files.filter(file => !file.pad).map(file => file.size));
    assert.deepEqual(payloadFiles.map(file => file.path), targetPayload.map(file => file.path));
    if (format === "v2") {
        assert.equal(target.infoHashV1, null, "The target must exercise pure v2 verification");
        assert.equal(target.infoHashV2?.length, 64);
        assert.equal(target.v2FileRoots, targetPayload.filter(file => file.size > 0).length);
        assert.equal(target.v2PieceLayers.length, 1);
        assert.deepEqual(target.v2PieceLayers[0], { path: targetPayload[0]!.path, sourcePieces: 129, targetPieces: 33 });
        assert(payloadFiles.every(file => file.size === 0 || file.offset % target.pieceLength === 0),
            "v2 target files are not piece-aligned");
    }
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
    await lab.checkpoint({ check: "renamed-index-verifies-changed-target-layout", format, sourcePieceLength: oldTorrent.pieceLength,
        targetPieceLength: target.pieceLength, targetPieceCount: target.pieceCount, verifiedBytes: expectedBytes,
        mappedFiles: planned.staging!.files.length, sourceInfoHash: oldTorrent.infoHashV2 ?? oldTorrent.infoHashV1,
        targetInfoHash: target.infoHashV2 ?? target.infoHashV1, v2FileRoots: target.v2FileRoots,
        v2PieceLayers: target.v2PieceLayers, v2WholeFileVerification: planned.analysis!.whole_file_v2_verification });

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
    for (const name of ["fixtures", "profile", "source", "destination", "hash-input"]) {
        const target = resolve(lab.root, name);
        assert(target.startsWith(`${resolve(lab.root)}${sep}`), "Cleanup escaped the owned fixture");
        try { await rm(target, { recursive: true, force: true }); }
        catch (error) { failure ??= error; }
    }
    await lab.finish(failure);
}
if (failure)
    throw failure;
