import { createConnection } from "node:net";
import { readFile } from "node:fs/promises";
import { discoveryPath } from "./discovery.ts";
import { encodeFrame, FrameDecoder } from "./framing.ts";
import { CHROMIUM_EXTENSION_ID, FIREFOX_EXTENSION_ID } from "./protocol.ts";

export async function runNativeMessaging(argv = process.argv): Promise<boolean> {
    const chromiumOrigin = `chrome-extension://${CHROMIUM_EXTENSION_ID}/`;
    const explicit = argv.includes("--native-messaging");
    const allowed = argv.includes(chromiumOrigin) || argv.includes(FIREFOX_EXTENSION_ID);
    if (!explicit && !allowed) return false;
    if (!allowed) throw new Error("Unknown browser extension");

    const discovery = JSON.parse(await readFile(discoveryPath(), "utf8")) as { pipe: string };
    if (!/^\\\\\.\\pipe\\qbutt-content-browser-[0-9a-f-]{36}$/.test(discovery.pipe)) {
        throw new Error("Invalid browser bridge endpoint");
    }
    await new Promise<void>((resolve, reject) => {
        const socket = createConnection(discovery.pipe);
        const input = new FrameDecoder();
        const output = new FrameDecoder();
        let finished = false;
        const finish = (error?: Error) => {
            if (finished) return;
            finished = true;
            process.stdin.removeListener("data", receive);
            process.stdin.pause();
            socket.destroy();
            error ? reject(error) : resolve();
        };
        const receive = (chunk: Buffer) => {
            try { input.push(chunk, (value) => socket.write(encodeFrame(value))); }
            catch (error) { finish(error as Error); }
        };
        socket.once("connect", () => {
            process.stdin.on("data", receive);
            process.stdin.resume();
        });
        socket.on("data", (chunk: Buffer) => {
            try { output.push(chunk, (value) => process.stdout.write(encodeFrame(value))); }
            catch (error) { finish(error as Error); }
        });
        socket.once("error", finish);
        socket.once("close", () => finish());
        process.stdin.once("end", () => {
            try { input.finish(); finish(); }
            catch (error) { finish(error as Error); }
        });
    });
    return true;
}
