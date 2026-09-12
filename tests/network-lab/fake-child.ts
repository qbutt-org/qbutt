// Compiled only into an isolated test bundle. This is never a production transport.
import assert from "node:assert/strict";
import { readFileSync, renameSync, writeFileSync } from "node:fs";
import { createServer, type Server, type Socket } from "node:net";

const specification = JSON.parse(readFileSync(process.env.QBUTT_LAB_CHILD_FIXTURE!, "utf8")) as {
    mode: "no-auth" | "wrong-credentials" | "incompatible"; evidencePath: string;
};
assert(["no-auth", "wrong-credentials", "incompatible"].includes(specification.mode));
const evidence = { mode: specification.mode, hello: 0, opened: 0, closed: 0,
    offeredMethods: [] as number[][], rejectedAuthentication: 0, bytesAfterRejection: 0,
    commandsAfterRejection: [] as number[] };
const servers: Server[] = [];
const sockets = new Set<Socket>();
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
        let result: Record<string, unknown>;
        if (request.method === "hello") {
            evidence.hello++;
            result = { name: "qbutt-net", protocol: specification.mode === "incompatible" ? 2 : 1, maxFrameBytes: 65536 };
        }
        else if (request.method === "open") {
            const server = createServer(socket => {
                sockets.add(socket);
                socket.on("error", () => socket.destroy());
                socket.on("close", () => { sockets.delete(socket); evidence.closed++; saveEvidence(); });
                let phase: "greeting" | "auth" | "rejected" = "greeting";
                let input = Buffer.alloc(0);
                socket.on("data", chunk => {
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
                        evidence.rejectedAuthentication++;
                        phase = "rejected";
                        socket.write(Buffer.from([1, 1]));
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
            result = { host: "127.0.0.1", port: endpoint.port, pathId: request.pathId,
                generation: request.generation, interfaceName: request.interfaceName,
                socksUsername: "fixture-user", socksPassword: "fixture-password" };
        }
        else {
            result = {};
        }
        saveEvidence();
        process.stdout.write(JSON.stringify({ v: 1, id: request.id, result }) + "\n");
    }
}
for (const socket of sockets)
    socket.destroy();
await Promise.all(servers.map(server => new Promise<void>((resolve, reject) =>
    server.close(error => error ? reject(error) : resolve()))));
saveEvidence();
