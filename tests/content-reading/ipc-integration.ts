import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { zipSync } from "../../components/content/node_modules/fflate";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";
import { HelperBrowserSession } from "../content-browser/helper-parent";

function require(value: unknown, message: string): asserts value {
    if (!value) throw new Error(message);
}

const directory = await mkdtemp(join(tmpdir(), "qbutt-reader-ipc-"));
let delay = 26_000;
let redirects = false;
let redirectedRequests = 0;
let visionRequestBytes = 0;
let apiRequests = 0;
let started: (() => void) | undefined;

function waitForApiRequest(stage: string): Promise<void> {
    return new Promise((resolve, reject) => {
        const deadline = setTimeout(() => {
            started = undefined;
            reject(new Error(`AI request did not reach the fixture during ${stage}`));
        }, 45_000);
        started = () => { clearTimeout(deadline); resolve(); };
    });
}

const server = Bun.serve({
    hostname: "127.0.0.1", port: 0,
    async fetch(request) {
        if (new URL(request.url).pathname === "/redirected") {
            ++redirectedRequests;
            return new Response("Unexpected redirect");
        }
        require(!request.headers.has("cookie"), "Source cookies reached the translation service");
        require(request.headers.get("authorization") === "Bearer generated-reader-ipc-fixture", "Missing API authorization");
        const text = await request.text();
        const body = JSON.parse(text);
        ++apiRequests;
        if (body.response_format.json_schema.schema.properties.regions) {
            visionRequestBytes = Buffer.byteLength(text);
        }
        started?.();
        started = undefined;
        if (redirects) return Response.redirect(new URL("/redirected", request.url), 307);
        await new Promise<void>(resolve => {
            const timer = setTimeout(resolve, delay);
            request.signal.addEventListener("abort", () => { clearTimeout(timer); resolve(); }, { once: true });
        });
        const value = body.response_format.json_schema.schema.properties.regions
            ? { regions: [{ id: "region-1", text: "猫は庭にいます。", x: 0.1, y: 0.1, width: 0.7, height: 0.4,
                layout: { x: 0.1, y: 0.1, width: 0.7, height: 0.4 }, orientation: "horizontal", ink: "dark",
                inkBoxes: [{ x: 0.1, y: 0.1, width: 0.7, height: 0.4 }] }] }
            : { blocks: JSON.parse(body.messages[1].content).blocks.map((block: { id: string; text: string }) => ({
                id: block.id, text: "Кошка ждёт в саду.",
            })), terms: [] };
        return Response.json({ choices: [{ finish_reason: "stop", message: { content: JSON.stringify(value) } }] });
    },
});
const executable = process.env.QBUTT_CONTENT_BIN;
const browserParents = new Map<number, HelperBrowserSession>();
let onBrowserReply: (() => void) | undefined;
const appDirectory = resolve(process.env.QBUTT_CONTENT_APP_DIR || (executable ? dirname(executable) : resolve(import.meta.dir, "../..")));
function startService() {
    const handle = Bun.spawn({
        cmd: [...(executable ? [executable] : [process.execPath, "components/content/main.ts"]),
            "--data-dir", join(directory, "profile"), "--app-dir", appDirectory],
        stdin: "pipe", stdout: "pipe", stderr: "ignore", windowsHide: true,
        env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: join(directory, "browser.json") },
    });
    browserParents.set(handle.pid, new HelperBrowserSession(join(directory, `browser-${handle.pid}.json`), message => {
        handle.stdin.write(message);
        handle.stdin.flush();
        onBrowserReply?.();
    }));
    return handle;
}
const processHandle = startService();
let nextId = 0;
type Event = { id: number; event: string; data?: any };
async function* events(handle: ReturnType<typeof startService>): AsyncGenerator<Event> {
    let buffer = "";
    const decoder = new TextDecoder();
    for await (const chunk of handle.stdout) {
        buffer += decoder.decode(chunk, { stream: true });
        let end: number;
        while ((end = buffer.indexOf("\n")) >= 0) {
            const event = JSON.parse(buffer.slice(0, end));
            buffer = buffer.slice(end + 1);
            if (!browserParents.get(handle.pid)!.receive(event))
                yield event;
        }
    }
}
const stream = events(processHandle);
function send(method: string, params: Record<string, unknown> = {}): number {
    const id = ++nextId;
    processHandle.stdin.write(JSON.stringify({ id, method, params }) + "\n");
    return id;
}
async function receive(id: number, expectError = false, label = `request ${id}`): Promise<any> {
    let timer: ReturnType<typeof setTimeout>;
    const deadline = new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`Timed out waiting for ${label}`)), 45_000);
    });
    try {
        while (true) {
            const { value, done } = await Promise.race([stream.next(), deadline]);
            require(!done, "Content service ended before a terminal response");
            if (value.id !== id) continue;
            if (value.event === "error") {
                require(expectError, `Content service error: ${value.data?.message}`);
                return value.data;
            }
            if (value.event === "complete") {
                require(!expectError, "Expected a rejected request");
                return value.data;
            }
        }
    }
    finally {
        clearTimeout(timer!);
    }
}
const request = (method: string, params: Record<string, unknown> = {}, expectError = false, label = method) =>
    receive(send(method, params), expectError, label);
const configure = (model: string) => request("configureSource", { id: "ai", settings: {
    provider: "openai-compatible", baseUrl: `http://127.0.0.1:${server.port}/v1`,
    apiKey: "generated-reader-ipc-fixture", model,
} });

try {
    await configure("fixture-slow");
    const path = join(directory, "original.html");
    await writeFile(path, "<p>猫は庭にいます。</p>");
    const document = await request("reader.local", { path }) as ReaderDocument;
    const page = await request("reader.chapter", { documentId: document.id, chapterId: document.chapters[0]!.id }) as ReaderPage;
    const start = performance.now();
    const translated = await request("reader.translate", { page }) as ReaderPage;
    require(translated.html?.includes("Кошка") && performance.now() - start >= 25_000,
        "AI transport inherited the source's shorter deadline");

    delay = 0;
    await configure("fixture-vision");
    const imagePath = join(directory, "legal-image.png");
    const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a0u8AAAAASUVORK5CYII="), value => value.charCodeAt(0));
    await writeFile(imagePath, Buffer.concat([png, Buffer.alloc(1024 * 1024)]));
    const comic = await request("reader.local", { path: imagePath }) as ReaderDocument;
    const imagePage = await request("reader.chapter", { documentId: comic.id, chapterId: comic.chapters[0]!.id }) as ReaderPage;
    const recognized = await request("reader.translate", { page: imagePage }) as ReaderPage;
    require(recognized.regions?.length === 1, "Main IPC lost the translated OCR layer");
    require(visionRequestBytes > 1024 * 1024, "Vision fixture did not exercise a body beyond the source transport limit");
    const requestsBeforeCorrection = apiRequests;
    const correction = "Кошка терпеливо ждёт у ворот.";
    const corrected = await request("reader.correct", {
        documentId: comic.id, pageId: imagePage.id, regionId: recognized.regions[0]!.id, text: correction,
    }) as ReaderPage;
    require(corrected.regions?.[0]?.translation === correction,
        "Main IPC could not find the image translation cached under the configured provider");
    const cachedCorrection = await request("reader.translate", { page: imagePage }) as ReaderPage;
    require(cachedCorrection.cached && cachedCorrection.regions?.[0]?.translation === correction
        && apiRequests === requestsBeforeCorrection, "Image correction was lost or repeated an AI request");
    const originalImage = await request("reader.chapter", { documentId: comic.id, chapterId: comic.chapters[0]!.id }) as ReaderPage;
    require(!originalImage.regions?.length && originalImage.images?.[0]?.url === imagePage.images?.[0]?.url,
        "Correcting the translated overlay changed the original image layer");

    const cbzPath = join(directory, "three-pages.cbz");
    await writeFile(cbzPath, zipSync({ "1.png": png, "2.png": png, "3.png": png }));
    const pagedComic = await request("reader.local", { path: cbzPath }) as ReaderDocument;
    const position = await request("reader.position", {
        documentId: pagedComic.id, chapterId: pagedComic.chapters[0]!.id, imageIndex: 1,
    });
    require(position.imageIndex === 1 && !Object.hasOwn(position, "offset"),
        "Native image-only position was rejected or given a synthetic text offset");

    await configure("fixture-redirect");
    redirects = true;
    const beforeRedirect = apiRequests;
    const redirectError = await request("reader.translate", { page }, true, "redirect refusal");
    require(apiRequests === beforeRedirect + 1,
        `AI redirect refusal did not reach the fixture: ${redirectError?.message}`);
    require(redirectedRequests === 0, "An AI redirect forwarded credentials to another endpoint");
    redirects = false;

    await configure("fixture-cancel");
    delay = 30_000;
    const connecting = waitForApiRequest("cancellation");
    const id = send("reader.translate", { page });
    const cancellation = receive(id, true, "cancelled translation");
    await Promise.race([connecting, cancellation.then(outcome => {
        throw new Error(`AI request ended before cancellation: ${outcome?.message}`);
    })]);
    const cancelledAt = performance.now();
    send("cancel", { id });
    await cancellation;
    require(performance.now() - cancelledAt < 1500, "Main IPC cancellation left an AI request waiting");
    await request("search", { query: `http://127.0.0.1:${server.port}/private`, category: "all" }, true);

    await configure("fixture-eof");
    const active = waitForApiRequest("EOF shutdown");
    const activeId = send("reader.translate", { page });
    const finishedBeforeEof = receive(activeId, true, "EOF translation").then(outcome => {
        throw new Error(`AI request ended before EOF: ${outcome?.message}`);
    });
    await Promise.race([active, finishedBeforeEof]);
    // Both positions need broker context replies over stdin before the fixture closes it.
    let replies = 0;
    const positionsReady = new Promise<void>(resolve => {
        onBrowserReply = () => {
            if (++replies === 2) resolve();
        };
    });
    send("reader.position", { documentId: document.id, chapterId: document.chapters[0]!.id, offset: 0.83 });
    send("reader.position", { documentId: pagedComic.id, chapterId: pagedComic.chapters[0]!.id, imageIndex: 2, offset: 0.37 });
    await processHandle.stdin.flush();
    let replyTimer: ReturnType<typeof setTimeout>;
    try {
        await Promise.race([positionsReady, new Promise<never>((_, reject) => {
            replyTimer = setTimeout(() => reject(new Error("Reader position browser context did not finish")), 10_000);
        })]);
    }
    finally {
        clearTimeout(replyTimer!);
        onBrowserReply = undefined;
    }
    processHandle.stdin.end();
    const shutdownDeadline = setTimeout(() => processHandle.kill(), 5000);
    try {
        const exitCode = await processHandle.exited;
        require(exitCode === 0, `EOF failed to abort active AI and drain accepted reader persistence (exit ${exitCode})`);
    }
    finally {
        clearTimeout(shutdownDeadline);
    }

    const resumed = startService();
    const resumeDeadline = setTimeout(() => resumed.kill(), 5000);
    try {
        resumed.stdin.write(JSON.stringify({ id: 1, method: "reader.local", params: { path } }) + "\n");
        resumed.stdin.write(JSON.stringify({ id: 2, method: "reader.local", params: { path: cbzPath } }) + "\n");
        let reopened: ReaderDocument | undefined;
        let reopenedComic: ReaderDocument | undefined;
        for await (const event of events(resumed)) {
            if (event.id !== 1 && event.id !== 2) continue;
            require(event.event !== "error", `Reader restart failed: ${event.data?.message}`);
            if (event.event === "complete") {
                if (event.id === 1) reopened = event.data as ReaderDocument;
                else reopenedComic = event.data as ReaderDocument;
                if (reopened && reopenedComic) break;
            }
        }
        require(reopened?.position?.offset === 0.83 && reopened.position.chapterId === document.chapters[0]!.id,
            "EOF lost an accepted position while cancelling active AI");
        require(reopenedComic?.id === pagedComic.id && reopenedComic.position?.imageIndex === 2
            && reopenedComic.position.chapterId === pagedComic.chapters[0]!.id
            && reopenedComic.position.offset === 0.37,
            "EOF/restart lost the native CBZ image or its internal scroll offset");
    }
    finally {
        resumed.stdin.end();
        await resumed.exited;
        clearTimeout(resumeDeadline);
    }
    console.info("PASS real main IPC: explicit local AI, no cookies, >25s response, >1MB vision body, image correction/cache/original, redirects rejected, cancellation, public-source boundary, active AI + text/image positions + EOF + CBZ restart at image 3/3 and internal offset");
}
finally {
    if (processHandle.exitCode === null) processHandle.stdin.end();
    const timer = setTimeout(() => processHandle.kill(), 5000);
    await processHandle.exited;
    clearTimeout(timer);
    await Promise.all([...browserParents.values()].map(parent => parent.close()));
    server.stop(true);
    await rm(directory, { recursive: true, force: true });
}
