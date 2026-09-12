import { cp, link, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, requireCondition, startSeed, verifyPayload, waitFor } from "../lab";

interface RepairStatus {
    id: string;
    state: "idle" | "analyzing" | "analyzed" | "applying" | "recheck_started" | "failed" | "expired";
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
                    requireCondition(lab.manifest.filesystemNegatives.reparse?.status === "ready", "Reparse fixture is unsupported on this filesystem");
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
                const operation = await response.json() as RepairStatus;
                const analysis = await waitFor("read-only repair analysis", () => lab.json<RepairStatus>("qbuttRepair/status"),
                    status => status.state === "analyzed" || status.state === "failed");
                requireCondition(JSON.stringify(await snapshot(destination)) === JSON.stringify(before),
                    `${name}/${variant}: read-only analysis changed candidate data`);
                if (guardedTarget)
                    requireCondition(JSON.stringify(await snapshot(guardedTarget)) === JSON.stringify(guardedBefore), "Reparse target was modified");
                if (variant === "hardlink")
                    requireCondition(sha256(await readFile(join(lab.root, `alias-${name}.bin`))) === lab.manifest.payload[0]!.sha256,
                        "Hardlink alias was modified");
                if (variant === "hardlink" || variant === "reparse") {
                    requireCondition(analysis.state === "failed", `${variant}: unsafe candidate was accepted`);
                    await lab.checkpoint({ name, variant, check: "unsafe-path-rejected", readOnly: true });
                }
                else {
                    requireCondition(analysis.state === "analyzed" && analysis.analysis, `Repair analysis failed: ${analysis.error}`);
                    const summary = analysis.analysis;
                    let previewWriteBlocked = false;
                    try { await writeFile(join(destination, "bundle", variant === "renamed" ? "renamed.bin" : "alpha.bin"), Buffer.from([1])); }
                    catch (error) { previewWriteBlocked = ["EPERM", "EACCES", "EBUSY"].includes((error as NodeJS.ErrnoException).code ?? ""); }
                    requireCondition(previewWriteBlocked, "Preview did not hold candidate against external writes");
                    requireCondition(JSON.stringify(await snapshot(destination)) === JSON.stringify(before), "Blocked write changed candidate data");
                    if (variant === "grow" || variant === "renamed" || variant === "unknown")
                        requireCondition(summary.verified_bytes === summary.expected_bytes, `${variant}: valid content was not recognized`);
                    else
                        requireCondition(summary.verified_bytes < summary.expected_bytes, `${variant}: damaged content was reported fully verified`);
                    // Consent and retained operation identity are both server-side admission checks.
                    let consentRejected = false;
                    try { await lab.request("qbuttRepair/apply", { id: operation.id }); }
                    catch (error) { consentRejected = String(error).includes("HTTP 400"); }
                    requireCondition(consentRejected, "Apply without explicit consent was accepted");
                    let staleRejected = false;
                    try { await lab.request("qbuttRepair/apply", { id: "stale-operation", consent: "true" }); }
                    catch (error) { staleRejected = String(error).includes("HTTP 409"); }
                    requireCondition(staleRejected, "Stale repair operation was accepted");
                    await lab.request("qbuttRepair/apply", { id: operation.id, consent: "true" });
                    const applied = await waitFor("managed repair apply", () => lab.json<RepairStatus>("qbuttRepair/status"),
                        status => status.state === "recheck_started" || status.state === "failed");
                    requireCondition(applied.state === "recheck_started", `Repair apply failed: ${applied.error}`);
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
                        requireCondition((await snapshot(destination))[path] === before[path], "Unknown user file changed");
                    }
                    await lab.checkpoint({ name, variant, check: "analyze-consent-repair-recheck", readOnly: true,
                        reusedVerifiedBytes: summary.verified_bytes, verifiedBytes, exactSizes: true });
                }
                await lab.request("qbuttRepair/cancel", { id: operation.id });
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
