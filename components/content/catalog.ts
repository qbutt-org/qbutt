import { join } from "node:path";
import { CookieJar } from "tough-cookie";

import type { Provider, ProviderContext, SearchRequest, Source, Variant, Work } from "./contracts";
import type { BrowserBridge } from "./browser";
import { SearchEngine } from "./engine";
import { sourceFetch, safeError } from "./net";
import { readingProviders } from "./providers/reading";
import { gamingProviders } from "./providers/games";
import { videoProviders } from "./providers/video";
import { SearchResults } from "./results";
import { SourceSettings } from "./settings";

type Emit = (event: string, data: unknown) => void;

interface SourceSession {
    jar: CookieJar;
    browserCookies: { domain: string; path: string; key: string }[];
    refresh: Promise<void>;
}

export const translationSource: Source = {
    id: "ai",
    name: "Translation",
    url: "https://openrouter.ai",
    categories: [],
    enabled: true,
    settings: [
        { name: "provider", label: "Provider", type: "select", value: "auto", options: [
            { value: "auto", label: "Automatic" },
            { value: "codex", label: "Codex" },
            { value: "openrouter", label: "OpenRouter" },
            { value: "openai-compatible", label: "OpenAI compatible" },
        ] },
        { name: "apiKey", label: "API key", type: "password" },
        { name: "model", label: "Model", type: "text" },
        { name: "baseUrl", label: "API address", type: "text" },
    ],
};

export class Catalog {
    readonly engine: SearchEngine;
    readonly providers: Provider[] = [...videoProviders, ...gamingProviders, ...readingProviders];
    private readonly sessions = new Map<string, SourceSession>();

    constructor(readonly dataDir: string, readonly appDir: string, readonly settings: SourceSettings,
        private readonly browser: BrowserBridge) {
        this.engine = new SearchEngine(join(appDir, "qbutt-search.exe"), join(dataDir, "indexers"));
    }

    async sources(signal: AbortSignal): Promise<Source[]> {
        const engines = await this.indexers(signal);

        // The richer game adapters own publication variants and instructions.
        // Keep their simpler definitions in the engine catalog but expose only
        // one selectable source for the same site in the product.
        const replaced = new Set(["byrutor", "freetp", "onlinefix", "fitgirl", "torrentigruha"]);
        const native = await Promise.all(this.providers.map(provider => this.settings.describe(provider.source)));
        return [...native, ...engines.filter(source => !replaced.has(source.id)),
            await this.settings.describe(translationSource), await this.settings.describe({
                id: "browser", name: "Browser", url: "https://github.com/qbutt-org/qbutt", categories: [], enabled: true,
                settings: [{ name: "browser", label: "Browser", type: "select", value: "auto", options: [
                    { value: "auto", label: "Automatic" },
                    ...this.browser.status().browsers.map(browser => ({ value: browser.id, label: browser.name })),
                ] }],
            })];
    }

    private async indexers(signal: AbortSignal): Promise<Source[]> {
        let sources: Source[] = [];
        await this.engine.request("sources", {}, event => {
            if (event.event === "sources")
                sources = event.data as Source[];
        }, signal);
        return sources;
    }

    async browserSessions(signal: AbortSignal, selected?: string[]): Promise<{
        cookies: Record<string, string>; userAgents: Record<string, string>;
    }> {
        const cookies: Record<string, string> = {};
        const userAgents: Record<string, string> = {};
        const status = this.browser.status();
        if (!status.ready)
            return { cookies, userAgents };
        const sources = (await this.indexers(signal)).filter(source => selected
            ? selected.includes(source.id) : source.enabled || source.id === "rutracker");
        for (const source of sources) {
            const session = await this.browser.cookies(source.url, { signal });
            cookies[source.id] = session.map(cookie => `${cookie.name}=${cookie.value}`).join("; ");
            if (status.userAgent)
                userAgents[source.id] = status.userAgent;
        }
        return { cookies, userAgents };
    }

    async context(id: string, signal: AbortSignal): Promise<ProviderContext> {
        const source = this.providers.find(provider => provider.source.id === id)?.source;
        const defaults = Object.fromEntries((source?.settings || []).filter(setting => setting.value !== undefined)
            .map(setting => [setting.name, String(setting.value)]));
        const values = { ...defaults, ...await this.settings.values(id) };
        let session = this.sessions.get(id);
        if (!session) {
            session = { jar: new CookieJar(), browserCookies: [], refresh: Promise.resolve() };
            this.sessions.set(id, session);
        }

        if (source) {
            const current = session;
            current.refresh = current.refresh.catch(() => {}).then(async () => {
                signal.throwIfAborted();
                const cookies = values.cookie ? [] : await this.browser.cookies(source.url, { signal });
                for (const previous of current.browserCookies)
                    await current.jar.store.removeCookie(previous.domain, previous.path, previous.key);
                current.browserCookies = [];
                for (const cookie of cookies) {
                    const expires = cookie.expirationDate ? `; Expires=${new Date(cookie.expirationDate * 1000).toUTCString()}` : "";
                    const domain = cookie.hostOnly ? "" : `; Domain=${cookie.domain}`;
                    const stored = await current.jar.setCookie(`${cookie.name}=${cookie.value}${domain}; Path=${cookie.path}`
                        + (cookie.secure ? "; Secure" : "") + expires, source.url, { ignoreError: true });
                    if (stored)
                        current.browserCookies.push({ domain: stored.domain!, path: stored.path!, key: stored.key });
                }
            });
            await current.refresh;
        }

        return {
            dataDir: this.dataDir,
            appDir: this.appDir,
            signal,
            settings: values,
            fetch: sourceFetch(signal, session.jar, source && values.cookie
                ? { origin: new URL(values.baseUrl || source.url).origin, value: values.cookie } : undefined,
                this.browser.status().userAgent),
            saveSettings: changes => this.settings.configure(id, undefined, changes),
        };
    }

    async search(request: SearchRequest, signal: AbortSignal, emit: Emit): Promise<void> {
        const query = request.query?.trim();
        if (!query || query.length > 500)
            throw new Error("Enter a title or a link.");

        const results = new SearchResults(query, emit);
        const selected = request.sources?.length ? new Set(request.sources) : undefined;
        const providers: Provider[] = [];
        for (const provider of this.providers) {
            const source = await this.settings.describe(provider.source);
            if (!source.enabled || (selected && !selected.has(source.id)))
                continue;
            if (request.category !== "all" && !source.categories.includes(request.category))
                continue;
            providers.push(provider);
        }

        // Each adapter yields a complete page; pages from independent sources
        // appear immediately, while a small worker pool bounds memory and load.
        const searchProviders = async (title: string) => {
            let next = 0;
            const worker = async () => {
                while (next < providers.length && !signal.aborted) {
                    const provider = providers[next++];
                    emit("source", { sourceId: provider.source.id, status: "searching" });
                    try {
                        const works = await provider.search({ ...request, query: title }, await this.context(provider.source.id, signal));
                        signal.throwIfAborted();
                        let count = 0;
                        for (const work of works) {
                            if (results.add(work))
                                ++count;
                        }
                        emit("source", { sourceId: provider.source.id, status: "complete", count });
                    }
                    catch (error) {
                        if (!signal.aborted)
                            emit("source", { sourceId: provider.source.id, status: "error", message: safeError(error) });
                    }
                }
            };
            await Promise.all(Array.from({ length: Math.min(4, providers.length) }, worker));
        };

        const engineSelected = request.sources?.filter(id => !this.providers.some(provider => provider.source.id === id));
        const searchEngine = !selected || !!engineSelected?.length;
        const searchIndexers = async (title: string, expanded = request.expanded) => {
            if (!searchEngine)
                return;
            try {
                const session = await this.browserSessions(signal, engineSelected?.length ? engineSelected : undefined);
                const counts = new Map<string, number>();
                await this.engine.request("search", {
                    ...request,
                    query: title,
                    expanded,
                    ...(engineSelected?.length ? { sources: engineSelected } : {}),
                    exclude: ["byrutor", "freetp", "onlinefix", "fitgirl", "torrentigruha"],
                    ...session,
                }, event => {
                    if (event.event === "result") {
                        const work = event.data as Work;
                        if (results.add(work)) {
                            const sourceId = work.variants[0].sourceId;
                            counts.set(sourceId, (counts.get(sourceId) || 0) + 1);
                        }
                    }
                    else if (event.event === "source") {
                        const source = event.data as { sourceId: string; status: string; count?: number };
                        emit("source", { ...source, ...(source.count === undefined ? {} : { count: counts.get(source.sourceId) || 0 }) });
                    }
                    else if (event.event !== "summary")
                        emit(event.event, event.data);
                }, signal);
            }
            catch (error) {
                if (!signal.aborted)
                    emit("source", { sourceId: "indexers", status: "error", message: safeError(error) });
            }
        };

        await Promise.all([searchIndexers(query), searchProviders(query)]);
        for (const alias of results.titleAliases(query)) {
            signal.throwIfAborted();
            results.addQuery(alias);
            await Promise.all([searchIndexers(alias), searchProviders(alias)]);
        }
        if (!selected && !request.expanded && results.size < 10)
            await searchIndexers(query, true);
        signal.throwIfAborted();
        results.associatePublications();
        emit("summary", { count: results.size });
    }

    async inspect(variant: Variant, signal: AbortSignal): Promise<Work> {
        const provider = this.providers.find(item => item.source.id === variant.sourceId);
        if (provider) {
            if (!provider.inspect)
                throw new Error("This source has no additional item details.");
            return provider.inspect(variant, await this.context(provider.source.id, signal));
        }

        let details: Work | undefined;
        await this.engine.request("inspect", { variant, ...await this.browserSessions(signal, [variant.sourceId]) }, event => {
            if (event.event === "details")
                details = event.data as Work;
        }, signal);
        if (!details)
            throw new Error("This item could not be opened.");
        // The indexer owns release details; catalog association supplies episode labels.
        details.variants = details.variants.map(item => item.transport === "torrent"
            && item.sourceId === variant.sourceId && item.id === variant.id
            && item.url === variant.url && item.infoHash === variant.infoHash
            ? { ...item, episode: item.episode ?? variant.episode, season: item.season ?? variant.season,
                notes: item.notes ?? variant.notes }
            : item);
        return details;
    }

    async acquisitionHeaders(sourceId: string, url: string, headers: Record<string, string>): Promise<Record<string, string>> {
        const values = await this.settings.values(sourceId);
        const source = this.providers.find(provider => provider.source.id === sourceId)?.source;
        const origin = source ? new URL(values.baseUrl || source.url).origin : undefined;
        const cookie = origin === new URL(url).origin && values.cookie
            ? values.cookie : await this.sessions.get(sourceId)?.jar.getCookieString(url);
        return cookie ? { ...headers, Cookie: cookie } : headers;
    }

    close(): void {
        this.engine.close();
    }
}
