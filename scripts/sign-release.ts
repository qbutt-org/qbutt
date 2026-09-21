import assert from "node:assert/strict";
import { createHash, createPrivateKey, createPublicKey, sign, verify } from "node:crypto";
import { createReadStream } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const [artifactDirectory, keyFile] = process.argv.slice(2);
assert(artifactDirectory && keyFile, "Usage: bun scripts/sign-release.ts <artifact-directory> <private-key.pem>");
const root = resolve(artifactDirectory);
const key = createPrivateKey(await readFile(keyFile));
assert.equal(key.asymmetricKeyType, "ed25519", "The release key must be Ed25519");
const publicKey = createPublicKey(key);
const header = await readFile(new URL("../src/base/releasepublickey.h", import.meta.url), "utf8");
const expected = header.match(/#define QBUTT_RELEASE_PUBLIC_KEY_HEX "([0-9a-f]{64})"/)?.[1];
const actual = Buffer.from(publicKey.export({ format: "jwk" }).x!, "base64url").toString("hex");
assert(expected && actual === expected, "The signing key does not match the application's release trust anchor");

const checksums = await readFile(join(root, "SHA256SUMS.txt"));
assert(checksums.length > 0 && checksums.length <= 65536, "Invalid checksum file size");
const lines = checksums.toString("ascii").trim().split(/\r?\n/);
assert.equal(lines.length, 2, "Expected the portable archive and build manifest");
const manifest = JSON.parse(await readFile(join(root, "portable", "build-manifest.json"), "utf8"));
assert.equal(manifest.product, "qbutt");
assert.equal(manifest.sourceWorktreeChanged, false, "Release signing requires a clean source build");
assert.match(manifest.version, /^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/);
const archiveName = `qbutt-${manifest.version}-windows-x64.zip`;
for (const [index, name] of [archiveName, "build-manifest.json"].entries()) {
    const hash = createHash("sha256");
    for await (const chunk of createReadStream(index === 0 ? join(root, name) : join(root, "portable", name)))
        hash.update(chunk);
    assert.equal(lines[index], `${hash.digest("hex")}  ${name}`, "An artifact changed after its checksums were written");
}
const signature = sign(null, checksums, key);
assert(signature.length === 64 && verify(null, checksums, publicKey, signature), "Release signature verification failed");
await writeFile(join(root, "SHA256SUMS.txt.sig"), signature);
console.log(`Signed ${archiveName}; publish SHA256SUMS.txt and SHA256SUMS.txt.sig with the bundle.`);
