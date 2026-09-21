// Compiled only into an isolated test bundle. This is never a production transport.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { createServer, type Server, type Socket } from "node:net";

const specification = JSON.parse(readFileSync(process.env.QBUTT_LAB_CHILD_FIXTURE!, "utf8")) as {
    mode: string; evidencePath: string; dns: Record<string, string>; lookupHost: string;
};
const protocolVersion = 6;
const upstreamRevision = "d3ec342d441b086ec4318332f59dd05d8a2b5697";
const configuredServerId = (name: string) => createHash("sha256")
    .update(`qbutt-configured-server-v1\0${name === "fault-fixture-2" ? "127.0.0.21" : "127.0.0.20"}`)
    .digest("hex");
assert(["no-auth", "wrong-credentials", "incompatible", "dns-success", "dns-request-error", "dns-malformed",
    "dns-nonnumeric", "dns-wrong-family", "dns-too-many", "dns-timeout", "dns-crash", "dns-result-extra",
    "dns-error-message-extra", "status-delay", "status-delay-extra", "status-decrease", "gateway-rollover",
    "close-error", "legacy-v4", "legacy-v5",
    "hello-extra", "hello-wrong-upstream", "open-envelope-extra", "open-result-extra", "status-result-extra"]
    .includes(specification.mode));
const dnsMode = specification.mode.startsWith("dns-");
const initialEvidence = { mode: specification.mode, protocol: protocolVersion, methods: [] as string[], hello: 0, opened: 0,
    status: 0, closed: 0, closeRequests: 0,
    offeredMethods: [] as number[][], rejectedAuthentication: 0, bytesAfterRejection: 0,
    commandsAfterRejection: [] as number[], ports: [] as number[], resolved: 0,
    dns: specification.dns, lookupHostMatched: false, processStarts: 0, rolloverEvents: 0 };
let evidence = initialEvidence;
try {
    const previous = JSON.parse(readFileSync(specification.evidencePath, "utf8"));
    if (previous.mode === specification.mode)
        evidence = { ...initialEvidence, ...previous };
}
catch {}
evidence.processStarts++;
const servers: Server[] = [];
const sockets = new Set<Socket>();
const openedPaths = new Map<string, { pathId: string; generation: number }>();
const gatewayPaths = new Map<string, { pathId: string; generation: number }>();
const gatewayPorts = new Map<string, number>();
function saveEvidence() {
    writeFileSync(`${specification.evidencePath}.next`, JSON.stringify(evidence));
    renameSync(`${specification.evidencePath}.next`, specification.evidencePath);
}
saveEvidence();
let buffered = "";
for await (const chunk of Bun.stdin.stream()) {
    buffered += new TextDecoder().decode(chunk);
    assert(Buffer.byteLength(buffered) <= 65536, "Fixture received oversized control frame");
    for (;;) {
        const newline = buffered.indexOf("\n");
        if (newline < 0)
            break;
        const request = JSON.parse(buffered.slice(0, newline));
        buffered = buffered.slice(newline + 1);
        assert.equal(request.v, protocolVersion, "Parent must use protocol v6");
        assert(Number.isSafeInteger(request.id) && request.id > 0, "Parent sent an invalid request identity");
        assert.equal(typeof request.method, "string", "Parent omitted the control method");
        evidence.methods.push(request.method);
        let result: Record<string, unknown>;
        if (request.method === "hello") {
            assert.deepEqual(Object.keys(request).sort(), ["id", "method", "v"]);
            evidence.hello++;
            result = { name: "qbutt-net", protocol: specification.mode === "incompatible" ? 2 : protocolVersion,
                upstreamRevision: specification.mode === "hello-wrong-upstream" ? "0".repeat(40) : upstreamRevision,
                maxFrameBytes: 65536 };
            if (specification.mode === "hello-extra")
                result.unexpected = true;
        }
        else if (request.method === "list") {
            assert.deepEqual(Object.keys(request).sort(),
                request.proxyName === undefined ? ["configPath", "id", "method", "v"]
                    : ["configPath", "id", "method", "proxyName", "v"]);
            const names = ["fault-fixture", "fault-fixture-2"];
            assert(request.proxyName === undefined || names.includes(request.proxyName));
            result = { proxies: names.filter(name => request.proxyName === undefined || request.proxyName === name)
                .map(name => ({ name, type: "socks5", configuredServerId: configuredServerId(name) })) };
        }
        else if (request.method === "open") {
            assert.deepEqual(Object.keys(request).sort(),
                ["configPath", "configuredServerId", "dns", "generation", "id", "interfaceName", "method", "pathId", "proxyName", "reserveNames", "v"]);
            assert.equal(request.configuredServerId, configuredServerId(request.proxyName));
            assert.deepEqual(request.dns, specification.dns, "Open must carry the configured DNS policy");
            const server = createServer(socket => {
                sockets.add(socket);
                socket.on("error", () => socket.destroy());
                socket.on("close", () => { sockets.delete(socket); evidence.closed++; saveEvidence(); });
                let phase: "greeting" | "auth" | "command" | "rejected" = "greeting";
                let input = Buffer.alloc(0);
                socket.on("data", chunk => {
                    if (phase === "command") {
                        // DNS control fixtures do not implement payload forwarding.
                        socket.end(Buffer.from([5, 7, 0, 1, 127, 0, 0, 1, 0, 0]));
                        return;
                    }
                    if (phase === "rejected") {
                        evidence.bytesAfterRejection += chunk.length;
                        if (chunk.length >= 2 && chunk[0] === 5)
                            evidence.commandsAfterRejection.push(chunk[1]!);
                        saveEvidence();
                        socket.destroy();
                        return;
                    }
                    input = Buffer.concat([input, chunk]);
                    if (input.length > 1024) {
                        socket.destroy();
                        return;
                    }
                    if (phase === "greeting" && input.length >= 2 && input.length >= 2 + input[1]!) {
                        evidence.offeredMethods.push([...input.subarray(2, 2 + input[1]!)]);
                        input = input.subarray(2 + input[1]!);
                        phase = specification.mode === "no-auth" ? "rejected" : "auth";
                        socket.write(Buffer.from([5, phase === "rejected" ? 0 : 2]));
                    }
                    if (phase === "auth" && input.length >= 3 + input[1]!
                        && input.length >= 3 + input[1]! + input[2 + input[1]!]!) {
                        if (dnsMode) {
                            phase = "command";
                            socket.write(Buffer.from([1, 0]));
                        }
                        else {
                            evidence.rejectedAuthentication++;
                            phase = "rejected";
                            socket.write(Buffer.from([1, 1]));
                        }
                    }
                    saveEvidence();
                });
            });
            await new Promise<void>((resolve, reject) => {
                server.once("error", reject);
                server.listen(0, "127.0.0.1", resolve);
            });
            servers.push(server);
            const endpoint = server.address();
            assert(endpoint && typeof endpoint !== "string");
            evidence.opened++;
            evidence.ports.push(endpoint.port);
            openedPaths.set(request.pathId, { pathId: request.pathId, generation: request.generation });
            if ((specification.mode === "gateway-rollover") && (evidence.processStarts === 2)
                && (evidence.rolloverEvents === 0) && (gatewayPaths.size === 1)) {
                const retired = gatewayPaths.values().next().value!;
                evidence.rolloverEvents++;
                saveEvidence();
                process.stdout.write(JSON.stringify({ v: protocolVersion, id: 0, event: "gatewayClosed",
                    pathId: retired.pathId, generation: retired.generation, reason: "gateway_closed" }) + "\n");
                await Bun.sleep(30000);
            }
            result = { host: "127.0.0.1", port: endpoint.port, pathId: request.pathId,
                configuredServerId: request.configuredServerId,
                generation: request.generation, interfaceName: request.interfaceName,
                socksUsername: "fixture-user", socksPassword: "fixture-password",
                capabilities: { tcp: "supported", udp: "source-unsupported", dns: "path-tcp",
                    publicTcp: "unknown", publicUdp: "unknown", measurement: "not-probed" } };
            if (specification.mode === "open-result-extra")
                result.unexpected = true;
        }
        else if ((request.method === "gateway.open") || (request.method === "gateway.renew")) {
            assert(specification.mode === "gateway-rollover", "Path-only fixture received an unexpected gateway request");
            assert.deepEqual(Object.keys(request).sort(), request.method === "gateway.open"
                ? ["gateway", "generation", "id", "method", "pathId", "v"]
                : ["generation", "id", "method", "pathId", "v"]);
            const opened = openedPaths.get(request.pathId);
            assert.deepEqual({ pathId: request.pathId, generation: request.generation }, opened,
                "Gateway request did not preserve an opened path generation");
            if (request.method === "gateway.open") {
                assert.deepEqual(Object.keys(request.gateway).sort(), ["caPath", "certificatePath", "controlAddress",
                    "datagramAddress", "port", "privateKeyPath", "serverName", "tcp", "ttlSeconds", "udp"]);
                assert(request.gateway.tcp === true && request.gateway.udp === false);
                gatewayPaths.set(request.pathId, opened!);
                gatewayPorts.set(request.pathId, request.gateway.port);
            }
            const port = gatewayPorts.get(request.pathId)!;
            result = { pathId: request.pathId, generation: request.generation,
                publicEndpoint: `8.8.8.8:${port}`, tcp: true, udp: false,
                expiresUnixMilli: Date.now() + 90000, relayHost: "127.0.0.1", relayPort: 9 };
        }
        else if (request.method === "gateway.close") {
            assert(specification.mode === "gateway-rollover", "Path-only fixture received an unexpected gateway close");
            assert.deepEqual(Object.keys(request).sort(), ["generation", "id", "method", "pathId", "v"]);
            assert.deepEqual({ pathId: request.pathId, generation: request.generation }, gatewayPaths.get(request.pathId),
                "Gateway close did not preserve an active lease generation");
            if (evidence.rolloverEvents === 1) {
                const retired = [...gatewayPaths.values()].find(path => path.pathId !== request.pathId)!;
                evidence.rolloverEvents++;
                saveEvidence();
                process.stdout.write(JSON.stringify({ v: protocolVersion, id: 0, event: "gatewayClosed",
                    pathId: retired.pathId, generation: retired.generation, reason: "gateway_closed" }) + "\n");
                await Bun.sleep(30000);
            }
            gatewayPaths.delete(request.pathId);
            gatewayPorts.delete(request.pathId);
            result = {};
        }
        else if (request.method === "status") {
            assert.deepEqual(Object.keys(request).sort(), ["id", "method", "v"]);
            evidence.status++;
            const decreasing = (specification.mode === "status-decrease") && (evidence.status > 1);
            result = { paths: [...openedPaths.values()].map(openedPath => ({ ...openedPath,
                transport: { state: "disabled", recommended: "" }, wire: {
                relayDownloadBytes: decreasing ? 1 : 11, relayUploadBytes: decreasing ? 2 : 12,
                carrierDownloadBytes: decreasing ? 3 : 13, carrierUploadBytes: decreasing ? 4 : 14,
                carrierDownloadPackets: decreasing ? 5 : 15, carrierUploadPackets: decreasing ? 6 : 16,
                relayDownloadCopies: decreasing ? 7 : 17,
            } })) };
            if ((specification.mode === "status-result-extra") || (specification.mode === "status-delay-extra"))
                result.unexpected = true;
            if ((specification.mode === "status-delay") || (specification.mode === "status-delay-extra")) {
                saveEvidence();
                await Bun.sleep(2000);
            }
        }
        else if (request.method === "close") {
            assert.deepEqual(Object.keys(request).sort(), ["generation", "id", "method", "pathId", "v"]);
            assert.deepEqual({ pathId: request.pathId, generation: request.generation }, openedPaths.get(request.pathId),
                "Close did not preserve the opened path generation");
            evidence.closeRequests++;
            saveEvidence();
            if (specification.mode === "close-error") {
                process.stdout.write(JSON.stringify({ v: protocolVersion, id: request.id,
                    error: { code: "path_close_failed", message: "path_close_failed" } }) + "\n");
                continue;
            }
            for (const socket of sockets)
                socket.destroy();
            await Promise.all(servers.map(server => new Promise<void>((resolve, reject) =>
                server.close(error => error ? reject(error) : resolve()))));
            servers.length = 0;
            openedPaths.delete(request.pathId);
            gatewayPaths.delete(request.pathId);
            gatewayPorts.delete(request.pathId);
            result = {};
        }
        else if (request.method === "resolve") {
            assert.deepEqual(Object.keys(request).sort(),
                ["family", "generation", "host", "id", "method", "pathId", "v"]);
            assert(dnsMode, "Authentication fixture unexpectedly received DNS work");
            assert.deepEqual({ pathId: request.pathId, generation: request.generation }, openedPaths.get(request.pathId),
                "Lookup did not preserve the opened path generation");
            evidence.resolved++;
            evidence.lookupHostMatched = request.host === specification.lookupHost;
            assert(evidence.lookupHostMatched && request.family === "ipv4", "Unexpected lookup request");
            saveEvidence();
            if (specification.mode === "dns-timeout")
                continue;
            if (specification.mode === "dns-crash")
                process.exit(95);
            if (((specification.mode === "dns-request-error") || (specification.mode === "dns-error-message-extra"))
                && evidence.resolved === 1) {
                process.stdout.write(JSON.stringify({ v: protocolVersion, id: request.id,
                    error: specification.mode === "dns-request-error"
                        ? { code: "path_dns_failed", message: "path_dns_failed" }
                        : { code: "path_dns_failed", message: `${request.host} fixture-password` } }) + "\n");
                continue;
            }
            let addresses: unknown = ["192.0.2.41", "192.0.2.42"];
            if (specification.mode === "dns-malformed")
                addresses = { unexpected: "fixture-sensitive-value" };
            if (specification.mode === "dns-nonnumeric")
                addresses = [request.host];
            if (specification.mode === "dns-wrong-family")
                addresses = ["2001:db8::41"];
            if (specification.mode === "dns-too-many")
                addresses = Array.from({ length: 65 }, (_, index) => `192.0.2.${index + 1}`);
            result = { addresses };
            if (specification.mode === "dns-result-extra")
                result.diagnostic = `${request.host} fixture-password`;
        }
        else {
            throw new Error(`Path-only fixture received unexpected method ${request.method}`);
        }
        saveEvidence();
        const response: Record<string, unknown> = { v: specification.mode === "legacy-v4" ? 4
            : specification.mode === "legacy-v5" ? 5 : protocolVersion,
            id: request.id, result };
        if ((specification.mode === "open-envelope-extra") && (request.method === "open"))
            response.unexpected = true;
        process.stdout.write(JSON.stringify(response) + "\n");
    }
}
for (const socket of sockets)
    socket.destroy();
await Promise.all(servers.map(server => new Promise<void>((resolve, reject) =>
    server.close(error => error ? reject(error) : resolve()))));
saveEvidence();
