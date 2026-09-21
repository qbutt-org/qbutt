// Bounded live adapter probes. Credentials and public addresses stay out of evidence.
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createConnection, isIPv4 } from "node:net";
import { networkInterfaces, tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { createInterface } from "node:readline";
import { Readable } from "node:stream";
import { sha256 } from "../fixtures/generate";
import { allowLabNetwork } from "../windows-firewall";

const binary = process.env.QBUTT_PROBE_EXE ?? "";
const configuration = process.env.QBUTT_PROBE_CONFIG ?? "";
const indices = (process.env.QBUTT_PROBE_INDICES ?? "").split(",").map(Number);
const interfaceName = process.env.QBUTT_LAB_NATIVE_INTERFACE ?? "";
const nativeAddress = networkInterfaces()[interfaceName]?.find(item => item.family === "IPv4" && !item.internal)?.address;
assert(binary && configuration && nativeAddress && process.env.QBUTT_PROBE_INDICES,
    "Set QBUTT_PROBE_EXE, QBUTT_PROBE_CONFIG, QBUTT_PROBE_INDICES and QBUTT_LAB_NATIVE_INTERFACE");
assert(indices.length <= 8 && new Set(indices).size === indices.length
    && indices.every(index => Number.isSafeInteger(index) && index >= 0), "Choose up to eight distinct node indices");
const original = await readFile(configuration);
let document: { proxies: Record<string, any>[] };
try { document = Bun.YAML.parse(original.toString("utf8")) as typeof document; }
catch { throw new Error("Cannot parse probe configuration; contents omitted"); }
assert(Array.isArray(document.proxies) && indices.every(index => document.proxies[index]), "Selected node is missing");
await allowLabNetwork([process.execPath, binary]);
const root = await mkdtemp(join(tmpdir(), "qbutt-transport-capabilities-"));
const child = Bun.spawn([binary, "--stdio"], { stdin: "pipe", stdout: "pipe", stderr: "pipe", windowsHide: true });
const diagnostics = new Response(child.stderr).text();
const lines = createInterface({ input: Readable.from(child.stdout) });
const replies = lines[Symbol.asyncIterator]();
const report: Record<string, any> = { status: "running", protocol: 7, binarySha256: sha256(await readFile(binary)),
    probes: [], scope: "Point-in-time outbound HTTPS and DNS/UDP over selected adapters; not throughput or physical VPN-bypass proof" };
let requestId = 0;
let failure: unknown;

async function bounded<T>(operation: Promise<T>, milliseconds: number) {
    let timer: ReturnType<typeof setTimeout>;
    try {
        return await Promise.race([operation, new Promise<never>((_, reject) => {
            timer = setTimeout(() => reject(new Error("probe_timeout")), milliseconds);
        })]);
    }
    finally { clearTimeout(timer!); }
}

async function request(method: string, fields: object = {}) {
    const id = ++requestId;
    child.stdin.write(JSON.stringify({ v: 7, id, method, ...fields }) + "\n");
    await child.stdin.flush();
    const next = await bounded(replies.next(), 20000);
    assert(!next.done && next.value.length <= 65536, "invalid_control_reply");
    const reply = JSON.parse(next.value);
    assert(reply.id === id && reply.v === 7, "invalid_control_identity");
    if (reply.error) throw new Error(`control_${reply.error.code}`);
    return reply.result;
}

interface Endpoint { host: string; port: number; socksUsername: string; socksPassword: string }

async function https(endpoint?: Endpoint) {
    const config = ["silent", "show-error", "max-time = 25", 'url = "https://api.ipify.org"',
        'write-out = "\\n%{http_code}"', "ipv4"];
    if (endpoint) config.push('noproxy = ""', `proxy = "socks5h://${endpoint.host}:${endpoint.port}"`,
        `proxy-user = "${endpoint.socksUsername}:${endpoint.socksPassword}"`);
    else config.push('noproxy = "*"', 'proxy = ""', `interface = "${nativeAddress}"`);
    const started = performance.now();
    const curl = Bun.spawn(["curl.exe", "--config", "-"], { stdin: "pipe", stdout: "pipe", stderr: "pipe",
        timeout: 30000, windowsHide: true });
    curl.stdin.write(config.join("\n") + "\n");
    curl.stdin.end();
    const [exit, output] = await Promise.all([curl.exited, new Response(curl.stdout).text(),
        new Response(curl.stderr).text()]);
    const parts = output.trim().split("\n");
    const http = Number(parts.pop());
    const address = parts.join("\n").trim();
    return { exit, http, address: isIPv4(address) ? address : "", elapsedMs: Math.round(performance.now() - started) };
}

async function udp(endpoint: Endpoint) {
    const control = createConnection(endpoint.port, endpoint.host);
    const datagram = createSocket("udp4");
    const input = control[Symbol.asyncIterator]();
    let pending = Buffer.alloc(0);
    async function exact(length: number) {
        while (pending.length < length) {
            const next = await input.next();
            assert(!next.done, "socks_closed");
            pending = Buffer.concat([pending, next.value]);
            assert(pending.length <= 4096, "socks_response_limit");
        }
        const result = pending.subarray(0, length);
        pending = pending.subarray(length);
        return result;
    }
    try {
        await bounded((async () => {
            control.write(Buffer.from([5, 1, 2]));
            assert.deepEqual(await exact(2), Buffer.from([5, 2]), "socks_auth_method");
            const user = Buffer.from(endpoint.socksUsername), password = Buffer.from(endpoint.socksPassword);
            control.write(Buffer.concat([Buffer.from([1, user.length]), user, Buffer.from([password.length]), password]));
            assert.deepEqual(await exact(2), Buffer.from([1, 0]), "socks_auth");
            control.write(Buffer.from([5, 3, 0, 1, 0, 0, 0, 0, 0, 0]));
            const association = await exact(10);
            assert.deepEqual(association.subarray(0, 8), Buffer.from([5, 0, 0, 1, 127, 0, 0, 1]), "socks_associate");
            const port = association.readUInt16BE(8);
            assert(port > 0, "socks_port");
            await new Promise<void>((accept, reject) => {
                datagram.once("error", reject);
                datagram.bind(0, "127.0.0.1", accept);
            });
            await new Promise<void>(accept => datagram.connect(port, "127.0.0.1", accept));
            const envelope = Buffer.from([0, 0, 0, 1, 1, 1, 1, 1, 0, 53]);
            const query = Buffer.concat([randomBytes(2), Buffer.from([1, 0, 0, 1, 0, 0, 0, 0, 0, 0]),
                Buffer.from([7]), Buffer.from("example"), Buffer.from([3]), Buffer.from("com"), Buffer.from([0, 0, 1, 0, 1])]);
            const response = new Promise<Buffer>((accept, reject) => {
                datagram.once("error", reject);
                datagram.once("message", accept);
            });
            datagram.send(Buffer.concat([envelope, query]));
            const reply = await response;
            assert(reply.length > 22 && reply.length <= 4096 && reply.subarray(0, 10).equals(envelope)
                && reply.subarray(10, 12).equals(query.subarray(0, 2)) && (reply[12]! & 0x80) !== 0
                && reply.readUInt16BE(14) === 1 && reply.subarray(22, 22 + query.length - 12).equals(query.subarray(12)),
            "uncorrelated_udp_reply");
            assert((reply[13]! & 15) === 0 && reply.readUInt16BE(16) > 0, "dns_answer_missing");
        })(), 20000);
        return { passed: true };
    }
    finally {
        control.destroy();
        try { datagram.close(); }
        catch (error) { if ((error as NodeJS.ErrnoException).code !== "ERR_SOCKET_DGRAM_NOT_RUNNING") throw error; }
    }
}

try {
    assert.equal((await request("hello")).protocol, 7);
    const native = await https();
    report.native = { exit: native.exit, http: native.http, validAddress: Boolean(native.address) };
    assert(native.exit === 0 && native.http === 200 && native.address, "native_reference_failed");
    for (const [ordinal, index] of indices.entries()) {
        const node: Record<string, any> = { ...document.proxies[index], name: "probe" };
        const configPath = join(root, "node.json");
        await writeFile(configPath, JSON.stringify({ proxies: [node] }));
        const listed = await request("list", { configPath, proxyName: "probe" });
        const generation = ordinal + 1;
        const endpoint = await request("open", { configPath, proxyName: "probe", pathId: "probe", generation,
            configuredServerId: listed.proxies[0].configuredServerId, interfaceName,
            dns: { server: "1.1.1.1:53", bootstrapServer: "1.1.1.1:53", family: "ipv4" } }) as Endpoint;
        assert(endpoint.host === "127.0.0.1" && Number.isInteger(endpoint.port)
            && /^[0-9a-f]+$/.test(endpoint.socksUsername) && /^[0-9a-f]+$/.test(endpoint.socksPassword), "invalid_endpoint");
        const tcp = await https(endpoint);
        let datagram: object;
        try { datagram = await udp(endpoint); }
        catch (error) { datagram = { passed: false,
            reason: error instanceof Error && error.message === "probe_timeout" ? "timeout" : "socket_or_response" }; }
        let pathDNS: object;
        try {
            const resolved = await request("resolve", { pathId: "probe", generation, host: "api.ipify.org", family: "ipv4" });
            assert(Array.isArray(resolved.addresses) && resolved.addresses.length > 0
                && resolved.addresses.every(isIPv4), "invalid_path_dns_reply");
            pathDNS = { passed: true, addressCount: resolved.addresses.length };
        }
        catch { pathDNS = { passed: false }; }
        const status = await request("status");
        const wire = status.paths.find((path: any) => path.pathId === "probe")?.wire;
        const variant = node.obfs ?? node.network;
        const result = { index, type: listed.proxies[0].type,
            variant: ["gecko", "salamander", "grpc", "tcp", "ws", "xhttp"].includes(variant) ? variant : null,
            https: { exit: tcp.exit, http: tcp.http, validAddress: Boolean(tcp.address),
                differentFromNative: Boolean(tcp.address && tcp.address !== native.address), elapsedMs: tcp.elapsedMs },
            udp: datagram, pathDNS, wire };
        report.probes.push(result);
        console.log(JSON.stringify(result));
        await request("close", { pathId: "probe", generation });
    }
    await request("shutdown");
    child.stdin.end();
    assert.equal(await bounded(child.exited, 10000), 0, "child_shutdown_failed");
    report.emptyDiagnostics = (await diagnostics).length === 0;
    assert(report.emptyDiagnostics, "unexpected_child_diagnostics");
    report.status = report.probes.every((item: any) => item.https.exit === 0 && item.https.http === 200
        && item.https.differentFromNative && item.udp.passed && item.pathDNS.passed) ? "passed" : "partial";
}
catch (error) {
    failure = error;
    report.status = "failed";
    report.error = error instanceof Error && /^[a-z_]{1,64}$/.test(error.message)
        ? error.message : "probe_control_or_reference_failed";
}
finally {
    if (child.exitCode === null) { child.kill(); await child.exited; }
    lines.close();
    const owned = await realpath(root);
    assert(dirname(owned) === await realpath(tmpdir()) && basename(owned).startsWith("qbutt-transport-capabilities-"));
    await rm(join(owned, "node.json"), { force: true });
    report.originalProfileUnchanged = sha256(await readFile(configuration)) === sha256(original);
    await writeFile(join(root, "evidence.json"), JSON.stringify(report, null, 2) + "\n");
}
assert(report.originalProfileUnchanged, "Source profile changed during read-only probes");
console.log(JSON.stringify({ status: report.status, evidence: join(root, "evidence.json") }));
if (failure) throw new Error("Capability probe failed; see redacted evidence");
if (report.status !== "passed") process.exitCode = 1;
