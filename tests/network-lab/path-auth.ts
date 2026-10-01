import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
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
    return basename(path) === basename(original)
        || basename(path) === "qt.conf"
        || !extname(path) || [".dll", ".qm"].includes(extname(path).toLowerCase());
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
    protocol: number; methods: string[];
    hello: number; opened: number; status: number; closed: number; closeRequests: number; offeredMethods: number[][];
    rejectedAuthentication: number; bytesAfterRejection: number; ports: number[];
    resolved: number; lookupHostMatched: boolean; dns: Record<string, string>;
    processStarts: number; rolloverEvents: number;
}

interface PathStatus {
    busy: boolean; open: boolean; pinned: boolean; processId: number;
    dns: Record<string, string>;
    nodes: { name: string; configuredServerId: string }[];
    paths: { pathId: string; generation: number; open: boolean; dns: Record<string, string>;
        configuredServerId: string; capabilities: Record<string, string>;
        gateway: { state: string; tcp: boolean; udp: boolean; publicEndpoint?: string };
        wire?: Record<string, number> }[];
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
    for (const mode of ["no-auth", "wrong-credentials", "incompatible", "legacy-v4", "legacy-v5", "legacy-v6", "legacy-v7", "hello-extra", "hello-wrong-upstream",
        "open-envelope-extra", "open-result-extra", "status-result-extra", "dns-success", "dns-request-error",
        "dns-malformed", "dns-nonnumeric", "dns-wrong-family", "dns-too-many", "dns-timeout", "dns-crash",
        "dns-result-extra", "dns-error-message-extra", "status-delay", "status-delay-extra", "status-decrease",
        "gateway-rollover", "gateway-terminal", "close-error"] as const) {
        const dnsMode = mode.startsWith("dns-");
        const gatewayRollover = mode === "gateway-rollover";
        const gatewayTerminal = mode === "gateway-terminal";
        const handshakeFailure = ["incompatible", "legacy-v4", "legacy-v5", "legacy-v6", "legacy-v7", "hello-extra", "hello-wrong-upstream"].includes(mode);
        const responseFailure = ["open-envelope-extra", "open-result-extra", "status-result-extra"].includes(mode);
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
            for (const action of ["dns", "gateway", "resolve"]) {
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
            const gatewaySettings = (configuredServerId: string, controlPort: number, port: number) => ({
                configuredServerId, controlAddress: `127.0.0.1:${controlPort}`, datagramAddress: "", serverName: "127.0.0.1",
                caPath: configPath, certificatePath: configPath, privateKeyPath: configPath,
                port: String(port), tcp: "true", udp: "false",
            });
            if (gatewayRollover || gatewayTerminal)
            {
                await lab.request("qbuttPaths/list", { configPath });
                const listed = await waitFor("gateway server identities", readStatus, state => !state.busy && state.nodes.length === 2);
                const firstId = listed.nodes.find(node => node.name === "fault-fixture")!.configuredServerId;
                const secondId = listed.nodes.find(node => node.name === "fault-fixture-2")!.configuredServerId;
                assert.match(firstId, /^[0-9a-f]{64}$/);
                assert.match(secondId, /^[0-9a-f]{64}$/);
                assert.notEqual(firstId, secondId, "Two fixture servers shared an edge identity");
                await assert.rejects(lab.request("qbuttPaths/gateway", gatewaySettings("", 1, 45000)), /HTTP 400/);
                await assert.rejects(lab.request("qbuttPaths/gateway", gatewaySettings("f".repeat(64), 1, 45000)), /HTTP 400/);
                await lab.request("qbuttPaths/gateway", gatewaySettings(firstId, 1, 45000));
                await lab.request("qbuttPaths/gateway", gatewaySettings(secondId, 2, 45001));
            }
            await lab.request("qbuttPaths/open", { configPath, proxyName: "fault-fixture",
                interfaceName: "Loopback Pseudo-Interface 1" });
            const status = await waitFor("fault child response", readStatus, status =>
                (handshakeFailure || responseFailure) ? (!status.open && status.processId === 0) : !status.busy, 15000);
            if (handshakeFailure || responseFailure) {
                assert(!status.open && status.processId === 0, "Incompatible child remained active");
            }
            else if (mode === "status-decrease") {
                assert(status.open && status.processId > 0, "Counter monotonicity fixture path was not opened");
                await waitFor("decreasing telemetry rejection", readStatus,
                    state => !state.open && state.processId === 0, 15000);
                const statusEvidence = JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence;
                assert(statusEvidence.status >= 2, "Decreasing counter snapshot was not exercised");
            }
            else if (gatewayRollover || gatewayTerminal) {
                assert(status.open && status.processId > 0 && status.paths.length === 1
                    && status.paths[0]!.gateway.state === "leased", "First gateway path was not leased");
                await lab.request("qbuttPaths/open", { configPath, proxyName: "fault-fixture-2",
                    interfaceName: "Loopback Pseudo-Interface 1" });
                const beforeRollover = await waitFor("two leased paths", readStatus, state => !state.busy
                    && state.paths.filter(path => path.open && path.gateway.state === "leased").length === 2
                    && (!gatewayTerminal || state.processId !== status.processId), 20000);
                const firstPath = beforeRollover.paths.find(path => path.configuredServerId
                    === beforeRollover.nodes.find(node => node.name === "fault-fixture")!.configuredServerId)!;
                const secondPath = beforeRollover.paths.find(path => path.configuredServerId
                    === beforeRollover.nodes.find(node => node.name === "fault-fixture-2")!.configuredServerId)!;
                assert.equal(firstPath.gateway.publicEndpoint, "8.8.8.8:45000");
                assert.equal(secondPath.gateway.publicEndpoint, "8.8.8.8:45001");
                const malformedPathId = `0${beforeRollover.paths.find(path => path.open)!.pathId}`;
                await assert.rejects(lab.request("qbuttPaths/stop", { pathId: malformedPathId }), /HTTP 400/);
                const afterRejectedStop = await readStatus();
                assert.deepEqual(afterRejectedStop.paths.map(path => [path.pathId, path.generation, path.open]),
                    beforeRollover.paths.map(path => [path.pathId, path.generation, path.open]),
                    "A non-canonical path identity changed active paths");
                const rolloverEvidence = JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence;
                if (gatewayTerminal) {
                    assert(rolloverEvidence.processStarts >= 3 && rolloverEvidence.rolloverEvents === 2,
                        "Gateway terminal event did not race with reconnecting paths");
                    await lab.request("qbuttPaths/stop", { pathId: firstPath.pathId });
                    const recovered = await waitFor("terminal event while another path stops", readStatus, state => !state.busy
                        && state.paths.some(path => path.pathId === firstPath.pathId && !path.open)
                        && state.paths.some(path => path.pathId === secondPath.pathId && path.open
                            && path.generation > secondPath.generation), 20000);
                    assert.equal(recovered.paths.filter(path => path.open).length, 1,
                        "Terminal recovery resurrected the explicitly stopped path");
                    const events = JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence;
                    assert(events.processStarts >= 4 && events.rolloverEvents === 3,
                        "Gateway terminal event did not race with an explicit stop");
                    await lab.checkpoint({ check: "multi-path-terminal-rollover-preserves-intent",
                        processStarts: events.processStarts, rolloverEvents: events.rolloverEvents,
                        stoppedPathId: firstPath.pathId, retainedPathId: secondPath.pathId });
                }
                else {
                    await lab.request("qbuttPaths/gateway", gatewaySettings(firstPath.configuredServerId, 3, 45002));
                    const recovered = await waitFor("selected gateway generation replacement", readStatus, state => !state.busy
                        && state.paths.some(path => path.pathId === firstPath.pathId && path.open
                            && path.generation > firstPath.generation && path.gateway.publicEndpoint === "8.8.8.8:45002"), 20000);
                    const retainedPath = recovered.paths.find(path => path.pathId === secondPath.pathId)!;
                    assert(retainedPath.open && retainedPath.generation === secondPath.generation
                        && retainedPath.gateway.publicEndpoint === "8.8.8.8:45001",
                    "Updating one gateway disturbed the other edge's lease");
                    await lab.request("qbuttPaths/stop", { pathId: firstPath.pathId });
                    const stopRecovered = await waitFor("selected gateway stop", readStatus, state => !state.busy
                        && state.paths.some(path => path.pathId === firstPath.pathId && !path.open));
                    assert(stopRecovered.paths.some(path => path.pathId === secondPath.pathId && path.open
                        && path.generation === secondPath.generation && path.gateway.publicEndpoint === "8.8.8.8:45001"),
                    "Stopping one gateway disturbed the other edge's lease");
                    assert.equal(rolloverEvidence.processStarts, 1, "Updating one gateway restarted the transport child");
                    await lab.checkpoint({ check: "per-edge-gateway-rollover-preserves-other-lease",
                        processStarts: rolloverEvidence.processStarts,
                        updatedPathId: firstPath.pathId, retainedPathId: retainedPath.pathId });
                }
            }
            else if ((mode === "status-delay") || (mode === "status-delay-extra") || (mode === "close-error")) {
                assert(status.open && status.processId > 0, "Close-contract fixture path was not opened");
                const path = status.paths.find(path => path.open)!;
                if ((mode === "status-delay") || (mode === "status-delay-extra")) {
                    await waitFor("delayed telemetry request", async () =>
                        JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence,
                        evidence => evidence.status > 0);
                }
                await lab.request("qbuttPaths/stop", { pathId: path.pathId });
                const stopped = await waitFor("path close outcome", readStatus, state => !state.busy && !state.open
                    && (((mode === "close-error") || (mode === "status-delay-extra"))
                        ? state.processId === 0 : state.processId === status.processId), 15000);
                assert(stopped.paths.every(path => !path.open), "Closed path remained active in application state");
                const closeEvidence = JSON.parse(await readFile(evidencePath, "utf8")) as FaultEvidence;
                assert.equal(closeEvidence.closeRequests, mode === "status-delay-extra" ? 0 : 1,
                    "Path close crossed the child contract unexpectedly");
            }
            else if (dnsMode) {
                assert(status.open && status.processId > 0 && status.pinned, "DNS fixture path was not opened");
                const path = status.paths.find(path => path.open)!;
                assert(path, "Opened path identity is missing");
                assert.deepEqual(path.dns, dns, "Opened path lost its DNS policy");
                assert.deepEqual(path.capabilities, { tcp: "supported", udp: "source-unsupported", dns: "path-tcp",
                    publicTcp: "unknown", publicUdp: "unknown", measurement: "not-probed" });
                assert.deepEqual(path.gateway, { state: "outgoing-only", tcp: false, udp: false },
                    "Path-only fixture unexpectedly acquired a public gateway lease");
                if (mode === "dns-success") {
                    const metered = await waitFor("strict transport counters", readStatus,
                        state => Boolean(state.paths.find(candidate => candidate.pathId === path.pathId)?.wire));
                    assert.deepEqual(metered.paths.find(candidate => candidate.pathId === path.pathId)!.wire, {
                        relayDownloadBytes: 11, relayUploadBytes: 12,
                        carrierDownloadBytes: 13, carrierUploadBytes: 14,
                        carrierDownloadPackets: 15, carrierUploadPackets: 16,
                        relayDownloadCopies: 17,
                    });
                }
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
            assert.equal(observed.protocol, 8, "Fixture evidence did not record protocol v8");
            assert(observed.methods.every(method => ["hello", "list", "open", "resolve", "status", "close",
                "gateway.open", "gateway.renew", "gateway.close"].includes(method)),
                "Path-only fixture received a gateway or unrelated control request");
            assert(observed.hello === observed.processStarts,
                "Fault scenario handshake count did not match its child processes");
            if (handshakeFailure)
                assert(observed.opened === 0, "Open was sent after incompatible hello");
            else if (responseFailure) {
                assert.equal(observed.opened, 1, "Response-contract scenario did not reach open");
                if (mode === "status-result-extra")
                    assert(observed.status > 0, "Invalid status shape was not exercised");
            }
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
        else {
            assert(lab.exitCode !== null, "Stop the application before cleaning its fixture");
            const root = await realpath(lab.root);
            for (const name of ["fixtures", "profile", "download", "child-fixture.json"]) {
                const target = join(lab.root, name);
                try { assert.equal(dirname(await realpath(target)), root, "Cleanup escaped its generated fixture"); }
                catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") continue; throw error; }
                await rm(target, { recursive: true, force: true });
            }
        }
    }
}
finally {
    process.env.QBUTT_LAB_EXE = original;
    delete process.env.QBUTT_LAB_CHILD_FIXTURE;
}
if (failures.length)
    throw new AggregateError(failures, "Native route authentication or child contract failed");
assert.equal(dirname(await realpath(bundle)), await realpath(tmpdir()), "Runtime cleanup escaped temp");
await rm(bundle, { recursive: true, force: true });
