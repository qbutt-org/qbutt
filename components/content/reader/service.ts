import { mkdir, readFile, writeFile, rename, stat } from "node:fs/promises";
import { join, dirname } from "node:path";
import { pathToFileURL } from "node:url";
import { load } from "cheerio";
import type { ProviderContext, Variant } from "../contracts";
import { readingProviders, loadReadingChapter } from "../providers/reading";
import { openLocalDocument, readLocalChapter, type LocalDocument } from "./archive";
import { contentId, type ReaderDocument, type ReaderPage, type ReaderEvent } from "./types";

function safeId(value: unknown): string {
    if (typeof value !== "string" || !/^[a-f0-9]{64}$/.test(value)) {
        throw new Error("Некорректный идентификатор книги или главы.");
    }
    return value;
}

async function readJson<T>(path: string): Promise<T | undefined> {
    try {
        return JSON.parse(await readFile(path, "utf8"));
    }
    catch (error) {
        if ((error as NodeJS.ErrnoException).code === "ENOENT") {
            return undefined;
        }
        throw error;
    }
}

async function saveJson(path: string, value: unknown): Promise<void> {
    await mkdir(dirname(path), { recursive: true });
    const temporary = path + "." + crypto.randomUUID() + ".tmp";
    await writeFile(temporary, JSON.stringify(value), "utf8");
    await rename(temporary, path);
}

const documentMutations = new Map<string, Promise<void>>();
let pendingMutations = 0;

async function mutateDocument<T>(context: ProviderContext, documentId: string, operation: () => Promise<T>): Promise<T> {
    if (pendingMutations >= 128) {
        throw new Error("Слишком много изменений книги одновременно. Повторите действие.");
    }
    const key = documentPath(context, documentId);
    const previous = documentMutations.get(key) ?? Promise.resolve();
    const current = previous.then(() => {
        context.signal.throwIfAborted();
        return operation();
    });
    const settled = current.then(() => {}, () => {});
    documentMutations.set(key, settled);
    ++pendingMutations;
    try {
        return await current;
    }
    finally {
        --pendingMutations;
        if (documentMutations.get(key) === settled) {
            documentMutations.delete(key);
        }
    }
}

function documentPath(context: ProviderContext, id: string): string {
    return join(context.dataDir, "reader", "documents", safeId(id) + ".json");
}

function pagePath(context: ProviderContext, documentId: string, pageId: string): string {
    return join(context.dataDir, "reader", "pages", safeId(documentId), safeId(pageId) + ".json");
}

async function documentFor(context: ProviderContext, id: unknown): Promise<ReaderDocument | LocalDocument> {
    const document = await readJson<ReaderDocument | LocalDocument>(documentPath(context, safeId(id)));
    if (!document) {
        throw new Error("Откройте книгу заново.");
    }
    return document;
}

async function cacheImage(url: string, context: ProviderContext, headers?: Record<string, string>): Promise<string> {
    const directory = join(context.dataDir, "reader", "images");
    const key = contentId(url);
    const metadataPath = join(directory, key + ".json");
    const saved = await readJson<{ path: string }>(metadataPath);
    if (saved && await stat(saved.path).then(file => file.size > 0, () => false)) {
        return saved.path;
    }
    const response = await context.fetch(url, { headers, signal: context.signal });
    if (!response.ok) {
        throw new Error(`Не удалось загрузить страницу: HTTP ${response.status}.`);
    }
    const type = response.headers.get("content-type")?.split(";")[0] ?? "";
    const extension = ({ "image/jpeg": ".jpg", "image/png": ".png", "image/webp": ".webp", "image/gif": ".gif", "image/bmp": ".bmp" } as Record<string, string>)[type];
    if (!extension) {
        throw new Error("Источник вернул страницу вместо изображения.");
    }
    const limit = 32 * 1024 * 1024;
    if (Number(response.headers.get("content-length")) > limit || !response.body) {
        await response.body?.cancel();
        throw new Error("Изображение превышает допустимый размер.");
    }
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
        while (true) {
            context.signal.throwIfAborted();
            const { value, done } = await reader.read();
            if (done) {
                break;
            }
            size += value.byteLength;
            if (size > limit) {
                throw new Error("Изображение превышает допустимый размер.");
            }
            chunks.push(value);
        }
    }
    catch (error) {
        await reader.cancel().catch(() => {});
        throw error;
    }
    finally {
        reader.releaseLock();
    }
    if (!size) {
        throw new Error("Источник вернул пустое изображение.");
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
        bytes.set(chunk, offset);
        offset += chunk.byteLength;
    }
    await mkdir(directory, { recursive: true });
    const path = join(directory, key + extension);
    await writeFile(path, bytes);
    await saveJson(metadataPath, { path });
    return path;
}

async function cachePageImages(page: ReaderPage, sourceId: string, context: ProviderContext, onEvent: ReaderEvent): Promise<void> {
    const abort = new AbortController();
    const imageContext = { ...context, signal: AbortSignal.any([context.signal, abort.signal]) };
    const headers = sourceId === "mangalib" ? { Referer: "https://mangalib.me/", "Site-Id": "1" }
        : sourceId === "mangadex" ? { Referer: "https://mangadex.org/" }
        : sourceId === "nhentai" ? { Referer: "https://nhentai.net/" }
        : sourceId === "ranobelib" ? { Referer: "https://ranobelib.me/", "Site-Id": "3" } : undefined;
    const jobs: (() => Promise<void>)[] = [];
    for (const image of page.images ?? []) {
        jobs.push(async () => { image.url = await cacheImage(image.url, imageContext, headers); });
    }
    const $ = page.html ? load(page.html) : undefined;
    if ($) {
        $("img[src]").each((_, image) => {
            const url = $(image).attr("src")!;
            if (/^https?:/.test(url)) {
                jobs.push(async () => { $(image).attr("src", pathToFileURL(await cacheImage(url, imageContext, headers)).href); });
            }
        });
    }
    let next = 0;
    let completed = 0;
    const count = jobs.length;
    let failure: unknown;
    await Promise.allSettled(Array.from({ length: Math.min(3, count) }, async () => {
        try {
            while (next < jobs.length) {
                imageContext.signal.throwIfAborted();
                const job = jobs[next++]!;
                await job();
                onEvent("progress", { completed: ++completed, total: count, message: "Загрузка иллюстраций" });
            }
        }
        catch (error) {
            failure ??= error;
            abort.abort();
        }
    }));
    if (failure) {
        throw failure;
    }
    if ($) {
        page.html = $("body").html() ?? "";
    }
}

export async function handleReaderRequest(method: string, params: Record<string, unknown>, context: ProviderContext,
    onEvent: ReaderEvent = () => {}): Promise<unknown> {
    if (method === "reader.open" || method === "reader.local") {
        if (method === "reader.local") {
            if (typeof params.path !== "string") {
                throw new Error("Выберите файл книги.");
            }
            const document = await openLocalDocument(params.path, context.dataDir);
            return mutateDocument(context, document.id, async () => {
                const prior = await readJson<ReaderDocument>(documentPath(context, document.id));
                document.position = prior?.position;
                await saveJson(documentPath(context, document.id), document);
                return document;
            });
        }
        const variant = params.variant as Variant | undefined;
        const provider = readingProviders.find(entry => entry.source.id === variant?.sourceId);
        if (!variant || !provider?.inspect) {
            throw new Error("Этот источник не поддерживает чтение.");
        }
        const address = new URL(variant.url);
        const expectedHost = variant.sourceId === "syosetu" ? "ncode.syosetu.com" : new URL(provider.source.url).hostname;
        if (address.protocol !== "https:" || address.hostname !== expectedHost) {
            throw new Error("Адрес произведения не принадлежит выбранному источнику.");
        }
        const id = contentId(variant.id);
        const prior = await readJson<ReaderDocument>(documentPath(context, id));
        let document: ReaderDocument;
        try {
            const details = await provider.inspect(variant, context);
            document = { id, title: details.title, sourceId: variant.sourceId, chapters: details.chapters ?? [],
                workId: (params.work as { id?: string } | undefined)?.id ?? details.id };
        }
        catch (error) {
            if (!prior || context.signal.aborted) {
                throw error;
            }
            return mutateDocument(context, id, async () => {
                const latest = await readJson<ReaderDocument>(documentPath(context, id)) ?? prior;
                latest.workId = (params.work as { id?: string } | undefined)?.id ?? latest.workId;
                await saveJson(documentPath(context, id), latest);
                return latest;
            });
        }
        if (!document.chapters.length) {
            throw new Error("В этом произведении нет доступных глав.");
        }
        return mutateDocument(context, id, async () => {
            const latest = await readJson<ReaderDocument>(documentPath(context, id));
            document.position = latest?.position;
            await saveJson(documentPath(context, id), document);
            return document;
        });
    }
    if (method === "reader.position") {
        const id = safeId(params.documentId);
        return mutateDocument(context, id, async () => {
            const document = await documentFor(context, id);
            const chapterId = typeof params.chapterId === "string" ? params.chapterId : "";
            if (!document.chapters.some(chapter => chapter.id === chapterId)
                || (params.offset === undefined && params.imageIndex === undefined)
                || (params.offset !== undefined && (typeof params.offset !== "number" || !Number.isFinite(params.offset)))
                || (params.imageIndex !== undefined && (typeof params.imageIndex !== "number"
                    || !Number.isSafeInteger(params.imageIndex) || params.imageIndex < 0))) {
                throw new Error("Некорректная позиция чтения.");
            }
            document.position = { chapterId,
                ...(params.offset === undefined ? {} : { offset: Math.min(1, Math.max(0, params.offset as number)) }),
                ...(params.imageIndex === undefined ? {} : { imageIndex: params.imageIndex as number }) };
            await saveJson(documentPath(context, document.id), document);
            return document.position;
        });
    }
    if (method === "reader.chapter") {
        const document = await documentFor(context, params.documentId);
        const selected = document.chapters.find(chapter => chapter.id === (params.chapterId ?? (params.chapter as { id?: string })?.id));
        if (!selected) {
            throw new Error("Глава отсутствует в этой книге.");
        }
        const path = pagePath(context, document.id, selected.id);
        const cached = await readJson<ReaderPage>(path);
        if (cached) {
            return { ...cached, cached: true };
        }
        const page = document.local ? await readLocalChapter(document as LocalDocument, selected.id)
            : { ...await loadReadingChapter(document.sourceId, selected, context), id: selected.id, documentId: document.id };
        if (!document.local) {
            if (page.kind === "text") {
                onEvent("page", page);
            }
            await cachePageImages(page, document.sourceId, context, onEvent);
        }
        await saveJson(path, page);
        return page;
    }
    throw new Error("Неизвестная команда читалки.");
}
