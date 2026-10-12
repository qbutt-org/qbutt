import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFile, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";

// Only this acceptance run trusts its generated local TLS endpoint. The
// provider, parser, URL checks and shipping SourceHttp are compiled unchanged.
const [buildDirectory, executable, certificate, key, hostList, fixture] = process.argv.slice(2);
assert([buildDirectory, executable, certificate, key, fixture].every(value => value && isAbsolute(value)),
    "Expected absolute build, executable, certificate, key and fixture paths");
const hosts = hostList?.split(",") ?? [];
assert(hosts.length > 0 && hosts.every(host => /^[a-z0-9.-]+$/.test(host)), "Invalid fixture host list");

const repository = resolve(import.meta.dir, "..");
const source = join(repository, "src", "content", "src", "http.rs");
const original = await readFile(source);
const originalHash = createHash("sha256").update(original).digest("hex");
const status = Bun.spawnSync(["git", "status", "--porcelain", "--", source], {
    cwd: repository,
    stdout: "pipe",
    stderr: "pipe",
});
assert.equal(status.exitCode, 0, status.stderr.toString());
assert.equal(status.stdout.toString().trim(), "", "SourceHttp is already changed; refusing to overwrite it");

const backupDirectory = await mkdtemp(join(tmpdir(), "qbutt-native-http-fixture-"));
const backup = join(backupDirectory, "http.rs");
await copyFile(source, backup);

const certificatePath = certificate.replaceAll("\\", "/");
assert(!certificatePath.includes('"#') && !certificatePath.includes("\n"), "Invalid certificate path");
const originalText = original.toString("utf8");
const needle = /(\.dns_resolver\(Arc::new\(PublicResolver\)\)\r?\n)/g;
assert.equal([...originalText.matchAll(needle)].length, 1, "SourceHttp builder changed; fixture overlay needs review");
const lineEnding = originalText.includes("\r\n") ? "\r\n" : "\n";
const override = hosts.map(host =>
    `            .resolve("${host}", "127.0.0.1:443".parse().expect("fixture socket"))${lineEnding}`,
).join("");
const trust = [
    "            .add_root_certificate(",
    `                reqwest::Certificate::from_pem(include_bytes!(r#"${certificatePath}"#))`,
    "                    .expect(\"fixture certificate\"),",
    "            )",
].join(lineEnding) + lineEnding;
const patched = Buffer.from(originalText.replace(needle, `$1${override}${trust}`));

async function run(command: string[]) {
    const process = Bun.spawn(command, {
        cwd: repository,
        env: processEnv,
        stdout: "inherit",
        stderr: "inherit",
    });
    const code = await process.exited;
    assert.equal(code, 0, `${command[0]} exited with ${code}`);
}

const processEnv = { ...process.env };
let patchedSource = false;
let restored = false;
try {
    await writeFile(source, patched);
    patchedSource = true;
    await run(["cmake", "--build", buildDirectory, "--config", "RelWithDebInfo",
        "--target", "qbutt-quick-acceptance", "--parallel", "6"]);
    await run([process.execPath, fixture, executable, certificate, key]);
} finally {
    if (patchedSource) {
        const current = await readFile(source);
        assert(current.equals(patched), `SourceHttp changed during the fixture; original saved at ${backup}`);
        await copyFile(backup, source);
        restored = createHash("sha256").update(await readFile(source)).digest("hex") === originalHash;
        assert(restored, `SourceHttp restoration failed; original saved at ${backup}`);
        await run(["cmake", "--build", buildDirectory, "--config", "RelWithDebInfo",
            "--target", "qbutt-quick-acceptance", "--parallel", "6"]);
    }
    if (restored || !patchedSource)
        await rm(backupDirectory, { recursive: true, force: true });
}
