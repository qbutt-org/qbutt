import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { join, resolve } from "node:path";
import type { Event, Work } from "../../components/content/contracts";

export class ContentClient {
    private readonly process: ChildProcessWithoutNullStreams;
    private readonly exited: Promise<number | null>;
    private readonly requests = new Map<number, {
        events: Event[];
        resolve(events: Event[]): void;
        reject(error: Error): void;
    }>();
    private sequence = 0;

    constructor(directory: string) {
        const appDir = resolve(process.env.QBUTT_CONTENT_APP_DIR || "C:/Temp/qbutt-content-build");
        const command = process.env.QBUTT_CONTENT_BIN
            ? [process.env.QBUTT_CONTENT_BIN] : [process.execPath, "components/content/main.ts"];
        this.process = spawn(command[0], [...command.slice(1), "--app-dir", appDir,
            "--data-dir", join(directory, "profile"), "--download-dir", join(directory, "downloads")], {
            windowsHide: true,
            stdio: ["pipe", "pipe", "pipe"],
            env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: join(directory, "browser.json") },
        });
        this.process.stderr.resume();
        this.exited = new Promise(resolve => this.process.once("exit", resolve));
        this.process.on("error", error => this.fail(error));
        this.process.on("exit", code => this.fail(new Error(`Content helper exited: ${code}`)));

        let buffer = "";
        this.process.stdout.setEncoding("utf8");
        this.process.stdout.on("data", chunk => {
            buffer += chunk;
            let end: number;
            while ((end = buffer.indexOf("\n")) !== -1) {
                const event = JSON.parse(buffer.slice(0, end)) as Event;
                buffer = buffer.slice(end + 1);
                const request = this.requests.get(event.id);
                if (!request) continue;
                request.events.push(event);
                if (event.event === "complete" || event.event === "error") {
                    this.requests.delete(event.id);
                    request.resolve(request.events);
                }
            }
        });
    }

    request(method: string, params: Record<string, unknown>, timeout = 120_000): Promise<Event[]> {
        const id = ++this.sequence;
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                this.requests.delete(id);
                reject(new Error(`Timed out: ${method}`));
            }, timeout);
            this.requests.set(id, {
                events: [],
                resolve: events => { clearTimeout(timer); resolve(events); },
                reject: error => { clearTimeout(timer); reject(error); },
            });
            this.process.stdin.write(JSON.stringify({ id, method, params }) + "\n");
        });
    }

    async close(): Promise<void> {
        this.process.stdin.end();
        const timeout = setTimeout(() => this.process.kill(), 10_000);
        const code = await this.exited;
        clearTimeout(timeout);
        assert.equal(code, 0, "Content helper must exit cleanly after EOF");
    }

    private fail(error: Error): void {
        for (const request of this.requests.values()) request.reject(error);
        this.requests.clear();
    }
}

export function completed(events: Event[]): void {
    assert(!events.some(event => event.event === "error"), JSON.stringify(events.at(-1)));
    assert.equal(events.filter(event => event.event === "complete").length, 1);
}

export function searchWorks(events: Event[]): Work[] {
    completed(events);
    const current = new Map<string, Work>();
    for (const event of events) {
        if (event.event === "result") {
            const work = event.data as Work;
            current.set(work.id, work);
        }
        else if (event.event === "removeResult") current.delete((event.data as { id: string }).id);
    }
    return [...current.values()];
}
