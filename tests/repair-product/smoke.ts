import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { cp, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";

import { generateFixtures } from "../fixtures/generate";

const driver = process.env.QBUTT_REPAIR_PREVIEW_DRIVER;
const python = process.env.QBUTT_LAB_PYTHON;
assert(driver, "Set QBUTT_REPAIR_PREVIEW_DRIVER to the built offscreen driver");
assert(python, "Set QBUTT_LAB_PYTHON to the pinned fixture Python");
const root = await mkdtemp(join(tmpdir(), "qbutt-repair-product-"));
const fixtures = await generateFixtures(python, join(root, "fixtures"));

interface SnapshotEntry { path: string; type: "directory" | "file" | "link"; size?: number; sha256?: string }
async function snapshot(directory: string): Promise<SnapshotEntry[]> {
    const result: SnapshotEntry[] = [];
    async function visit(path: string): Promise<void> {
        for (const entry of await readdir(path, { withFileTypes: true })) {
            const file = join(path, entry.name);
            const relativePath = relative(directory, file).replaceAll("\\", "/");
            if (entry.isSymbolicLink()) result.push({ path: relativePath, type: "link" });
            else if (entry.isDirectory()) {
                result.push({ path: relativePath, type: "directory" });
                await visit(file);
            }
            else if (entry.isFile()) {
                const bytes = await readFile(file);
                result.push({ path: relativePath, type: "file", size: bytes.length,
                    sha256: createHash("sha256").update(bytes).digest("hex") });
            }
        }
    }
    await visit(directory);
    return result.sort((a, b) => a.path.localeCompare(b.path));
}

async function run(name: string, destination: string, roots: string, mode: "normal" | "inplace" | "cancel" | "refusal",
    explicitSource = "-") {
    const output = join(root, `${name}.json`);
    const screenshot = join(root, `${name}.png`);
    const child = Bun.spawn([driver!, join(fixtures, "v1.torrent"), destination, roots, mode, screenshot, output, explicitSource], {
        cwd: root, stdout: "pipe", stderr: "pipe", timeout: 40000, windowsHide: true,
        env: { ...process.env, QT_PLUGIN_PATH: process.env.QT_PLUGIN_PATH ?? dirname(driver!) },
    });
    const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
    assert.equal(code, 0, `${name} driver failed: ${stdout}${stderr}`);
    return JSON.parse(await readFile(output, "utf8"));
}

const normal = join(fixtures, "variants", "grow");
const explicitRoot = join(fixtures, "variants", "renamed");
const explicitSource = join(explicitRoot, "bundle", "skip.bin");
const corruptCandidate = join(normal, "bundle", "nested", "beta.bin");
const corruptBytes = await readFile(corruptCandidate);
corruptBytes[0] ^= 0xff;
await writeFile(corruptCandidate, corruptBytes);
const beforeNormal = await snapshot(normal);
const beforeExplicit = await snapshot(explicitRoot);
const normalEvidence = await run("normal", normal, normal, "normal", explicitSource);
assert.equal(normalEvidence.rows, 5);
assert(normalEvidence.heartbeats > 0);
assert.equal(normalEvidence.applyEnabled, true);
assert.equal(normalEvidence.explicitMapping, true);
assert.equal(normalEvidence.screenshotSaved, true);
assert.equal(normalEvidence.oversized, 1);
assert(normalEvidence.changed >= 2);
assert.notEqual(normalEvidence.candidateText, normalEvidence.verifiedText);
assert(!/^0\D/.test(normalEvidence.networkText), "missing payload was reported as zero network bytes");
assert.match(normalEvidence.temporaryText, /required.*available/i);
assert(!normalEvidence.timeout);
assert.deepEqual(await snapshot(normal), beforeNormal, "read-only preview changed target/source data");
assert.deepEqual(await snapshot(explicitRoot), beforeExplicit, "explicit mapping preview changed source data");
assert((await stat(join(root, "normal.png"))).size > 0, "offscreen preview screenshot is empty");

const inPlaceEvidence = await run("inplace", normal, explicitRoot, "inplace");
assert.equal(inPlaceEvidence.sourceRootsEnabled, false);
assert(inPlaceEvidence.firstSource.startsWith(normal.replaceAll("\\", "/")),
    "in-place preview reported bytes from a disabled external source root");
assert.deepEqual(await snapshot(normal), beforeNormal, "in-place preview changed target data");
assert.deepEqual(await snapshot(explicitRoot), beforeExplicit, "in-place preview read or changed an external source");

const cancelRoot = join(root, "cancel-source");
await cp(join(fixtures, "seed"), cancelRoot, { recursive: true });
for (let index = 0; index < 4000; ++index) {
    const path = join(cancelRoot, "noise", String(Math.floor(index / 100)), `${index}.bin`);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, Buffer.from([index & 0xff]));
}
const beforeCancel = await snapshot(cancelRoot);
const cancelEvidence = await run("cancel", cancelRoot, cancelRoot, "cancel");
assert.equal(cancelEvidence.closedAfterCancel, true);
assert(cancelEvidence.heartbeats > 0, "event loop did not remain responsive while cancelling preview");
assert.match(cancelEvidence.status, /Cancelling the read-only preview/i, "large-tree preview completed before cancellation");
assert(!cancelEvidence.timeout);
assert.deepEqual(await snapshot(cancelRoot), beforeCancel, "cancelled preview changed source data");

const hardlink = join(fixtures, "variants", "hardlink");
const hardlinkTarget = join(root, "hardlink-target");
await mkdir(hardlinkTarget);
const beforeHardlink = await snapshot(hardlink);
const beforeHardlinkTarget = await snapshot(hardlinkTarget);
const hardlinkEvidence = await run("hardlink", hardlinkTarget, hardlink, "refusal");
assert.match(hardlinkEvidence.status, /hardlink|reparse|refus/i);
assert.equal(hardlinkEvidence.applyEnabled, false);
assert.deepEqual(await snapshot(hardlink), beforeHardlink, "refused hardlink preview changed data");
assert.deepEqual(await snapshot(hardlinkTarget), beforeHardlinkTarget, "refused hardlink preview created target data");

const reparse = join(fixtures, "variants", "reparse");
const reparseTarget = join(fixtures, "variants", "reparse-target");
const beforeReparse = await snapshot(reparse);
const beforeReparseTarget = await snapshot(reparseTarget);
const reparseEvidence = await run("reparse", reparse, reparse, "refusal");
assert.match(reparseEvidence.status, /hardlink|reparse|refus/i);
assert.equal(reparseEvidence.applyEnabled, false);
assert.deepEqual(await snapshot(reparse), beforeReparse, "refused reparse preview changed its target tree");
assert.deepEqual(await snapshot(reparseTarget), beforeReparseTarget, "refused reparse preview changed linked data");

const missingRootEvidence = await run("missing-root", normal, join(root, "absent-source"), "refusal");
assert.match(missingRootEvidence.status, /source directory|existing absolute/i);
assert.equal(missingRootEvidence.applyEnabled, false);
assert(!(await snapshot(join(root, "profile"))).some(entry => entry.type !== "directory"),
    "preview created a profile, resume or journal file before consent");

const evidence = { outcome: "PASS", checks: 6, normal: normalEvidence, inPlace: inPlaceEvidence, cancel: cancelEvidence,
    hardlink: hardlinkEvidence.status, reparse: reparseEvidence.status, missingRoot: missingRootEvidence.status,
    screenshot: join(root, "normal.png") };
for (const entry of await readdir(root, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const target = resolve(root, entry.name);
    assert(!entry.isSymbolicLink() && dirname(target) === resolve(root), "Cleanup escaped the owned preview fixture");
    await rm(target, { recursive: true, force: true });
}
await writeFile(join(root, "evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
console.log(JSON.stringify({ evidence: join(root, "evidence.json") }));
