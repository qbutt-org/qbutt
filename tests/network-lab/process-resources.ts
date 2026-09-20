import assert from "node:assert/strict";
import { join } from "node:path";

interface ResourceTarget { role: "app" | "qbutt-net"; pid: number; executable: string }
interface ProcessResources extends ResourceTarget {
    creationTime100ns: string; // Exact Windows FILETIME, retained as a string.
    cpuMilliseconds: number;
    cpuOneCorePercent: number;
    sampledPeakWorkingSetBytes: number;
    sampledPeakPrivateBytes: number;
    processIo: { readOperations: number; writeOperations: number; otherOperations: number;
        readBytes: number; writeBytes: number; otherBytes: number };
}
export interface ResourceWindow {
    startedAt: string;
    finishedAt: string;
    elapsedMilliseconds: number;
    sampleIntervalMilliseconds: number;
    sampleCount: number;
    maximumSampleGapMilliseconds: number;
    logicalProcessors: number;
    processes: ProcessResources[];
    semantics: string;
    // Bounds measured in the runner's clock. Actual counter reads occur between
    // each command and its acknowledgement; they cannot be claimed simultaneous.
    boundaryBounds: { start: { sent: number; acknowledged: number }; stop: { sent: number; acknowledged: number } };
}

export async function prepareResourceSampler(python: string, targets: ResourceTarget[], root: string) {
    assert(process.platform === "win32", "The benchmark resource sampler requires Windows");
    const child = Bun.spawn([python, join(import.meta.dir, "process-resources.py"), JSON.stringify(targets)], {
        stdin: "pipe", stdout: "pipe", stderr: Bun.file(join(root, "process-resources.stderr.log")), windowsHide: true,
    });
    const reader = child.stdout.getReader();
    let buffer = "";
    let start: ResourceWindow["boundaryBounds"]["start"];
    async function read<T>(): Promise<T> {
        let timer: ReturnType<typeof setTimeout>;
        try {
            return await Promise.race([(async () => {
                for (;;) {
                    const newline = buffer.indexOf("\n");
                    if (newline >= 0) {
                        const line = buffer.slice(0, newline);
                        buffer = buffer.slice(newline + 1);
                        return JSON.parse(line) as T;
                    }
                    const chunk = await reader.read();
                    assert(!chunk.done, `Resource sampler exited before replying; inspect ${root}`);
                    buffer += new TextDecoder().decode(chunk.value);
                }
            })(), new Promise<never>((_, reject) => {
                timer = setTimeout(() => reject(new Error("Resource sampler response timed out")), 5000);
            })]);
        }
        finally { clearTimeout(timer!); }
    }
    async function command<T>(value: string) {
        const sent = performance.now();
        child.stdin.write(`${value}\n`);
        await child.stdin.flush();
        const response = await read<T>();
        return { response, bounds: { sent, acknowledged: performance.now() } };
    }
    async function close() {
        // A Windows venv executable can be a launcher. EOF reaches the actual
        // helper as well; killing only the launcher could leave it waiting.
        if (child.exitCode === null) child.stdin.end();
        await child.exited;
    }
    try {
        const ready = await read<{ ready: boolean }>();
        assert(ready.ready, "Resource sampler did not open the exact process identities");
    }
    catch (error) { await close(); throw error; }
    return {
        async start() {
            assert(!start, "Resource window has already started");
            const { response, bounds } = await command<{ started: boolean }>("start");
            assert(response.started, "Resource sampler did not start");
            start = bounds;
        },
        async stop(): Promise<ResourceWindow> {
            assert(start, "Resource window has not started");
            const { response, bounds } = await command<Omit<ResourceWindow, "boundaryBounds"> & { finished: boolean }>("stop");
            assert(response.finished && await child.exited === 0, "Resource sampler did not finish successfully");
            const { finished, ...window } = response;
            assert(window.elapsedMilliseconds > 0 && window.sampleCount >= 2, "Resource window is empty");
            return { ...window, boundaryBounds: { start, stop: bounds } };
        },
        close,
    };
}
