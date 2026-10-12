import { dirname, join, resolve } from "node:path";

import { BrowserBridge, type BrowserChoice } from "./browser";
import { Catalog } from "./catalog";
import { maxMessageBytes, type Request, type ResolvedContent, type SearchRequest, type Variant, type Work } from "./contracts";
import { DownloadQueue } from "./downloads";
import { resolveVariant } from "./media";
import { publicUrl, safeError } from "./net";
import { NetworkProxy } from "./networkproxy";
import { productExecutable } from "./products";
import { handleReaderRequest } from "./reader/service";
import { assertExactVariant, linkId, publicReference } from "./references";
import { readingOriginal } from "./providers/reading-originals";
import { readingPublication } from "./providers/reading";
import { yaniPublication } from "./providers/video/yani";
import { mergeVariants, mergeWorkMetadata, rankVariants } from "./results";
import { SourceSettings } from "./settings";
import { openLink, share } from "./sharing";
import { assertLibraryEntrySize, LibraryStore, type VariantFilters } from "./store";
import { PlaybackStreams } from "./streams";
import { handleSubtitleRequest } from "./subtitles/service";

const persistenceMethods = new Set([
    "configureSource", "remember", "progress", "remove",
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
    const downloadDir = resolve(argument("--download-dir") || join(process.env.USERPROFILE || dataDir, "Downloads"));
    const store = await LibraryStore.open(dataDir);
    const settings = new SourceSettings(store);
    const browserSettings = await settings.values("browser");
    const browser = new BrowserBridge({
        browser: (browserSettings.browser || "auto") as BrowserChoice,
        emit: (event, data) => { void send(0, event, data); },
    });
    const catalog = new Catalog(dataDir, appDir, settings, browser);
    const mediaNetwork = await NetworkProxy.start();
    const playback = new PlaybackStreams();
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

    const downloads = new DownloadQueue({
        dataDir, downloadDir, appDir, store,
        resolve: (selected: Variant, signal: AbortSignal) => resolveVariant(selected, catalog, signal, browser, mediaNetwork.url),
        emit: (event: string, data: unknown) => send(0, event, data),
    });

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
        await Promise.all([downloads.close(), browser.close(), playback.close()]);
        await Promise.allSettled(requests.map(request => request.task));
        await mediaNetwork.close();
        store.close();
    }

    async function dispatch(method: string, params: Record<string, unknown>, signal: AbortSignal,
        emit: (event: string, data: unknown) => void | Promise<void>): Promise<unknown> {
        if (method.startsWith("reader.") || method.startsWith("subtitles.")) {
            const context = await catalog.context(typeof params.sourceId === "string" ? params.sourceId
                : (params.variant as Variant | undefined)?.sourceId || "reader", signal);
            if (method === "reader.translate" || method === "reader.ahead" || method === "reader.correct"
                || method === "subtitles.ahead") {
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
            if (method.startsWith("subtitles.")) {
                const track = params.track as Record<string, unknown> | undefined;
                const original = playback.originalSubtitle(params.url, track?.url);
                const request = original ? { ...params, url: original.url, headers: original.headers,
                    track: { ...track, url: original.subtitleUrl } } : params;
                return handleSubtitleRequest(method, request, context, emit);
            }
            return handleReaderRequest(method, params, context, emit);
        }

        switch (method) {
        case "search": {
            const query = String(params.query || "").trim();
            if (query.startsWith("qbutt:")) {
                return dispatch("openLink", { url: query }, signal, emit);
            }
            if (/^https?:\/\//i.test(query)) {
                const address = await publicUrl(query);
                const anime = yaniPublication(address);
                if (anime) {
                    emit("result", await catalog.inspect(anime, signal));
                    return;
                }
                const original = readingPublication(address) || readingOriginal(address.href);
                if (original) {
                    emit("result", await catalog.inspect(original, signal));
                    return;
                }
                const video = /(?:^|\.)(?:youtube\.com|youtu\.be|vk\.com|vkvideo\.ru|rutube\.ru|x\.com|twitter\.com)$/.test(address.hostname)
                    || /\.(?:m3u8|mpd|mp4|mkv|webm)(?:$|\?)/i.test(address.href);
                if (video || /\.torrent$/i.test(address.pathname)) {
                    const title = decodeURIComponent(address.pathname.split("/").filter(Boolean).at(-1) || address.hostname);
                    const id = linkId(query);
                    await settings.configure(`acquisition:${id}`, undefined, { url: query });
                    const selected: Variant = { id, sourceId: "link", sourceName: address.hostname,
                        title, url: query, transport: video ? "video" : "torrent" };
                    emit("result", { id, title, category: video ? "movies" : "all", variants: [selected] });
                    return;
                }
                return dispatch("discover", { url: query }, signal, emit);
            }
            if (query.startsWith("magnet:")) {
                if (!new URL(query).searchParams.getAll("xt").some(hash => /^urn:bt(?:ih|mh):[a-z\d]+$/i.test(hash)))
                    throw new Error("This magnet link has no torrent hash.");
                const id = linkId(query);
                await settings.configure(`acquisition:${id}`, undefined, { url: query });
                emit("result", { id, title: new URL(query).searchParams.get("dn") || "Torrent",
                    category: "all", variants: [{ id, sourceId: "link", sourceName: "Magnet",
                        title: new URL(query).searchParams.get("dn") || "Torrent", transport: "torrent", url: query, magnet: query }] });
                return;
            }
            await catalog.search({ ...params, query, category: params.category || "all" } as SearchRequest, signal, emit);
            return;
        }
        case "sources":
            emit("sources", await catalog.sources(signal));
            return;
        case "checkSource": {
            const id = String(params.id || "");
            if (!id || id.length > 128) throw new Error("Select a source.");
            const inspected = await catalog.checkSource(id, signal, {
                interactive: params.interactive === true,
                resolve: (selected, options) => resolveVariant(selected, catalog, signal, browser, mediaNetwork.url, options),
            });
            if (inspected) {
                const latest = catalog.latestWork({ ...inspected.work, variants: [inspected.publication] });
                emit("details", {
                    ...inspectedWork(latest, inspected.work, inspected.publication,
                        typeof params.language === "string" ? params.language : undefined, latest),
                    inspectedPublication: { sourceId: inspected.publication.sourceId,
                        id: inspected.publication.id, url: inspected.publication.url },
                });
            }
            emit("sources", await catalog.sources(signal));
            return;
        }
        case "configureSource": {
            const id = String(params.id || "");
            if (!id || id.length > 128)
                throw new Error("Select a source.");
            const changes = params.settings ? Object.fromEntries(Object.entries(object(params.settings))
                .map(([key, value]) => [key, String(value)])) : undefined;
            if (id === "browser" && changes?.browser && !["auto", "edge", "chrome", "firefox"].includes(changes.browser))
                throw new Error("Select an installed browser.");
            if (catalog.providers.some(provider => provider.source.id === id) || id === "ai" || id === "browser") {
                await settings.configure(id, typeof params.enabled === "boolean" ? params.enabled : undefined, changes);
                if (id === "browser")
                    browser.setBrowser((changes?.browser || "auto") as BrowserChoice);
            }
            else
                await catalog.engine.request("configureSource", { ...params, settings: changes }, () => {}, signal);
            emit("sources", await catalog.sources(signal));
            return;
        }
        case "refresh":
        case "discover":
            await catalog.engine.request(method === "refresh" ? "update" : "discover", params,
                event => {
                    if (event.event !== "sources")
                        emit(event.event, event.data);
                }, signal);
            emit("sources", await catalog.sources(signal));
            return;
        case "browserStatus":
            emit("browser", { ...await browser.status(signal, true), qcatchInstalled: !!await productExecutable("qcatch", appDir) });
            return;
        case "connectBrowser": {
            const choice = String(params.browser || "auto");
            if (!["auto", "edge", "chrome", "firefox"].includes(choice))
                throw new Error("Select an installed browser.");
            await settings.configure("browser", undefined, { browser: choice });
            browser.setBrowser(choice as BrowserChoice);
            emit("browser", await browser.connectBrowser(choice as BrowserChoice, signal));
            return;
        }
        case "inspect": {
            const selected = variant(params.variant);
            const current = params.work ? work(params.work) : undefined;
            if (selected.sourceId === "link" && current) {
                emit("details", current);
                return;
            }
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
        case "inspectWork": {
            let current = work(params.work);
            const language = typeof params.language === "string" ? params.language : undefined;
            const publications = [...new Map(current.variants.filter(item => item.needsInspection)
                .map(item => [`${item.sourceId}:${item.id}`, item])).values()];
            let next = 0;
            const worker = async () => {
                while (next < publications.length && !signal.aborted) {
                    const selected = publications[next++];
                    const sourceSignal = AbortSignal.any([signal, AbortSignal.timeout(45_000)]);
                    emit("source", { sourceId: selected.sourceId, status: "searching" });
                    const showDetails = (details: Work) => {
                        sourceSignal.throwIfAborted();
                        current = inspectedWork(current, details, selected, language, catalog.latestWork(current));
                        emit("details", { ...current,
                            inspectedPublication: { sourceId: selected.sourceId, id: selected.id, url: selected.url } });
                    };
                    try {
                        showDetails(await catalog.inspect(selected, sourceSignal, showDetails));
                        emit("source", { sourceId: selected.sourceId, status: "complete" });
                    }
                    catch (error) {
                        if (!signal.aborted)
                            emit("source", { sourceId: selected.sourceId, status: "error", message: safeError(error) });
                    }
                }
            };
            await Promise.all(Array.from({ length: Math.min(3, publications.length) }, worker));
            signal.throwIfAborted();
            return;
        }
        case "resolve": {
            const selected = variant(params.variant);
            const resolved = await resolveVariant(selected, catalog, signal, browser, mediaNetwork.url);
            emit("resolved", await playback.prepare(resolved, await catalog.context(selected.sourceId, signal, selected.url)));
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
        case "remove":
            store.remove(String(params.id || ""));
            return;
        case "share": {
            const selectedWork = work(params.work);
            const selected = params.variant ? variant(params.variant) : undefined;
            const variants = params.mode === "work" ? selectedWork.variants : selected ? [selected] : [];
            for (const candidate of variants.filter(item => item.sourceId === "link" && !item.url.startsWith("magnet:"))) {
                const saved = await settings.values(`acquisition:${candidate.id}`);
                const original = saved.url || candidate.url;
                if (publicReference(original) !== new URL(original).href)
                    throw new Error("This temporary media address cannot be shared. Share its source page instead.");
            }
            emit("share", { url: share({ mode: params.mode === "work" ? "work" : "exact", work: selectedWork,
                variant: selected }) });
            return;
        }
        case "openLink": {
            const item = openLink(String(params.url || ""));
            if (item.action) {
                await publicUrl(item.variant!.url);
                emit("details", { ...item.work, captureAction: item.action });
                return;
            }
            if (item.mode === "work" && !item.work.variants.length) {
                emit("query", { query: item.work.title, category: item.work.category });
                return;
            }
            const preferred = item.variant;
            if (!preferred) {
                emit("details", item.work);
                return;
            }
            if (preferred.sourceId === "link") {
                if (!preferred.url.startsWith("magnet:"))
                    await publicUrl(preferred.url);
                emit("details", { ...item.work, preferredVariant: { id: preferred.id, sourceId: preferred.sourceId } });
                return;
            }
            const details = await catalog.inspect(preferred, signal);
            const exact = details.variants.find(candidate => candidate.id === preferred.id
                && candidate.sourceId === preferred.sourceId);
            if (!exact)
                throw new Error("The shared version is no longer listed by its source. Search for another version explicitly.");
            assertExactVariant(preferred, exact);
            const otherPublications = item.mode === "work" ? item.work.variants.filter(candidate =>
                candidate.sourceId !== exact.sourceId || candidate.url !== exact.url) : [];
            emit("details", { ...details, ...(item.mode === "work" ? { id: item.work.id, title: item.work.title,
                category: item.work.category } : {}),
                preferredVariant: { id: exact.id, sourceId: exact.sourceId },
                variants: [exact, ...details.variants.filter(candidate => candidate !== exact), ...otherPublications] });
            return;
        }
        case "jobs":
            emit("jobs", downloads.jobs());
            return;
        case "download": {
            const request = params.resolved
                ? { ...params, resolved: playback.original(object(params.resolved) as unknown as ResolvedContent) } : params;
            const job = downloads.enqueue(request);
            emit("job", job);
            return;
        }
        case "cancelDownload":
            downloads.cancel(String(params.id || ""));
            return;
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
