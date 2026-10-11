import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { cp, mkdir, readFile } from "node:fs/promises";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";

interface CargoPackage {
    id: string;
    name: string;
    version: string;
    source: string | null;
    license: string | null;
    license_file: string | null;
    repository: string | null;
    authors: string[];
    manifest_path: string;
}

interface CargoMetadata {
    packages: CargoPackage[];
    workspace_members: string[];
    resolve: { nodes: { id: string }[] };
}

interface CargoLock {
    package: { name: string; version: string; source?: string; checksum?: string }[];
}

const { values } = parseArgs({
    options: {
        bundle: { type: "string" },
        cargo: { type: "string", default: "cargo" },
        rustc: { type: "string", default: "rustc" },
    },
});
assert(values.bundle, "Usage: bun scripts/collect-rust-notices.ts --bundle <bundle> [--cargo <cargo.exe>] [--rustc <rustc.exe>]");

const repository = resolve(import.meta.dir, "..");
const output = resolve(values.bundle, "licenses/rust");
const outputRelative = relative(repository, output);
assert(outputRelative === ".." || outputRelative.startsWith(`..${sep}`) || isAbsolute(outputRelative),
    "Keep generated notices outside source");
const target = "x86_64-pc-windows-msvc";

// These published crates omit the repository's license files. Keep the exact
// source commit and file checksums until a replacement crate includes them.
const kdabLicenses = [
    {
        path: "LICENSES/MIT.txt",
        sha256: "b85dcd3e453d05982552c52b5fc9e0bdd6d23c6f8e844b984a88af32570b0cc0",
    },
    {
        path: "LICENSES/Apache-2.0.txt",
        sha256: "074e6e32c86a4c0ef8b3ed25b721ca23aca83df277cd88106ef7177c354615ff",
    },
];
const repositoryLicenses = [
    {
        crates: ["clang-format@0.3.0"],
        repository: "KDAB/clang-format-rs",
        commit: "82e49611760ce781e3753dde57975ba576b286c3",
        files: kdabLicenses,
    },
    {
        crates: [
            "cxx-qt@0.10.0",
            "cxx-qt-build@0.10.0",
            "cxx-qt-gen@0.10.0",
            "cxx-qt-lib@0.10.0",
            "cxx-qt-macro@0.10.0",
            "qt-build-utils@0.10.0",
        ],
        repository: "KDAB/cxx-qt",
        commit: "61562797ba25558a7c7dba8017deca038f3ee9e4",
        files: kdabLicenses,
    },
    {
        crates: ["codespan-reporting@0.11.1"],
        repository: "brendanzab/codespan",
        commit: "fd389a13f5bb6d625b71e2e4694b26e127f393f9",
        files: [{
            path: "LICENSE",
            sha256: "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4",
        }],
    },
];

async function command(args: string[]): Promise<Buffer> {
    const child = Bun.spawn(args, { cwd: repository, stdout: "pipe", stderr: "pipe", windowsHide: true });
    const timeout = setTimeout(() => child.kill(), 60000);
    try {
        const [stdout, stderr, code] = await Promise.all([
            new Response(child.stdout).arrayBuffer(), new Response(child.stderr).text(), child.exited,
        ]);
        assert.equal(code, 0, `${basename(args[0])} ${args[1]} failed: ${stderr.slice(0, 4096)}`);
        return Buffer.from(stdout);
    }
    finally {
        clearTimeout(timeout);
    }
}

function sha256(bytes: Uint8Array): string {
    return createHash("sha256").update(bytes).digest("hex");
}

const toolchainBytes = await readFile(join(repository, "rust-toolchain.toml"));
const toolchain = Bun.TOML.parse(toolchainBytes.toString("utf8")) as { toolchain: { channel: string } };
assert(/^\d+\.\d+\.\d+$/.test(toolchain.toolchain.channel), "Rust toolchain must name an exact release");
const compiler = (await command([values.rustc!, "--version", "--verbose"])).toString("utf8").trim();
assert(compiler.split(/\r?\n/).includes(`release: ${toolchain.toolchain.channel}`), "Rust compiler differs from rust-toolchain.toml");
assert(compiler.split(/\r?\n/).includes(`host: ${target}`), "Expected the Windows x64 MSVC Rust toolchain");
const cargoLockBytes = await readFile(join(repository, "Cargo.lock"));
const cargoLock = Bun.TOML.parse(cargoLockBytes.toString("utf8")) as CargoLock;
const metadata = JSON.parse((await command([
    values.cargo!, "metadata", "--locked", "--offline", "--format-version", "1", "--filter-platform", target,
])).toString("utf8")) as CargoMetadata;
assert(metadata.resolve, "Cargo dependency graph is missing");
const resolved = new Set(metadata.resolve.nodes.map(node => node.id));
const packages = metadata.packages.filter(item => resolved.has(item.id) && !metadata.workspace_members.includes(item.id))
    .sort((left, right) => `${left.name}@${left.version}`.localeCompare(`${right.name}@${right.version}`, "en"));

const notices = [];
const fetchedLicenses = new Map<string, Buffer>();
for (const item of packages) {
    assert.equal(item.source, "registry+https://github.com/rust-lang/crates.io-index",
        `Expected a checksummed crates.io dependency: ${item.name}@${item.version}`);
    const checksum = cargoLock.package.find(entry => entry.name === item.name
        && entry.version === item.version && entry.source === item.source)?.checksum;
    assert(checksum, `Missing Cargo.lock checksum: ${item.name}@${item.version}`);
    const root = dirname(item.manifest_path);
    const archive = resolve(root, "../../..", "cache", basename(dirname(root)), `${item.name}-${item.version}.crate`);
    assert.equal(sha256(await readFile(archive)), checksum, `Cargo source archive differs from Cargo.lock: ${item.name}`);
    const prefix = `${item.name}-${item.version}/`;
    const entries = (await command(["tar.exe", "-tf", archive])).toString("utf8").trim().split(/\r?\n/);
    const files = entries.filter(path => /^(?:licen[sc]e|copying|notice|patents|copyright)(?:[._-].*)?$/i.test(basename(path)));
    if (item.license_file) {
        files.push(prefix + relative(root, resolve(root, item.license_file)).replaceAll("\\", "/"));
    }
    const licenseEntries = [];
    for (const file of [...new Set(files)].sort()) {
        assert(file.startsWith(prefix), `License outside crate source: ${item.name}`);
        const path = file.slice(prefix.length);
        assert(!path.split("/").includes("..") && !isAbsolute(path), `License outside crate source: ${item.name}`);
        const bytes = await command(["tar.exe", "-xOf", archive, file]);
        assert(bytes.length <= 1024 * 1024, `License exceeds 1 MiB: ${item.name}/${path}`);
        assert(!bytes.includes(0), `License is not plain text: ${item.name}/${path}`);
        const hash = sha256(bytes);
        const destination = join(output, `${item.name}-${item.version}`, path);
        await mkdir(dirname(destination), { recursive: true });
        await Bun.write(destination, bytes);
        licenseEntries.push({ file: `${item.name}-${item.version}/${path}`, sha256: hash });
    }
    if (!files.length) {
        const source = repositoryLicenses.find(entry => entry.crates.includes(`${item.name}@${item.version}`));
        assert(source, `No upstream license text: ${item.name}@${item.version}`);
        const vcs = JSON.parse((await command(["tar.exe", "-xOf", archive, prefix + ".cargo_vcs_info.json"])).toString("utf8"));
        assert.equal(vcs.git.sha1, source.commit, `License source commit differs from crate: ${item.name}`);
        for (const file of source.files) {
            const url = `https://raw.githubusercontent.com/${source.repository}/${source.commit}/${file.path}`;
            let bytes = fetchedLicenses.get(url);
            if (!bytes) {
                const response = await fetch(url, { signal: AbortSignal.timeout(30000) });
                assert(response.ok, `Cannot download upstream license: ${url} (${response.status})`);
                bytes = Buffer.from(await response.arrayBuffer());
                fetchedLicenses.set(url, bytes);
            }
            assert.equal(sha256(bytes), file.sha256, `Upstream license checksum differs: ${url}`);
            const destination = join(output, prefix, file.path);
            await mkdir(dirname(destination), { recursive: true });
            await Bun.write(destination, bytes);
            licenseEntries.push({ file: prefix + file.path, sha256: file.sha256, source: url });
        }
        const sourceText = (await command(["tar.exe", "-xOf", archive])).toString("utf8");
        const copyrights = [...new Set(sourceText.match(/SPDX-FileCopyrightText: [^\r\n]+/g))].sort();
        if (copyrights.length) {
            const text = Buffer.from(copyrights.join("\n") + "\n");
            await Bun.write(join(output, prefix, "COPYRIGHT.txt"), text);
            licenseEntries.push({ file: prefix + "COPYRIGHT.txt", sha256: sha256(text) });
        }
    }
    notices.push({
        name: item.name,
        version: item.version,
        license: item.license,
        repository: item.repository,
        authors: item.authors,
        source: `https://static.crates.io/crates/${item.name}/${item.name}-${item.version}.crate`,
        sourceSha256: checksum,
        licenseFiles: licenseEntries,
    });
}

const rustSysroot = (await command([values.rustc!, "--print", "sysroot"])).toString("utf8").trim();
const rustDocumentation = join(rustSysroot, "share/doc/rust");
await mkdir(join(output, "standard-library"), { recursive: true });
await cp(join(rustDocumentation, "COPYRIGHT-library.html"), join(output, "standard-library/COPYRIGHT-library.html"));
await cp(join(rustDocumentation, "licenses"), join(output, "standard-library/licenses"), { recursive: true });
await Bun.write(join(output, "dependencies.json"), JSON.stringify({
    schemaVersion: 1,
    target,
    compiler,
    cargoLockSha256: sha256(cargoLockBytes),
    toolchainFileSha256: sha256(toolchainBytes),
    scope: "Locked Windows workspace dependency graph, including build and development dependencies.",
    packages: notices,
}, null, 2) + "\n");
console.log(`Collected ${notices.length} Rust crate notices and the pinned standard-library notices.`);
