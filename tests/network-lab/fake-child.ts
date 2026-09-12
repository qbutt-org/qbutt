// Compiled only into an isolated test bundle. This is never a production transport.
import assert from "node:assert/strict";
import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { createServer, type Server, type Socket } from "node:net";

const specification = JSON.parse(readFileSync(process.env.QBUTT_LAB_CHILD_FIXTURE!, "utf8")) as {
    mode: string; evidencePath: string; dns: Record<string, string>; lookupHost: string;
};
assert(["no-auth", "wrong-credentials", "incompatible", "dns-success", "dns-request-error", "dns-malformed",
    "dns-nonnumeric", "dns-wrong-family", "dns-too-many", "dns-timeout", "dns-crash"].includes(specification.mode));
const dnsMode = specification.mode.startsWith("dns-");
const evidence = { mode: specification.mode, hello: 0, opened: 0, closed: 0,
    offeredMethods: [] as number[][], rejectedAuthentication: 0, bytesAfterRejection: 0,
    commandsAfterRejection: [] as number[], ports: [] as number[], resolved: 0,
    dns: specification.dns, lookupHostMatched: false };
const servers: Server[] = [];
const sockets = new Set<Socket>();
let openedPath: { pathId: string; generation: number } | undefined;
function saveEvidence() {
    writeFileSync(`${specification.evidencePath}.next`, JSON.stringify(evidence));
    renameSync(`${specification.evidencePath}.next`, specification.evidencePath);
}
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
        assert.equal(request.v, 2, "Parent must use protocol v2");
        let result: Record<string, unknown>;
        if (request.method === "hello") {
            evidence.hello++;
            result = { name: "qbutt-net", protocol: specification.mode === "incompatible" ? 1 : 2, maxFrameBytes: 65536 };
        }
        else if (request.method === "open") {
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
            openedPath = { pathId: request.pathId, generation: request.generation };
            result = { host: "127.0.0.1", port: endpoint.port, pathId: request.pathId,
                generation: request.generation, interfaceName: request.interfaceName,
                socksUsername: "fixture-user", socksPassword: "fixture-password",
                capabilities: { tcp: "supported", udp: "source-unsupported", dns: "path-tcp",
                    publicTcp: "unknown", publicUdp: "unknown", measurement: "not-probed" } };
        }
        else if (request.method === "resolve") {
            assert(dnsMode, "Authentication fixture unexpectedly received DNS work");
            assert.deepEqual({ pathId: request.pathId, generation: request.generation }, openedPath,
                "Lookup did not preserve the opened path generation");
            evidence.resolved++;
            evidence.lookupHostMatched = request.host === specification.lookupHost;
            assert(evidence.lookupHostMatched && request.family === "ipv4", "Unexpected lookup request");
            saveEvidence();
            if (specification.mode === "dns-timeout")
                continue;
            if (specification.mode === "dns-crash")
                process.exit(95);
            if (specification.mode === "dns-request-error" && evidence.resolved === 1) {
                process.stdout.write(JSON.stringify({ v: 2, id: request.id,
                    error: { code: "fixture-sensitive-code", message: `${request.host} fixture-password` } }) + "\n");
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
            result = { addresses, diagnostic: `${request.host} fixture-password` };
        }
        else {
            result = {};
        }
        saveEvidence();
        process.stdout.write(JSON.stringify({ v: 2, id: request.id, result }) + "\n");
    }
}
for (const socket of sockets)
    socket.destroy();
await Promise.all(servers.map(server => new Promise<void>((resolve, reject) =>
    server.close(error => error ? reject(error) : resolve()))));
saveEvidence();
