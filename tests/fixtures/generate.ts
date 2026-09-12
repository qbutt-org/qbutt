import { createHash } from "node:crypto";
import { appendFile, cp, link, lstat, mkdir, mkdtemp, readFile, rename, stat, symlink, truncate, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

export interface PayloadFile {
    path: string;
    size: number;
    sha256: string;
}

export interface TorrentFixture {
    name: string;
    file: string;
    pieceLength: number;
    pieceCount: number;
    infoHashV1: string | null;
    infoHashV2: string | null;
    files: { index: number; path: string; size: number; offset: number; pad: boolean }[];
}

export interface FixtureManifest {
    version: 1;
    generator: { libtorrent: string };
    payload: PayloadFile[];
    torrents: TorrentFixture[];
    variants: { name: string; changes: string[]; mapping?: Record<string, string> }[];
    filesystemNegatives: Record<string, { status: "ready" | "unsupported"; code?: string }>;
}

export function sha256(bytes: Uint8Array): string {
    return createHash("sha256").update(bytes).digest("hex");
}

// Counter-mode SHA-256 is deterministic across operating systems and has no
// compressible pattern that could make throughput measurements misleading.
function payloadBytes(name: string, size: number): Buffer {
    const result = Buffer.alloc(size);
    for (let offset = 0, counter = 0; offset < size; offset += 32, ++counter)
        createHash("sha256").update(`qbutt-fixture-v1:${name}:${counter}`).digest().copy(result, offset);
    return result;
}

export async function generateFixtures(python: string, output?: string): Promise<string> {
    const root = output ? resolve(output) : await mkdtemp(join(tmpdir(), "qbutt-fixtures-"));
    // An explicit output must be new: the fixture tool never overwrites a user's tree.
    if (output)
        await mkdir(root);
    const sizes: [string, number][] = [
        ["bundle/alpha.bin", 16384 + 123],
        ["bundle/nested/beta.bin", 32768 + 777],
        ["bundle/skip.bin", 2 * 1024 * 1024 + 61],
        ["bundle/Юникод/данные.bin", 4097],
        ["bundle/empty.bin", 0],
    ];
    const payload: PayloadFile[] = [];
    for (const [path, size] of sizes) {
        const bytes = payloadBytes(path, size);
        await mkdir(dirname(join(root, "seed", path)), { recursive: true });
        await writeFile(join(root, "seed", path), bytes);
        payload.push({ path, size, sha256: sha256(bytes) });
    }
    await writeFile(join(root, "payload.json"), JSON.stringify(payload, null, 2) + "\n");
    const generated = Bun.spawn([python, join(import.meta.dir, "torrents.py"), root], {
        stdout: "pipe", stderr: "pipe",
    });
    const [exitCode, stdout, stderr] = await Promise.all([
        generated.exited, new Response(generated.stdout).text(), new Response(generated.stderr).text(),
    ]);
    if (exitCode !== 0)
        throw new Error(`libtorrent fixture generator exited ${exitCode}: ${stderr}`);
    const metadata = JSON.parse(stdout) as { libtorrent: string; torrents: TorrentFixture[] };
    const variants: FixtureManifest["variants"] = [];
    for (const name of ["corrupt", "grow", "shrink", "renamed", "source-mutated", "inserted", "unknown", "hardlink", "reparse"]) {
        await cp(join(root, "seed"), join(root, "variants", name), { recursive: true });
        variants.push({ name, changes: [] });
    }
    const variant = (name: string, file = "bundle/alpha.bin") => join(root, "variants", name, file);
    const corrupt = await readFile(variant("corrupt"));
    corrupt[16384] = corrupt[16384]! ^ 0xff;
    await writeFile(variant("corrupt"), corrupt);
    variants.find(item => item.name === "corrupt")!.changes = ["alpha.bin byte 16384 flipped; v1 boundary piece also contains beta.bin"];
    await appendFile(variant("grow"), Buffer.alloc(8193, 0xa5));
    variants.find(item => item.name === "grow")!.changes = ["alpha.bin has an 8193-byte extra tail"];
    await truncate(variant("shrink"), 10001);
    variants.find(item => item.name === "shrink")!.changes = ["alpha.bin truncated to 10001 bytes"];
    await rename(variant("renamed"), variant("renamed", "bundle/renamed.bin"));
    const renamed = variants.find(item => item.name === "renamed")!;
    renamed.changes = ["alpha.bin renamed; content unchanged"];
    renamed.mapping = { "bundle/alpha.bin": "bundle/renamed.bin" };
    const indexed = await stat(variant("source-mutated"));
    await writeFile(join(root, "variants", "source-mutated", "index-before-mutation.json"), JSON.stringify({
        path: "bundle/alpha.bin", size: indexed.size, sha256: payload[0]!.sha256,
    }, null, 2) + "\n");
    const changed = await readFile(variant("source-mutated"));
    changed[1] = changed[1]! ^ 0x80;
    await writeFile(variant("source-mutated"), changed);
    variants.find(item => item.name === "source-mutated")!.changes = ["same-size mutation after metadata/hash snapshot"];
    const original = await readFile(variant("inserted"));
    await writeFile(variant("inserted"), Buffer.concat([original.subarray(0, 1000), Buffer.from([42]), original.subarray(1000)]));
    variants.find(item => item.name === "inserted")!.changes = ["one byte inserted at offset 1000; no universal delta reuse expected"];
    await writeFile(variant("unknown", "bundle/user-notes.txt"), "Unknown user file must survive repair.\n");
    variants.find(item => item.name === "unknown")!.changes = ["unknown user-notes.txt must be preserved"];
    const filesystemNegatives: FixtureManifest["filesystemNegatives"] = {};
    try {
        await link(variant("hardlink"), join(root, "variants", "hardlink-alias.bin"));
        if ((await stat(variant("hardlink"))).nlink < 2)
            throw new Error("Filesystem did not report a hardlink alias");
        filesystemNegatives.hardlink = { status: "ready" };
    }
    catch (error) {
        filesystemNegatives.hardlink = { status: "unsupported", code: (error as NodeJS.ErrnoException).code ?? String(error) };
    }
    try {
        await rename(variant("reparse", "bundle/nested"), join(root, "variants", "reparse-target"));
        await symlink(join(root, "variants", "reparse-target"), variant("reparse", "bundle/nested"), "junction");
        if (!(await lstat(variant("reparse", "bundle/nested"))).isSymbolicLink())
            throw new Error("Filesystem did not report a reparse point");
        filesystemNegatives.reparse = { status: "ready" };
    }
    catch (error) {
        filesystemNegatives.reparse = { status: "unsupported", code: (error as NodeJS.ErrnoException).code ?? String(error) };
    }
    const manifest: FixtureManifest = {
        version: 1, generator: { libtorrent: metadata.libtorrent }, payload,
        torrents: metadata.torrents, variants, filesystemNegatives,
    };
    await writeFile(join(root, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
    return root;
}

if (import.meta.main) {
    const python = process.env.QBUTT_LAB_PYTHON;
    if (!python)
        throw new Error("Set QBUTT_LAB_PYTHON to Python with tests/fixtures/requirements.txt installed");
    console.log(JSON.stringify({ fixtures: await generateFixtures(python, process.argv[2]) }));
}
