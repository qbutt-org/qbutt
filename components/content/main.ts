import { dirname, join, resolve } from "node:path";

import { BrowserBridge } from "./browser";
import { Catalog } from "./catalog";
import { maxMessageBytes, type Request, type SearchRequest, type Variant, type Work } from "./contracts";
import { safeError } from "./net";
import { readingOriginal } from "./providers/reading-originals";
import { handleReaderRequest } from "./reader/service";
import { mergeVariants, mergeWorkMetadata, rankVariants } from "./results";
import { SourceSettings } from "./settings";
import { assertLibraryEntrySize, LibraryStore, type VariantFilters } from "./store";

const persistenceMethods = new Set([
    "configureSource", "remember", "progress",
    "reader.position", "reader.glossary", "reader.correct",
]);

function argument(name: string): string | undefined {
    const index = Bun.argv.indexOf(name);
    return index === -1 ? undefined : Bun.argv[index + 1];
}

function object(value: unknown): Record<string, unknown> {
    if (!value || typeof value !== "object" || Array.isArray(value))
        throw new Error("The request is incomplete.");
    return value as Record<string, unknown>;
}

function variant(value: unknown): Variant {
    const item = object(value) as unknown as Variant;
    if (!item.id || !item.sourceId || !item.title || typeof item.url !== "string"
        || !["torrent", "video", "book", "manga", "file"].includes(item.transport))
        throw new Error("Select an available variant.");
    return item;
}

function work(value: unknown): Work {
    const item = object(value) as unknown as Work;
    if (!item.id || !item.title || !Array.isArray(item.variants)
        || !["all", "games", "movies", "tv", "anime", "software", "books", "manga"].includes(item.category))
        throw new Error("Select a title.");
    return item;
}

function variantFilters(value: unknown): VariantFilters {
    const filters = object(value);
    const result: VariantFilters = {};
    for (const key of ["translation", "language"] as const) {
        if (typeof filters[key] === "string") result[key] = filters[key];
    }
    if (filters.episode) {
        const episode = object(filters.episode);
        if (typeof episode.episode === "string" && typeof episode.season === "string")
            result.episode = { episode: episode.episode, season: episode.season };
    }
    if (filters.source) {
        const source = object(filters.source);
        if (typeof source.sourceId === "string" && typeof source.player === "string"
            && ["torrent", "video", "book", "manga", "file"].includes(String(source.transport)))
            result.source = { sourceId: source.sourceId, player: source.player,
                transport: source.transport as Variant["transport"] };
    }
    return result;
}

function inspectedWork(current: Work | undefined, details: Work, selected: Variant, language?: string, latest?: Work): Work {
    const refreshed = new Set(details.variants.map(item => `${item.sourceId}:${item.id}`));
    let metadata = mergeWorkMetadata(current, details);
    if (latest) {
        metadata = mergeWorkMetadata(details, mergeWorkMetadata(current, latest));
    }
    else if (current && details.variants.every(item => item.publicationKind === "upload" || item.transport === "torrent")
        && current.variants.some(item => item.publicationKind !== "upload" && item.transport !== "torrent")) {
        metadata = mergeWorkMetadata(details, current);
    }
    const variants = mergeVariants([
        ...latest?.variants.filter(item => !item.needsInspection || !current?.variants.some(ready =>
            !ready.needsInspection && ready.sourceId === item.sourceId && ready.url === item.url)) || [],
        ...current?.variants || [],
    ]);
    return {
        ...metadata,
        id: latest?.id || current?.id || details.id,
        category: details.category === "all" && current ? current.category : metadata.category,
        variants: rankVariants([
            ...variants.filter(item => (item.sourceId !== selected.sourceId || item.url !== selected.url)
                && !refreshed.has(`${item.sourceId}:${item.id}`)),
            ...details.variants,
        ], language, metadata),
    };
}

async function run(): Promise<void> {
    const dataDir = resolve(argument("--data-dir") || join(process.env.LOCALAPPDATA || process.cwd(), "qbutt", "content"));
    const appDir = resolve(argument("--app-dir") || dirname(process.execPath));
    const store = await LibraryStore.open(dataDir);
    const settings = new SourceSettings(store);
    const browser = new BrowserBridge({
        emit: (event, data) => { void send(0, event, data); },
    });
    const catalog = new Catalog(dataDir, appDir, settings, browser);
    const pending = new Map<number, { controller: AbortController; method: string; task: Promise<void> }>();
    let closing = false;
    let outputBytes = 0;

    const send = (id: number, event: string, data?: unknown) => {
        if (closing)
            return Promise.resolve();
        const line = JSON.stringify({ id, event, ...(data === undefined ? {} : { data }) }) + "\n";
        const size = Buffer.byteLength(line);
        if (size > maxMessageBytes || outputBytes + size > 64 * 1024 * 1024) {
            void shutdown();
            return Promise.resolve();
        }
        outputBytes += size;
        return new Promise<void>(resolve => {
            process.stdout.write(line, () => {
                outputBytes -= size;
                resolve();
            });
        });
    };

    async function shutdown(): Promise<void> {
        if (closing)
            return;
        closing = true;
        process.stdin.pause();
        const requests = [...pending.values()];
        for (const request of requests) {
            if (!persistenceMethods.has(request.method))
                request.controller.abort();
        }
        await Promise.allSettled(requests.filter(request => persistenceMethods.has(request.method))
            .map(request => request.task));
        catalog.close();
        await browser.close();
        await Promise.allSettled(requests.map(request => request.task));
        store.close();
    }

    async function dispatch(method: string, params: Record<string, unknown>, signal: AbortSignal,
        emit: (event: string, data: unknown) => void | Promise<void>): Promise<unknown> {
        if (method.startsWith("reader.")) {
            const context = await catalog.context(typeof params.sourceId === "string" ? params.sourceId
                : (params.variant as Variant | undefined)?.sourceId || "reader", signal);
            if (method === "reader.translate" || method === "reader.ahead" || method === "reader.correct") {
                const ai = await settings.values("ai");
                context.settings = {
                    ...context.settings,
                    aiProvider: ai.provider || "auto", aiModel: ai.model || "",
                    aiApiKey: ai.apiKey || "", aiBaseUrl: ai.baseUrl || "",
                };
            }
            if (method === "reader.open" && params.work) {
                const current = work(params.work);
                params = { ...params, work: store.find(current)?.work || current };
            }
            return handleReaderRequest(method, params, context, emit);
        }

        switch (method) {
        case "search": {
            const query = String(params.query || "").trim();
            if (/^https?:\/\//i.test(query)) {
                const publication = readingOriginal(query);
                if (!publication) throw new Error("This link is not supported.");
                emit("result", await catalog.inspect(publication, signal));
                return;
            }
            if (/^(?:qbutt:|magnet:)/i.test(query))
                throw new Error("This link is not supported.");
            await catalog.search({ ...params, query, category: params.category || "all" } as SearchRequest, signal, emit);
            return;
        }
        case "configureSource": {
            const id = String(params.id || "");
            if (id !== "ai") throw new Error("Select an available source.");
            const changes = Object.fromEntries(Object.entries(object(params.settings))
                .map(([key, value]) => [key, String(value)]));
            await settings.configure(id, undefined, changes);
            return;
        }
        case "inspect": {
            const selected = variant(params.variant);
            const current = params.work ? work(params.work) : undefined;
            const showDetails = (details: Work) => {
                signal.throwIfAborted();
                emit("details", {
                    ...inspectedWork(current, details, selected, typeof params.language === "string" ? params.language : undefined,
                        catalog.latestWork(current)),
                    inspectedPublication: { sourceId: selected.sourceId, id: selected.id, url: selected.url },
                });
            };
            showDetails(await catalog.inspect(selected, signal, showDetails));
            return;
        }
        case "library":
            for (const entry of store.entries(String(params.mode || "all"))) {
                signal.throwIfAborted();
                assertLibraryEntrySize(entry);
                await emit("library", [entry]);
            }
            await emit("library", []);
            return;
        case "remember":
            return store.remember(work(params.work), {
                ...(params.variant ? { variant: variant(params.variant) } : {}),
                ...(params.variantFilters ? { variantFilters: variantFilters(params.variantFilters) } : {}),
                ...(typeof params.favorite === "boolean" ? { favorite: params.favorite } : {}),
                ...(params.opened === true ? { lastOpened: new Date().toISOString() } : {}),
            });
        case "progress": {
            const progress: Record<string, string | number> = {};
            for (const [key, value] of Object.entries(params)) {
                if (key !== "id" && (typeof value === "string" || (typeof value === "number" && Number.isFinite(value))))
                    progress[key] = value;
            }
            store.update(String(params.id || ""), { progress });
            return;
        }
        default:
            throw new Error("This action is not supported.");
        }
    }

    function receive(input: string): void {
        let request: Request;
        try {
            const message = JSON.parse(input);
            if (message.browserReply) {
                browser.receive(message.browserReply);
                return;
            }
            request = message;
            if (!Number.isSafeInteger(request.id) || request.id <= 0 || typeof request.method !== "string"
                || pending.has(request.id))
                throw new Error("Invalid request.");
        }
        catch {
            void shutdown();
            return;
        }

        if (request.method === "cancel") {
            pending.get(Number(request.params?.id))?.controller.abort();
            send(request.id, "complete");
            return;
        }
        if (pending.size >= 128) {
            send(request.id, "error", { message: "Too many actions are running. Wait for them to finish." });
            return;
        }

        const controller = new AbortController();
        const task = dispatch(request.method, request.params || {}, controller.signal,
            (event, data) => send(request.id, event, data)).then(result => {
            controller.signal.throwIfAborted();
            send(request.id, "complete", result);
        }).catch(error => {
            send(request.id, "error", { message: safeError(error),
                ...(typeof error?.code === "string" ? { code: error.code } : {}),
                ...(typeof error?.url === "string" ? { url: error.url } : {}) });
        }).finally(() => {
            pending.delete(request.id);
        });
        pending.set(request.id, { controller, method: request.method, task });
    }

    let input = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", chunk => {
        input += chunk;
        let end: number;
        while (!closing && (end = input.indexOf("\n")) !== -1) {
            const line = input.slice(0, end);
            input = input.slice(end + 1);
            if (Buffer.byteLength(line) > maxMessageBytes) {
                void shutdown();
                return;
            }
            if (line.trim())
                receive(line);
        }
        if (Buffer.byteLength(input) > maxMessageBytes)
            void shutdown();
    });
    process.stdin.on("end", () => void shutdown());
    process.on("SIGINT", () => void shutdown());
    process.on("SIGTERM", () => void shutdown());
}

void run().catch(() => { process.exitCode = 1; });
