import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, waitFor } from "../lab";

const original = process.env.QBUTT_LAB_EXE;
assert(original, "Set QBUTT_LAB_EXE to a deployed qbutt bundle");
const bundle = await mkdtemp(join(tmpdir(), "qbutt-auth-bundle-"));
await cp(dirname(original), bundle, { recursive: true, filter: path => {
    if (["profile", ".git"].includes(basename(path)))
        return false;
    return !extname(path) || [".exe", ".dll", ".qm"].includes(extname(path).toLowerCase());
} });
const childPath = join(bundle, "qbutt-net.exe");
const compiled = Bun.spawn([process.execPath, "build", "--compile", join(import.meta.dir, "fake-child.ts"), "--outfile", childPath],
    { stdout: "pipe", stderr: "pipe", timeout: 60000 });
const [exitCode, stdout, stderr] = await Promise.all([
    compiled.exited, new Response(compiled.stdout).text(), new Response(compiled.stderr).text(),
]);
assert(exitCode === 0, `Fake child compilation failed: ${stdout}\n${stderr}`);
process.env.QBUTT_LAB_EXE = join(bundle, basename(original));
const childSha256 = sha256(await readFile(childPath));

interface FaultEvidence {
    hello: number; opened: number; closed: number; offeredMethods: number[][];
    rejectedAuthentication: number; bytesAfterRejection: number;
}

const failures: unknown[] = [];
try {
    for (const mode of ["no-auth", "wrong-credentials", "incompatible"] as const) {
        const lab = await createLab(`path-${mode}`);
        let failure: unknown;
        const evidencePath = join(lab.root, "child-evidence.json");
        const configPath = join(lab.root, "child-fixture.json");
        await writeFile(configPath, JSON.stringify({ mode, evidencePath }));
        process.env.QBUTT_LAB_CHILD_FIXTURE = configPath;
        try {
            await lab.start();
            await lab.request("qbuttPaths/open", { configPath, proxyName: "fault-fixture",
                edgeId: "fault-edge", interfaceName: "Loopback Pseudo-Interface 1" });
            const status = await waitFor("fault child response", () => lab.json<{
                busy: boolean; open: boolean; processId: number;
            }>("qbuttPaths/status"), status => !status.busy);
            if (mode === "incompatible") {
                assert(!status.open && status.processId === 0, "Incompatible child remained active");
            }
            else {
                assert(status.open, "Compatible fault child was not opened");
                const initial = await waitFor("session SOCKS rejection before peer traffic", async () =>
                    JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence,
                evidence => evidence.closed > 0 && evidence.offeredMethods.length > 0);
                const hash = await lab.add("v1-public", join(lab.root, "download"));
                await lab.request("torrents/start", { hashes: hash });
                await lab.request("torrents/addPeers", { hashes: hash, peers: "127.0.0.2:45678" });
                const child = await waitFor("native SOCKS rejection", async () =>
                    JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence,
                evidence => evidence.closed > initial.closed && evidence.offeredMethods.length > initial.offeredMethods.length);
                if (mode === "wrong-credentials")
                    assert(child.rejectedAuthentication > initial.rejectedAuthentication, "Peer RFC1929 failure was not exercised");
                assert((await lab.info(hash)).completed === 0, "Rejected route transferred torrent payload");
            }
            await lab.shutdown();
            const observed = JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence;
            assert(observed.hello === 1, "Fault scenario did not use one version handshake");
            if (mode === "incompatible")
                assert(observed.opened === 0, "Open was sent after incompatible hello");
            else {
                assert(observed.offeredMethods.every(methods => methods.length === 1 && methods[0] === 2),
                    "A session or peer SOCKS socket offered unauthenticated fallback");
                assert(observed.bytesAfterRejection === 0, "A socket continued after SOCKS rejection");
            }
            await lab.checkpoint({ check: "native-route-auth-and-child-contract", mode,
                fakeChildSha256: childSha256, ...observed });
        }
        catch (error) {
            failure = error;
            try { await lab.shutdown(); }
            catch (shutdownError) { console.error(String(shutdownError)); }
        }
        await lab.finish(failure);
        if (failure)
            failures.push(failure);
    }
}
finally {
    process.env.QBUTT_LAB_EXE = original;
    delete process.env.QBUTT_LAB_CHILD_FIXTURE;
}
if (failures.length)
    throw new AggregateError(failures, "Native route authentication or child contract failed");
