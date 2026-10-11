import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { cp, lstat, mkdir, mkdtemp, open, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";

const { values } = parseArgs({
    options: {
        destination: { type: "string" },
        cache: { type: "string" },
    },
});
if (!values.destination) {
    throw new Error("Usage: bun scripts/package-media.ts --destination <portable bundle/media> [--cache <download cache>]");
}

const source = resolve(import.meta.dir, "../resources/media");
const destination = resolve(values.destination);
const sourceRelative = relative(resolve(import.meta.dir, ".."), destination);
if (basename(destination).toLowerCase() !== "media"
    || (!isAbsolute(sourceRelative) && sourceRelative !== ".."
        && !sourceRelative.startsWith(`..${sep}`))) {
    throw new Error("Media output must be an external bundle/media directory.");
}

const lock = JSON.parse(await readFile(join(source, "component-lock.json"), "utf8"));
if (lock.schemaVersion !== 2) {
    throw new Error("Unsupported media component lock.");
}
const temporary = await realpath(await mkdtemp(join(tmpdir(), "qbutt-media-")));
const stage = join(temporary, "media");

interface RuntimePin {
    url: string;
    sha256: string;
    size: number;
    files: Record<string, string>;
}

async function verify(file: string, expected: string): Promise<void> {
    const hash = createHash("sha256");
    for await (const chunk of createReadStream(file)) {
        hash.update(chunk);
    }
    if (hash.digest("hex") !== expected) {
        throw new Error(`SHA256 mismatch: ${basename(file)}`);
    }
}

async function packageRuntime(pin: RuntimePin): Promise<void> {
    const members = Object.keys(pin.files);
    for (const member of members) {
        if (!member || member.includes("\\") || member.includes(":")
            || member.split("/").some(part => !part || part === "." || part === "..")) {
            throw new Error(`Invalid pinned media path: ${member}`);
        }
    }
    const cache = values.cache ? resolve(values.cache) : join(temporary, "downloads");
    await mkdir(cache, { recursive: true });
    const archive = join(cache, basename(new URL(pin.url).pathname));
    if (!await Bun.file(archive).exists()) {
        const partial = join(temporary, `${basename(archive)}.partial`);
        const response = await fetch(pin.url, { signal: AbortSignal.timeout(600_000) });
        if (!response.ok || !response.body) throw new Error(`Media tool download failed: HTTP ${response.status}`);
        const file = await open(partial, "wx");
        let received = 0;
        try {
            for await (const chunk of response.body) {
                received += chunk.byteLength;
                if (received > pin.size) throw new Error("Media download exceeds the pinned archive size.");
                await file.writeFile(chunk);
            }
        }
        finally { await file.close(); }
        if (received !== pin.size) throw new Error("Media download differs from the pinned archive size.");
        await verify(partial, pin.sha256);
        await cp(partial, archive, { errorOnExist: true, force: false });
    }
    if (Bun.file(archive).size !== pin.size) throw new Error("Cached media archive differs from the pinned size.");
    await verify(archive, pin.sha256);
    const tar = join(process.env.SystemRoot || "C:/Windows", "System32", "tar.exe");
    const listing = Bun.spawn([tar, "-tf", archive], { stdout: "pipe", stderr: "inherit" });
    const entries = (await new Response(listing.stdout).text()).trimEnd().split(/\r?\n/);
    if (await listing.exited !== 0 || entries.sort().join("\n") !== members.sort().join("\n")) {
        throw new Error("The media archive differs from its pinned file inventory.");
    }
    const extracted = await mkdtemp(join(temporary, "runtime-"));
    const child = Bun.spawn([tar, "-xf", archive], { cwd: extracted, stdout: "inherit", stderr: "inherit" });
    if (await child.exited !== 0) throw new Error("The pinned media archive could not be extracted.");
    for (const [output, expected] of Object.entries(pin.files)) {
        const file = join(extracted, output);
        const info = await lstat(file);
        if (!info.isFile() || info.isSymbolicLink()) throw new Error("The media archive contains an invalid file.");
        await verify(file, expected);
        await mkdir(dirname(join(stage, output)), { recursive: true });
        await cp(file, join(stage, output));
    }
}

try {
    await mkdir(stage);
    const profilePaths = [
        "mpv.conf", "input.conf", "certs", "scripts", "script-modules", "script-opts", "shaders", "fonts", "framegen",
        "tools/bin", "tools/hardware", "tools/framegen-vsov", "tools/framegen-models", "tools/framegen-manager/notices",
    ];
    for (const entry of profilePaths) {
        await cp(join(source, "profile", entry), join(stage, "profile", entry), { recursive: true });
    }
    await cp(join(source, "licenses"), join(stage, "licenses"), { recursive: true });
    await cp(join(source, "component-lock.json"), join(stage, "component-lock.json"));
    await packageRuntime(lock.runtime);
    for (const [entry, hash] of Object.entries(lock.bundledHashes as Record<string, string>)) {
        await verify(join(stage, entry), hash);
    }

    await writeFile(join(stage, ".qbutt-media.json"), `${JSON.stringify({ schemaVersion: 1, mpv: lock.mpv.version })}\n`);

    await mkdir(dirname(destination), { recursive: true });
    const existing = await lstat(destination).catch(error => {
        if (error.code === "ENOENT") {
            return null;
        }
        throw error;
    });
    if (existing) {
        if (!existing.isDirectory() || existing.isSymbolicLink()
            || !(await Bun.file(join(destination, ".qbutt-media.json")).exists())) {
            throw new Error("Existing media output is not owned by the qbutt packager. Choose a clean bundle.");
        }
        await rm(destination, { recursive: true });
    }
    await cp(stage, destination, { recursive: true });
    console.log(`Bundled pinned mpv, FFmpeg and yt-dlp with ModernX, FX and frame generation profile: ${destination}`);
}
finally {
    await rm(temporary, { recursive: true, force: true });
}
