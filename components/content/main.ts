import { dirname, join, resolve } from "node:path";

import { BrowserBridge, runNativeMessaging, type BrowserChoice } from "./browser";
import { Catalog } from "./catalog";
import type { Request, ResolvedContent, SearchRequest, Variant, Work } from "./contracts";
import { DownloadQueue } from "./downloads";
import { resolveVariant } from "./media";
import { mediaTool } from "./media-runtime";
import { publicUrl, safeError } from "./net";
import { NetworkProxy } from "./networkproxy";
import { handleReaderRequest } from "./reader/service";
import { assertExactVariant, linkId, publicReference } from "./references";
import { readingOriginal } from "./providers/reading-originals";
import { yaniPublication } from "./providers/video/yani";
import { mergeWorkMetadata, rankVariants } from "./results";
import { SourceSettings } from "./settings";
import { openLink, share } from "./sharing";
import { LibraryStore } from "./store";
import { PlaybackStreams } from "./streams";
import { handleSubtitleRequest } from "./subtitles/service";

const maxMessageBytes = 16 * 1024 * 1024;
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

async function run(): Promise<void> {
    if (await runNativeMessaging(Bun.argv))
        return;

    const dataDir = resolve(argument("--data-dir") || join(process.env.LOCALAPPDATA || process.cwd(), "qbutt", "content"));
    const appDir = resolve(argument("--app-dir") || dirname(process.execPath));
    const downloadDir = resolve(argument("--download-dir") || join(process.env.USERPROFILE || dataDir, "Downloads"));
    const store = await LibraryStore.open(dataDir);
    const settings = new SourceSettings(store);
    const browserSettings = await settings.values("browser");
    const browser = new BrowserBridge({ browser: (browserSettings.browser || "auto") as BrowserChoice });
    await browser.start();
    const catalog = new Catalog(dataDir, appDir, settings, browser);
    const mediaNetwork = await NetworkProxy.start();
    const playback = new PlaybackStreams();
    const pending = new Map<number, { controller: AbortController; method: string; task: Promise<void> }>();
    let closing = false;
    let outputBytes = 0;

    const send = (id: number, event: string, data?: unknown) => {
        if (closing)
            return;
        const line = JSON.stringify({ id, event, ...(data === undefined ? {} : { data }) }) + "\n";
        const size = Buffer.byteLength(line);
        if (size > maxMessageBytes || outputBytes + size > 64 * 1024 * 1024) {
            void shutdown();
            return;
        }
        outputBytes += size;
        process.stdout.write(line, () => { outputBytes -= size; });
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
        emit: (event: string, data: unknown) => void): Promise<unknown> {
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
        case "mediaRuntime": {
            const [mpv, ffprobe, ytDlp] = await Promise.all([
                mediaTool(appDir, "mpv", signal),
                mediaTool(appDir, "ffprobe", signal),
                mediaTool(appDir, "yt-dlp", signal),
            ]);
            return { mpv, ffprobe, tools: dirname(ytDlp) };
        }
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
                const original = readingOriginal(address.href);
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
            emit("browser", browser.status());
            return;
        case "connectBrowser": {
            const choice = String(params.browser || "auto");
            if (!["auto", "edge", "chrome", "firefox"].includes(choice))
                throw new Error("Select an installed browser.");
            await settings.configure("browser", undefined, { browser: choice });
            browser.setBrowser(choice as BrowserChoice);
            emit("browser", await browser.connectBrowser(choice as BrowserChoice));
            return;
        }
        case "inspect": {
            const selected = variant(params.variant);
            const current = params.work ? work(params.work) : undefined;
            if (selected.sourceId === "link" && current) {
                emit("details", current);
                return;
            }
            const details = await catalog.inspect(selected, signal);
            const refreshed = new Set(details.variants.map(item => `${item.sourceId}:${item.id}`));
            const metadata = current && details.variants.every(item => item.publicationKind === "upload" || item.transport === "torrent")
                && current.variants.some(item => item.publicationKind !== "upload" && item.transport !== "torrent")
                ? mergeWorkMetadata(details, current) : mergeWorkMetadata(current, details);
            emit("details", {
                ...metadata,
                id: current?.id || details.id,
                category: details.category === "all" && current ? current.category : metadata.category,
                variants: rankVariants([
                    ...(current?.variants.filter(item => (item.sourceId !== selected.sourceId || item.url !== selected.url)
                        && !refreshed.has(`${item.sourceId}:${item.id}`)) || []),
                    ...details.variants,
                ]),
            });
            return;
        }
        case "resolve": {
            const selected = variant(params.variant);
            const resolved = await resolveVariant(selected, catalog, signal, browser, mediaNetwork.url);
            emit("resolved", await playback.prepare(resolved, await catalog.context(selected.sourceId, signal)));
            return;
        }
        case "library":
            emit("library", store.entries(String(params.mode || "all")).map(entry => ({ ...entry,
                work: { ...entry.work, variants: entry.variant ? [entry.variant] : entry.work.variants.slice(0, 1) },
            })));
            return;
        case "remember":
            return store.remember(work(params.work), {
                ...(params.variant ? { variant: variant(params.variant) } : {}),
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
            const selected = params.variant ? variant(params.variant) : undefined;
            if (params.mode !== "work" && selected?.sourceId === "link" && !selected.url.startsWith("magnet:")) {
                const saved = await settings.values(`acquisition:${selected.id}`);
                const original = saved.url || selected.url;
                if (publicReference(original) !== new URL(original).href)
                    throw new Error("This temporary media address cannot be shared. Share its source page instead.");
            }
            emit("share", { url: share({ mode: params.mode === "work" ? "work" : "exact", work: work(params.work),
                variant: selected }) });
            return;
        }
        case "openLink": {
            const item = openLink(String(params.url || ""));
            if (item.mode === "work") {
                emit("query", { query: item.work.title, category: item.work.category });
                return;
            }
            if (item.variant!.sourceId === "link") {
                if (!item.variant!.url.startsWith("magnet:"))
                    await publicUrl(item.variant!.url);
                emit("details", item.work);
                return;
            }
            const details = await catalog.inspect(item.variant!, signal);
            const exact = details.variants.find(candidate => candidate.id === item.variant!.id
                && candidate.sourceId === item.variant!.sourceId);
            if (!exact)
                throw new Error("The shared version is no longer listed by its source. Search for another version explicitly.");
            assertExactVariant(item.variant!, exact);
            emit("details", { ...details, preferredVariant: { id: exact.id, sourceId: exact.sourceId },
                variants: [exact, ...details.variants.filter(candidate => candidate !== exact)] });
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
            request = JSON.parse(input);
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
