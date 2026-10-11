import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, realpath, stat, writeFile } from "node:fs/promises";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";

interface Notice {
    file: string;
    sha256: string;
    text: string;
}

interface Dependency {
    name: string;
    version: string;
    source: string;
    integrity?: string;
    notices: Notice[];
}

interface GoModule {
    Path: string;
    Version?: string;
    Sum?: string;
    Dir?: string;
    Replace?: GoModule;
}

const { values } = parseArgs({
    options: {
        bundle: { type: "string" },
        go: { type: "string", default: "go" },
    },
});
assert(values.bundle, "Usage: bun scripts/collect-content-notices.ts --bundle <bundle> --go <pinned go.exe>");

const repository = resolve(import.meta.dir, "..");
const content = join(repository, "components/content");
const search = join(repository, "components/search-engine");
const output = resolve(values.bundle, "licenses");
const outputRelative = relative(repository, output);
assert(outputRelative === ".." || outputRelative.startsWith(`..${sep}`) || isAbsolute(outputRelative),
    "Keep generated notices outside source");

async function command(args: string[], cwd: string): Promise<string> {
    const child = Bun.spawn(args, {
        cwd,
        env: { ...process.env, GOTOOLCHAIN: "local" },
        stdout: "pipe",
        stderr: "pipe",
        windowsHide: true,
    });
    const timeout = setTimeout(() => child.kill(), 60000);
    try {
        const [stdout, stderr, code] = await Promise.all([
            new Response(child.stdout).text(),
            new Response(child.stderr).text(),
            child.exited,
        ]);
        assert.equal(code, 0, `${basename(args[0])} ${args[1]} failed: ${stderr.slice(0, 4096)}`);
        return stdout.trim();
    }
    finally {
        clearTimeout(timeout);
    }
}

async function notices(root: string, recursive: boolean): Promise<Notice[]> {
    const pending = [root];
    const result: Notice[] = [];
    let inspected = 0;
    while (pending.length) {
        const directory = pending.pop()!;
        for (const entry of await readdir(directory, { withFileTypes: true })) {
            assert(++inspected < 100000, "Dependency notice search exceeded its file limit");
            const file = join(directory, entry.name);
            if (entry.isDirectory() && recursive && ![".git", "node_modules"].includes(entry.name)) {
                pending.push(file);
            }
            else if (entry.isFile() && /^(?:licen[sc]e|copying|notice|patents|copyright)(?:[._-].*)?$/i.test(entry.name)) {
                assert((await stat(file)).size <= 1024 * 1024, "Notice exceeds 1 MiB");
                const bytes = await readFile(file);
                assert(!bytes.includes(0), "Notice is not plain text");
                result.push({
                    file: relative(root, file).replaceAll("\\", "/"),
                    sha256: createHash("sha256").update(bytes).digest("hex"),
                    text: new TextDecoder("utf-8", { fatal: true }).decode(bytes),
                });
            }
        }
    }
    return result.sort((left, right) => left.file.localeCompare(right.file, "en"));
}

async function javascriptDependencies(): Promise<Dependency[]> {
    const pending = [content];
    const visited = new Set<string>();
    const result: Dependency[] = [];
    while (pending.length) {
        const directory = await realpath(pending.pop()!);
        if (visited.has(directory)) {
            continue;
        }
        visited.add(directory);
        const metadata = JSON.parse(await readFile(join(directory, "package.json"), "utf8"));
        if (directory !== content) {
            const files = await notices(directory, true);
            if (!files.length) {
                const supplemental = `${metadata.name.replaceAll("/", "_")}-${metadata.version}.txt`;
                const bytes = await readFile(join(repository, "dist/licenses", supplemental));
                files.push({
                    file: `qbutt-supplemental/${supplemental}`,
                    sha256: createHash("sha256").update(bytes).digest("hex"),
                    text: bytes.toString("utf8"),
                });
            }
            result.push({
                name: metadata.name,
                version: metadata.version,
                source: `https://registry.npmjs.org/${metadata.name}/-/${metadata.name.split("/").at(-1)}-${metadata.version}.tgz`,
                notices: files,
            });
        }
        for (const dependency of Object.keys(metadata.dependencies ?? {})) {
            let parent = directory;
            let found = false;
            while (parent.startsWith(content)) {
                const candidate = join(parent, "node_modules", dependency);
                if (await Bun.file(join(candidate, "package.json")).exists()) {
                    pending.push(candidate);
                    found = true;
                    break;
                }
                parent = dirname(parent);
            }
            assert(found, `Missing installed dependency: ${dependency}`);
        }
    }
    return result.sort((left, right) => `${left.name}@${left.version}`.localeCompare(`${right.name}@${right.version}`, "en"));
}

async function goDependencies(): Promise<Dependency[]> {
    const build = JSON.parse(await command([values.go!, "version", "-m", "-json", join(values.bundle!, "qbutt-search.exe")], search));
    assert.equal(build.Path, "qbutt.org/search-engine/cmd/qbutt-search", "Unexpected search executable");
    const listed: GoModule[] = JSON.parse(`[${(await command([values.go!, "list", "-m", "-json", "all"], search)).replace(/\r?\n(?=\{)/g, ",")}]`);
    const modules = new Map(listed.map(module => [module.Path, module]));
    const result: Dependency[] = [];
    for (const dependency of build.Deps as GoModule[]) {
        const local = modules.get(dependency.Path);
        assert(local && local.Dir && local.Version && local.Sum, `Versioned module required: ${dependency.Path}`);
        assert(!dependency.Replace && !local.Replace, "Unpinned local Go replacement");
        assert.equal(local.Version, dependency.Version, "Source and binary module versions differ");
        assert.equal(local.Sum, dependency.Sum, "Source and binary module checksums differ");
        const escapedPath = local.Path.replace(/[A-Z]/g, letter => `!${letter.toLowerCase()}`);
        const escapedVersion = local.Version.replace(/[A-Z]/g, letter => `!${letter.toLowerCase()}`);
        const files = await notices(local.Dir, true);
        assert(files.length, `No license notice for ${local.Path}@${local.Version}`);
        result.push({
            name: local.Path,
            version: local.Version,
            integrity: local.Sum,
            source: `https://proxy.golang.org/${escapedPath}/@v/${escapedVersion}.zip`,
            notices: files,
        });
    }
    return result.sort((left, right) => left.name.localeCompare(right.name, "en"));
}

const [javascript, go, component] = await Promise.all([
    javascriptDependencies(),
    goDependencies(),
    notices(search, false),
]);
assert(component.length, "Search engine license/attribution is missing");
const manifest = {
    schemaVersion: 1,
    contentLockSha256: createHash("sha256").update(await readFile(join(content, "bun.lock"))).digest("hex"),
    components: [{ name: "qbutt-search", notices: component }],
    javascript,
    go,
};
const sections = [
    "qbutt content/search dependency notices",
    "License texts below come from the exact installed production dependency graph and built Go modules. Explicit supplemental notices identify archive omissions. Source archive URLs identify the recorded versions.",
];
for (const notice of component) {
    sections.push(`\n=== qbutt-search / ${notice.file} ===\n${notice.text}`);
}
for (const dependency of [...javascript, ...go]) {
    sections.push(`\n=== ${dependency.name}@${dependency.version} ===\nSource: ${dependency.source}`);
    for (const notice of dependency.notices) {
        sections.push(`\n--- ${notice.file} ---\n${notice.text}`);
    }
}

await mkdir(output, { recursive: true });
await Promise.all([
    writeFile(join(output, "qbutt-content-notices.json"), `${JSON.stringify(manifest, null, 2)}\n`),
    writeFile(join(output, "qbutt-content-notices.txt"), `${sections.join("\n\n")}\n`),
]);
console.log(`Collected ${javascript.length} JavaScript packages and ${go.length} Go modules.`);
