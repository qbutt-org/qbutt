import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { join, resolve } from "node:path";
import { sha256 } from "../fixtures/generate";
import type { createLab } from "../lab";

export async function snapshot(root: string, excludedDirectory?: string, prefix = ""):
    Promise<Record<string, { size: number; sha256: string }>> {
    const result: Record<string, { size: number; sha256: string }> = {};
    for (const entry of await readdir(join(root, prefix), { withFileTypes: true })) {
        const path = join(prefix, entry.name);
        if (excludedDirectory && resolve(root, path) === resolve(excludedDirectory))
            continue;
        assert(!entry.isSymbolicLink(), "Snapshot must not follow a reparse point");
        if (entry.isDirectory())
            Object.assign(result, await snapshot(root, excludedDirectory, path));
        else {
            const bytes = await readFile(join(root, path));
            result[path] = { size: bytes.length, sha256: sha256(bytes) };
        }
    }
    return result;
}

export async function assertRecoverySuspended(lab: Awaited<ReturnType<typeof createLab>>, hash: string) {
    await lab.request("torrents/start", { hashes: hash });
    // The upstream info cache refreshes every 1.5 s. Observe past two refresh
    // periods, including the final sample, instead of accepting the old state.
    const deadline = performance.now() + 3500;
    for (;;) {
        const info = await lab.info(hash);
        assert(info.state.startsWith("stopped") || info.state === "missingFiles",
            `Pending recovery admitted a normal writer: ${info.state}`);
        if (performance.now() >= deadline)
            break;
        await Bun.sleep(250);
    }
}
