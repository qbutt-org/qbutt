import assert from "node:assert/strict";
import { cp, link, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";

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
let failure: unknown;
try {
    await lab.start();
    for (const name of ["v1", "v2", "hybrid"]) {
        const seed = await startSeed(lab.python, lab.fixtures, name, lab.root);
        try {
            const variants = name === "v1"
                ? ["corrupt", "grow", "shrink", "renamed", "unknown", "inserted", "source-mutated", "hardlink", "reparse"]
                : ["corrupt", "grow", "shrink"];
            for (const variant of variants) {
                const destination = join(lab.root, "candidates", name, variant);
                await mkdir(join(lab.root, "candidates", name), { recursive: true });
                await cp(join(lab.fixtures, "variants", variant), destination, { recursive: true });
                if (variant === "hardlink")
                    await link(join(destination, "bundle", "alpha.bin"), join(lab.root, `alias-${name}.bin`));
                const guardedTarget = variant === "reparse" ? join(lab.fixtures, "variants", "reparse-target") : undefined;
                if (guardedTarget)
                    assert(lab.manifest.filesystemNegatives.reparse?.status === "ready", "Reparse fixture is unsupported on this filesystem");
                const guardedBefore = guardedTarget ? await snapshot(guardedTarget) : undefined;
                const hash = await lab.add(name, destination);
                if (variant === "renamed") {
                    await lab.request("torrents/renameFile", {
                        hash, oldPath: "bundle/alpha.bin", newPath: "bundle/renamed.bin",
                    });
                }
                await waitFor("candidate stopped", () => lab.info(hash), info => info.state.startsWith("stopped"));
                const before = await snapshot(destination);
                const response = await lab.request("qbuttRepair/analyze", { hash });
                let operation = await response.json() as RepairStatus;
                let analysis = await waitFor("read-only repair analysis", () => lab.json<RepairStatus>("qbuttRepair/status"),
                    status => status.state === "analyzed" || status.state === "failed");
                assert(JSON.stringify(await snapshot(destination)) === JSON.stringify(before),
                    `${name}/${variant}: read-only analysis changed candidate data`);
                if (guardedTarget)
                    assert(JSON.stringify(await snapshot(guardedTarget)) === JSON.stringify(guardedBefore), "Reparse target was modified");
                if (variant === "hardlink")
                    assert(sha256(await readFile(join(lab.root, `alias-${name}.bin`))) === lab.manifest.payload[0]!.sha256,
                        "Hardlink alias was modified");
                if (variant === "hardlink" || variant === "reparse") {
                    assert(analysis.state === "failed", `${variant}: unsafe candidate was accepted`);
                    await lab.checkpoint({ name, variant, check: "unsafe-path-rejected", readOnly: true });
                    await lab.request("qbuttRepair/cancel", { id: operation.id });
                }
                else {
                    assert(analysis.state === "analyzed" && analysis.analysis, `Repair analysis failed: ${analysis.error}`);
                    let previewWriteBlocked = false;
                    try { await writeFile(join(destination, "bundle", variant === "renamed" ? "renamed.bin" : "alpha.bin"), Buffer.from([1])); }
                    catch (error) { previewWriteBlocked = ["EPERM", "EACCES", "EBUSY"].includes((error as NodeJS.ErrnoException).code ?? ""); }
                    assert(previewWriteBlocked, "Preview did not hold candidate against external writes");
                    assert(JSON.stringify(await snapshot(destination)) === JSON.stringify(before), "Blocked write changed candidate data");
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
                        assert(JSON.stringify(await snapshot(destination)) === JSON.stringify(before), "Re-analysis changed candidate data");
                    }
                    const summary = analysis.analysis!;
                    if (variant === "grow" || variant === "renamed" || variant === "unknown")
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
                    assert(applied.state === "checked", `Repair apply/recheck failed: ${applied.error}`);
                    await lab.request("qbuttRepair/cancel", { id: operation.id });
                    await waitFor("repair engine recheck", () => lab.info(hash), info => info.state.startsWith("stopped"));
                    await lab.request("torrents/start", { hashes: hash });
                    await lab.request("torrents/addPeers", { hashes: hash, peers: `${seed.host}:${seed.port}` });
                    await waitFor("repaired download", () => lab.info(hash), info => info.progress === 1);
                    await lab.request("torrents/stop", { hashes: hash });
                    await waitFor("repaired stop", () => lab.info(hash), info => info.state === "stoppedUP");
                    const expected = lab.manifest.payload.map(file => ({
                        ...file, path: variant === "renamed" && file.path === "bundle/alpha.bin" ? "bundle/renamed.bin" : file.path,
                    }));
                    const verifiedBytes = await verifyPayload(destination, expected);
                    if (variant === "unknown") {
                        const path = join("bundle", "user-notes.txt");
                        assert((await snapshot(destination))[path] === before[path], "Unknown user file changed");
                    }
                    await lab.checkpoint({ name, variant, check: "analyze-consent-repair-recheck", readOnly: true,
                        reusedVerifiedBytes: summary.verified_bytes, verifiedBytes, exactSizes: true });
                }
                await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
                await waitFor("candidate removal", () => lab.json<unknown[]>("torrents/info"), torrents => torrents.length === 0);
            }
        }
        finally {
            await seed.stop();
        }
    }
    await lab.shutdown();
}
catch (error) {
    failure = error;
    try { await lab.shutdown(); }
    catch (shutdownError) { console.error(String(shutdownError)); }
}
await lab.finish(failure);
if (failure)
    throw failure;
