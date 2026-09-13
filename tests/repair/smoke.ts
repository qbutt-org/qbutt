import assert from "node:assert/strict";
import { cp, link, mkdir, readFile, readdir, rm, stat, unlink, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, startSeed, verifyPayload, waitFor, type TorrentFile } from "../lab";

interface RepairStatus {
    id: string;
    state: "idle" | "analyzing" | "analyzed" | "applying" | "recheck_started" | "checked" | "failed" | "expired";
    error?: string;
    analysis?: {
        expected_bytes: number;
        verified_bytes: number;
        valid_pieces: number;
        unverified_pieces: number;
        whole_file_v2_verification: boolean;
        files: { path: string; expected_size: number; actual_size: number; verified_bytes: number; problems: string[] }[];
    };
}

// Do not follow a junction while snapshotting the candidate; its separately
// owned target is checked explicitly in the reparse scenario.
async function snapshot(root: string, prefix = ""): Promise<Record<string, string>> {
    const result: Record<string, string> = {};
    for (const item of await readdir(join(root, prefix), { withFileTypes: true })) {
        const path = join(prefix, item.name);
        if (item.isSymbolicLink())
            result[path] = "reparse";
        else if (item.isDirectory())
            Object.assign(result, await snapshot(root, path));
        else
            result[path] = sha256(await readFile(join(root, path)));
    }
    return result;
}

const lab = await createLab("repair");
const finalSnapshots: { path: string; files: Record<string, string> }[] = [];
const protectedSnapshots: { path: string; files: Record<string, string> }[] = [];
let failure: unknown;
try {
    await lab.start();
    for (const bypass of [false, true]) {
        await lab.request("app/setPreferences", { json: JSON.stringify({ bypass_local_auth: bypass }) });
        const headers = { Origin: lab.origin, Referer: `${lab.origin}/` };
        if (bypass) {
            const ordinary = await fetch(`${lab.origin}/api/v2/app/version`, { headers, signal: AbortSignal.timeout(5000) });
            assert(ordinary.ok, "Localhost authentication exemption was not active");
        }
        for (const action of ["status", "analyze", "apply", "cancel"]) {
            const response = await fetch(`${lab.origin}/api/v2/qbuttRepair/${action}`, {
                method: action === "status" ? "GET" : "POST", headers,
                body: action === "status" ? undefined : new URLSearchParams({ hash: "0".repeat(40), id: "unauthenticated", consent: "true" }),
                signal: AbortSignal.timeout(5000),
            });
            assert(response.status === 403, `Repair ${action} admitted unauthenticated access with localhost bypass=${bypass}`);
        }
    }
    await lab.request("app/setPreferences", { json: JSON.stringify({ bypass_local_auth: false }) });
    await lab.checkpoint({ check: "repair-api-requires-authentication-with-localhost-exemption" });
    for (const name of ["v1", "v2", "hybrid"]) {
        const seed = await startSeed(lab.python, lab.fixtures, name, lab.root);
        try {
            const variants = name === "v1"
                ? ["corrupt", "grow", "shrink", "renamed", "unknown", "inserted", "source-mutated", "missing-nonzero", "missing-empty", "missing-empty-directory", "hardlink", "reparse"]
                : ["corrupt", "grow", "shrink", "missing-nonzero", "missing-empty", "missing-empty-directory"];
            for (const variant of variants) {
                const destination = join(lab.root, "candidates", name, variant);
                const missingPath = variant === "missing-empty-directory" ? "bundle/new-empty/deep/empty.bin"
                    : variant === "missing-empty" ? "bundle/empty.bin"
                    : variant === "missing-nonzero" ? "bundle/nested/beta.bin" : undefined;
                const missingSize = variant.startsWith("missing-empty") ? 0
                    : missingPath ? lab.manifest.payload.find(item => item.path === missingPath)!.size : undefined;
                await mkdir(join(lab.root, "candidates", name), { recursive: true });
                await cp(missingPath ? join(lab.fixtures, "seed") : join(lab.fixtures, "variants", variant),
                    destination, { recursive: true });
                if (variant === "hardlink")
                    await link(join(destination, "bundle", "alpha.bin"), join(lab.root, `alias-${name}.bin`));
                const guardedTarget = variant === "reparse" ? join(lab.fixtures, "variants", "reparse-target") : undefined;
                if (guardedTarget)
                    assert(lab.manifest.filesystemNegatives.reparse?.status === "ready", "Reparse fixture is unsupported on this filesystem");
                const guardedBefore = guardedTarget ? await snapshot(guardedTarget) : undefined;
                if (guardedTarget)
                    protectedSnapshots.push({ path: guardedTarget, files: guardedBefore! });
                const hash = await lab.add(name, destination);
                if (variant === "renamed") {
                    await lab.request("torrents/renameFile", {
                        hash, oldPath: "bundle/alpha.bin", newPath: "bundle/renamed.bin",
                    });
                    await waitFor("renamed torrent mapping", () => lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`),
                        files => files.some(file => file.name === "bundle/renamed.bin"));
                }
                else if (variant === "missing-empty-directory") {
                    await lab.request("torrents/renameFile", {
                        hash, oldPath: "bundle/empty.bin", newPath: missingPath!,
                    });
                    await waitFor("missing empty directory mapping", () => lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`),
                        files => files.some(file => file.name === missingPath));
                }
                await waitFor("candidate stopped", () => lab.info(hash), info => info.state.startsWith("stopped"));
                if (missingPath)
                    variant === "missing-empty-directory"
                        ? await rm(join(destination, "bundle", "new-empty"), { recursive: true, force: true })
                        : await unlink(join(destination, missingPath));
                const before = await snapshot(destination);
                const response = await lab.request("qbuttRepair/analyze", { hash });
                let operation = await response.json() as RepairStatus;
                let analysis = await waitFor("read-only repair analysis", () => lab.json<RepairStatus>("qbuttRepair/status"),
                    status => status.state === "analyzed" || status.state === "failed");
                assert.deepEqual(await snapshot(destination), before,
                    `${name}/${variant}: read-only analysis changed candidate data`);
                if (variant === "missing-empty-directory")
                    await assert.rejects(stat(join(destination, "bundle", "new-empty")), { code: "ENOENT" },
                        "Read-only analysis created a missing target directory");
                if (guardedTarget)
                    assert.deepEqual(await snapshot(guardedTarget), guardedBefore, "Reparse target was modified");
                if (variant === "hardlink")
                    assert(sha256(await readFile(join(lab.root, `alias-${name}.bin`))) === lab.manifest.payload[0]!.sha256,
                        "Hardlink alias was modified");
                if (variant === "hardlink" || variant === "reparse") {
                    assert(analysis.state === "failed", `${variant}: unsafe candidate was accepted`);
                    await lab.checkpoint({ name, variant, check: "unsafe-path-rejected", analysisReadOnly: true });
                    await lab.request("qbuttRepair/cancel", { id: operation.id });
                }
                else {
                    assert(analysis.state === "analyzed" && analysis.analysis, `Repair analysis failed: ${analysis.error}`);
                    let previewWriteBlocked = false;
                    try { await writeFile(join(destination, "bundle", variant === "renamed" ? "renamed.bin" : "alpha.bin"), Buffer.from([1])); }
                    catch (error) { previewWriteBlocked = ["EPERM", "EACCES", "EBUSY"].includes((error as NodeJS.ErrnoException).code ?? ""); }
                    assert(previewWriteBlocked, "Preview did not hold candidate against external writes");
                    assert.deepEqual(await snapshot(destination), before, "Blocked write changed candidate data");
                    if (name === "v1" && variant === "corrupt") {
                        await lab.request("qbuttRepair/cancel", { id: operation.id });
                        const alpha = join(destination, "bundle", "alpha.bin");
                        await writeFile(alpha, await readFile(alpha));
                        const previousId = operation.id;
                        operation = await (await lab.request("qbuttRepair/analyze", { hash })).json() as RepairStatus;
                        analysis = await waitFor("analysis after cancellation", () => lab.json<RepairStatus>("qbuttRepair/status"),
                            status => status.state === "analyzed" || status.state === "failed");
                        assert(analysis.state === "analyzed" && analysis.analysis && operation.id !== previousId,
                            "Cancellation did not release locks and permit a fresh analysis");
                        assert.deepEqual(await snapshot(destination), before, "Re-analysis changed candidate data");
                    }
                    if (name === "v1" && variant === "missing-empty") {
                        const racedPath = join(destination, missingPath!);
                        let raced = false;
                        try {
                            await writeFile(racedPath, Buffer.alloc(0));
                            raced = true;
                        }
                        catch (error) {
                            assert(["EPERM", "EACCES", "EBUSY"].includes((error as NodeJS.ErrnoException).code ?? ""),
                                `Unexpected missing-target race error: ${String(error)}`);
                        }
                        if (raced) {
                            const racedSnapshot = await snapshot(destination);
                            await lab.request("qbuttRepair/apply", { id: operation.id, consent: "true" });
                            const rejected = await waitFor("raced missing target rejection", () => lab.json<RepairStatus>("qbuttRepair/status"),
                                status => status.state === "failed");
                            assert(rejected.error, "Missing-target race failed without a diagnostic");
                            assert.deepEqual(await snapshot(destination), racedSnapshot,
                                "Rejected missing-target race changed candidate data");
                            await lab.request("qbuttRepair/cancel", { id: operation.id });
                            await unlink(racedPath);
                            const previousId = operation.id;
                            operation = await (await lab.request("qbuttRepair/analyze", { hash })).json() as RepairStatus;
                            analysis = await waitFor("analysis after missing-target race", () => lab.json<RepairStatus>("qbuttRepair/status"),
                                status => status.state === "analyzed" || status.state === "failed");
                            assert(analysis.state === "analyzed" && analysis.analysis && operation.id !== previousId,
                                "Rejected missing-target race did not permit a fresh analysis");
                            assert.deepEqual(await snapshot(destination), before,
                                "Missing-target race cleanup changed candidate data");
                        }
                        await lab.checkpoint({ name, variant, check: "missing-target-race",
                            outcome: raced ? "identity-change-rejected" : "external-create-blocked" });
                    }
                    const summary = analysis.analysis!;
                    assert(summary.expected_bytes === lab.manifest.payload.reduce((sum, file) => sum + file.size, 0),
                        "Repair byte accounting included padding or omitted payload");
                    if (missingPath)
                        assert(summary.files.some(file => resolve(file.path) === join(destination, missingPath) && file.actual_size === -1
                            && file.expected_size === missingSize),
                            "Read-only analysis did not identify the absent target");
                    if (variant === "corrupt") {
                        // v1's damaged boundary piece owns 16 KiB of payload.
                        // Aligned v2/hybrid damage only alpha's 123-byte last
                        // piece, unless v2 metadata requires whole-file proof.
                        const invalidBytes = name === "v1" ? 16384 : summary.whole_file_v2_verification ? 16507 : 123;
                        assert(summary.verified_bytes === summary.expected_bytes - invalidBytes,
                            `${name}: incorrect verified-byte accounting for the corrupted boundary piece`);
                    }
                    if (variant === "grow" || variant === "renamed" || variant === "unknown" || variant.startsWith("missing-empty"))
                        assert(summary.verified_bytes === summary.expected_bytes, `${variant}: valid content was not recognized`);
                    else
                        assert(summary.verified_bytes < summary.expected_bytes, `${variant}: damaged content was reported fully verified`);
                    // Consent and retained operation identity are both server-side admission checks.
                    let consentRejected = false;
                    try { await lab.request("qbuttRepair/apply", { id: operation.id }); }
                    catch (error) { consentRejected = String(error).includes("HTTP 400"); }
                    assert(consentRejected, "Apply without explicit consent was accepted");
                    let staleRejected = false;
                    try { await lab.request("qbuttRepair/apply", { id: "stale-operation", consent: "true" }); }
                    catch (error) { staleRejected = String(error).includes("HTTP 409"); }
                    assert(staleRejected, "Stale repair operation was accepted");
                    await lab.request("qbuttRepair/apply", { id: operation.id, consent: "true" });
                    const applied = await waitFor("managed repair apply", () => lab.json<RepairStatus>("qbuttRepair/status"),
                        status => status.state === "checked" || status.state === "failed");
                    await lab.request("qbuttRepair/cancel", { id: operation.id });
                    assert(applied.state === "checked", `Repair apply/recheck failed: ${applied.error}`);
                    if (variant === "missing-nonzero")
                        assert.deepEqual(await snapshot(destination), before, "Managed recheck created or changed payload before download");
                    if (variant.startsWith("missing-empty"))
                        assert((await stat(join(destination, missingPath!))).size === 0,
                            "Managed apply did not create the exact empty target under its exclusive guard");
                    await waitFor("repair engine recheck", () => lab.info(hash), info => info.state.startsWith("stopped"));
                    await lab.request("torrents/start", { hashes: hash });
                    await lab.request("torrents/addPeers", { hashes: hash, peers: `${seed.host}:${seed.port}` });
                    await waitFor("repaired download", () => lab.info(hash), info => info.progress === 1);
                    await lab.request("torrents/stop", { hashes: hash });
                    await waitFor("repaired stop", () => lab.info(hash), info => info.state === "stoppedUP");
                    const expected = lab.manifest.payload.map(file => ({
                        ...file, path: variant === "renamed" && file.path === "bundle/alpha.bin" ? "bundle/renamed.bin"
                            : variant === "missing-empty-directory" && file.path === "bundle/empty.bin" ? missingPath! : file.path,
                    }));
                    const verifiedBytes = await verifyPayload(destination, expected);
                    if (variant === "unknown") {
                        const path = join("bundle", "user-notes.txt");
                        assert((await snapshot(destination))[path] === before[path], "Unknown user file changed");
                    }
                    await lab.checkpoint({ name, variant, check: "analyze-consent-repair-recheck", analysisReadOnly: true,
                        reusedVerifiedBytes: summary.verified_bytes, verifiedBytes, exactSizes: true });
                }
                finalSnapshots.push({ path: destination, files: await snapshot(destination) });
                await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
                await waitFor("candidate removal", () => lab.json<unknown[]>("torrents/info"), torrents => torrents.length === 0);
            }
        }
        finally {
            await seed.stop();
        }
    }
    await lab.shutdown();
    for (const candidate of finalSnapshots)
        assert.deepEqual(await snapshot(candidate.path), candidate.files,
            "Torrent removal or shutdown changed candidate files");
    for (const target of protectedSnapshots)
        assert.deepEqual(await snapshot(target.path), target.files,
            "Torrent removal or shutdown changed a rejected reparse target");
    await lab.checkpoint({ check: "payload-preserved-after-remove-and-shutdown",
        candidates: finalSnapshots.length, protectedTargets: protectedSnapshots.length });
}
catch (error) {
    failure = error;
    try { await lab.shutdown(); }
    catch (shutdownError) { console.error(String(shutdownError)); }
}
await lab.finish(failure);
if (failure)
    throw failure;
