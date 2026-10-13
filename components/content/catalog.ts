import { createHash } from "node:crypto";
import { Cookie, CookieJar } from "tough-cookie";

import type { Provider, ProviderContext, SearchRequest, Variant, Work } from "./contracts";
import type { BrowserBridge } from "./browser";
import { ContentError } from "./errors";
import { sourceFetch, safeError } from "./net";
import { readingProviders } from "./providers/reading";
import { gamingProviders } from "./providers/games";
import { videoProviders } from "./providers/video";
import { SearchResults } from "./results";
import { SourceSettings, type SourceCheck } from "./settings";
import { WebViewSessions, type WebViewCookie, type WebViewCookieIdentity, type WebViewSession } from "./webview";

type Emit = (event: string, data: unknown) => void;

interface SourceSession {
    jar: CookieJar;
    browserCookies: { domain: string; path: string; key: string }[];
    refresh: Promise<void>;
    clearances: Map<string, { provenance: "webview"; jar: CookieJar; userAgent: string;
        configuredCookieHash: string; expiresAt: number }>;
}

function inspectionKey(variant: Variant): string {
    return JSON.stringify([variant.sourceId, variant.id, variant.url, variant.edition, variant.version,
        variant.episode, variant.season, variant.translation, variant.player]);
}

export class Catalog {
    readonly providers: Provider[] = [...videoProviders, ...gamingProviders, ...readingProviders];
    private readonly sessions = new Map<string, SourceSession>();
    private readonly inspections = new Map<string, { work: Work; expiresAt: number }>();
    private searchResults?: SearchResults;

    constructor(readonly dataDir: string, readonly appDir: string, readonly settings: SourceSettings,
        private readonly browser: BrowserBridge, private readonly webview = new WebViewSessions(dataDir, appDir)) {}

    async context(id: string, signal: AbortSignal, publicationUrl?: string): Promise<ProviderContext> {
        const source = this.providers.find(provider => provider.source.id === id)?.source;
        const defaults = Object.fromEntries((source?.settings || []).filter(setting => setting.value !== undefined)
            .map(setting => [setting.name, String(setting.value)]));
        const values = { ...defaults, ...await this.settings.values(id) };
        let session = this.sessions.get(id);
        if (!session) {
            session = { jar: new CookieJar(), browserCookies: [], refresh: Promise.resolve(), clearances: new Map() };
            this.sessions.set(id, session);
        }

        if (source) {
            const current = session;
            const sessionUrl = publicationUrl || values.baseUrl || source.url;
            const configuredCookie = values.cookie && new URL(sessionUrl).origin === new URL(values.baseUrl || source.url).origin;
            current.refresh = current.refresh.catch(() => {}).then(async () => {
                signal.throwIfAborted();
                const cookies = configuredCookie ? [] : await this.browser.cookies(sessionUrl, { signal });
                for (const previous of current.browserCookies)
                    await current.jar.store.removeCookie(previous.domain, previous.path, previous.key);
                current.browserCookies = [];
                for (const cookie of cookies) {
                    const expires = cookie.expirationDate ? `; Expires=${new Date(cookie.expirationDate * 1000).toUTCString()}` : "";
                    const domain = cookie.hostOnly ? "" : `; Domain=${cookie.domain}`;
                    const sameSite = cookie.sameSite === "no_restriction" ? "None"
                        : cookie.sameSite === "lax" || cookie.sameSite === "strict" ? cookie.sameSite : "";
                    const stored = await current.jar.setCookie(`${cookie.name}=${cookie.value}${domain}; Path=${cookie.path}`
                        + (cookie.secure ? "; Secure" : "") + (cookie.httpOnly ? "; HttpOnly" : "")
                        + (sameSite ? `; SameSite=${sameSite}` : "") + expires, sessionUrl, { ignoreError: true });
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
            searchVkVideos: query => this.browser.searchVkVideos(query, signal),
            fetch: sourceFetch(signal, url => {
                const configuredCookie = source && url.origin === new URL(values.baseUrl || source.url).origin ? values.cookie : undefined;
                return this.clearance(session, url.href, configuredCookie) || { jar: session.jar };
            }, source && values.cookie
                ? { origin: new URL(values.baseUrl || source.url).origin, value: values.cookie } : undefined,
                (await this.browser.status(signal)).userAgent),
            saveSettings: changes => this.settings.configure(id, undefined, changes),
        };
    }

    private clearance(session: SourceSession | undefined, url: string, configuredCookie?: string) {
        const origin = new URL(url).origin;
        const clearance = session?.clearances.get(origin);
        const configuredCookieHash = createHash("sha256").update(configuredCookie || "").digest("hex");
        if (clearance && (clearance.expiresAt <= Date.now() || clearance.configuredCookieHash !== configuredCookieHash)) {
            session!.clearances.delete(origin);
            return undefined;
        }
        return clearance;
    }

    private async applyClearance(id: string, url: string, result: WebViewSession, configuredCookie?: string): Promise<void> {
        const previous = this.clearance(this.sessions.get(id), url, configuredCookie);
        const jar = previous ? await previous.jar.clone() : new CookieJar();
        const host = new URL(url).hostname;
        const retained = new Set<string>();
        for (const cookie of await jar.getCookies(url)) {
            if (!cookie.hostOnly || cookie.domain !== host || cookie.path !== "/") continue;
            if (cookie.maxAge != null) retained.add(cookie.key);
            else await jar.store.removeCookie(cookie.domain, cookie.path, cookie.key);
        }
        let expiresAt = Date.now() + 30 * 60_000;
        for (const cookie of result.cookies) {
            if (cookie.hostOnly && cookie.domain === host && cookie.path === "/" && retained.has(cookie.name)) continue;
            if (cookie.expires !== undefined && cookie.expires * 1000 <= Date.now()) continue;
            const stored = await jar.setCookie(`${cookie.name}=${cookie.value}; Path=${cookie.path}`
                + (cookie.hostOnly ? "" : `; Domain=${cookie.domain}`)
                + (cookie.secure ? "; Secure" : "") + (cookie.httpOnly ? "; HttpOnly" : "")
                + (cookie.expires === undefined ? "" : `; Expires=${new Date(cookie.expires * 1000).toUTCString()}`), url, { ignoreError: true });
            if (stored && cookie.expires !== undefined) expiresAt = Math.min(expiresAt, cookie.expires * 1000);
        }
        this.sessions.get(id)!.clearances.set(new URL(url).origin,
            { provenance: "webview", jar, userAgent: result.userAgent, expiresAt,
                configuredCookieHash: createHash("sha256").update(configuredCookie || "").digest("hex") });
    }

    private async withSourceSession<T>(id: string, url: string | undefined, signal: AbortSignal,
        operation: (context: ProviderContext) => Promise<T>, options: { interactive?: boolean } = {}): Promise<T> {
        const run = async (contextUrl = url) => {
            const context = await this.context(id, signal, contextUrl);
            context.withSourceSession = (requestUrl, request) => this.withSourceSession(id, requestUrl, signal, request, options);
            return operation(context);
        };
        try {
            return await run();
        }
        catch (error) {
            signal.throwIfAborted();
            if (!(error instanceof ContentError) || error.code !== "browser_check_required") throw error;
            const challengeUrl = error.url || url;
            if (!challengeUrl) throw error;
            const address = new URL(challengeUrl);
            const source = this.providers.find(provider => provider.source.id === id)?.source;
            const values = await this.settings.values(id);
            const configuredCookie = source && address.origin === new URL(values.baseUrl || source.url).origin ? values.cookie : undefined;
            const initialCookies = (configuredCookie || "").split(/;\s*/).filter(Boolean).map(value => {
                const cookie = Cookie.parse(value);
                if (!cookie) throw new ContentError("source_session_invalid", "The configured source session is invalid.", challengeUrl);
                return { name: cookie.key, value: cookie.value, domain: address.hostname, path: "/",
                    secure: address.protocol === "https:", httpOnly: true, hostOnly: true };
            });
            const configuredSession = createHash("sha256").update(JSON.stringify(initialCookies)).digest("hex");
            const active = this.clearance(this.sessions.get(id), challengeUrl, configuredCookie);
            if (new Set(initialCookies.map(cookie => cookie.name)).size !== initialCookies.length)
                throw new ContentError("source_session_invalid", "The configured source session is invalid.", challengeUrl);
            const seeds: WebViewCookie[] = active ? [] : initialCookies;
            const retainedCookies: WebViewCookieIdentity[] = [];
            for (const cookie of await active?.jar.getCookies(challengeUrl) || []) {
                if (!cookie.hostOnly || cookie.domain !== address.hostname || cookie.path !== "/") continue;
                if (cookie.maxAge != null) {
                    retainedCookies.push({ name: cookie.key, domain: cookie.domain, path: cookie.path, hostOnly: true });
                    continue;
                }
                seeds.push({ name: cookie.key, value: cookie.value, domain: cookie.domain, path: cookie.path,
                    secure: !!cookie.secure, httpOnly: !!cookie.httpOnly, hostOnly: true,
                    ...(cookie.expires instanceof Date ? { expires: cookie.expires.getTime() / 1000 } : {}) });
            }
            const result = await this.webview.check(challengeUrl, signal, options.interactive, seeds,
                { configuredSession, authoritativeCookies: !!active, retainedCookies });
            signal.throwIfAborted();
            await this.applyClearance(id, challengeUrl, result, configuredCookie);
            return run(url || challengeUrl);
        }
    }

    private async requireAuthorization(id: string): Promise<void> {
        const source = this.providers.find(provider => provider.source.id === id)?.source;
        if (source && !await this.settings.hasConfiguredAuthorization(source))
            throw new ContentError("source_authorization_required", "Save your account session in Sources before using this source.", source.url);
    }

    async sourceOperation<T>(id: string, signal: AbortSignal, check: SourceCheck,
        operation: (context: ProviderContext) => Promise<T>, options: { interactive?: boolean } = {}): Promise<T> {
        if (id === "link") return operation(await this.context(id, signal));
        let reported = false;
        const run = (context: ProviderContext) => {
            reported = false;
            context.reportError = error => {
                signal.throwIfAborted();
                reported = true;
                this.settings.recordHealth(id, error, check);
            };
            return operation(context);
        };
        try {
            await this.requireAuthorization(id);
            const result = await this.withSourceSession(id,
                check.method === "search" ? undefined : check.variant.url, signal, run, options);
            signal.throwIfAborted();
            if (!reported) this.settings.recordHealth(id, undefined, check);
            return result;
        }
        catch (error) {
            if (!signal.aborted || signal.reason?.name === "TimeoutError")
                this.settings.recordHealth(id, error, check);
            throw error;
        }
    }

    async search(request: SearchRequest, signal: AbortSignal, emit: Emit): Promise<void> {
        const query = request.query?.trim();
        if (!query || query.length > 500)
            throw new Error("Enter a title or a link.");

        const results = new SearchResults(query, emit, request.language);
        this.searchResults = results;
        const selected = request.sources?.length ? new Set(request.sources) : undefined;
        const providers: Provider[] = [];
        for (const provider of this.providers) {
            const source = await this.settings.describe(provider.source);
            if (!source.enabled || (selected && !selected.has(source.id)))
                continue;
            if (request.category !== "all" && !source.categories.includes(request.category))
                continue;
            if (!await this.settings.hasConfiguredAuthorization(source))
                continue;
            providers.push(provider);
        }
        const categories = new Map<string, Provider[]>();
        for (const provider of providers) {
            const category = provider.source.categories[0] || "all";
            if (!categories.has(category)) categories.set(category, []);
            categories.get(category)!.push(provider);
        }
        const scheduled: Provider[] = [];
        while (scheduled.length < providers.length) {
            for (const queue of categories.values()) {
                const provider = queue.shift();
                if (provider) scheduled.push(provider);
            }
        }

        // Adapters may publish a page before enriching its metadata. Independent
        // sources and individual enrichments update the same result collection.
        const searchProviders = async (title: string) => {
            let next = 0;
            const worker = async () => {
                while (next < scheduled.length && !signal.aborted) {
                    const provider = scheduled[next++];
                    const sourceSignal = AbortSignal.any([signal, AbortSignal.timeout(45_000)]);
                    emit("source", { sourceId: provider.source.id, status: "searching" });
                    try {
                        const accepted = new Set<string>();
                        const accept = (work: Work) => {
                            sourceSignal.throwIfAborted();
                            if (results.add(work)) accepted.add(work.id);
                        };
                        const works = await this.sourceOperation(provider.source.id, sourceSignal,
                            { method: "search", request: { query: title, category: request.category } },
                            context => provider.search({ ...request, query: title }, context, accept));
                        signal.throwIfAborted();
                        for (const work of works) accept(work);
                        emit("source", { sourceId: provider.source.id, status: "complete", count: accepted.size });
                    }
                    catch (error) {
                        if (!signal.aborted) {
                            const message = safeError(error);
                            emit("source", { sourceId: provider.source.id, status: "error", message });
                        }
                    }
                }
            };
            await Promise.all(Array.from({ length: Math.min(4, providers.length) }, worker));
        };

        await searchProviders(query);
        for (const alias of results.titleAliases(query)) {
            signal.throwIfAborted();
            results.addQuery(alias);
            await searchProviders(alias);
        }
        signal.throwIfAborted();
        emit("summary", { count: results.size });
    }

    async checkSource(id: string, signal: AbortSignal, options: {
        interactive?: boolean;
        resolve?: (variant: Variant, options: { interactive?: boolean }) => Promise<unknown>;
    } = {}): Promise<{ work: Work; publication: Variant } | undefined> {
        const provider = this.providers.find(provider => provider.source.id === id);
        const check = this.settings.failedCheck(id)
            || { method: "search" as const, request: { query: "Sintel", category: "all" as const } };
        try {
            if (check.method === "resolve") {
                if (!options.resolve) throw new Error("The source resolution retry is unavailable.");
                await options.resolve(check.variant, { interactive: options.interactive });
            }
            else if (check.method === "inspect") {
                this.inspections.delete(inspectionKey(check.variant));
                return { work: await this.inspect(check.variant, signal, undefined, options), publication: check.variant };
            }
            else if (provider) {
                await this.sourceOperation(provider.source.id, signal, check,
                    context => provider.search(check.request, context), options);
            }
            else throw new Error("Select an available source.");
        }
        catch (error) {
            signal.throwIfAborted();
            this.settings.recordHealth(id, error, check);
        }
    }

    async inspect(variant: Variant, signal: AbortSignal, emit?: (work: Work) => void,
        options: { interactive?: boolean } = {}): Promise<Work> {
        const key = inspectionKey(variant);
        const cached = this.inspections.get(key);
        if (cached && cached.expiresAt > Date.now()) {
            try {
                await this.requireAuthorization(variant.sourceId);
                return cached.work;
            }
            catch (error) {
                if (!signal.aborted || signal.reason?.name === "TimeoutError")
                    this.settings.recordHealth(variant.sourceId, error, { method: "inspect", variant });
                throw error;
            }
        }
        const work = await this.inspectPublication(variant, signal, emit, options);
        signal.throwIfAborted();
        this.inspections.delete(key);
        this.inspections.set(key, { work, expiresAt: Date.now() + 5 * 60_000 });
        if (this.inspections.size > 64) this.inspections.delete(this.inspections.keys().next().value!);
        return work;
    }

    private async inspectPublication(variant: Variant, signal: AbortSignal, emit?: (work: Work) => void,
        options: { interactive?: boolean } = {}): Promise<Work> {
        const provider = this.providers.find(item => item.source.id === variant.sourceId);
        if (!provider) throw new Error("Select an available source.");
        return this.sourceOperation(provider.source.id, signal, { method: "inspect", variant },
            context => {
                if (!provider.inspect) throw new Error("This source has no additional item details.");
                return provider.inspect(variant, context, emit);
            }, options);
    }

    async acquisitionHeaders(sourceId: string, url: string, headers: Record<string, string>): Promise<Record<string, string>> {
        const values = await this.settings.values(sourceId);
        const source = this.providers.find(provider => provider.source.id === sourceId)?.source;
        const origin = source ? new URL(values.baseUrl || source.url).origin : undefined;
        const clearance = this.clearance(this.sessions.get(sourceId), url, origin === new URL(url).origin ? values.cookie : undefined);
        if (clearance) {
            const cookie = await clearance.jar.getCookieString(url);
            const scoped = Object.fromEntries(Object.entries(headers).filter(([name]) => !/^(cookie|user-agent)$/i.test(name)));
            return { ...scoped, ...(cookie ? { Cookie: cookie } : {}), "User-Agent": clearance.userAgent };
        }
        const cookie = origin === new URL(url).origin && values.cookie
            ? values.cookie : await this.sessions.get(sourceId)?.jar.getCookieString(url);
        return cookie ? { ...headers, Cookie: cookie } : headers;
    }

    latestWork(work: Work | undefined): Work | undefined {
        return work ? this.searchResults?.find(work) : undefined;
    }

    close(): void {
        this.webview.close();
    }
}
