import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { cp, mkdir, mkdtemp, open, readFile, readdir, rm, stat, statfs, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { snapshot } from "./staging-checks";
import {
    cleanupMarkedLabs, cleanupOwnedMarker, createOwnership, ownershipMarker, registerOwnedVolume,
} from "./storage-volume";
import { createLab, verifyPayload, waitFor } from "../lab";

interface Status {
    id: string;
    state: string;
    error?: string;
    staging?: {
        payload_path: string;
        required_bytes: string;
        available_bytes: string;
        finalized: boolean;
    };
}

await cleanupMarkedLabs();
const lab = await createLab("storage-faults");
assert(process.platform === "win32", "Storage fault acceptance requires Windows");
assert(resolve(lab.root).startsWith(resolve(process.env.TEMP!) + "\\qbutt-storage-faults-"),
    "Storage fault resources must stay in the owned temporary lab");

async function run(command: string, args: string[], timeout = 30000) {
    const child = Bun.spawn([command, ...args], { stdout: "pipe", stderr: "pipe" });
    let timer: ReturnType<typeof setTimeout>;
    const exited = Promise.race([child.exited, new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
            child.kill();
            reject(new Error(`${command} timed out after ${timeout} ms`));
        }, timeout);
    })]);
    let exitCode: number;
    let stdout: string;
    let stderr: string;
    try {
        [exitCode, stdout, stderr] = await Promise.all([
            exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
        ]);
    }
    finally { clearTimeout(timer!); }
    assert(exitCode === 0, `${command} exited ${exitCode}: ${stderr || stdout}`);
    return stdout;
}

async function forceDismount(path: string) {
    let diagnostic = "";
    for (let attempt = 0; attempt < 10; ++attempt) {
        const child = Bun.spawn(["mountvol.exe", path, "/P"], { stdout: "pipe", stderr: "pipe" });
        const [exitCode, stdout, stderr] = await Promise.all([
            child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
        ]);
        diagnostic = `exit ${exitCode}: ${stderr || stdout}`;
        if (!await mountedVolume(path))
            return;
        await Bun.sleep(100);
    }
    assert.fail(`Forced dismount left the directory mounted (${diagnostic})`);
}

async function mountedVolume(path: string) {
    const child = Bun.spawn(["mountvol.exe", path, "/L"], { stdout: "pipe", stderr: "pipe" });
    const [exitCode, stdout] = await Promise.all([child.exited, new Response(child.stdout).text()]);
    return exitCode === 0 ? stdout.trim() : undefined;
}

async function ensureEmptyMount(path: string) {
    const deadline = Date.now() + 5000;
    do {
        try { await mkdir(path); }
        catch (error) {
            if ((error as NodeJS.ErrnoException).code !== "EEXIST")
                await Bun.sleep(50);
        }
        try {
            if ((await readdir(path)).length === 0)
                return;
        }
        catch (error) {
            if ((error as NodeJS.ErrnoException).code !== "ENOENT")
                throw error;
        }
        await Bun.sleep(50);
    } while (Date.now() < deadline);
    throw new Error(`VHDX mount point did not become an empty directory: ${path}`);
}

let diskpartIndex = 0;
async function diskpart(commands: string[]) {
    const script = join(lab.root, `diskpart-${++diskpartIndex}.txt`);
    await writeFile(script, [...commands, "exit"].join("\r\n") + "\r\n", "ascii");
    try { return await run("diskpart.exe", ["/s", script], 60000); }
    finally { await rm(script, { force: true }); }
}

async function imageVolumeId(path: string) {
    const output = await run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
        `(Get-DiskImage -ImagePath '${path}' | Get-Disk | Get-Partition | Get-Volume).UniqueId`]);
    const id = output.trim();
    assert(id.startsWith("\\\\?\\Volume{"), `No volume identity for ${path}`);
    return id;
}

async function imageAttached(path: string) {
    const output = await run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
        `(Get-DiskImage -ImagePath '${path}').Attached`]);
    const state = output.trim().toLowerCase();
    assert(state === "true" || state === "false", `Unknown VHDX attachment state for ${path}: ${output}`);
    return state === "true";
}

async function killedOrphanIsCleanedByNextProcess() {
    const root = await mkdtemp(join(tmpdir(), "qbutt-storage-faults-orphan-"));
    const helper = join(import.meta.dir, "storage-volume.ts");
    const stderr = join(root, "orphan.stderr.log");
    const child = Bun.spawn([process.execPath, helper, "--create-orphan", root], {
        stdout: "pipe", stderr: Bun.file(stderr),
    });
    const reader = child.stdout.getReader();
    let output = "";
    try {
        while (!output.includes("\n")) {
            const chunk = await reader.read();
            assert(!chunk.done, `Orphan creator exited before readiness; inspect ${stderr}`);
            output += new TextDecoder().decode(chunk.value);
        }
    }
    finally { reader.releaseLock(); }
    const ready = JSON.parse(output.split("\n")[0]!) as {
        ready: boolean; marker: string; image: string; mount: string; id: string;
    };
    assert(ready.ready && child.exitCode === null, "Orphan creator did not retain its process");
    assert.equal(resolve(ready.marker), ownershipMarker(root), "Orphan creator returned a different marker");
    assert(existsSync(ready.image), "Orphan creator omitted its VHDX");
    assert.equal((await mountedVolume(ready.mount))?.toLowerCase(), ready.id.toLowerCase(),
        "Orphan creator omitted its directory mount");
    child.kill();
    await child.exited;
    assert(existsSync(ready.marker) && existsSync(ready.image), "Killed creator cleaned its own resources");
    await run(process.execPath, [helper, "--preflight"]);
    assert(!existsSync(ready.marker) && !existsSync(ready.image), "Next process retained orphan-owned storage");
    assert(!await mountedVolume(ready.mount), "Next process retained the orphan directory mount");
    const contents = await readdir(root);
    assert(contents.every(name => name === "mounted-volume" || name === "orphan.stderr.log"),
        `Unexpected orphan cleanup artifacts: ${contents.join(", ")}`);
    await rm(root, { recursive: true });
}

class VirtualDisk {
    readonly path: string;
    readonly mount: string;
    private id = "";
    private attached = false;
    private mounted = false;

    constructor(name: string, mount: string) {
        this.path = join(lab.root, `${name}.vhdx`);
        this.mount = mount;
        assert(resolve(this.path).startsWith(resolve(lab.root) + "\\"), "VHDX escaped the owned lab");
        assert(resolve(this.mount).startsWith(resolve(lab.root) + "\\"), "VHDX mount escaped the owned lab");
    }

    get root() {
        assert(this.mounted, "VHDX is not mounted");
        return this.mount;
    }

    get isAttached() {
        return this.attached;
    }

    async create() {
        assert(!existsSync(this.path), "VHDX path is already in use");
        await ensureEmptyMount(this.mount);
        await diskpart([
            `create vdisk file="${this.path}" maximum=96 type=expandable`,
            `select vdisk file="${this.path}"`, "attach vdisk", "convert gpt", "create partition primary",
            "gpt attributes=0x8000000000000000",
            "format fs=ntfs quick label=QBUTTFAULT",
        ]);
        this.attached = true;
        this.id = await imageVolumeId(this.path);
        await registerOwnedVolume(lab.root, this.path, this.id);
        await this.mountVolume();
    }

    async detach() {
        assert(this.attached, "VHDX is already detached");
        assert(await imageAttached(this.path), "Owned VHDX is not attached before detach");
        if (this.mounted) {
            const mountedId = (await run("mountvol.exe", [this.mount, "/L"])).trim();
            assert.equal(mountedId.toLowerCase(), this.id.toLowerCase(),
                "Directory mount no longer names the owned VHDX");
            await forceDismount(this.mount);
            this.mounted = false;
        }
        await diskpart([`select vdisk file="${this.path}"`, "detach vdisk"]);
        assert(!await imageAttached(this.path), "Owned VHDX remained attached after detach");
        this.attached = false;
    }

    async attach() {
        assert(!this.attached, "VHDX is already attached");
        await diskpart([`select vdisk file="${this.path}"`, "attach vdisk"]);
        this.attached = true;
        assert(await imageAttached(this.path), "Owned VHDX did not attach");
        assert(await imageVolumeId(this.path) === this.id, "Attached VHDX volume identity changed");
        await this.mountVolume();
    }

    async dispose() {
        if (existsSync(this.path)) {
            const mountedId = await mountedVolume(this.mount);
            if (mountedId?.toLowerCase() === this.id.toLowerCase())
                await forceDismount(this.mount);
            if (await imageAttached(this.path)) {
                await diskpart([`select vdisk file="${this.path}"`, "detach vdisk"]);
                assert(!await imageAttached(this.path), "Owned VHDX remained attached during cleanup");
            }
        }
        this.attached = false;
        this.mounted = false;
        await rm(this.path, { force: true });
        assert(!existsSync(this.path), "Owned VHDX was not removed");
    }

    private async mountVolume() {
        assert(!this.mounted, "VHDX is already mounted");
        const rememberedId = await mountedVolume(this.mount);
        if (rememberedId) {
            assert.equal(rememberedId.toLowerCase(), this.id.toLowerCase(),
                "Remembered directory mount names a different volume");
            this.mounted = true;
            return;
        }
        await ensureEmptyMount(this.mount);
        await run("mountvol.exe", [this.mount, this.id]);
        const mountedId = (await run("mountvol.exe", [this.mount, "/L"])).trim();
        assert.equal(mountedId.toLowerCase(), this.id.toLowerCase(), "Directory mount names a different volume");
        this.mounted = true;
    }

}

function mappings(format = "v1") {
    const torrent = lab.manifest.torrents.find(item => item.name === format)!;
    return Object.fromEntries(torrent.files.filter(file => !file.pad)
        .map(file => [file.index, join(lab.fixtures, "seed", file.path)]));
}

async function plan(hash: string) {
    const operation = await (await lab.request("qbuttRepair/analyze", {
        hash, mode: "staged", mappings: JSON.stringify(mappings()),
    })).json() as Status;
    const planned = await waitFor("storage fault staging plan", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "planned" || status.state === "failed");
    assert.equal(planned.state, "planned", planned.error);
    assert(planned.staging, "Staging plan omitted storage accounting");
    return { operation, planned, transaction: dirname(planned.staging.payload_path) };
}

async function failedPrepare(id: string, pattern: RegExp) {
    await lab.request("qbuttRepair/prepare", { id, consent: "true" });
    const failed = await waitFor("storage fault rejection", () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "failed");
    assert.match(failed.error ?? "", pattern);
    return failed;
}

async function prepareReady(id: string, label: string) {
    await lab.request("qbuttRepair/prepare", { id, consent: "true" });
    const ready = await waitFor(label, () => lab.json<Status>("qbuttRepair/status"),
        status => status.state === "ready_to_commit" || status.state === "failed", 90000);
    assert.equal(ready.state, "ready_to_commit", ready.error);
    return ready;
}

async function removeTorrent(hash: string) {
    await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
    await waitFor("storage fault torrent removal", () => lab.json<unknown[]>("torrents/info"), items => items.length === 0);
}

async function fillBelow(root: string, required: number) {
    const free = async () => {
        const info = await statfs(root);
        return Number(info.bavail) * Number(info.bsize);
    };
    const target = Math.max(512 * 1024, Math.min(required - 65536, 1024 * 1024));
    const before = await free();
    assert(before > required + 1024 * 1024, "VHDX starts without enough space for the disk-full control");
    const filler = join(root, "allocated-filler.bin");
    const handle = await open(filler, "wx");
    const buffer = Buffer.alloc(1024 * 1024, 0x5a);
    try {
        for (let remaining = before - target; remaining > 0;) {
            const count = Math.min(remaining, buffer.length);
            const result = await handle.write(buffer, 0, count);
            assert(result.bytesWritten === count, "Disk filler write was short");
            remaining -= count;
        }
        await handle.sync();
    }
    finally { await handle.close(); }
    const after = await free();
    assert(after < required, `Disk-full fixture left ${after} bytes for a ${required}-byte plan`);
    return { filler, before, after };
}

async function createCandidate(path: string, unknown: string) {
    await cp(join(lab.fixtures, "variants", "grow"), path, { recursive: true });
    await writeFile(join(path, "unknown-save.dat"), unknown);
}

const disks: VirtualDisk[] = [];
let fileLock: ReturnType<typeof Bun.spawn> | undefined;
let failure: unknown;
try {
    await killedOrphanIsCleanedByNextProcess();
    await lab.checkpoint({ check: "fault-killed-volume-owner-cleaned-by-next-process" });
    const mount = join(lab.root, "mounted-volume");
    const primary = new VirtualDisk("primary", mount);
    const replacement = new VirtualDisk("replacement", mount);
    disks.push(primary, replacement);
    await createOwnership(lab.root, disks.map(disk => disk.path));
    await primary.create();
    await lab.start();

    {
        const destination = join(primary.root, "disk-full-target");
        await createCandidate(destination, "preserve through disk-full rejection");
        const before = await snapshot(destination);
        const hash = await lab.add("v1", destination);
        await waitFor("disk-full target stopped", () => lab.info(hash), status => status.state.startsWith("stopped"));
        const { operation, planned, transaction } = await plan(hash);
        const required = Number(planned.staging!.required_bytes);
        const filled = await fillBelow(primary.root, required);
        await failedPrepare(operation.id, /additional bytes|target volume/i);
        await assert.rejects(stat(transaction), { code: "ENOENT" }, "Disk-full rejection created staging data");
        assert.deepEqual(await snapshot(destination), before, "Disk-full rejection changed target or unknown files");
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await removeTorrent(hash);
        await unlink(filled.filler);
        await lab.checkpoint({ check: "disk-full-fails-before-staging-without-inplace-fallback",
            requiredBytes: required, availableBytes: filled.after });
    }

    {
        const destination = join(primary.root, "disappearing-volume-target");
        await createCandidate(destination, "preserve across disappearing volume");
        const before = await snapshot(destination);
        const hash = await lab.add("v1", destination);
        await waitFor("disappearing-volume target stopped", () => lab.info(hash), status => status.state.startsWith("stopped"));
        let { operation, transaction } = await plan(hash);
        await prepareReady(operation.id, "disappearing-volume staging ready");
        await lab.shutdown();
        try {
            await primary.detach();
            await lab.start();
            await waitFor("detached-volume torrent initialized", () => lab.info(hash),
                status => status.state.startsWith("stopped") || status.state === "missingFiles");
            operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as Status;
            const failed = await waitFor("detached-volume recovery rejected", () => lab.json<Status>("qbuttRepair/status"),
                status => status.state !== "analyzing");
            assert.equal(failed.state, "failed", "Detached target admitted recovery");
            assert.match(failed.error ?? "", /target volume|local fixed drive|directory|path/i);
        }
        finally {
            await lab.shutdown();
            if (!primary.isAttached)
                await primary.attach();
        }
        await lab.start();
        await waitFor("remounted-volume torrent initialized", () => lab.info(hash),
            status => status.state.startsWith("stopped") || status.state === "missingFiles");
        assert.deepEqual(await snapshot(destination, transaction), before,
            "Disappearing volume changed target or unknown files");
        operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as Status;
        await waitFor("remounted-volume journal recovered", () => lab.json<Status>("qbuttRepair/status"),
            status => status.state === "ready_to_commit" || status.state === "failed");
        await lab.request("qbuttRepair/rollback", { id: operation.id, consent: "true" });
        const rolledBack = await waitFor("remounted-volume rollback", () => lab.json<Status>("qbuttRepair/status"),
            status => status.staging?.finalized === true || status.state === "failed");
        assert.equal(rolledBack.state, "rolled_back", rolledBack.error);
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await removeTorrent(hash);
        assert.deepEqual(await snapshot(destination, transaction), before,
            "Rollback after remount changed target or unknown files");
        await lab.checkpoint({ check: "detached-volume-recovery-fails-closed-and-remount-rolls-back" });
    }

    {
        const relativeDestination = "same-path-volume-target";
        let replacementBefore: Awaited<ReturnType<typeof snapshot>>;
        await lab.shutdown();
        try {
            await primary.detach();
            await replacement.create();
            const replacementDestination = join(replacement.root, relativeDestination);
            await createCandidate(replacementDestination, "replacement volume unknown data");
            replacementBefore = await snapshot(replacementDestination);
            await replacement.detach();
        }
        finally {
            if (replacement.isAttached)
                await replacement.detach();
            if (!primary.isAttached)
                await primary.attach();
        }
        await lab.start();
        const destination = join(primary.root, relativeDestination);
        await createCandidate(destination, "original volume unknown data");
        const originalBefore = await snapshot(destination);
        const hash = await lab.add("v1", destination);
        await waitFor("same-path target stopped", () => lab.info(hash), status => status.state.startsWith("stopped"));
        let { operation, transaction } = await plan(hash);
        await prepareReady(operation.id, "same-path original staging ready");
        await lab.shutdown();
        try {
            await primary.detach();
            await replacement.attach();
            await lab.start();
            await waitFor("replacement-volume torrent initialized", () => lab.info(hash),
                status => status.state.startsWith("stopped") || status.state === "missingFiles");
            operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as Status;
            const recovered = await waitFor("replacement-volume journal loaded", () => lab.json<Status>("qbuttRepair/status"),
                status => status.state === "ready_to_commit" || status.state === "failed");
            assert.equal(recovered.state, "ready_to_commit", recovered.error);
            await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
            const failed = await waitFor("replacement-volume commit rejected", () => lab.json<Status>("qbuttRepair/status"),
                status => status.state === "failed");
            assert.match(failed.error ?? "", /volume or directory layout changed/i);
            await assert.rejects(stat(transaction), { code: "ENOENT" }, "Replacement volume received staging data");
            assert.deepEqual(await snapshot(join(replacement.root, relativeDestination)), replacementBefore!,
                "Rejected replacement volume or its unknown data changed");
        }
        finally {
            await lab.shutdown();
            if (replacement.isAttached)
                await replacement.detach();
            if (!primary.isAttached)
                await primary.attach();
        }
        await lab.start();
        await waitFor("original-volume torrent initialized", () => lab.info(hash),
            status => status.state.startsWith("stopped") || status.state === "missingFiles");
        assert.deepEqual(await snapshot(destination, transaction), originalBefore,
            "Original volume changed while detached");
        operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as Status;
        await waitFor("original-volume journal recovered", () => lab.json<Status>("qbuttRepair/status"),
            status => status.state === "ready_to_commit" || status.state === "failed");
        await lab.request("qbuttRepair/rollback", { id: operation.id, consent: "true" });
        const rolledBack = await waitFor("original-volume rollback", () => lab.json<Status>("qbuttRepair/status"),
            status => status.staging?.finalized === true || status.state === "failed");
        assert.equal(rolledBack.state, "rolled_back", rolledBack.error);
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await removeTorrent(hash);
        assert.deepEqual(await snapshot(destination, transaction), originalBefore,
            "Rollback changed the restored original volume");
        await lab.checkpoint({ check: "same-path-different-volume-identity-rejected-before-write" });
    }

    {
        let destination = join(primary.root, "long-case-layout");
        let index = 0;
        while (join(destination, "bundle", "nested", "beta.bin").length <= 260)
            destination = join(destination, `segment-${String(index++).padStart(2, "0")}-${"x".repeat(14)}`);
        assert(destination.length < 260, "Case-sensitive directory itself must remain addressable by fsutil");
        assert(join(destination, "bundle", "nested", "beta.bin").length > 260,
            "Long-path fixture did not cross the legacy Windows path boundary");
        await mkdir(destination, { recursive: true });
        await run("fsutil.exe", ["file", "setCaseSensitiveInfo", destination, "enable"]);
        const caseState = await run("fsutil.exe", ["file", "queryCaseSensitiveInfo", destination]);
        assert.match(caseState, /enabled/i, "Directory did not become case-sensitive");
        await cp(join(lab.fixtures, "variants", "grow"), destination, { recursive: true });
        await writeFile(join(destination, "Save.dat"), "uppercase unknown");
        await writeFile(join(destination, "save.dat"), "lowercase unknown");
        assert.equal(await readFile(join(destination, "Save.dat"), "utf8"), "uppercase unknown");
        assert.equal(await readFile(join(destination, "save.dat"), "utf8"), "lowercase unknown");
        const hash = await lab.add("v1", destination);
        await waitFor("long case-sensitive target stopped", () => lab.info(hash), status => status.state.startsWith("stopped"));
        const { operation, transaction } = await plan(hash);
        await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
        const ready = await waitFor("long case-sensitive staging ready", () => lab.json<Status>("qbuttRepair/status"),
            status => status.state === "ready_to_commit" || status.state === "failed", 90000);
        assert.equal(ready.state, "ready_to_commit", ready.error);
        await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
        const committed = await waitFor("long case-sensitive commit", () => lab.json<Status>("qbuttRepair/status"),
            status => status.staging?.finalized === true || status.state === "failed");
        assert.equal(committed.state, "committed", committed.error);
        await verifyPayload(destination, lab.manifest.payload);
        assert.equal(await readFile(join(destination, "Save.dat"), "utf8"), "uppercase unknown");
        assert.equal(await readFile(join(destination, "save.dat"), "utf8"), "lowercase unknown");
        const final = await snapshot(destination, transaction);
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await removeTorrent(hash);
        assert.deepEqual(await snapshot(destination, transaction), final,
            "Removal changed a long case-sensitive layout or unknown files");
        await lab.checkpoint({ check: "long-case-sensitive-staged-commit-preserves-distinct-unknown-files",
            longestTargetPath: join(destination, "bundle", "nested", "beta.bin").length });
    }

    {
        const destination = join(lab.root, "third-party-handle-target");
        await createCandidate(destination, "preserve through sharing conflict");
        const original = await snapshot(destination);
        const hash = await lab.add("v1", destination);
        await waitFor("sharing-conflict target stopped", () => lab.info(hash), status => status.state.startsWith("stopped"));
        let { operation, transaction } = await plan(hash);
        await lab.request("qbuttRepair/prepare", { id: operation.id, consent: "true" });
        const ready = await waitFor("sharing-conflict staging ready", () => lab.json<Status>("qbuttRepair/status"),
            status => status.state === "ready_to_commit" || status.state === "failed", 90000);
        assert.equal(ready.state, "ready_to_commit", ready.error);
        const lockedPath = join(destination, "bundle", "alpha.bin");
        fileLock = Bun.spawn([lab.python, join(import.meta.dir, "file-lock.py"), lab.root, lockedPath],
            { stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(lab.root, "file-lock.stderr.log")) });
        const reader = fileLock.stdout.getReader();
        let output = "";
        try {
            while (!output.includes("\n")) {
                const chunk = await reader.read();
                assert(!chunk.done, "Third-party file lock ended before readiness");
                output += new TextDecoder().decode(chunk.value);
            }
        }
        finally { reader.releaseLock(); }
        const lockReady = JSON.parse(output.split("\n")[0]!) as { ready: boolean };
        assert(lockReady.ready && fileLock.exitCode === null, "Third-party file lock was not retained");
        await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
        const failed = await waitFor("sharing-conflict commit rejected", () => lab.json<Status>("qbuttRepair/status"),
            status => status.state === "failed");
        assert.match(failed.error ?? "", /Cannot own|writers|Windows error/i);
        assert.deepEqual(await snapshot(destination, transaction), original,
            "Sharing-conflict rejection changed original or unknown files");
        fileLock.stdin.end();
        assert.equal(await fileLock.exited, 0, "Third-party lock did not release cleanly");
        fileLock = undefined;
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await lab.shutdown();
        await lab.start();
        await waitFor("sharing-conflict recovery torrent stopped", () => lab.info(hash),
            status => status.state.startsWith("stopped") || status.state === "missingFiles");
        operation = await (await lab.request("qbuttRepair/analyze", { hash, mode: "recover" })).json() as Status;
        await waitFor("sharing-conflict journal recovered", () => lab.json<Status>("qbuttRepair/status"),
            status => status.state === "ready_to_commit" || status.state === "failed");
        await lab.request("qbuttRepair/commit", { id: operation.id, consent: "true" });
        const recovered = await waitFor("sharing-conflict retry committed", () => lab.json<Status>("qbuttRepair/status"),
            status => status.staging?.finalized === true || status.state === "failed");
        assert.equal(recovered.state, "committed", recovered.error);
        await verifyPayload(destination, lab.manifest.payload);
        assert.equal(await readFile(join(destination, "unknown-save.dat"), "utf8"),
            "preserve through sharing conflict");
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await removeTorrent(hash);
        await lab.checkpoint({ check: "third-party-handle-rejects-before-rename-and-recovery-commits" });
    }
}
catch (error) { failure = error; }
finally {
    if (fileLock && fileLock.exitCode === null) {
        fileLock.stdin.end();
        await fileLock.exited;
    }
    try { await lab.shutdown(); }
    catch (error) { failure ??= error; }
    let storageClean = true;
    for (const disk of disks.reverse()) {
        try { await disk.dispose(); }
        catch (error) { storageClean = false; failure ??= error; }
    }
    if (storageClean && existsSync(ownershipMarker(lab.root))) {
        try { await cleanupOwnedMarker(ownershipMarker(lab.root)); }
        catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure)
    throw failure;
