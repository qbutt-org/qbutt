import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import type { Event } from "./contracts";

interface PendingRequest {
    emit(event: Event): void;
    resolve(): void;
    reject(error: Error): void;
}

export class SearchEngine {
    private child?: ChildProcessWithoutNullStreams;
    private nextId = 0;
    private readonly pending = new Map<number, PendingRequest>();

    constructor(private readonly executable: string, private readonly dataDir: string) {}

    request(method: string, params: Record<string, unknown>, emit: (event: Event) => void,
        signal?: AbortSignal): Promise<void> {
        if (signal?.aborted)
            return Promise.reject(signal.reason);

        const child = this.start();
        const id = ++this.nextId;

        return new Promise((resolve, reject) => {
            const cancel = () => {
                child.stdin.write(JSON.stringify({ id: ++this.nextId, method: "cancel", params: { requestId: id } }) + "\n");
                this.pending.delete(id);
                reject(new DOMException("Cancelled", "AbortError"));
            };
            const finish = (error?: Error) => {
                signal?.removeEventListener("abort", cancel);
                if (error)
                    reject(error);
                else
                    resolve();
            };

            this.pending.set(id, { emit, resolve: () => finish(), reject: error => finish(error) });
            signal?.addEventListener("abort", cancel, { once: true });
            child.stdin.write(JSON.stringify({ id, method, params }) + "\n", error => {
                if (!error)
                    return;

                this.pending.delete(id);
                finish(new Error("The search engine stopped responding."));
            });
        });
    }

    close(): void {
        this.child?.stdin.end();
        this.child?.kill();
    }

    private start(): ChildProcessWithoutNullStreams {
        if (this.child)
            return this.child;

        const child = spawn(this.executable, ["--data-dir", this.dataDir], {
            windowsHide: true,
            stdio: ["pipe", "pipe", "pipe"],
        });
        this.child = child;
        child.stderr.resume();

        const fail = () => {
            if (this.child !== child)
                return;

            this.child = undefined;
            for (const request of this.pending.values())
                request.reject(new Error("The built-in search engine is unavailable."));
            this.pending.clear();
        };

        child.on("error", fail);
        child.on("exit", fail);

        let buffer = "";
        child.stdout.setEncoding("utf8");
        child.stdout.on("data", chunk => {
            buffer += chunk;
            if (buffer.length > 16 * 1024 * 1024) {
                child.kill();
                fail();
                return;
            }

            let newline: number;
            while ((newline = buffer.indexOf("\n")) !== -1) {
                const line = buffer.slice(0, newline);
                buffer = buffer.slice(newline + 1);

                let event: Event;
                try {
                    event = JSON.parse(line);
                }
                catch {
                    child.kill();
                    fail();
                    return;
                }

                const request = this.pending.get(event.id);
                if (!request)
                    continue;

                if (event.event === "complete" || event.event === "error") {
                    this.pending.delete(event.id);
                    if (event.event === "complete")
                        request.resolve();
                    else {
                        const data = event.data as { message?: string } | undefined;
                        request.reject(new Error(data?.message || "The source is unavailable."));
                    }
                }
                else
                    request.emit(event);
            }
        });

        return child;
    }
}
