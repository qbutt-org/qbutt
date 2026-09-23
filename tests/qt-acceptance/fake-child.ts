// Deterministic transport process for the Qt acceptance scenario. It owns real
// loopback listeners but never forwards user traffic or reads a subscription.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { createServer, isIP, type Server } from "node:net";
import { isAbsolute } from "node:path";

const evidencePath = process.env.QBUTT_QT_CHILD_EVIDENCE;
assert(evidencePath, "QBUTT_QT_CHILD_EVIDENCE is required");
const servers = new Map<string, { server: Server; pathId: string; generation: number; proxyName: string; reserveNames: string[];
    wire: { relayDownloadBytes: number; relayUploadBytes: number } }>();
const username = "acceptance-user";
const password = "QBUTT_ACCEPTANCE_SECRET";
const protocolVersion = 8;
const names = ["Alpha", "Beta", "https://user:pass@example.invalid/sub?token=QBUTT_ACCEPTANCE_SECRET#publicEndpoint=198.51.100.44,[2001:db8::44]", "Alpha reserve"];
const serverHost = (name: string) => `127.0.0.${name === "Alpha reserve" ? 20 : names.indexOf(name) + 20}`;
const configuredServerId = (name: string) => {
    assert(names.includes(name), "Unknown acceptance proxy name");
    return createHash("sha256").update("qbutt-configured-server-v1\0" + serverHost(name)).digest("hex");
};
const controlPath = process.env.QBUTT_QT_CHILD_CONTROL;
const control = (): { failedNames?: string[]; recommended?: Record<string, string>; delayStatus?: string;
    failReplacement?: string } => {
    if (!controlPath)
        return {};
    try { return JSON.parse(readFileSync(controlPath, "utf8")); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return {}; throw error; }
};
const delayedTokens = new Set<string>();
const evidence = { protocol: protocolVersion, hello: 0, listed: 0, status: 0, authenticated: 0, rejectedCredentials: 0,
    payloadBoundaries: 0, delayedStatus: 0, retiredIngress: 0, failedReplacements: 0,
    statusPending: false, eofObserved: false,
    opened: [] as object[], closed: [] as object[], retiredOnEof: [] as object[] };
const save = () => writeFileSync(evidencePath, JSON.stringify(evidence, null, 2) + "\n");
const requireKeys = (request: Record<string, unknown>, keys: string[]) =>
    assert.deepEqual(Object.keys(request).sort(), keys.sort());
const requirePathGeneration = (request: Record<string, unknown>) => {
    assert.equal(typeof request.pathId, "string");
    assert(String(request.pathId).length > 0 && Buffer.byteLength(String(request.pathId)) <= 128);
    assert(Number.isSafeInteger(request.generation) && Number(request.generation) > 0);
};
const requireDns = (value: unknown) => {
    assert(value && (typeof value === "object") && !Array.isArray(value));
    const dns = value as Record<string, unknown>;
    assert.deepEqual(Object.keys(dns).sort(), ["bootstrapServer", "family", "server"]);
    for (const endpoint of [dns.server, dns.bootstrapServer]) {
        assert.equal(typeof endpoint, "string");
        const match = /^(?:\[([^\]]+)\]|([^:]+)):(\d+)$/.exec(String(endpoint));
        assert(match && isIP(match[1] ?? match[2]!) > 0);
        const port = Number(match[3]);
        assert(Number.isSafeInteger(port) && port > 0 && port <= 65535);
    }
    assert(["ipv4", "ipv6", "dual"].includes(String(dns.family)));
};

function authenticatedServer(wire: { relayDownloadBytes: number; relayUploadBytes: number }): Server {
    return createServer(socket => {
        let input = Buffer.alloc(0);
        let stage: "methods" | "credentials" | "request" | "payload" = "methods";
        socket.on("error", () => {});
        socket.on("data", (chunk: Buffer) => {
            input = Buffer.concat([input, chunk]);
            for (;;) {
                if (stage === "methods") {
                    if (input.length < 2)
                        return;
                    const version = input[0]!;
                    const length = input[1]!;
                    if (input.length < 2 + length)
                        return;
                    const methods = input.subarray(2, 2 + length);
                    input = input.subarray(2 + length);
                    if ((version !== 5) || (methods.length === 0) || !methods.includes(2)) {
                        socket.end(Buffer.from([5, 0xff]));
                        return;
                    }
                    socket.write(Buffer.from([5, 2]));
                    stage = "credentials";
                    continue;
                }
                if (stage === "credentials") {
                    if (input.length < 2)
                        return;
                    const version = input[0]!;
                    const userLength = input[1]!;
                    if (input.length < 3 + userLength)
                        return;
                    const passwordLength = input[2 + userLength]!;
                    if (input.length < 3 + userLength + passwordLength)
                        return;
                    const suppliedUser = input.subarray(2, 2 + userLength).toString();
                    const suppliedPassword = input.subarray(3 + userLength, 3 + userLength + passwordLength).toString();
                    input = input.subarray(3 + userLength + passwordLength);
                    const accepted = (version === 1)
                        && (suppliedUser === username) && (suppliedPassword === password);
                    if (accepted)
                        evidence.authenticated++;
                    else
                        evidence.rejectedCredentials++;
                    save();
                    socket.write(Buffer.from([1, accepted ? 0 : 1]));
                    if (!accepted) {
                        socket.end();
                        return;
                    }
                    stage = "request";
                    continue;
                }
                if (stage === "request") {
                    if (input.length < 4)
                        return;
                    if ((input[3] === 3) && (input.length < 5))
                        return;
                    const addressLength = input[3] === 1 ? 4 : input[3] === 4 ? 16 : input[3] === 3 ? input[4]! : -1;
                    const headerLength = input[3] === 3 ? 5 : 4;
                    if (addressLength < 0) {
                        socket.end();
                        return;
                    }
                    const requestLength = headerLength + addressLength + 2;
                    if (input.length < requestLength)
                        return;
                    const valid = (input[0] === 5) && (input[1] === 1) && (input[2] === 0);
                    input = input.subarray(requestLength);
                    if (!valid) {
                        socket.end(Buffer.from([5, 7, 0, 1, 127, 0, 0, 1, 0, 0]));
                        return;
                    }
                    socket.write(Buffer.from([5, 0, 0, 1, 127, 0, 0, 1, 0, 0]));
                    stage = "payload";
                    continue;
                }
                if (input.length > 0) {
                    evidence.payloadBoundaries++;
                    wire.relayUploadBytes += input.length;
                    wire.relayDownloadBytes += input.length;
                    save();
                    socket.write(input);
                    input = Buffer.alloc(0);
                }
                return;
            }
        });
    });
}

let input = "";
for await (const chunk of Bun.stdin.stream()) {
    input += new TextDecoder().decode(chunk);
    assert(Buffer.byteLength(input) <= 65536, "Control input exceeded the protocol bound");
    for (;;) {
        const newline = input.indexOf("\n");
        if (newline < 0)
            break;
        const request = JSON.parse(input.slice(0, newline)) as Record<string, unknown>;
        input = input.slice(newline + 1);
        assert.equal(request.v, protocolVersion);
        assert(Number.isSafeInteger(request.id) && Number(request.id) > 0, "Control request id must be a positive safe integer");
        assert.equal(typeof request.method, "string");
        let result: Record<string, unknown> = {};
        if (request.method === "hello") {
            requireKeys(request, ["id", "method", "v"]);
            evidence.hello++;
            result = { protocol: protocolVersion, name: "qbutt-net", upstreamRevision: "d3ec342d441b086ec4318332f59dd05d8a2b5697",
                maxFrameBytes: 65536 };
        }
        else if (request.method === "list") {
            requireKeys(request, request.proxyNames === undefined ? ["configPath", "id", "method", "v"]
                : ["configPath", "id", "method", "proxyNames", "v"]);
            assert.equal(typeof request.configPath, "string");
            assert(isAbsolute(String(request.configPath)));
            evidence.listed++;
            const selected = request.proxyNames as string[] | undefined;
            assert(!selected || (selected.length > 0 && selected.length <= 4 && selected.every(name => names.includes(name))));
            result = { proxies: names.filter(name => !selected || selected.includes(name))
                .map(name => ({ name, type: name === names[2] ? "malicious-name" : "acceptance",
                    configuredServerId: configuredServerId(name), serverHost: serverHost(name) })) };
        }
        else if (request.method === "open" || request.method === "transport.replace") {
            requireKeys(request,
                ["configPath", "configuredServerId", "dns", "generation", "id", "interfaceName", "method", "pathId", "proxyName", "reserveNames", "reserveServerIds", "v",
                    ...(request.method === "transport.replace" ? ["nextGeneration"] : [])]);
            requirePathGeneration(request);
            requireDns(request.dns);
            assert.equal(typeof request.configPath, "string");
            assert(isAbsolute(String(request.configPath)));
            assert.equal(typeof request.proxyName, "string");
            assert.equal(typeof request.interfaceName, "string");
            assert(String(request.proxyName).length > 0 && String(request.interfaceName).length > 0);
            assert.equal(request.configuredServerId, configuredServerId(String(request.proxyName)));
            const pathId = String(request.pathId);
            const oldKey = `${pathId}:${request.generation}`;
            const old = request.method === "transport.replace" ? servers.get(oldKey) : undefined;
            if (request.method === "transport.replace") {
                assert(old && old.reserveNames.includes(String(request.proxyName)));
                assert(Number(request.nextGeneration) > old.generation);
                if (control().failReplacement === request.proxyName) {
                    evidence.failedReplacements++;
                    save();
                    process.stdout.write(JSON.stringify({ v: protocolVersion, id: request.id,
                        error: { code: "transport_open_failed", message: "transport_open_failed" } }) + "\n");
                    continue;
                }
            }
            const generation = Number(request.nextGeneration ?? request.generation);
            const reserveNames = request.reserveNames as string[];
            const reserveServerIds = request.reserveServerIds as Record<string, string>;
            assert(Array.isArray(reserveNames));
            assert.deepEqual(Object.keys(reserveServerIds).sort(), [...reserveNames].sort());
            assert(reserveNames.every(name => configuredServerId(name) === reserveServerIds[name]));
            const key = `${pathId}:${generation}`;
            assert(!servers.has(key), "The parent attempted to open a duplicate path generation");
            const wire = { relayDownloadBytes: 0, relayUploadBytes: 0 };
            const server = authenticatedServer(wire);
            await new Promise<void>((resolve, reject) => {
                server.once("error", reject);
                server.listen(0, "127.0.0.1", resolve);
            });
            servers.set(key, { server, pathId, generation, proxyName: String(request.proxyName), reserveNames, wire });
            const address = server.address();
            assert(address && typeof address !== "string");
            evidence.opened.push({ pathId, generation, proxyName: request.proxyName, port: address.port });
            if (old) {
                // The candidate listener exists before the old generation retires.
                process.stdout.write(JSON.stringify({ v: protocolVersion, id: 0, event: "incomingTcp",
                    pathId, generation: old.generation, remote: "198.51.100.17:2345", publicEndpoint: "127.0.0.1:40000",
                    relayHost: "127.0.0.1", relayPort: 40001, relayToken: "ab".repeat(32) }) + "\n");
                evidence.retiredIngress++;
                await new Promise<void>(resolve => old.server.close(() => resolve()));
                servers.delete(oldKey);
                evidence.closed.push({ pathId, generation: old.generation });
            }
            result = {
                pathId, generation, interfaceName: String(request.interfaceName), host: "127.0.0.1", port: address.port,
                configuredServerId: request.configuredServerId,
                socksUsername: username, socksPassword: password,
                capabilities: { tcp: "supported", udp: "source-supported", dns: "path-tcp",
                    publicTcp: "unknown", publicUdp: "unknown", measurement: "not-probed" },
            };
        }
        else if (request.method === "resolve") {
            requireKeys(request, ["family", "generation", "host", "id", "method", "pathId", "v"]);
            requirePathGeneration(request);
            assert.equal(typeof request.host, "string");
            assert(String(request.host).length > 0);
            assert(["ipv4", "ipv6"].includes(String(request.family)));
            result = { addresses: request.family === "ipv6" ? ["::1"] : ["127.0.0.1"] };
        }
        else if (request.method === "status") {
            requireKeys(request, ["id", "method", "v"]);
            evidence.status++;
            const state = control();
            const delayToken = state.delayStatus;
            if (((servers.size === 3) && (evidence.delayedStatus === 0))
                || (delayToken && !delayedTokens.has(delayToken))) {
                if (delayToken)
                    delayedTokens.add(delayToken);
                evidence.delayedStatus++;
                evidence.statusPending = true;
                save();
                await Bun.sleep(1000);
                evidence.statusPending = false;
            }
            result = { paths: [...servers.values()].map(path => ({
                pathId: path.pathId, generation: path.generation,
                transport: { state: state.recommended?.[path.proxyName] ? "ready"
                    : path.reserveNames.length ? "unknown" : "disabled",
                recommended: state.recommended?.[path.proxyName] ?? "" },
                health: state.failedNames?.includes(path.proxyName)
                    ? { state: "failed", protocol: "tcp-dns", checkedAtUnixMilli: Date.now(), reason: "dns-round-trip-failed" }
                    : { state: "reachable", protocol: "tcp-dns", checkedAtUnixMilli: Date.now() },
                wire: { relayDownloadBytes: path.wire.relayDownloadBytes, relayUploadBytes: path.wire.relayUploadBytes, carrierDownloadBytes: 0,
                    carrierUploadBytes: 0, carrierDownloadPackets: 0, carrierUploadPackets: 0,
                    relayDownloadCopies: 0 },
            })) };
        }
        else if (request.method === "close") {
            requireKeys(request, ["generation", "id", "method", "pathId", "v"]);
            requirePathGeneration(request);
            const key = `${request.pathId}:${request.generation}`;
            const path = servers.get(key);
            assert(path, "The parent attempted to close an unknown path generation");
            await new Promise<void>(resolve => path.server.close(() => resolve()));
            servers.delete(key);
            evidence.closed.push({ pathId: request.pathId, generation: request.generation });
        }
        else {
            throw new Error(`Unexpected control method: ${request.method}`);
        }
        save();
        process.stdout.write(JSON.stringify({ v: protocolVersion, id: request.id, result }) + "\n");
    }
}

const remaining = [...servers.values()];
await Promise.all(remaining.map(path => new Promise<void>(resolve => path.server.close(() => resolve()))));
evidence.retiredOnEof.push(...remaining.map(path => ({ pathId: path.pathId, generation: path.generation })));
evidence.eofObserved = true;
save();
