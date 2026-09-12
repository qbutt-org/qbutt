import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createConnection } from "node:net";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, waitFor } from "../lab";
import { allowLabNetwork } from "../windows-firewall";

const original = process.env.QBUTT_LAB_EXE;
assert(original, "Set QBUTT_LAB_EXE to a deployed qbutt bundle");
const bundle = await mkdtemp(join(tmpdir(), "qbutt-auth-bundle-"));
await cp(dirname(original), bundle, { recursive: true, filter: path => {
    if (["profile", ".git"].includes(basename(path)))
        return false;
    return !extname(path) || [".exe", ".dll", ".qm"].includes(extname(path).toLowerCase());
} });
const childPath = join(bundle, "qbutt-net.exe");
await allowLabNetwork([process.execPath]);
const compiled = Bun.spawn([process.execPath, "build", "--compile", join(import.meta.dir, "fake-child.ts"), "--outfile", childPath],
    { stdout: "pipe", stderr: "pipe", timeout: 60000, windowsHide: true });
const [exitCode, stdout, stderr] = await Promise.all([
    compiled.exited, new Response(compiled.stdout).text(), new Response(compiled.stderr).text(),
]);
assert(exitCode === 0, `Fake child compilation failed: ${stdout}\n${stderr}`);
process.env.QBUTT_LAB_EXE = join(bundle, basename(original));
const childSha256 = sha256(await readFile(childPath));

interface FaultEvidence {
    hello: number; opened: number; closed: number; offeredMethods: number[][];
    rejectedAuthentication: number; bytesAfterRejection: number; ports: number[];
    resolved: number; lookupHostMatched: boolean; dns: Record<string, string>;
}

interface PathStatus {
    busy: boolean; open: boolean; pinned: boolean; processId: number;
    dns: Record<string, string>;
    paths: { pathId: string; generation: number; open: boolean; dns: Record<string, string>;
        capabilities: Record<string, string> }[];
    resolution?: { requestId: number; pathId: string; generation: number; family: string;
        state: string; addresses?: string[]; errorCode?: string };
}

const lookupHost = "fixture-sensitive-host.invalid";
function assertRedacted(status: PathStatus) {
    const publicStatus = JSON.stringify(status);
    for (const sensitive of [lookupHost, "fixture-password", "fixture-user", "fixture-sensitive-code", "fixture-sensitive-value"])
        assert(!publicStatus.includes(sensitive), "DNS status leaked child text, a hostname or credentials");
    if (status.resolution)
        assert(!Object.hasOwn(status.resolution, "host"), "DNS resolution echoed a hostname");
}

async function assertListenerClosed(port: number) {
    const socket = createConnection({ host: "127.0.0.1", port });
    try {
        await new Promise<void>((resolve, reject) => {
            socket.once("connect", () => reject(new Error("Stopped child still accepts payload connections")));
            socket.once("error", error => (error as NodeJS.ErrnoException).code === "ECONNREFUSED" ? resolve() : reject(error));
            socket.setTimeout(2000, () => reject(new Error("Stopped child listener check timed out")));
        });
    }
    finally { socket.destroy(); }
}

const failures: unknown[] = [];
try {
    for (const mode of ["no-auth", "wrong-credentials", "incompatible", "dns-success", "dns-request-error",
        "dns-malformed", "dns-nonnumeric", "dns-wrong-family", "dns-too-many", "dns-timeout", "dns-crash"] as const) {
        const dnsMode = mode.startsWith("dns-");
        const lab = await createLab(`path-${mode}`);
        let failure: unknown;
        const evidencePath = join(lab.root, "child-evidence.json");
        const configPath = join(lab.root, "child-fixture.json");
        const dns = { server: "127.0.0.1:53", bootstrapServer: "127.0.0.1:54", family: "ipv4" };
        const readStatus = async () => {
            const status = await lab.json<PathStatus>("qbuttPaths/status");
            assertRedacted(status);
            return status;
        };
        await writeFile(configPath, JSON.stringify({ mode, evidencePath, dns, lookupHost }));
        process.env.QBUTT_LAB_CHILD_FIXTURE = configPath;
        try {
            await lab.start();
            for (const action of ["dns", "resolve"]) {
                const unauthenticated = await fetch(`${lab.origin}/api/v2/qbuttPaths/${action}`, {
                    method: "POST", headers: { Origin: lab.origin, Referer: `${lab.origin}/` },
                    body: new URLSearchParams(action === "dns" ? dns
                        : { pathId: "3", generation: "1", host: lookupHost, family: "ipv4" }),
                    signal: AbortSignal.timeout(5000),
                });
                assert.equal(unauthenticated.status, 403, `${action} allowed an unauthenticated request`);
                await unauthenticated.text();
                await assert.rejects(lab.request(`qbuttPaths/${action}`), /HTTP 405/);
            }
            await lab.request("qbuttPaths/dns", dns);
            await assert.rejects(lab.request("qbuttPaths/dns", { ...dns, server: "resolver.invalid:53" }), /HTTP 400/);
            assert.deepEqual((await readStatus()).dns, dns, "Invalid DNS policy changed saved settings");
            await lab.request("qbuttPaths/open", { configPath, proxyName: "fault-fixture",
                edgeId: "fault-edge", interfaceName: "Loopback Pseudo-Interface 1" });
            const status = await waitFor("fault child response", readStatus, status => !status.busy);
            if (mode === "incompatible") {
                assert(!status.open && status.processId === 0, "Incompatible child remained active");
            }
            else if (dnsMode) {
                assert(status.open && status.processId > 0 && status.pinned, "DNS fixture path was not opened");
                const path = status.paths.find(path => path.open)!;
                assert(path, "Opened path identity is missing");
                assert.deepEqual(path.dns, dns, "Opened path lost its DNS policy");
                assert.deepEqual(path.capabilities, { tcp: "supported", udp: "source-unsupported", dns: "path-tcp",
                    publicTcp: "unknown", publicUdp: "unknown", measurement: "not-probed" });
                const laterDns = { server: "[::1]:5353", bootstrapServer: "127.0.0.1:55", family: "dual" };
                await lab.request("qbuttPaths/dns", laterDns);
                const configured = await readStatus();
                assert.deepEqual(configured.dns, laterDns);
                assert.deepEqual(configured.paths.find(candidate => candidate.pathId === path.pathId)!.dns, dns,
                    "Changing defaults mutated an already opened path");
                const request = { pathId: path.pathId, generation: String(path.generation), host: lookupHost, family: "ipv4" };
                await assert.rejects(lab.request("qbuttPaths/resolve", { ...request, generation: String(path.generation + 1) }), /HTTP 400/);
                const pending = await (await lab.request("qbuttPaths/resolve", request)).json() as PathStatus;
                assertRedacted(pending);
                assert(pending.resolution && pending.resolution.requestId > 0, "Resolution request identity is missing");
                if (mode === "dns-timeout") {
                    assert.equal(pending.resolution.state, "pending");
                    await assert.rejects(lab.request("qbuttPaths/resolve", request), /HTTP 409/);
                }
                let resolved = await waitFor("bounded DNS result", readStatus,
                    state => !state.busy && state.resolution?.state !== "pending", 15000);
                assert(resolved.resolution, "Completed resolution is missing");
                assert.equal(resolved.resolution.requestId, pending.resolution.requestId);
                assert.equal(resolved.resolution.pathId, path.pathId);
                assert.equal(resolved.resolution.generation, path.generation);
                assert.equal(resolved.resolution.family, "ipv4");
                if (mode === "dns-success") {
                    assert.equal(resolved.resolution.state, "complete");
                    assert.deepEqual(resolved.resolution.addresses, ["192.0.2.41", "192.0.2.42"]);
                    assert(resolved.open && resolved.processId === status.processId, "Successful lookup closed the path");
                }
                else if (mode === "dns-request-error") {
                    assert.equal(resolved.resolution.state, "failed");
                    assert.equal(resolved.resolution.errorCode, "path_dns_failed");
                    assert.deepEqual(resolved.resolution.addresses, []);
                    assert(resolved.open && resolved.processId === status.processId, "Request error killed a usable path");
                    const retry = await (await lab.request("qbuttPaths/resolve", request)).json() as PathStatus;
                    assertRedacted(retry);
                    assert(retry.resolution && retry.resolution.requestId > pending.resolution.requestId);
                    resolved = await waitFor("lookup retry on retained path", readStatus,
                        state => !state.busy && state.resolution?.state !== "pending");
                    assert(resolved.resolution?.state === "complete");
                    assert.deepEqual(resolved.resolution.addresses, ["192.0.2.41", "192.0.2.42"]);
                    assert(resolved.resolution.requestId === retry.resolution.requestId && resolved.processId === status.processId);
                }
                else {
                    assert.equal(resolved.resolution.state, "failed");
                    assert.equal(resolved.resolution.errorCode, "transport_failed");
                    assert.deepEqual(resolved.resolution.addresses, []);
                    assert(!resolved.open && resolved.pinned && resolved.processId === 0 && resolved.paths.every(path => !path.open),
                        "Malformed, stalled or crashed DNS transport did not block pinned paths");
                }
            }
            else {
                assert(status.open, "Compatible fault child was not opened");
                const initial = JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence;
                assert(initial.closed === 0 && initial.offeredMethods.length === 0,
                    "Managed mode opened a session-wide SOCKS fallback before peer traffic");
                const hash = await lab.add("v1-public", join(lab.root, "download"));
                await lab.request("torrents/start", { hashes: hash });
                await lab.request("torrents/addPeers", { hashes: hash, peers: "127.0.0.2:45678" });
                const child = await waitFor("managed peer SOCKS rejection", async () =>
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
                    "A managed peer SOCKS socket offered unauthenticated fallback");
                assert(observed.bytesAfterRejection === 0, "A socket continued after SOCKS rejection");
                assert.deepEqual(observed.dns, dns);
                if (dnsMode) {
                    assert.equal(observed.resolved, mode === "dns-request-error" ? 2 : 1,
                        "Wrong-generation or duplicate requests crossed the control boundary");
                    assert(observed.lookupHostMatched, "Child did not receive the requested hostname");
                }
            }
            for (const port of observed.ports)
                await assertListenerClosed(port);
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
