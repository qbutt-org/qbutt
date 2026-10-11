import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { zipSync, strToU8 } from "../../components/content/node_modules/fflate";
import { handleReaderRequest } from "../../components/content/reader/service";
import type { ProviderContext } from "../../components/content/contracts";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";

function require(condition: unknown, message: string): asserts condition {
    if (!condition) throw new Error(message);
}

const root = await mkdtemp(join(tmpdir(), "qbutt-reading-scheduler-"));
let delay = 20;
let started: (() => void) | undefined;
let active = 0;
let concurrent = 0;
let calls = 0;
const server = Bun.serve({
    hostname: "127.0.0.1", port: 0,
    async fetch(request) {
        require(request.headers.get("authorization") === "Bearer legal-fixture-key", "API authorization missing");
        const body = await request.json() as any;
        ++calls;
        concurrent = Math.max(concurrent, ++active);
        started?.();
        started = undefined;
        try {
            await new Promise<void>(resolve => {
                const timer = setTimeout(resolve, delay);
                request.signal.addEventListener("abort", () => { clearTimeout(timer); resolve(); }, { once: true });
            });
            const schema = body.response_format.json_schema.schema;
            const prompt = body.messages[1].content;
            const value = schema.properties.regions
                ? { regions: [{ id: "region-1", text: "猫は庭にいます。", x: 0.1, y: 0.1, width: 0.7, height: 0.4,
                    layout: { x: 0.1, y: 0.1, width: 0.7, height: 0.4 }, orientation: "horizontal", ink: "dark",
                    inkBoxes: [{ x: 0.1, y: 0.1, width: 0.7, height: 0.4 }] }] }
                : { blocks: JSON.parse(prompt).blocks.map((block: { id: string; text: string }) => ({ id: block.id, text: `Перевод ${block.text}` })), terms: [] };
            return Response.json({ choices: [{ finish_reason: "stop", message: { content: JSON.stringify(value) } }] });
        }
        finally {
            --active;
        }
    },
});
const context: ProviderContext = {
    appDir: process.env.QBUTT_CONTENT_APP_DIR,
    signal: new AbortController().signal,
    dataDir: join(root, "profile"),
    settings: { aiProvider: "openai-compatible", aiBaseUrl: `http://127.0.0.1:${server.port}/v1`, aiApiKey: "legal-fixture-key", aiModel: "fixture-translator" },
    fetch,
};
const pages: ReaderPage[] = [];
const onEvent = (event: string, data: unknown) => {
    if (event === "page") pages.push(data as ReaderPage);
};

try {
    const archive: Record<string, Uint8Array> = {
        "META-INF/container.xml": strToU8('<container><rootfile full-path="book.opf"/></container>'),
    };
    const chapters = Array.from({ length: 12 }, (_, index) => `chapter${index}`);
    archive["book.opf"] = strToU8(`<package><metadata><title>Legal scheduler fixture</title></metadata><manifest>${chapters.map(id => `<item id="${id}" href="${id}.html"/>`).join("")}</manifest><spine>${chapters.map(id => `<itemref idref="${id}"/>`).join("")}</spine></package>`);
    for (const [index, id] of chapters.entries()) archive[`${id}.html`] = strToU8(`<p>凛は庭に入りました。${index}</p>`);
    const path = join(root, "story.epub");
    await writeFile(path, zipSync(archive));
    const document = await handleReaderRequest("reader.local", { path }, context) as ReaderDocument;
    const first = document.chapters[0]!.id;
    const initial = await handleReaderRequest("reader.ahead", { documentId: document.id, chapterId: first, offset: 0.1 }, context, onEvent) as { processed: number };
    require(initial.processed === 1 && pages.length === 1, "Unread distant chapters were translated unnecessarily");
    pages.length = 0;
    const nearEnd = await handleReaderRequest("reader.ahead", { documentId: document.id, chapterId: first, offset: 0.99 }, context, onEvent) as { processed: number };
    require(nearEnd.processed >= 2 && nearEnd.processed <= 3 && pages.some(page => page.id === document.chapters[1]!.id), "Approaching the end did not prefetch the next chapter");
    require(concurrent === 1, "Ahead translation used concurrent AI jobs");

    delay = 600;
    pages.length = 0;
    const changingContext = { ...context, settings: { ...context.settings, aiModel: "fixture-observation" } };
    const beginning = new Promise<void>(resolve => { started = resolve; });
    const moving = handleReaderRequest("reader.ahead", { documentId: document.id, chapterId: first, offset: 0 }, changingContext, onEvent);
    await beginning;
    await new Promise(resolve => setTimeout(resolve, 250));
    await handleReaderRequest("reader.position", { documentId: document.id, chapterId: first, offset: 0.99 }, changingContext);
    await moving;
    require(pages.some(page => page.id === document.chapters[1]!.id), "Active queue ignored the real reading position");

    const image = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a0u8AAAAASUVORK5CYII="), value => value.charCodeAt(0));
    const comicPath = join(root, "comic.cbz");
    await writeFile(comicPath, zipSync(Object.fromEntries(chapters.map((_, index) => [`page${index}.png`, image]))));
    const comic = await handleReaderRequest("reader.local", { path: comicPath }, context) as ReaderDocument;
    pages.length = 0;
    const recognizing = new Promise<void>(resolve => { started = resolve; });
    const seeking = handleReaderRequest("reader.ahead", { documentId: comic.id, chapterId: comic.chapters[0]!.id, imageIndex: 0 }, context, onEvent);
    await recognizing;
    await new Promise(resolve => setTimeout(resolve, 250));
    await handleReaderRequest("reader.position", { documentId: comic.id, chapterId: comic.chapters[0]!.id, imageIndex: 9 }, context);
    const seekResult = await seeking as { processed: number };
    require(seekResult.processed <= 7 && pages.length > 0 && pages[0]!.translatedImages?.includes(9)
        && !pages[0]!.translatedImages?.includes(0), "Seek did not cancel obsolete OCR and prioritize the visible page");

    delay = 5000;
    const cancellation = new AbortController();
    const cancellableContext = { ...context, signal: cancellation.signal, settings: { ...context.settings, aiModel: "fixture-cancel" } };
    const connecting = new Promise<void>(resolve => { started = resolve; });
    const pending = handleReaderRequest("reader.ahead", { documentId: document.id, chapterId: first, offset: 0.99 }, cancellableContext);
    await connecting;
    const count = calls;
    const cancelledAt = performance.now();
    cancellation.abort();
    let cancelled = false;
    try { await pending; }
    catch { cancelled = true; }
    require(cancelled && performance.now() - cancelledAt < 1500 && calls === count, "Cancellation left work running or started another translation");
    console.info("PASS bounded ahead window, one AI job, reading-position adaptation, obsolete OCR cancellation, queue cancellation");
}
finally {
    server.stop(true);
    await rm(root, { recursive: true, force: true });
}
