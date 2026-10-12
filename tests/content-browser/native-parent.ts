import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { BrowserBridge, type BrowserChoice } from "../../components/content/browser";
import type { BrowserResponse } from "../../components/content/browser/protocol";

// This is a real Qt/Rust parent process, not a second JavaScript broker.
export class NativeBrowserFixture extends BrowserBridge {
    private readonly parent;
    private readonly exited: Promise<number | null>;
    private readonly ready: Promise<void>;
    private generation = 1;
    onSessionReply?: (generation: number, response: BrowserResponse) => void;

    constructor(options: { browser?: BrowserChoice; discoveryPath?: string }) {
        const executable = process.env.QBUTT_QUICK_ACCEPTANCE_EXE;
        assert(executable, "Set QBUTT_QUICK_ACCEPTANCE_EXE to the built native acceptance host");
        const discovery = options.discoveryPath ?? process.env.QBUTT_BROWSER_BRIDGE_DISCOVERY;
        assert(discovery, "An isolated browser discovery path is required");
        const parent = spawn(executable, ["--browser-relay-host"], {
            windowsHide: true,
            env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery,
                QT_QPA_PLATFORM: "offscreen", QT_QUICK_BACKEND: "software",
                QT_COMMAND_LINE_PARSER_NO_GUI_MESSAGE_BOXES: "1" },
        });
        super({ browser: options.browser, emit: (event, data) => {
            parent.stdin.write(JSON.stringify({ generation: this.generation, event, data }) + "\n");
        } });
        this.parent = parent;
        this.exited = new Promise(resolve => parent.once("exit", resolve));
        this.ready = new Promise((resolve, reject) => {
            parent.once("error", reject);
            parent.once("exit", () => reject(new Error("Native browser fixture stopped before readiness")));
            let input = "";
            parent.stdout.setEncoding("utf8");
            parent.stdout.on("data", (chunk: string) => {
                input += chunk;
                assert(Buffer.byteLength(input) <= 2 * 768 * 1024, "Native fixture output exceeds the limit");
                let end: number;
                while ((end = input.indexOf("\n")) >= 0) {
                    const message = JSON.parse(input.slice(0, end));
                    input = input.slice(end + 1);
                    if (message.ready)
                        resolve();
                    else if (message.generation === this.generation && message.browserReply)
                        this.receive(message.browserReply);
                    else if (message.browserReply)
                        this.onSessionReply?.(message.generation, message.browserReply);
                }
            });
        });
        parent.stderr.resume();
    }

    async start(initialize = true): Promise<void> {
        let timer: ReturnType<typeof setTimeout>;
        try {
            await Promise.race([this.ready, new Promise<never>((_, reject) => {
                timer = setTimeout(() => reject(new Error("Native fixture startup timed out")), 15_000);
            })]);
        }
        finally {
            clearTimeout(timer!);
        }
        if (initialize)
            this.forward(1, "browserInitialize");
        await this.status(undefined, initialize);
    }

    forward(generation: number, event: string, data: unknown = {}): void {
        this.parent.stdin.write(JSON.stringify({ generation, event, data }) + "\n");
    }

    async close(): Promise<void> {
        await super.close();
        this.parent.stdin.end();
        let timer: ReturnType<typeof setTimeout>;
        try {
            const result = await Promise.race([this.exited, new Promise<never>((_, reject) => {
                timer = setTimeout(() => {
                    this.parent.kill();
                    reject(new Error("Native browser fixture shutdown timed out"));
                }, 10_000);
            })]);
            assert.equal(result, 0, "Native fixture must drain the broker cleanly");
        }
        finally {
            clearTimeout(timer!);
        }
    }
}
