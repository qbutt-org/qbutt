import { MAX_MESSAGE_BYTES } from "../../components/content/browser/protocol.ts";

export function encodeFrame(value: unknown): Buffer {
    const body = Buffer.from(JSON.stringify(value), "utf8");
    if (!body.length || body.length > MAX_MESSAGE_BYTES) throw new Error("Browser message exceeds the limit");
    const header = Buffer.alloc(4);
    header.writeUInt32LE(body.length);
    return Buffer.concat([header, body]);
}

export class FrameDecoder {
    private buffered = Buffer.alloc(0);

    push(chunk: Uint8Array, receive: (value: unknown) => void): void {
        this.buffered = Buffer.concat([this.buffered, chunk]);
        while (this.buffered.length >= 4) {
            const length = this.buffered.readUInt32LE(0);
            if (!length || length > MAX_MESSAGE_BYTES) throw new Error("Invalid browser message length");
            if (this.buffered.length < length + 4) break;
            const body = this.buffered.subarray(4, length + 4);
            this.buffered = this.buffered.subarray(length + 4);
            receive(JSON.parse(body.toString("utf8")));
        }
        if (this.buffered.length > MAX_MESSAGE_BYTES + 4) throw new Error("Browser message exceeds the limit");
    }

    finish(): void {
        if (this.buffered.length) throw new Error("Truncated browser message");
    }
}
