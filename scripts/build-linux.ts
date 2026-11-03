import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { chmod, cp, lstat, mkdir, readFile, readdir, realpath, rename, rm } from "node:fs/promises";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";

import lock from "../upstream-lock.json";
import { verifiedArtifact, type ArtifactPin } from "./verified-artifact";

const { values } = parseArgs({
    options: {
        "qt-root": { type: "string" },
        "dependency-root": { type: "string" },
        "build-root": { type: "string" },
        "artifact-root": { type: "string" },
        "libtorrent-source": { type: "string" },
        "net-source": { type: "string" },
        jobs: { type: "string", default: "2" },
        "build-only": { type: "boolean", default: false },
        "package-only": { type: "boolean", default: false },
        "without-products": { type: "boolean", default: false },
        "qplay-bundle": { type: "string" },
        "qcatch-bundle": { type: "string" },
    },
});

assert(process.platform === "linux" && process.arch === "x64", "Run this entrypoint on Linux x86-64.");
assert(values["qt-root"] && values["dependency-root"] && values["build-root"] && values["artifact-root"],
    "Usage: bun scripts/build-linux.ts --qt-root <Qt gcc_64> --dependency-root <dependencies> --build-root <build> --artifact-root <artifacts> [--jobs 2] [--build-only | --package-only] [--without-products]");
assert(!(values["build-only"] && values["package-only"]), "Choose either --build-only or --package-only.");
const jobs = Number(values.jobs);
assert(Number.isSafeInteger(jobs) && jobs > 0 && jobs <= 64, "--jobs must be between 1 and 64.");

const repository = resolve(import.meta.dir, "..");
const qtRoot = await realpath(values["qt-root"]!);
const dependencies = resolve(values["dependency-root"]!);
const build = resolve(values["build-root"]!);
const artifacts = resolve(values["artifact-root"]!);
const portable = join(artifacts, "portable");
const libtorrent = resolve(values["libtorrent-source"] ?? join(dependencies, "qbutt-libtorrent"));
const net = resolve(values["net-source"] ?? join(dependencies, "qbutt-net"));
const boostPin = lock.windows.boost;
const libtorrentPin = lock.windows.libtorrent;
const pins = lock.linux;
const environment = {
    ...process.env,
    PATH: `${join(qtRoot, "bin")}:${process.env.PATH ?? ""}`,
    CARGO_BUILD_JOBS: String(jobs),
    CARGO_INCREMENTAL: "0",
    GOTOOLCHAIN: "local",
    CGO_ENABLED: "0",
    GOOS: "linux",
    GOARCH: "amd64",
    GOAMD64: "v1",
    GOMAXPROCS: String(jobs),
    LC_ALL: "C.UTF-8",
    LD_LIBRARY_PATH: "",
    QT_PLUGIN_PATH: "",
    QML_IMPORT_PATH: "",
    QML2_IMPORT_PATH: "",
};

function outsideSource(path: string): void {
    const part = relative(repository, path);
    assert(part === ".." || part.startsWith(`..${sep}`) || isAbsolute(part), "Generated outputs must be outside source.");
}
for (const path of [dependencies, build, artifacts]) outsideSource(path);
assert(build !== artifacts && !artifacts.startsWith(`${build}${sep}`), "Artifacts must be outside the build directory.");

async function command(arguments_: string[], cwd = repository, capture = false): Promise<string> {
    if (!capture) console.log(`> ${arguments_.join(" ")}`);
    const child = Bun.spawn(arguments_, {
        cwd,
        env: environment,
        stdout: capture ? "pipe" : "inherit",
        stderr: "inherit",
    });
    const output = capture ? await new Response(child.stdout).text() : "";
    const status = await child.exited;
    assert.equal(status, 0, `${basename(arguments_[0])} failed (${status}).`);
    return output.trimEnd();
}

async function sha256(path: string): Promise<string> {
    const hash = createHash("sha256");
    for await (const chunk of createReadStream(path)) hash.update(chunk);
    return hash.digest("hex");
}

async function buildInputs() {
    const [sourceRevision, sourceStatus, sourceDiff, untrackedPaths, compiler, cmake, ninja, go, rust] = await Promise.all([
        command(["git", "-C", repository, "rev-parse", "HEAD"], repository, true),
        command(["git", "-C", repository, "status", "--porcelain", "--untracked-files=all"], repository, true),
        command(["git", "-C", repository, "diff", "--binary", "HEAD"], repository, true),
        command(["git", "-C", repository, "ls-files", "--others", "--exclude-standard", "-z"], repository, true),
        command(["c++", "--version"], repository, true),
        command(["cmake", "--version"], repository, true),
        command(["ninja", "--version"], repository, true),
        command(["go", "version"], repository, true),
        command(["rustc", "--version", "--verbose"], repository, true),
    ]);
    const untracked = [];
    for (const path of untrackedPaths.split("\0").filter(Boolean)) {
        untracked.push(`${path}\0${await sha256(join(repository, path))}`);
    }
    const sourceFingerprint = createHash("sha256").update(sourceStatus).update("\0")
        .update(sourceDiff).update("\0").update(untracked.join("\0")).digest("hex");
    return {
        sourceRoot: await realpath(repository),
        dependencyRoot: dependencies,
        libtorrentSource: libtorrent,
        netSource: net,
        sourceRevision,
        sourceWorktreeChanged: Boolean(sourceStatus),
        sourceFingerprint,
        dependencyLockSha256: await sha256(join(repository, "upstream-lock.json")),
        cargoLockSha256: await sha256(join(repository, "Cargo.lock")),
        rustToolchainSha256: await sha256(join(repository, "rust-toolchain.toml")),
        qtRoot, compiler, cmake, ninja, go, rust, bun: Bun.version,
    };
}

async function dependencyInputs() {
    const [libtorrentRevision, libtorrentStatus, netRevision, netStatus] = await Promise.all([
        command(["git", "-C", libtorrent, "rev-parse", "HEAD"], repository, true),
        command(["git", "-C", libtorrent, "status", "--porcelain", "--untracked-files=all"], repository, true),
        command(["git", "-C", net, "rev-parse", "HEAD"], repository, true),
        command(["git", "-C", net, "status", "--porcelain", "--untracked-files=all"], repository, true),
    ]);
    return { libtorrentRevision, libtorrentStatus, netRevision, netStatus };
}

async function productionCacheHash(): Promise<string> {
    const path = join(build, "CMakeCache.txt");
    const cache = (await readFile(path, "utf8")).split("\n");
    for (const [key, expected] of Object.entries({
        CMAKE_HOME_DIRECTORY: repository,
        CMAKE_BUILD_TYPE: "Release",
        GUI: "ON", WEBUI: "ON", TESTING: "OFF", QBUTT_QT_ACCEPTANCE: "OFF",
        QBUTT_STAGING_FAULTS: "OFF", QBUTT_COMPLETION_FAULTS: "OFF",
    })) {
        const entry = cache.find(line => line.startsWith(`${key}:`));
        assert(entry && entry.slice(entry.indexOf("=") + 1) === expected,
            `Packaging requires ${key}=${expected} in the compiled CMake cache.`);
    }
    return sha256(path);
}

const binaryNames = ["qbutt", "qbutt-net", "qbutt-webview"] as const;
async function builtBinaryHashes(): Promise<Record<string, string>> {
    const hashes: Record<string, string> = {};
    for (const name of binaryNames) hashes[name] = await sha256(join(build, name));
    return hashes;
}

interface BuildReceipt {
    schemaVersion: number;
    inputs: Awaited<ReturnType<typeof buildInputs>>;
    dependencies: Awaited<ReturnType<typeof dependencyInputs>>;
    cmakeCacheSha256: string;
    binaries: Record<string, string>;
}

async function verifyBuildReceipt(receipt: BuildReceipt): Promise<void> {
    assert.equal(receipt.schemaVersion, 1, "Unknown Linux build receipt. Run a full build.");
    assert.deepEqual(await buildInputs(), receipt.inputs, "Source or toolchain inputs changed after compilation. Run a full build.");
    assert.deepEqual(await dependencyInputs(), receipt.dependencies, "Dependency sources changed after compilation. Run a full build.");
    assert.equal(await productionCacheHash(), receipt.cmakeCacheSha256,
        "The CMake configuration changed after compilation. Run a full build.");
    assert.deepEqual(await builtBinaryHashes(), receipt.binaries,
        "The compiled binaries changed after the build receipt was written. Run a full build.");
}

async function download(pin: { url: string; sha256: string; size?: number }, destination: string): Promise<void> {
    assert(/^[0-9a-f]{64}$/.test(pin.sha256) && new URL(pin.url).protocol === "https:", "Invalid dependency pin.");
    if (await Bun.file(destination).exists() && await sha256(destination) === pin.sha256) return;
    const response = await fetch(pin.url, { signal: AbortSignal.timeout(10 * 60_000) });
    assert(response.ok && response.body, `Download failed: ${pin.url} (HTTP ${response.status}).`);
    await mkdir(dirname(destination), { recursive: true });
    const temporary = `${destination}.partial`;
    try {
        await Bun.write(temporary, response);
        assert.equal(await sha256(temporary), pin.sha256, `Dependency checksum differs: ${basename(destination)}.`);
        if (pin.size !== undefined) assert.equal((await lstat(temporary)).size, pin.size);
        await cp(temporary, destination);
    }
    finally {
        await rm(temporary, { force: true });
    }
}

async function source(path: string, pin: { repository: string; commit: string }): Promise<void> {
    assert(/^[0-9a-f]{40}$/.test(pin.commit), "Source must pin an exact Git commit.");
    const exists = await lstat(path).catch(error => {
        if (error.code === "ENOENT") return null;
        throw error;
    });
    if (!exists) {
        await command(["git", "-C", dependencies, "clone", "--filter=blob:none", "--no-checkout", pin.repository, path]);
        await command(["git", "-C", path, "checkout", "--detach", pin.commit]);
    }
    assert.equal(await command(["git", "-C", path, "rev-parse", "HEAD"], repository, true), pin.commit,
        `Dependency revision differs: ${path}.`);
    assert.equal(await command(["git", "-C", path, "status", "--porcelain", "--untracked-files=all"], repository, true), "",
        `Dependency has source changes: ${path}.`);
    await command(["git", "-C", path, "submodule", "update", "--init", "--recursive"]);
}

await mkdir(dependencies, { recursive: true });
await mkdir(build, { recursive: true });
await mkdir(artifacts, { recursive: true });
assert.equal(Bun.version, pins.bun.version, "Bun differs from upstream-lock.json.");
const goVersion = await command(["go", "version"], repository, true);
assert(goVersion.startsWith(`go version go${pins.go.version} linux/amd64`), "Go differs from upstream-lock.json.");
const rustVersion = await command(["rustc", "--version", "--verbose"], repository, true);
const rustToolchain = Bun.TOML.parse(await readFile(join(repository, "rust-toolchain.toml"), "utf8")) as { toolchain: { channel: string } };
assert(rustVersion.split("\n").includes(`release: ${rustToolchain.toolchain.channel}`)
    && rustVersion.split("\n").includes("host: x86_64-unknown-linux-gnu"), "Rust differs from the pinned Linux toolchain.");
assert.equal(await command([join(qtRoot, "bin/qmake"), "-query", "QT_VERSION"], repository, true), pins.qt.version,
    "Qt differs from upstream-lock.json.");

const boost = join(dependencies, boostPin.directory);
const boostArchive = join(dependencies, basename(new URL(boostPin.url).pathname));
const libtorrentBuild = join(dependencies, "libtorrent-linux");
const receiptPath = join(build, "qbutt-build-receipt.json");
if (!values["package-only"]) {
    const inputs = await buildInputs();
    await rm(receiptPath, { force: true });
    await download(boostPin, boostArchive);
    if (!await Bun.file(join(boost, "boost/version.hpp")).exists()) {
        await command(["tar", "-xf", boostArchive, "-C", dependencies]);
    }
    await source(libtorrent, libtorrentPin);
    await source(net, lock.qbuttNet);
    const dependencySources = await dependencyInputs();
    await command(["cargo", "fetch", "--locked", "--target", "x86_64-unknown-linux-gnu"]);
    await command([
        join(qtRoot, "bin/qt-cmake"), "-S", libtorrent, "-B", join(libtorrentBuild, "build"), "-G", "Ninja",
        "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_CXX_COMPILER=c++", "-DCMAKE_CXX_STANDARD=20", "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
        `-DCMAKE_INSTALL_PREFIX=${join(libtorrentBuild, "install")}`, `-DBOOST_ROOT=${boost}`,
        "-DBoost_NO_SYSTEM_PATHS=ON", "-DBUILD_SHARED_LIBS=OFF", "-Ddeprecated-functions=OFF", "-Dstatic_runtime=OFF",
    ]);
    await command(["cmake", "--build", join(libtorrentBuild, "build"), "--parallel", String(jobs)]);
    await command(["cmake", "--install", join(libtorrentBuild, "build")]);
    await command(["go", "build", `-p=${jobs}`, "-mod=readonly", "-trimpath", "-o", join(build, "qbutt-net"), "./cmd/qbutt-net"], net);
    await command([
        join(qtRoot, "bin/qt-cmake"), "-S", repository, "-B", build, "-G", "Ninja",
        "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_CXX_COMPILER=c++", `-DCMAKE_PREFIX_PATH=${qtRoot};${join(libtorrentBuild, "install")}`,
        `-DBOOST_ROOT=${boost}`, "-DBoost_NO_SYSTEM_PATHS=ON", "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON",
        "-DCMAKE_INSTALL_RPATH=$ORIGIN/lib", "-DGUI=ON", "-DWEBUI=ON", "-DTESTING=OFF", "-DQBUTT_QT_ACCEPTANCE=OFF",
        "-DQBUTT_STAGING_FAULTS=OFF", "-DQBUTT_COMPLETION_FAULTS=OFF",
    ]);
    await command(["cmake", "--build", build, "--parallel", String(jobs)]);
    assert.deepEqual(await buildInputs(), inputs, "Source or toolchain inputs changed during compilation; no build receipt was written.");
    assert.deepEqual(await dependencyInputs(), dependencySources, "Dependency sources changed during compilation; no build receipt was written.");
    const receipt: BuildReceipt = {
        schemaVersion: 1, inputs, dependencies: dependencySources,
        cmakeCacheSha256: await productionCacheHash(),
        binaries: await builtBinaryHashes(),
    };
    await Bun.write(`${receiptPath}.partial`, JSON.stringify(receipt, null, 2) + "\n");
    await rename(`${receiptPath}.partial`, receiptPath);
}

if (values["build-only"]) {
    console.log(`Linux native build: ${build}`);
    process.exit(0);
}

assert(await Bun.file(receiptPath).exists(), "No successful Linux build receipt. Run the full build entrypoint first.");
const receipt = JSON.parse(await readFile(receiptPath, "utf8")) as BuildReceipt;
await verifyBuildReceipt(receipt);
const compiled = receipt.inputs;

const version = (await readFile(join(repository, "qbutt-version.txt"), "utf8")).trim();
assert(/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/.test(version), "Invalid qbutt version.");
const portableInfo = await lstat(portable).catch(error => {
    if (error.code === "ENOENT") return null;
    throw error;
});
if (portableInfo) {
    assert(portableInfo.isDirectory() && !portableInfo.isSymbolicLink()
        && await realpath(portable) === portable && await realpath(artifacts) === artifacts, "Unsafe portable output directory.");
    await rm(portable, { recursive: true });
}
await mkdir(portable);
for (const name of binaryNames) {
    await cp(join(build, name), join(portable, name));
    assert.equal(await sha256(join(portable, name)), receipt.binaries[name], `The compiled ${name} changed while staging.`);
    await chmod(join(portable, name), 0o755);
    await command(["strip", "--strip-unneeded", join(portable, name)]);
}
for (const name of ["install.sh", "uninstall.sh"]) {
    await cp(join(repository, "dist/linux", name), join(portable, name));
    await chmod(join(portable, name), 0o755);
}
await cp(join(repository, "dist/unix/menuicons/scalable/apps/qbittorrent.svg"), join(portable, "qbutt.svg"));

// The QML modules are statically linked into qbutt. Scan their imports directly;
// the executable is not a CMake QML module that qt_deploy_qml_imports can inspect.
const scanner = join(qtRoot, "libexec/qmlimportscanner");
const imports = JSON.parse(await command([
    scanner, "-rootPath", join(repository, "src/gui/quick/qml"), "-importPath", join(qtRoot, "qml"),
], repository, true)) as { name?: string; path?: string; relativePath?: string; plugin?: string }[];
const qmlPluginSources: string[] = [];
for (const module of imports) {
    if (!module.path) {
        assert.equal(module.name, "Qbutt.Content", `Unresolved QML import: ${module.name}.`);
        continue;
    }
    const part = relative(join(qtRoot, "qml"), module.path);
    assert(part !== ".." && !part.startsWith(`..${sep}`) && !isAbsolute(part), `QML import is outside the Qt SDK: ${module.name}.`);
    const output = join(portable, "qml", part);
    await cp(module.path, output, {
        recursive: true,
        dereference: true,
        filter: async path => {
            if (path.endsWith(".debug") || path.endsWith(".qmltypes") || path.includes(`${sep}.debug${sep}`)) return false;
            if (path !== module.path && (await lstat(path)).isDirectory()
                && await Bun.file(join(path, "qmldir")).exists()) return false;
            return true;
        },
    });
    if (module.plugin) {
        const plugin = join(output, `lib${module.plugin}.so`);
        assert(await Bun.file(plugin).exists(), `QML plugin is missing: ${module.name}.`);
        qmlPluginSources.push(join(module.path, `lib${module.plugin}.so`));
    }
}

const deployment = join(build, "linux-deployment");
await Bun.write(join(build, "qml-plugins.cmake"), `set(QBUTT_QML_PLUGINS\n${[...new Set(qmlPluginSources)].map(path => `    [==[${path}]==]`).join("\n")}\n)\n`);
await command([
    join(qtRoot, "bin/qt-cmake"), "-S", join(repository, "scripts/linux-deployment"), "-B", deployment,
    `-DQBUTT_BUILD_ROOT=${build}`, `-DQBUTT_BUNDLE_ROOT=${portable}`,
    `-DCMAKE_INSTALL_PREFIX=${portable}`, `-DCMAKE_PREFIX_PATH=${qtRoot}`,
]);
await command(["cmake", "--install", deployment]);
for (const name of [
    "icudtl.dat", "v8_context_snapshot.bin", "qtwebengine_resources.pak", "qtwebengine_resources_100p.pak",
    "qtwebengine_resources_200p.pak", "qtwebengine_devtools_resources.pak",
]) assert(await Bun.file(join(portable, "resources", name)).exists(), `Qt WebEngine resource is missing: ${name}.`);
assert(await Bun.file(join(portable, "libexec/QtWebEngineProcess")).exists(), "Qt WebEngine process is missing.");
await Bun.write(join(portable, "qt.conf"), "[Paths]\nPrefix=.\nLibraries=lib\nLibraryExecutables=libexec\nPlugins=plugins\nQmlImports=qml\nData=.\nTranslations=translations\n");
await Bun.write(join(portable, "libexec/qt.conf"), "[Paths]\nPrefix=..\nLibraries=lib\nLibraryExecutables=libexec\nPlugins=plugins\nData=.\nTranslations=translations\n");

const elfFiles: string[] = [];
const systemLibraries: { file: string; source: string; binaryPackage: string; binaryVersion: string; sourcePackage: string; sourceVersion: string }[] = [];
async function inspectDirectory(directory: string): Promise<void> {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) await inspectDirectory(path);
        else if (entry.isFile()) {
            const bytes = await Bun.file(path).slice(0, 4).bytes();
            if (Buffer.from(bytes).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) elfFiles.push(path);
        }
    }
}
await inspectDirectory(portable);
const systemPackages = new Set<string>();
for (const path of elfFiles) {
    const name = basename(path);
    if (await Bun.file(join(qtRoot, "lib", name)).exists()) continue;
    const original = join("/usr/lib/x86_64-linux-gnu", name);
    if (!await Bun.file(original).exists()) continue;
    const owner = (await command(["dpkg-query", "-S", `*/${name}`], repository, true)).split("\n")[0].split(": ")[0];
    const [binaryPackage, binaryVersion, sourcePackage, sourceVersion] = (await command([
        "dpkg-query", "-W", "-f=${binary:Package}\t${Version}\t${source:Package}\t${source:Version}", owner,
    ], repository, true)).split("\t");
    systemPackages.add(binaryPackage.split(":")[0]);
    systemLibraries.push({ file: relative(portable, path), source: await realpath(original), binaryPackage, binaryVersion, sourcePackage, sourceVersion });
}
const dynamicFiles: string[] = [];
for (const path of elfFiles) {
    const dynamic = await command(["readelf", "-d", path], portable, true);
    if (dynamic.includes("There is no dynamic section")) continue;
    dynamicFiles.push(path);
    const libraries = relative(dirname(path), join(portable, "lib"));
    await command(["patchelf", "--set-rpath", libraries ? `$ORIGIN/${libraries}` : "$ORIGIN", path]);
}
for (const path of dynamicFiles) {
    const linked = await command(["ldd", path], portable, true);
    const external = linked.split("\n").filter(line => line.includes("not found") || line.includes(qtRoot) || line.includes(build));
    assert.equal(external.length, 0,
        `Bundle dependency is unresolved or points to the SDK: ${relative(portable, path)}.\n${external.join("\n")}`);
}

const licenses = join(portable, "licenses");
await mkdir(join(licenses, "qt"), { recursive: true });
await mkdir(join(licenses, "system"));
for (const name of systemPackages) {
    await cp(join("/usr/share/doc", name, "copyright"), join(licenses, "system", `${name}.txt`));
}
await Bun.write(join(licenses, "system/dependencies.json"), JSON.stringify(systemLibraries, null, 2) + "\n");
for (const file of ["COPYING", "COPYING.GPLv2", "COPYING.GPLv3", "AUTHORS"]) await cp(join(repository, file), join(portable, file));
for (const [path, name] of [
    [join(net, "LICENSE"), "qbutt-net.txt"],
    [join(libtorrent, "LICENSE"), "libtorrent.txt"],
    [join(boost, "LICENSE_1_0.txt"), "boost.txt"],
    [join(repository, "src/icons/LICENSE-Tabler"), "Tabler-Icons.txt"],
]) await cp(path, join(licenses, name));
await cp(join(await command(["go", "env", "GOROOT"], repository, true), "LICENSE"), join(licenses, "go.txt"));
for (const family of lock.fonts.families) await cp(join(repository, "resources/fonts", family.license.file), join(licenses, family.license.file));
for (const license of lock.windows.qt.licenseFiles) {
    assert(basename(license.file) === license.file, "Unsafe license filename.");
    const cached = join(dependencies, "qt-licenses", pins.qt.version, license.file);
    await download(license, cached);
    await cp(cached, join(licenses, "qt", license.file));
}
await mkdir(join(licenses, "qt-sbom"));
for (const file of await readdir(join(qtRoot, "sbom"))) {
    if (file.endsWith(".spdx.json")) await cp(join(qtRoot, "sbom", file), join(licenses, "qt-sbom", file));
}
await command([process.execPath, join(repository, "scripts/collect-rust-notices.ts"), "--bundle", portable]);
await command([process.execPath, "scripts/collect-notices.ts", join(portable, "qbutt-net"), licenses, "go"], net);

const productArtifacts: Record<string, ArtifactPin> = {};
const productCandidates: Record<string, unknown> = {};
if (!values["without-products"]) {
    const products = lock.products as Record<string, { commit: string; version: string; protocolVersion: number; artifacts: { linuxPortable?: ArtifactPin } }>;
    for (const [name, product] of Object.entries(products)) {
        const candidatePath = values[`${name}-bundle` as "qplay-bundle" | "qcatch-bundle"];
        const output = join(portable, name);
        if (candidatePath) {
            const candidate = await realpath(candidatePath);
            outsideSource(candidate);
            const manifest = JSON.parse(await readFile(join(candidate, "build-manifest.json"), "utf8"));
            assert(manifest.product === name && manifest.platform === "linux-x64"
                && /^[0-9a-f]{40}$/.test(manifest.sourceRevision)
                && typeof manifest.version === "string"
                && manifest.protocolVersion === product.protocolVersion,
            `The ${name} candidate identity is incomplete.`);
            const candidateVersion = manifest.version.split(".").map(Number);
            const minimumVersion = product.version.split(".").map(Number);
            assert(candidateVersion.length === 3 && candidateVersion.every((part: number) => Number.isSafeInteger(part) && part >= 0),
                `The ${name} candidate version is invalid.`);
            for (let index = 0; index < 3; ++index) {
                assert(candidateVersion[index] >= minimumVersion[index], `The ${name} candidate version is incompatible.`);
                if (candidateVersion[index] > minimumVersion[index]) break;
            }
            const inventory = name === "qplay" ? manifest
                : JSON.parse(await readFile(join(candidate, "bundle.json"), "utf8"));
            assert(inventory.files && Object.keys(inventory.files).length > 0, `The ${name} file inventory is missing.`);
            await mkdir(output);
            for (const [path, hash] of Object.entries(inventory.files)) {
                assert(path && !isAbsolute(path) && !path.split("/").some(part => ["", ".", ".."].includes(part))
                    && !path.includes("\\"), "Unsafe component inventory path.");
                const input = join(candidate, path);
                const info = await lstat(input);
                assert(info.isFile() && !info.isSymbolicLink() && await realpath(input) === input,
                    `The ${name} candidate has a non-regular file: ${path}.`);
                assert.equal(await sha256(input), hash, `The ${name} candidate file differs: ${path}.`);
                await mkdir(dirname(join(output, path)), { recursive: true });
                await cp(input, join(output, path));
            }
            await cp(join(candidate, "build-manifest.json"), join(output, "build-manifest.json"));
            if (name === "qplay") {
                assert.equal(await readFile(join(candidate, "qplay.portable"), "utf8"), "qplay portable profile: userdata/config and userdata/data\n");
                await cp(join(candidate, "qplay.portable"), join(output, "qplay.portable"));
            }
            else await cp(join(candidate, "bundle.json"), join(output, "bundle.json"));
            productCandidates[name] = {
                version: manifest.version,
                sourceRevision: manifest.sourceRevision,
                sourceWorktreeChanged: manifest.sourceWorktreeChanged,
                manifestSha256: await sha256(join(candidate, "build-manifest.json")),
            };
            continue;
        }
        const pin = product.artifacts.linuxPortable;
        assert(pin && /^[0-9a-f]{40}$/.test(product.commit), `The ${name} Linux source and release pins are incomplete.`);
        const archive = join(dependencies, "products", basename(new URL(pin.url).pathname));
        await verifiedArtifact(pin, archive, AbortSignal.timeout(20 * 60_000));
        const entries = (await command(["unzip", "-Z1", archive], repository, true)).split("\n").filter(Boolean);
        assert(entries.length && new Set(entries).size === entries.length
            && entries.every(entry => !entry.includes("\\") && !entry.startsWith("/")
            && !entry.split("/").some(part => part === "..")), `Unsafe ${name} archive path.`);
        const modes = (await command(["unzip", "-Z", "-l", archive], repository, true)).split("\n")
            .filter(line => /^[bcdlps-][rwxstST-]{9}\s/.test(line));
        assert(modes.length === entries.length && modes.every(line => line.startsWith("-") || line.startsWith("d")),
            `The ${name} archive contains a non-regular entry.`);
        await mkdir(output);
        await command(["unzip", "-q", archive, "-d", output]);
        assert(await Bun.file(join(output, name)).exists(), `The ${name} Linux portable executable is missing.`);
        const manifest = JSON.parse(await readFile(join(output, "build-manifest.json"), "utf8"));
        assert(manifest.product === name && manifest.platform === "linux-x64"
            && manifest.version === product.version && manifest.sourceRevision === product.commit
            && manifest.protocolVersion === product.protocolVersion && manifest.sourceWorktreeChanged === false,
        `The ${name} release manifest differs from its pinned clean build identity.`);
        productArtifacts[name] = pin;
    }
}

await Bun.write(join(portable, "THIRD-PARTY-NOTICES.txt"), [
    "qbutt is a fork of qBittorrent. See COPYING, COPYING.GPLv2, COPYING.GPLv3 and AUTHORS.",
    `Source and build instructions: https://github.com/qbutt-org/qbutt`,
    "The native Rust content library and Qt Quick bridge are statically linked. Exact source checksums and license files are in licenses/rust/dependencies.json.",
    `qbutt-net source: ${lock.qbuttNet.repository.replace(/\.git$/, "")}/tree/${lock.qbuttNet.commit}. Its GPLv3 license and dependency notices are in licenses/.`,
    `Qt ${pins.qt.version} and Qt WebEngine ${pins.qt.webEngineVersion} are dynamically linked. License texts and SBOMs are in licenses/qt/ and licenses/qt-sbom/. Compatible modified Qt libraries can replace the bundled libraries.`,
    `Qt ${pins.qt.version} base and module sources: ${lock.windows.qt.sourceUrl}\nSHA-256: ${lock.windows.qt.sourceSha256}`,
    `Qt WebEngine publisher source archive for vendor build ${pins.qt.webEngineSource.vendorBuild}: ${pins.qt.webEngineSource.url}\nSHA-256: ${pins.qt.webEngineSource.sha256}`,
    "The publisher associates that source archive with the binary build ID. Its archive revision differs from the binary SBOM's declared Git locator; no matching public Git revision has been verified. Chromium notices and vendor build instructions are included in the SBOMs.",
    `libtorrent source: ${libtorrentPin.repository.replace(/\.git$/, "")}/tree/${libtorrentPin.commit}`,
    `Boost source: ${boostPin.url}`,
    "Fonts are covered by their SIL Open Font License files in licenses/. Tabler Icons are under MIT; see licenses/Tabler-Icons.txt.",
    "qplay and qcatch are independent products with their own runtime, profiles and notices. Their release artifacts are recorded in upstream-lock.json.",
].join("\n\n") + "\n");
const binaryHashes: Record<string, string> = {};
for (const name of ["qbutt", "qbutt-webview", "qbutt-net", "libexec/QtWebEngineProcess"]) binaryHashes[name] = await sha256(join(portable, name));
await verifyBuildReceipt(receipt);
const manifest = {
    product: "qbutt", version, platform: "linux-x64", configuration: "Release",
    ...compiled, qt: pins.qt, cmakeCacheSha256: receipt.cmakeCacheSha256,
    products: productArtifacts, productCandidates, systemLibraries, binaries: binaryHashes,
};
await Bun.write(join(portable, "build-manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
await cp(join(repository, "upstream-lock.json"), join(portable, "upstream-lock.json"));
const inventory: string[] = [];
async function inventoryDirectory(directory: string): Promise<void> {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) await inventoryDirectory(path);
        else {
            const part = relative(portable, path);
            assert(!part.includes("\n") && !part.includes("\r") && !part.includes("\\"), "Unsafe bundle filename.");
            inventory.push(`${await sha256(path)}  ${part}`);
        }
    }
}
await inventoryDirectory(portable);
await Bun.write(join(portable, "bundle.sha256"), inventory.sort().join("\n") + "\n");
const archive = join(artifacts, `qbutt-${version}-linux-x64.zip`);
await rm(archive, { force: true });
await command(["zip", "-q", "-r", "-9", "-y", archive, "."], portable);
await command(["unzip", "-tq", archive]);
const installerArchive = join(artifacts, `qbutt-${version}-linux-x64.tar.gz`);
await rm(installerArchive, { force: true });
await command(["tar", "--create", "--gzip", "--file", installerArchive, "--directory", portable, "."]);
await command(["gzip", "--test", installerArchive]);
const archiveMetadata = {
    product: "qbutt", version, platform: "linux-x64",
    sourceRevision: compiled.sourceRevision,
    sourceWorktreeChanged: compiled.sourceWorktreeChanged,
    buildManifestSha256: await sha256(join(portable, "build-manifest.json")),
    portable: { file: basename(archive), size: (await lstat(archive)).size, sha256: await sha256(archive) },
    installer: { file: basename(installerArchive), size: (await lstat(installerArchive)).size, sha256: await sha256(installerArchive) },
};
await Bun.write(join(artifacts, `qbutt-${version}-linux-x64.artifacts.json`), JSON.stringify(archiveMetadata, null, 2) + "\n");
console.log(`Portable archive: ${archive}\nSHA-256: ${archiveMetadata.portable.sha256}`);
console.log(`Installer archive: ${installerArchive}\nSHA-256: ${archiveMetadata.installer.sha256}`);
if (values["without-products"]) console.log("Diagnostic bundle: qplay and qcatch were explicitly omitted.");
