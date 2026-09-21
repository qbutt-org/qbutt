import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { createServer, type Socket } from "node:net";
import type { TorrentFixture } from "../fixtures/generate";

export interface ControlledPeerError {
    kind: "transport" | "protocol";
    message: string;
    code?: string;
    payloadBytes: number;
}

// A bounded plaintext peer whose initial choke can be released explicitly.
// Payload requests always use the generated torrent's real piece layout.
export async function startControlledPeer(torrent: TorrentFixture, payload: Buffer,
    errors: ControlledPeerError[], hasPieces = true) {
    assert(torrent.infoHashV1, "Controlled peer requires a v1 infohash");
    const infoHash = Buffer.from(torrent.infoHashV1, "hex");
    const sockets = new Set<Socket>();
    const counts = { connections: 0, interested: 0, requests: 0, payloadBytes: 0 };
    let choked = hasPieces;
    const unchoke = Buffer.from([0, 0, 0, 1, 1]);
    const server = createServer(socket => {
        sockets.add(socket);
        counts.connections++;
        let pending = Buffer.alloc(0);
        let handshaken = false;
        socket.on("error", error => errors.push({kind: "transport", message: String(error),
            code: (error as NodeJS.ErrnoException).code, payloadBytes: counts.payloadBytes}));
        socket.on("close", () => sockets.delete(socket));
        socket.on("data", chunk => {
            try {
                assert(Buffer.isBuffer(chunk), "Controlled peer requires binary socket data");
                pending = Buffer.concat([pending, chunk]);
                assert(pending.length <= 128 * 1024, "Fixture peer input exceeded its bound");
                if (!handshaken) {
                    if (pending.length < 68) return;
                    assert.equal(pending[0], 19);
                    assert.equal(pending.subarray(1, 20).toString(), "BitTorrent protocol");
                    assert(pending.subarray(28, 48).equals(infoHash));
                    const reply = Buffer.from(pending.subarray(0, 68));
                    reply.fill(0, 20, 28);
                    randomBytes(20).copy(reply, 48);
                    const bitfield = Buffer.alloc(5 + Math.ceil(torrent.pieceCount / 8));
                    bitfield.writeUInt32BE(bitfield.length - 4);
                    bitfield[4] = 5;
                    if (hasPieces) for (let index = 0; index < torrent.pieceCount; index++)
                        bitfield[5 + (index >> 3)]! |= 0x80 >> (index % 8);
                    socket.write(Buffer.concat([reply, bitfield, ...(choked ? [] : [unchoke])]));
                    pending = pending.subarray(68);
                    handshaken = true;
                }
                while (pending.length >= 4) {
                    const length = pending.readUInt32BE(0);
                    assert(length <= 65536, "Fixture peer frame exceeded its bound");
                    if (pending.length < length + 4) return;
                    const message = pending.subarray(4, 4 + length);
                    pending = pending.subarray(4 + length);
                    if (length === 0) continue;
                    if (message[0] === 2) counts.interested++;
                    if (message[0] !== 6) continue;
                    counts.requests++;
                    assert(!choked && hasPieces, "The app requested data without available unchoked pieces");
                    assert.equal(length, 13);
                    const piece = message.readUInt32BE(1);
                    const offset = message.readUInt32BE(5);
                    const bytes = message.readUInt32BE(9);
                    const from = piece * torrent.pieceLength + offset;
                    assert(piece < torrent.pieceCount && bytes > 0 && bytes <= 16384
                        && offset + bytes <= torrent.pieceLength && from + bytes <= payload.length);
                    const response = Buffer.alloc(13 + bytes);
                    response.writeUInt32BE(9 + bytes);
                    response[4] = 7;
                    response.writeUInt32BE(piece, 5);
                    response.writeUInt32BE(offset, 9);
                    payload.copy(response, 13, from, from + bytes);
                    socket.write(response);
                    counts.payloadBytes += bytes;
                }
            }
            catch (error) {
                errors.push({kind: "protocol", message: String(error), payloadBytes: counts.payloadBytes});
                socket.destroy();
            }
        });
    });
    await new Promise<void>((accept, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", accept);
    });
    const address = server.address();
    assert(address && typeof address !== "string");
    return { port: address.port, counts,
        release() { choked = false; for (const socket of sockets) socket.write(unchoke); },
        async close() {
            for (const socket of sockets) socket.destroy();
            await new Promise<void>(accept => server.close(() => accept()));
        },
    };
}
