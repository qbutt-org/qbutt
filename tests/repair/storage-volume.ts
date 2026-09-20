import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { tmpdir } from "node:os";

const markerName = ".qbutt-storage-faults.json";
const labPrefix = "qbutt-storage-faults-";
const imageNames = new Set(["primary.vhdx", "replacement.vhdx", "orphan.vhdx"]);

interface OwnedImage {
    path: string;
    volumeId: string;
}

interface Ownership {
    schema: 1;
    owner: "qbutt-storage-faults";
    root: string;
    mount: string;
    images: OwnedImage[];
}

function quotePowerShell(value: string) {
    return `'${value.replaceAll("'", "''")}'`;
}

async function invoke(command: string, args: string[], timeout = 60000) {
    const child = Bun.spawn([command, ...args], { stdout: "pipe", stderr: "pipe", windowsHide: true });
    let timer: ReturnType<typeof setTimeout>;
    const exited = Promise.race([child.exited, new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
            child.kill();
            reject(new Error(`${command} timed out after ${timeout} ms`));
        }, timeout);
    })]);
    try {
        const [exitCode, stdout, stderr] = await Promise.all([
            exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
        ]);
        return { exitCode, stdout, stderr };
    }
    finally { clearTimeout(timer!); }
}

async function run(command: string, args: string[], timeout = 60000) {
    const result = await invoke(command, args, timeout);
    assert.equal(result.exitCode, 0, `${command} exited ${result.exitCode}: ${result.stderr || result.stdout}`);
    return result.stdout;
}

async function mountedVolume(path: string) {
    const result = await invoke("mountvol.exe", [path, "/L"]);
    return result.exitCode === 0 ? result.stdout.trim() : undefined;
}

async function imageAttached(path: string) {
    const output = await run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
        `(Get-DiskImage -ImagePath ${quotePowerShell(path)}).Attached`]);
    const state = output.trim().toLowerCase();
    assert(state === "true" || state === "false", `Unknown VHDX attachment state for ${path}: ${output}`);
    return state === "true";
}

async function imageVolumeId(path: string) {
    const output = await run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
        `(Get-DiskImage -ImagePath ${quotePowerShell(path)} | Get-Disk | Get-Partition | Get-Volume).UniqueId`]);
    const id = output.trim();
    assert(id.startsWith("\\\\?\\Volume{"), `No volume identity for ${path}`);
    return id;
}

async function diskpart(root: string, commands: string[]) {
    const script = join(root, `diskpart-cleanup-${randomUUID()}.txt`);
    await writeFile(script, [...commands, "exit"].join("\r\n") + "\r\n", "ascii");
    try { await run("diskpart.exe", ["/s", script]); }
    finally { await rm(script, { force: true }); }
}

function validateRoot(path: string) {
    const root = resolve(path);
    const temporary = resolve(tmpdir());
    assert(dirname(root) === temporary, `Owned storage lab escaped the temporary root: ${root}`);
    assert(basename(root).startsWith(labPrefix), `Unexpected owned storage lab name: ${root}`);
    return root;
}

function validateOwnership(markerPath: string, value: unknown) {
    const root = validateRoot(dirname(resolve(markerPath)));
    assert(resolve(markerPath) === join(root, markerName), `Unexpected storage marker path: ${markerPath}`);
    assert(value && typeof value === "object", "Storage ownership marker is not an object");
    const ownership = value as Ownership;
    assert(ownership.schema === 1 && ownership.owner === "qbutt-storage-faults", "Unknown storage ownership marker");
    assert(resolve(ownership.root) === root, "Storage ownership root changed");
    assert(resolve(ownership.mount) === join(root, "mounted-volume"), "Storage ownership mount changed");
    assert(Array.isArray(ownership.images) && ownership.images.length > 0 && ownership.images.length <= 2,
        "Storage ownership image list is invalid");
    const paths = new Set<string>();
    for (const image of ownership.images) {
        const path = resolve(image.path);
        assert(dirname(path) === root && imageNames.has(basename(path)), `Unexpected owned VHDX path: ${path}`);
        assert(!paths.has(path.toLowerCase()), `Duplicate owned VHDX path: ${path}`);
        assert(image.volumeId === "" || /^\\\\\?\\Volume\{[0-9a-f-]+\}\\$/i.test(image.volumeId),
            `Invalid owned volume identity: ${image.volumeId}`);
        image.path = path;
        paths.add(path.toLowerCase());
    }
    ownership.root = root;
    ownership.mount = join(root, "mounted-volume");
    return ownership;
}

async function readOwnership(markerPath: string) {
    return validateOwnership(markerPath, JSON.parse(await readFile(markerPath, "utf8")));
}

async function saveOwnership(ownership: Ownership) {
    await writeFile(join(ownership.root, markerName), `${JSON.stringify(ownership, null, 2)}\n`, { flag: "w" });
}

async function forceDismount(path: string) {
    let diagnostic = "";
    for (let attempt = 0; attempt < 10; ++attempt) {
        const result = await invoke("mountvol.exe", [path, "/P"]);
        diagnostic = `exit ${result.exitCode}: ${result.stderr || result.stdout}`;
        if (!await mountedVolume(path))
            return;
        await Bun.sleep(100);
    }
    assert.fail(`Forced dismount left the owned mount active (${diagnostic})`);
}

export function ownershipMarker(root: string) {
    return join(validateRoot(root), markerName);
}

export async function createOwnership(rootPath: string, imagePaths: string[]) {
    const root = validateRoot(rootPath);
    await mkdir(join(root, "mounted-volume"), { recursive: true });
    const ownership = validateOwnership(join(root, markerName), {
        schema: 1,
        owner: "qbutt-storage-faults",
        root,
        mount: join(root, "mounted-volume"),
        images: imagePaths.map(path => ({ path: resolve(path), volumeId: "" })),
    });
    await saveOwnership(ownership);
}

export async function registerOwnedVolume(root: string, imagePath: string, volumeId: string) {
    const marker = ownershipMarker(root);
    const ownership = await readOwnership(marker);
    const image = ownership.images.find(item => item.path.toLowerCase() === resolve(imagePath).toLowerCase());
    assert(image, `VHDX is absent from its ownership marker: ${imagePath}`);
    assert(/^\\\\\?\\Volume\{[0-9a-f-]+\}\\$/i.test(volumeId), `Invalid VHDX volume identity: ${volumeId}`);
    image.volumeId = volumeId;
    await saveOwnership(ownership);
}

export async function cleanupOwnedMarker(markerPath: string) {
    const ownership = await readOwnership(markerPath);
    const identities = new Set(ownership.images.map(image => image.volumeId.toLowerCase()).filter(Boolean));
    for (const image of ownership.images) {
        if (existsSync(image.path) && await imageAttached(image.path)) {
            const actual = await imageVolumeId(image.path);
            if (image.volumeId)
                assert.equal(actual.toLowerCase(), image.volumeId.toLowerCase(), "Attached VHDX identity changed");
            identities.add(actual.toLowerCase());
        }
    }
    const mounted = await mountedVolume(ownership.mount);
    if (mounted) {
        assert(identities.has(mounted.toLowerCase()), "Cleanup refuses a mount not owned by the storage fixture");
        await forceDismount(ownership.mount);
    }
    for (const image of ownership.images) {
        if (!existsSync(image.path))
            continue;
        if (await imageAttached(image.path)) {
            await diskpart(ownership.root, [`select vdisk file="${image.path}"`, "detach vdisk"]);
            assert(!await imageAttached(image.path), `Owned VHDX remained attached: ${image.path}`);
        }
        await rm(image.path, { force: true });
        assert(!existsSync(image.path), `Owned VHDX remained after cleanup: ${image.path}`);
    }
    assert(!await mountedVolume(ownership.mount), "Owned directory mount survived cleanup");
    await rm(markerPath, { force: true });
}

export async function cleanupMarkedLabs() {
    const temporary = resolve(tmpdir());
    const entries = await readdir(temporary, { withFileTypes: true });
    for (const entry of entries) {
        if (!entry.isDirectory() || !entry.name.startsWith(labPrefix))
            continue;
        const marker = join(temporary, entry.name, markerName);
        if (existsSync(marker))
            await cleanupOwnedMarker(marker);
    }
}

async function createKilledOrphan(rootPath: string) {
    const root = validateRoot(rootPath);
    const image = join(root, "orphan.vhdx");
    const mount = join(root, "mounted-volume");
    await createOwnership(root, [image]);
    await diskpart(root, [
        `create vdisk file="${image}" maximum=32 type=expandable`,
        `select vdisk file="${image}"`, "attach vdisk", "convert gpt", "create partition primary",
        "gpt attributes=0x8000000000000000", "format fs=ntfs quick label=QBUTTFAULT",
    ]);
    assert(await imageAttached(image), "Orphan fixture VHDX did not attach");
    const id = await imageVolumeId(image);
    await registerOwnedVolume(root, image, id);
    await run("mountvol.exe", [mount, id]);
    assert.equal((await mountedVolume(mount))?.toLowerCase(), id.toLowerCase(), "Orphan fixture mount failed");
    process.stdout.write(`${JSON.stringify({ ready: true, marker: ownershipMarker(root), image, mount, id })}\n`);
    await new Promise(() => {});
}

if (import.meta.main) {
    if ((process.argv[2] === "--create-orphan") && process.argv[3])
        await createKilledOrphan(process.argv[3]);
    else if (process.argv[2] === "--preflight")
        await cleanupMarkedLabs();
    else
        throw new Error("Use --create-orphan <exact-root> or --preflight");
}
