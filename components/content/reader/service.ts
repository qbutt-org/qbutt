import { mkdir, readFile, writeFile, rename, stat } from "node:fs/promises";
import { join, dirname } from "node:path";
import { pathToFileURL } from "node:url";
import { load } from "cheerio";
import type { ProviderContext, Variant } from "../contracts";
import { readingProviders, loadReadingChapter } from "../providers/reading";
import { openLocalDocument, readLocalChapter, type LocalDocument } from "./archive";
import { contentId, type ReaderDocument, type ReaderPage, type ReaderEvent, type ReaderPosition } from "./types";
import { translationText } from "./html";
import { translateBlocks, recognizePage } from "./translation";
import type { TextBlock, TranslationOptions } from "./translation";
import { observeReading, readAhead } from "./scheduler";
import { cleanImageText } from "./image-processing";

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

export async function mutateDocument<T>(context: ProviderContext, documentId: string, operation: () => Promise<T>): Promise<T> {
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

interface GlossaryState {
    entries: Record<string, string>;
    manual: Record<string, string | null>;
}

interface TranslationSnapshot {
    source: string;
    manual: string;
}

async function readGlossary(context: ProviderContext, documentId: string): Promise<GlossaryState> {
    const saved = await readJson<GlossaryState | Record<string, string>>(
        join(context.dataDir, "reader", "glossaries", documentId + ".json"));
    if (saved && typeof saved.entries === "object" && typeof saved.manual === "object") {
        return {
            entries: Object.assign(Object.create(null), saved.entries),
            manual: Object.assign(Object.create(null), saved.manual),
        };
    }
    const entries = Object.assign(Object.create(null), saved ?? {});
    return { entries, manual: Object.assign(Object.create(null), entries) };
}

function manualSnapshot(manual: GlossaryState["manual"], source?: string): string {
    return JSON.stringify(Object.fromEntries(Object.entries(manual)
        .filter(([term]) => source === undefined || source.includes(term))
        .sort(([left], [right]) => left.localeCompare(right))));
}

function translationPath(context: ProviderContext, original: ReaderPage, targetLanguage: string, manual: GlossaryState["manual"]): string {
    const id = contentId(JSON.stringify({ original, targetLanguage, manual: manualSnapshot(manual),
        provider: context.settings.aiProvider ?? "auto", model: context.settings.aiModel ?? "",
        baseUrl: context.settings.aiBaseUrl ?? "", pipeline: original.kind === "images" ? 6 : 3 }));
    return join(context.dataDir, "reader", "translations", id + ".json");
}

export async function translateCachedBlocks(documentId: string, blocks: TextBlock[], options: Omit<TranslationOptions, "glossary">,
    context: ProviderContext, onProgress: (data: unknown) => void): Promise<Awaited<ReturnType<typeof translateBlocks>> & { snapshot: TranslationSnapshot }> {
    const source = blocks.map(block => block.text).join("\n");
    const state = await readGlossary(context, documentId);
    const snapshot = { source, manual: manualSnapshot(state.manual, source) };
    const glossary: Record<string, string> = Object.create(null);
    const relevant = Object.entries(state.entries).filter(([term]) => source.includes(term))
        .sort(([left], [right]) => Number(Object.hasOwn(state.manual, right)) - Number(Object.hasOwn(state.manual, left))
            || left.localeCompare(right));
    for (const [term, target] of relevant) {
        glossary[term] = target;
        if (!Object.hasOwn(state.manual, term) && JSON.stringify(glossary).length > 16_000) {
            delete glossary[term];
        }
    }
    const id = contentId(JSON.stringify({ documentId, blocks, options, manual: snapshot.manual,
        provider: context.settings.aiProvider ?? "auto", model: context.settings.aiModel ?? "",
        baseUrl: context.settings.aiBaseUrl ?? "", pipeline: 2 }));
    const path = join(context.dataDir, "reader", "translated-blocks", id + ".json");
    const cached = await readJson<Awaited<ReturnType<typeof translateBlocks>>>(path);
    const result = cached ?? await translateBlocks(blocks, { ...options, glossary }, context, onProgress);
    if (!cached) {
        await saveJson(path, result);
    }
    await mutateDocument(context, documentId, async () => {
        const latest = await readGlossary(context, documentId);
        if (manualSnapshot(latest.manual, source) !== snapshot.manual) {
            throw new Error("Словарь изменился во время перевода. Повторите перевод: готовые фрагменты сохранены.");
        }
        let count = Object.keys(latest.entries).length;
        let changed = false;
        for (const { source, target } of result.terms) {
            if (count >= 2000 || Object.hasOwn(latest.entries, source) || Object.hasOwn(latest.manual, source)) {
                continue;
            }
            latest.entries[source] = target;
            ++count;
            changed = true;
        }
        if (changed) {
            await saveJson(join(context.dataDir, "reader", "glossaries", documentId + ".json"), latest);
        }
    });
    return { ...result, snapshot };
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
            observeReading(document.id, document.position);
            await saveJson(documentPath(context, document.id), document);
            return document.position;
        });
    }
    if (method === "reader.glossary") {
        const document = await documentFor(context, params.documentId);
        if (params.entries !== undefined) {
            const entries = params.entries as Record<string, unknown>;
            if (!entries || typeof entries !== "object" || Array.isArray(entries) || Object.entries(entries).some(([name, value]) => !name.trim()
                || name.length > 200 || typeof value !== "string" || value.length > 500) || Object.keys(entries).length > 2000) {
                throw new Error("Некорректный словарь произведения.");
            }
        }
        return mutateDocument(context, document.id, async () => {
            const state = await readGlossary(context, document.id);
            if (params.entries !== undefined) {
                const entries = params.entries as Record<string, string>;
                let changed = false;
                for (const [source, target] of Object.entries(entries)) {
                    if (!Object.hasOwn(state.entries, source) || state.entries[source] !== target) {
                        state.manual[source] = target;
                        changed = true;
                    }
                }
                for (const source of Object.keys(state.entries)) {
                    if (!Object.hasOwn(entries, source)) {
                        state.manual[source] = null;
                        changed = true;
                    }
                }
                if (Object.keys(state.manual).length > 4000) {
                    throw new Error("Словарь достиг предела ручных изменений для этого произведения.");
                }
                if (changed) {
                    state.entries = Object.assign(Object.create(null), entries);
                    await saveJson(join(context.dataDir, "reader", "glossaries", document.id + ".json"), state);
                }
            }
            return state.entries;
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
    if (method === "reader.ahead") {
        const document = await documentFor(context, params.documentId);
        const chapterId = typeof params.chapterId === "string" ? params.chapterId : document.position?.chapterId ?? document.chapters[0]?.id;
        if (!chapterId || !document.chapters.some(chapter => chapter.id === chapterId)) {
            throw new Error("Глава отсутствует в этой книге.");
        }
        const observation: ReaderPosition = { chapterId,
            offset: typeof params.offset === "number" && Number.isFinite(params.offset) ? Math.min(1, Math.max(0, params.offset)) : 0,
            imageIndex: typeof params.imageIndex === "number" ? Math.max(0, Math.floor(params.imageIndex)) : undefined };
        return readAhead(document, observation, context.signal,
            (id, signal) => handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: id }, { ...context, signal }, onEvent) as Promise<ReaderPage>,
            (page, imageIndex, signal) => handleReaderRequest("reader.translate", { page, imageIndex, targetLanguage: params.targetLanguage },
                { ...context, signal }, onEvent) as Promise<ReaderPage>, onEvent);
    }
    if (method === "reader.translate" || method === "reader.correct") {
        const input = params.page as ReaderPage | undefined;
        const document = await documentFor(context, params.documentId ?? input?.documentId);
        const pageId = safeId(params.pageId ?? input?.id);
        const original = await readJson<ReaderPage>(pagePath(context, document.id, pageId));
        if (!original) {
            throw new Error("Сначала откройте главу.");
        }
        const targetLanguage = typeof params.targetLanguage === "string" ? params.targetLanguage : "ru";
        const correctionsPath = join(context.dataDir, "reader", "corrections", contentId(JSON.stringify({ original, targetLanguage })) + ".json");
        const initial = await mutateDocument(context, document.id, async () => {
            const state = await readGlossary(context, document.id);
            const path = translationPath(context, original, targetLanguage, state.manual);
            const cached = await readJson<ReaderPage>(path);
            const corrections = await readJson<Record<string, string>>(correctionsPath) ?? {};
            for (const region of cached?.regions ?? []) {
                region.translation = corrections[region.id] ?? region.translation;
            }
            if (method === "reader.correct") {
                const region = cached?.regions?.find(entry => entry.id === params.regionId);
                if (!cached || !region || typeof params.text !== "string" || !params.text.trim() || params.text.length > 10_000) {
                    throw new Error("Фрагмент перевода не найден.");
                }
                region.translation = params.text.trim();
                corrections[region.id] = region.translation;
                await saveJson(correctionsPath, corrections);
                await saveJson(path, cached);
            }
            return { manual: state.manual, cached };
        });
        const { cached } = initial;
        if (method === "reader.correct") {
            return cached;
        }
        const imageIndex = typeof params.imageIndex === "number" ? Math.floor(params.imageIndex) : 0;
        if (cached && (original.kind === "text" || cached.translatedImages?.includes(imageIndex))) {
            return { ...cached, cached: true };
        }
        const page: ReaderPage = cached ?? { ...original, original: { html: original.html, images: original.images }, translated: true };
        const snapshots: TranslationSnapshot[] = [];
        if (cached?.regions?.length) {
            const source = cached.regions.map(region => region.text).join("\n");
            snapshots.push({ source, manual: manualSnapshot(initial.manual, source) });
        }
        const progress = (data: unknown) => onEvent("progress", data);
        if (original.kind === "text") {
            const text = translationText(original.html ?? "");
            const translated: { id: string; text: string }[] = [];
            let start = 0;
            while (start < text.blocks.length) {
                let end = start;
                let characters = 0;
                while (end < text.blocks.length && end - start < 160 && characters < 24_000) {
                    characters += text.blocks[end++]!.text.length;
                }
                const batch = text.blocks.slice(start, end);
                const preceding = translated.slice(-2).map(block => ({ original: text.blocks[Number(block.id)]?.text.slice(-2000),
                    translation: block.text.slice(-2000) }));
                const result = await translateCachedBlocks(document.id, batch, { targetLanguage, context: JSON.stringify(preceding) }, context, progress);
                translated.push(...result.blocks);
                snapshots.push(result.snapshot);
                page.provider = result.provider;
                page.model = result.model;
                start = end;
                onEvent("progress", { message: "Перевод главы", completed: start, total: text.blocks.length });
            }
            page.html = text.apply(translated);
        }
        else {
            const image = original.images?.[imageIndex];
            if (!image) {
                throw new Error("Страница не найдена.");
            }
            const ocrPath = join(context.dataDir, "reader", "ocr", contentId(await readFile(image.url)) + ".layout-v2.json");
            const ocr = await readJson<Awaited<ReturnType<typeof recognizePage>>>(ocrPath)
                ?? await recognizePage(image.url, {}, context, progress);
            await saveJson(ocrPath, ocr);
            if (ocr.regions.length) {
                onEvent("progress", { phase: "cleaning" });
                const layer = await cleanImageText(image.url, ocr.regions, context);
                page.images = [...page.images ?? original.images ?? []];
                page.images[imageIndex] = { ...image, cleanedUrl: layer.cleanedUrl, width: layer.width, height: layer.height };
                const result = await translateCachedBlocks(document.id, ocr.regions.map(region => ({ id: region.id, text: region.text })),
                    { targetLanguage }, context, progress);
                snapshots.push(result.snapshot);
                const translated = new Map(result.blocks.map(block => [block.id, block.text]));
                page.regions = [...page.regions ?? [], ...ocr.regions.map(({ inkBoxes: _, ...region }) => ({ ...region, imageIndex,
                    id: `${imageIndex}:${region.id}`, translation: translated.get(region.id)! }))];
                page.provider = result.provider;
                page.model = result.model;
            }
            page.translatedImages = [...page.translatedImages ?? [], imageIndex];
        }
        return mutateDocument(context, document.id, async () => {
            const latestGlossary = await readGlossary(context, document.id);
            if (snapshots.some(snapshot => manualSnapshot(latestGlossary.manual, snapshot.source) !== snapshot.manual)) {
                throw new Error("Словарь изменился во время перевода. Повторите перевод: готовые фрагменты сохранены.");
            }
            const path = translationPath(context, original, targetLanguage, latestGlossary.manual);
            if (original.kind === "images") {
                const latest = await readJson<ReaderPage>(path);
                page.images = (page.images ?? original.images ?? []).map((image, index) =>
                    page.translatedImages?.includes(index) ? image : latest?.images?.[index] ?? image);
                const regions = new Map((latest?.regions ?? []).map(region => [region.id, region]));
                for (const region of page.regions ?? []) {
                    regions.set(region.id, region);
                }
                page.regions = [...regions.values()];
                page.translatedImages = [...new Set([...latest?.translatedImages ?? [], ...page.translatedImages ?? []])].sort((left, right) => left - right);
            }
            const corrections = await readJson<Record<string, string>>(correctionsPath) ?? {};
            for (const region of page.regions ?? []) {
                region.translation = corrections[region.id] ?? region.translation;
            }
            await saveJson(path, page);
            return page;
        });
    }
    throw new Error("Неизвестная команда читалки.");
}
