import assert from "node:assert/strict";
import { createServer, type Socket } from "node:net";

// One framed DNS request per TCP connection; every answer is generated locally.
export async function startDns(side: number, listenAddress = "127.0.0.1") {
    const sockets = new Set<Socket>();
    const queries: { family: number; slow: boolean; source: string }[] = [];
    const server = createServer(socket => {
        sockets.add(socket);
        socket.on("error", () => socket.destroy());
        socket.on("close", () => sockets.delete(socket));
        socket.setTimeout(3000, () => socket.destroy());
        let bytes = Buffer.alloc(0);
        socket.on("data", (chunk: Buffer) => {
            bytes = Buffer.concat([bytes, chunk]);
            if (bytes.length > 2048) return socket.destroy();
            if (bytes.length < 2 || bytes.length < bytes.readUInt16BE(0) + 2) return;
            socket.removeAllListeners("data");
            const request = bytes.subarray(2);
            let offset = 12;
            const labels: string[] = [];
            while (offset < request.length && request[offset] !== 0) {
                const length = request[offset++]!;
                if (length > 63 || offset + length > request.length) return socket.destroy();
                labels.push(request.subarray(offset, offset + length).toString("ascii"));
                offset += length;
            }
            if (offset + 5 !== request.length) return socket.destroy();
            const family = request.readUInt16BE(offset + 1);
            if (![1, 28].includes(family)) return socket.destroy();
            const slow = labels[0] === "slow";
            queries.push({ family, slow, source: socket.remoteAddress ?? "" });
            const header = Buffer.from(request.subarray(0, 12));
            header.writeUInt16BE(0x8180, 2);
            header.writeUInt16BE(1, 6);
            const answer = Buffer.alloc(family === 1 ? 16 : 28);
            answer.writeUInt16BE(0xc00c, 0);
            answer.writeUInt16BE(family, 2);
            answer.writeUInt16BE(1, 4);
            answer.writeUInt32BE(0, 6); // No cache: every assertion observes its own exchange.
            answer.writeUInt16BE(family === 1 ? 4 : 16, 10);
            if (family === 1) answer.set([127, 0, 0, side + 2], 12);
            else answer[27] = side + 2;
            const message = Buffer.concat([header, request.subarray(12), answer]);
            const frame = Buffer.alloc(message.length + 2);
            frame.writeUInt16BE(message.length);
            message.copy(frame, 2);
            const reply = () => { if (!socket.destroyed) socket.end(frame); };
            if (slow) setTimeout(reply, 600);
            else reply();
        });
    });
    await new Promise<void>((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, listenAddress, resolve);
    });
    const address = server.address();
    assert(address && typeof address !== "string");
    return { port: address.port, queries, async stop() {
        for (const socket of sockets) socket.destroy();
        await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    } };
}
