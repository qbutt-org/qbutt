import { appendFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import type { Request, Work } from "../../components/content/contracts";

const logPath = process.env.QBUTT_READER_UI_FIXTURE_LOG!;
const imagesDir = process.env.QBUTT_READER_UI_FIXTURE_IMAGES!;
const aheadCounts = new Map<string, number>();
const held = new Map<number, string>();
const corrections = new Map<string, string>();
const positions = new Map<string, { chapterId: string; offset?: number; imageIndex?: number }>([
    ["positioned", { chapterId: "positioned:chapter", offset: 0.95, imageIndex: 1 }],
]);

function log(event: string, data: Record<string, unknown>): void {
    appendFileSync(logPath, `${JSON.stringify({ event, ...data })}\n`);
}

function emit(id: number, event: string, data?: unknown): void {
    process.stdout.write(`${JSON.stringify({ id, event, data })}\n`);
}

function page(documentId: string, chapterId: string, translated: boolean): Record<string, unknown> {
    if (documentId === "volumes") {
        return { id: chapterId, documentId, title: "Generated chapter", kind: "text", cached: true,
            html: `<p>Original text of ${chapterId}.</p>` };
    }
    if (documentId === "text-position") {
        return { id: chapterId, documentId, title: "Legal generated prose", kind: "text", cached: true,
            html: Array.from({ length: 140 }, (_, index) => `<p>${index + 1}. The traveller followed the river through the valley.</p>`).join("") };
    }
    if (documentId === "hangul") {
        return { id: chapterId, documentId, title: "Original Korean chapter", kind: "text", cached: true,
            html: translated ? "<p>Вместе мы найдём новый мир.</p>" : "<p>우리가 함께라면 새로운 세상을 찾을 수 있습니다.</p>",
            translated };
    }

    const secondChapter = chapterId === `${documentId}:chapter-2`;
    if (!secondChapter && chapterId !== `${documentId}:chapter`)
        throw new Error(`Unexpected Reader lifecycle fixture chapter: ${chapterId}`);

    const imageNames = documentId === "small-image" ? ["small"]
        : secondChapter ? ["landscape", "portrait"] : ["portrait", "landscape", "tall"];
    return {
        id: chapterId,
        documentId,
        title: documentId === "small-image" ? "Small legal generated image"
            : secondChapter ? "Two legal generated pages" : "Three legal generated pages",
        kind: "images",
        cached: true,
        images: imageNames.map((name, index) => ({
            id: `image-${index}`,
            url: pathToFileURL(join(imagesDir, `${name}.png`)).href,
            ...(translated ? { cleanedUrl: pathToFileURL(join(imagesDir, `cleaned-${name}.png`)).href } : {}),
        })),
        ...(translated ? {
            translated: true,
            translatedImages: imageNames.map((_, index) => index),
            regions: imageNames.map((_, index) => ({
                id: `region-${index}`,
                imageIndex: index,
                text: "Original source text",
                translation: corrections.get(`${documentId}:${chapterId}:region-${index}`) ?? "Исходный перевод",
                x: 0.25, y: 0.2, width: 0.5, height: 0.3,
                layout: { x: 0.2, y: 0.15, width: 0.6, height: 0.4 },
                orientation: "horizontal",
                ink: "dark",
            })),
        } : {}),
    };
}

async function handle({ id, method, params = {} }: Request): Promise<void> {
    const documentId = String(params.documentId ?? (params.work as Work | undefined)?.id ?? "");
    log("request", { id, method, documentId, params });
    if (method === "reader.open") {
        if (documentId === "volumes") {
            const chapters = Array.from({ length: 121 }, (_, index) => ({
                id: `volumes:${index}`,
                title: index === 21 ? "Chapter 1 — The long journey beyond the mountains and the distant city by the sea, "
                    + "where the travellers followed the old river through the mist and discovered a forgotten library "
                    + "whose maps described every road they had taken and every place they still hoped to reach"
                    : index % 20 ? `Chapter ${index % 20}` : "Prologue",
                ...(index < 120 ? { volume: Math.floor(index / 20) + 1 } : {}),
            }));
            emit(id, "complete", { id: documentId, sourceId: "fixture-reader", title: "Generated multivolume novel",
                kind: "text", chapters, position: positions.get(documentId) });
            return;
        }
        const chapters = [{ id: `${documentId}:chapter`, title: "Cached original chapter" }];
        if (documentId !== "hangul" && documentId !== "small-image")
            chapters.push({ id: `${documentId}:chapter-2`, title: "Second cached original chapter" });

        const title = documentId === "pager" ? "Путешествие к далёкому городу, которого нет ни на одной карте" : documentId;
        emit(id, "complete", { id: documentId, workId: documentId, sourceId: "fixture-reader", title,
            kind: ["hangul", "text-position"].includes(documentId) ? "text" : "images", chapters, position: positions.get(documentId) });
    } else if (method === "reader.chapter") {
        const chapterId = (params.chapter as { id: string }).id;
        if (chapterId === "volumes:19") {
            emit(id, "error", { message: "Generated chapter is unavailable." });
            return;
        }
        const original = page(documentId, chapterId, false);
        log("original", { documentId, page: original });
        if (documentId === "text-position")
            emit(id, "page", original);
        emit(id, "complete", original);
    } else if (method === "reader.ahead") {
        const count = (aheadCounts.get(documentId) ?? 0) + 1;
        aheadCounts.set(documentId, count);
        emit(id, "page", page(documentId, String(params.chapterId), true));
        if (count === 1 && documentId !== "hangul") {
            held.set(id, documentId);
            log("held", { id, documentId });
        } else {
            await Bun.sleep(20);
            emit(id, "complete", { processed: 2, processingSeconds: 0.02 });
            log("ahead-complete", { id, documentId });
        }
    } else if (method === "reader.correct") {
        const chapterId = String(params.pageId);
        corrections.set(`${documentId}:${chapterId}:${String(params.regionId)}`, String(params.text));
        const corrected = page(documentId, chapterId, true);
        log("corrected", { documentId, page: corrected });
        emit(id, "complete", corrected);
    } else if (method === "cancel") {
        const target = Number(params.id);
        if (held.has(target)) {
            emit(target, "complete", { processed: 1, processingSeconds: 0.01 });
            log("cancelled-ahead", { id: target, documentId: held.get(target) });
            held.delete(target);
        }
        emit(id, "complete");
    } else if (method === "reader.position") {
        const position = {
            chapterId: String(params.chapterId),
            ...(typeof params.offset === "number" ? { offset: params.offset } : {}),
            ...(typeof params.imageIndex === "number" ? { imageIndex: params.imageIndex } : {}),
        };
        positions.set(documentId, position);
        emit(id, "complete", position);
    } else if (method === "library") {
        emit(id, "library", []);
        emit(id, "complete");
    } else if (method === "jobs") {
        emit(id, "jobs", []);
        emit(id, "complete");
    } else {
        throw new Error(`Unexpected Reader lifecycle fixture method: ${method}`);
    }
}

let buffer = "";
const decoder = new TextDecoder();
const tasks = new Set<Promise<void>>();
for await (const chunk of Bun.stdin.stream()) {
    buffer += decoder.decode(chunk, { stream: true });
    let end: number;
    while ((end = buffer.indexOf("\n")) >= 0) {
        const request = JSON.parse(buffer.slice(0, end)) as Request;
        buffer = buffer.slice(end + 1);
        const task = handle(request).catch(error => emit(request.id, "error", { message: String(error) }));
        tasks.add(task);
        void task.finally(() => tasks.delete(task));
    }
}
await Promise.all(tasks);
