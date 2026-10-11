import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import type { ProviderContext } from "../contracts";
import { mutateDocument, translateCachedBlocks } from "../reader/service";
import { TranslationError, type TextBlock } from "../reader/translation";
import { contentId } from "../reader/types";
import { loadSubtitleSource, subtitleRequest, subtitleSourceKey, type SubtitleRequest } from "./source";
import { parseTimedText, renderTimedText, type TimedTextCue, type TimedTextDocument } from "./timed-text";

interface SourceWindow {
    original: string;
    format: string;
    complete: boolean;
    start: number;
    end: number;
}

interface SubtitleCache {
    windows: Record<string, SourceWindow>;
    translations: Record<string, string>;
}

type SubtitleEvent = (event: string, data: unknown) => void;

async function readCache(path: string): Promise<SubtitleCache> {
    try {
        return JSON.parse(await readFile(path, "utf8"));
    }
    catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT")
            throw error;
        return { windows: {}, translations: {} };
    }
}

async function save(path: string, value: string, signal: AbortSignal): Promise<void> {
    const temporary = path + "." + crypto.randomUUID() + ".tmp";
    try {
        signal.throwIfAborted();
        await writeFile(temporary, value, "utf8");
        signal.throwIfAborted();
        await rename(temporary, path);
    }
    finally {
        await rm(temporary, { force: true });
    }
}

function identifiedCues(document: TimedTextDocument): { cue: TimedTextCue; key: string }[] {
    const occurrences = new Map<string, number>();
    return document.cues.map(cue => {
        const hash = contentId(JSON.stringify([cue.startMs, cue.endMs, cue.raw]));
        const ordinal = occurrences.get(hash) || 0;
        occurrences.set(hash, ordinal + 1);
        return { cue, key: `${hash}:${ordinal}` };
    });
}

function fragments(document: TimedTextDocument, start: number, end: number): TextBlock[] {
    const blocks = identifiedCues(document).filter(({ cue }) => cue.endMs > start * 1000 && cue.startMs < end * 1000)
        .flatMap(({ cue, key }) => cue.fragments.map((fragment, index) => ({ id: `${key}:${index}`, text: fragment.text })));
    if (blocks.length > 2048 || blocks.reduce((sum, block) => sum + block.text.length, 0) > 80_000)
        throw new Error("This subtitle window contains too much text to translate automatically.");
    return blocks;
}

function localTranslations(document: TimedTextDocument, translated: Record<string, string>): Map<string, string> {
    const result = new Map<string, string>();
    for (const { cue, key } of identifiedCues(document)) {
        cue.fragments.forEach((fragment, index) => {
            const value = translated[`${key}:${index}`];
            if (value !== undefined)
                result.set(fragment.id, value);
        });
    }
    return result;
}

function alreadyRussian(request: SubtitleRequest, blocks: TextBlock[]): boolean {
    const language = request.track.language?.toLowerCase().split(/[-_]/)[0];
    if (language === "ru" || language === "rus")
        return true;
    if (language && language !== "und" && language !== "unknown")
        return false;
    const text = blocks.map(block => block.text).join(" ").slice(0, 12_000);
    const letters = text.match(/\p{L}/gu)?.length || 0;
    const russian = text.match(/[А-Яа-яЁё]/g)?.length || 0;
    return letters > 0 && russian / letters > 0.85 && !/[ІіЇїЄєҐґЎў]/.test(text);
}

async function translatedAsset(directory: string, cache: SubtitleCache): Promise<{ text: string; format: string }> {
    const windows = Object.values(cache.windows).sort((left, right) => left.start - right.start);
    const first = windows[0];
    if (!first)
        throw new Error("No subtitle window was cached.");
    const firstDocument = parseTimedText(await readFile(join(directory, first.original), "utf8"), first.format);
    if (first.complete) {
        return { text: renderTimedText(firstDocument, localTranslations(firstDocument, cache.translations)),
            format: firstDocument.format };
    }

    // Embedded windows are extracted as ASS. Keep the exact style/header data and
    // aggregate dialogue packets so replacing a track preserves already translated rewind.
    const header = firstDocument.source.slice(0, firstDocument.cues[0]?.sourceStart ?? firstDocument.source.length);
    const cues = new Map<string, { start: number; raw: string }>();
    for (const window of windows) {
        const document = parseTimedText(await readFile(join(directory, window.original), "utf8"), window.format);
        const rendered = parseTimedText(renderTimedText(document, localTranslations(document, cache.translations)), window.format);
        identifiedCues(document).forEach(({ cue: original, key }, index) => {
            if (original.endMs > window.start * 1000 && original.startMs < window.end * 1000) {
                cues.set(key, { start: original.startMs, raw: rendered.cues[index]!.raw });
            }
        });
    }
    const ordered = [...cues.values()].sort((left, right) => left.start - right.start);
    return { text: header + ordered.map(cue => cue.raw).join("\n") + "\n", format: "ass" };
}

async function publish(directory: string, documentId: string, request: SubtitleRequest, window: SourceWindow,
    additions: TextBlock[], context: ProviderContext, onEvent: SubtitleEvent): Promise<SubtitleCache> {
    return mutateDocument(context, documentId, async () => {
        const path = join(directory, "cache.json");
        const cache = await readCache(path);
        cache.windows[String(window.start)] = window;
        for (const block of additions)
            cache.translations[block.id] = block.text;
        const asset = await translatedAsset(directory, cache);
        if (Buffer.byteLength(asset.text) > 16 * 1024 * 1024)
            throw new Error("The translated subtitle track is too large.");
        const revision = contentId(asset.text);
        const output = join(directory, `translated-${revision}.${asset.format}`);
        await save(output, asset.text, context.signal);
        await save(path, JSON.stringify(cache), context.signal);
        context.signal.throwIfAborted();
        onEvent("subtitle", { identity: request.identity, url: pathToFileURL(output).href,
            title: "Русский [AI-перевод]", language: "ru", revision, windowStart: window.start, windowEnd: window.end });
        return cache;
    });
}

export async function handleSubtitleRequest(method: string, params: Record<string, unknown>, context: ProviderContext,
    onEvent: SubtitleEvent = () => {}): Promise<unknown> {
    if (method !== "subtitles.ahead")
        throw new Error("Unknown subtitle request.");
    const request = subtitleRequest(params);
    const start = Math.floor(request.position / 60) * 60;
    const end = start + 120;
    const result = { identity: request.identity, windowStart: start, windowEnd: end };
    if (/^(?:hdmv_pgs_subtitle|pgs|dvd_subtitle|dvdsub|dvb_subtitle|dvbsub|xsub)$/i.test(request.track.codec || "")) {
        return { ...result, status: "unsupported", message: "This video contains image subtitles. Choose a text subtitle track to translate it." };
    }
    const sourceId = await subtitleSourceKey(request);
    const documentId = contentId(JSON.stringify({ sourceId, target: request.targetLanguage,
        provider: context.settings.aiProvider || "auto", model: context.settings.aiModel || "",
        endpoint: context.settings.aiBaseUrl || "", pipeline: 2 }));
    const directory = join(context.dataDir, "subtitles", documentId);
    await mkdir(directory, { recursive: true });
    let cache = await readCache(join(directory, "cache.json"));
    let window = cache.windows[String(start)];
    if (!window) {
        const source = await loadSubtitleSource(request, start, end, context);
        const document = parseTimedText(source.text, source.formatHint || undefined);
        const original = `original-${contentId(source.text)}.${document.format}`;
        await save(join(directory, original), source.text, context.signal);
        window = { original, format: document.format, complete: source.complete, start, end };
    }
    const document = parseTimedText(await readFile(join(directory, window.original), "utf8"), window.format);
    const blocks = fragments(document, start, end);
    if (alreadyRussian(request, blocks))
        return { ...result, status: "already-target" };
    let pending = blocks.filter(block => cache.translations[block.id] === undefined);
    if (!pending.length) {
        await publish(directory, documentId, request, window, [], context, onEvent);
        return { ...result, status: "ready" };
    }
    try {
        while (pending.length) {
            context.signal.throwIfAborted();
            const batch: TextBlock[] = [];
            let characters = 0;
            while (pending.length && batch.length < 48 && (characters < 6000 || !batch.length)) {
                const block = pending.shift()!;
                batch.push(block);
                characters += block.text.length;
            }
            const translated = await translateCachedBlocks(documentId, batch, {
                targetLanguage: "Russian",
                sourceLanguage: request.track.language,
                context: "These are timed film subtitles. Translate each fragment concisely and naturally for its on-screen duration. Preserve speaker meaning and consistent names. Formatting, timing, and subtitle control tags are managed separately.",
            }, context, data => onEvent("subtitleProgress", data));
            cache = await publish(directory, documentId, request, window, translated.blocks, context, onEvent);
            pending = pending.filter(block => cache.translations[block.id] === undefined);
        }
        return { ...result, status: "ready" };
    }
    catch (error) {
        context.signal.throwIfAborted();
        if (error instanceof TranslationError)
            return { ...result, status: "unavailable", message: error.message };
        throw error;
    }
}
