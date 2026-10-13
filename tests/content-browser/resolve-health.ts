import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Catalog } from "../../components/content/catalog";
import type { Provider, Variant } from "../../components/content/contracts";
import { ContentError } from "../../components/content/errors";
import { resolveVariant } from "../../components/content/media";
import { html } from "../../components/content/providers/http";
import { SourceSettings } from "../../components/content/settings";
import { LibraryStore } from "../../components/content/store";
import type { WebViewCheck, WebViewCookie } from "../../components/content/webview";

const directory = await mkdtemp(join(tmpdir(), "qbutt-resolve-health-"));
const origin = "https://example.com";
const apiOrigin = "https://example.org";
const title = "Generated exact release";
const secret = "generated-private-session";
let scenario = "http";
let healthy = false;
let inspections = 0;
const requests: { address: string; cookie: string; userAgent: string }[] = [];
const resolvedIds: string[] = [];
const nativeRequests: ({ url: string; interactive: boolean; cookies: WebViewCookie[] } & WebViewCheck)[] = [];
const nativeProfiles = new Map<string, { configuredSession?: string; cookies: Map<string, WebViewCookie> }>();
let clearanceValue = "refreshed";
let requiredAccount: string | undefined = "retained";
let notifyHang: (() => void) | undefined;
const captcha = '<form id="captcha-form"><input name="captcha"></form>';
const server = Bun.serve({ port: 0, async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/hang") {
        notifyHang?.();
        return new Promise<Response>(() => {});
    }
    if (path === "/metadata") return new Response(healthy ? "de" : "invalid metadata");
    if (scenario === "http" && !healthy) return new Response("Unavailable", { status: 503 });
    if (["human", "cross-origin", "redirect", "configured"].includes(scenario)) {
        const cookies = request.headers.get("cookie") || "";
        const accepted = cookies.includes(`fixture-clearance=${clearanceValue}`)
            && request.headers.get("user-agent") === "generated-native/1"
            && (scenario !== "configured" || (requiredAccount
                ? cookies.includes(`account=${requiredAccount}`) : !/(?:^|;\s*)account=/.test(cookies)));
        if (!accepted) return new Response(captcha, { headers: { "Content-Type": "text/html" } });
    }
    return new Response(`<h1>${title}</h1>`, { headers: { "Content-Type": "text/html" } });
} });
const store = await LibraryStore.open(directory);
const settings = new SourceSettings(store);
let browserHealthy = false;
const browser = { status: () => ({ ready: false, browsers: [], userAgent: "generated-qcatch/1" }),
    cookies: async () => [], async resolve() {
        if (!browserHealthy) throw new ContentError("browser_required", "Generated browser connection required");
        return { url: `${origin}/media.mp4?token=${secret}` };
    } };
const native = { async check(url: string, _signal: AbortSignal, interactive = false, cookies: WebViewCookie[] = [],
    options: WebViewCheck = {}) {
    nativeRequests.push({ url, interactive, cookies, ...options });
    if (scenario === "human" && !interactive) throw new ContentError("browser_check_required", "Generated human check", url);
    const host = new URL(url).hostname;
    let profile = nativeProfiles.get(host);
    if (!profile || profile.configuredSession !== options.configuredSession) {
        profile = { configuredSession: options.configuredSession, cookies: new Map() };
        nativeProfiles.set(host, profile);
    }
    if (options.authoritativeCookies) {
        const names = new Set([...cookies, ...(options.retainedCookies || [])].map(cookie => cookie.name));
        for (const name of profile.cookies.keys()) if (!names.has(name)) profile.cookies.delete(name);
    }
    for (const cookie of cookies) {
        if (options.authoritativeCookies || !profile.cookies.has(cookie.name)) profile.cookies.set(cookie.name, cookie);
    }
    if (scenario === "configured") {
        if (!options.authoritativeCookies && profile.cookies.has("account")) {
            const account = profile.cookies.get("account")!;
            profile.cookies.set("account", { ...account, value: "retained" });
        }
        if (profile.cookies.get("account")?.value !== requiredAccount)
            throw new ContentError("browser_check_required", "The original configured account cookie is no longer valid.", url);
    }
    profile.cookies.delete("removed");
    profile.cookies.set("fixture-clearance", { name: "fixture-clearance", value: clearanceValue,
        domain: host, path: "/", secure: true, httpOnly: true, hostOnly: true });
    return { url, userAgent: "generated-native/1", cookies: [...profile.cookies.values()] };
}, close() {} };
const catalog = new Catalog(directory, directory, settings, browser as never, native as never);
const selected: Variant = { id: "generated:release:42", sourceId: "generated", sourceName: "Generated",
    title, url: `${origin}/publication`, transport: "file", edition: "Standard", version: "Build42",
    episode: "2", season: "1", translation: "Generated dub", player: "Generated player",
    downloadUrl: `${origin}/private.bin?token=${secret}`,
    ...{ headers: { Cookie: secret }, cookie: secret, mediaUrl: `${origin}/private.mp4?token=${secret}` } };
const provider: Provider = { source: { id: "generated", name: "Generated", url: origin, enabled: true, categories: ["anime"] },
    async search() { return []; },
    async inspect(variant) {
        ++inspections;
        return { id: "generated:work", title, category: "anime", variants: [variant] };
    },
    async resolve(variant, context) {
        resolvedIds.push(variant.id);
        if (scenario === "browser") return { kind: "video", resolver: "browser", url: variant.url };
        if (scenario === "torrent") return { kind: "torrent", resolver: "direct", url: `${origin}/metadata` };
        if (scenario === "cancel") {
            await context.fetch(`${origin}/hang`);
            throw new Error("Canceled HTTP request unexpectedly completed");
        }
        if (scenario === "partial") context.reportError?.(new Error(`Optional release failed token=${secret}`));
        const address = scenario === "cross-origin" ? `${apiOrigin}/api/release/42` : variant.url;
        const $ = await html(address, context);
        return { kind: "file", resolver: "direct", title: $("h1").text(), url: `${origin}/file.bin?token=${secret}`,
            headers: { Authorization: `Bearer ${secret}` } };
    } };
catalog.providers.splice(0, catalog.providers.length, provider);
const originalContext = catalog.context.bind(catalog);
catalog.context = async (id, signal, publicationUrl) => {
    const context = await originalContext(id, signal, publicationUrl);
    return { ...context, fetch: async (address, init) => {
        const requested = new URL(address);
        const target = scenario === "redirect" && requested.origin === origin ? new URL(`${apiOrigin}/redirected/check`) : requested;
        const headers = await catalog.acquisitionHeaders(id, target.href, Object.fromEntries(new Headers(init?.headers)));
        headers["User-Agent"] ||= "generated-qcatch/1";
        const values = await settings.values(id);
        if (!headers.Cookie && target.origin === origin && values.cookie) headers.Cookie = values.cookie;
        requests.push({ address: target.href, cookie: headers.Cookie || "", userAgent: headers["User-Agent"] });
        const response = await fetch(new URL(target.pathname, server.url), { ...init, headers, signal });
        Object.defineProperty(response, "url", { value: target.href });
        return response;
    } };
};
const signal = AbortSignal.timeout(20_000);
const resolve = (variant = selected, options = {}) => resolveVariant(variant, catalog, signal, browser as never, "", options);
const retry = (interactive = false) => catalog.checkSource("generated", signal, { interactive, resolve });
function clearSession() {
    (catalog as unknown as { sessions: Map<string, { clearances: Map<string, unknown> }> }).sessions.get("generated")?.clearances.clear();
}

try {
    await assert.rejects(resolve(), /HTTP 503/);
    assert.equal(settings.failedCheck("generated")?.method, "resolve");
    const persisted = JSON.stringify(settings.failedCheck("generated"));
    assert(!persisted.includes(secret));
    assert(!persisted.includes("headers") && !persisted.includes("downloadUrl") && !persisted.includes("mediaUrl"));
    assert(persisted.includes("Build42") && persisted.includes("Standard") && persisted.includes("Generated player"));
    const reopened = await LibraryStore.open(directory);
    assert.equal(new SourceSettings(reopened).failedCheck("generated")?.method, "resolve");
    reopened.close();
    await catalog.inspect({ ...selected, id: "generated:other" }, signal);
    assert(settings.health("generated"), "An unrelated inspection must not clear a failed resolution");
    healthy = true;
    const beforeInspection = inspections;
    await retry();
    assert.equal(settings.health("generated"), undefined);
    assert.equal(inspections, beforeInspection, "Resolution retry must not substitute inspection");
    assert.deepEqual(resolvedIds, [selected.id, selected.id]);

    scenario = "human";
    await assert.rejects(resolve(), error => error instanceof ContentError && error.code === "browser_check_required");
    await retry(true);
    assert.equal(settings.health("generated"), undefined);
    assert.deepEqual(nativeRequests.map(request => request.interactive), [false, true]);
    assert(nativeRequests.every(request => request.url === selected.url));

    for (const crossOrigin of ["cross-origin", "redirect"]) {
        scenario = crossOrigin;
        clearSession();
        requests.length = 0;
        const beforeNative = nativeRequests.length;
        const result = await resolve();
        const challengeUrl = crossOrigin === "redirect" ? `${apiOrigin}/redirected/check` : `${apiOrigin}/api/release/42`;
        assert.equal(nativeRequests.length, beforeNative + 1);
        assert.equal(nativeRequests.at(-1)!.url, challengeUrl);
        assert.equal(requests.length, 2, "Cross-origin checks must retry the original operation once");
        assert(requests.every(request => request.address === challengeUrl));
        assert.equal(requests.at(-1)!.userAgent, "generated-native/1");
        assert(requests.at(-1)!.cookie.includes("fixture-clearance=refreshed"));
        assert.equal((await catalog.acquisitionHeaders("generated", selected.url, {})).Cookie, undefined);
        assert(result.url?.includes(secret));
        assert.equal(selected.url, `${origin}/publication`);
    }

    scenario = "configured";
    clearSession();
    await settings.configure("generated", undefined, { cookie: "account=old" });
    await resolve();
    assert.equal(nativeRequests.at(-1)!.cookies[0].value, "old");
    const refreshed = await catalog.acquisitionHeaders("generated", selected.url, { Cookie: "account=old", "User-Agent": "old" });
    assert(refreshed.Cookie.includes("account=retained") && refreshed.Cookie.includes("fixture-clearance=refreshed"));
    assert.equal(refreshed["User-Agent"], "generated-native/1");
    const configuredSession = nativeRequests.at(-1)!.configuredSession;
    const activeJar = (catalog as unknown as { sessions: Map<string, { clearances: Map<string, { jar: {
        setCookie(value: string, url: string): Promise<unknown>;
    } }> }> }).sessions.get("generated")!.clearances.get(origin)!.jar;
    await activeJar.setCookie("path-only=scoped; Path=/publication; Secure; HttpOnly", selected.url);
    await activeJar.setCookie("domain-only=scoped; Domain=example.com; Path=/; Secure", selected.url);
    await activeJar.setCookie("public-root=visible; Path=/", selected.url);
    await activeJar.setCookie("removed=obsolete; Path=/; Secure", selected.url);
    await activeJar.setCookie("account=rotated; Path=/; Secure; HttpOnly", selected.url);
    const cookieExpiry = Math.floor(Date.now() / 1000) + 60;
    await activeJar.setCookie(`finite-root=bounded; Path=/; Secure; Expires=${new Date(cookieExpiry * 1000).toUTCString()}`, selected.url);
    await activeJar.setCookie("relative-root=bounded; Path=/; Secure; Max-Age=60", selected.url);
    nativeProfiles.get("example.com")!.cookies.set("relative-root", { name: "relative-root", value: "native-stale",
        domain: "example.com", path: "/", secure: true, httpOnly: false, hostOnly: true });
    clearanceValue = "renewed";
    requiredAccount = "rotated";
    await resolve();
    assert.equal(nativeRequests.at(-1)!.cookies.find(cookie => cookie.name === "account")?.value, "rotated");
    assert.equal(nativeProfiles.get("example.com")!.cookies.get("account")?.value, "rotated",
        "An authoritative active snapshot must replace a stale persistent native cookie");
    assert.equal(nativeRequests.at(-1)!.authoritativeCookies, true);
    assert.deepEqual(nativeRequests.at(-1)!.retainedCookies, [
        { name: "relative-root", domain: "example.com", path: "/", hostOnly: true },
    ]);
    assert.equal(nativeProfiles.get("example.com")!.cookies.get("relative-root")?.value, "native-stale",
        "An excluded unknown-expiry identity must survive native snapshot deletion");
    assert.equal(nativeRequests.at(-1)!.configuredSession, configuredSession,
        "A refreshed account cookie must not reset the native profile's configured-session fingerprint");
    assert(!nativeRequests.at(-1)!.cookies.some(cookie => ["path-only", "domain-only", "relative-root"].includes(cookie.name)));
    assert(nativeRequests.at(-1)!.cookies.every(cookie => cookie.domain === "example.com" && cookie.path === "/" && cookie.hostOnly));
    const publicCookie = nativeRequests.at(-1)!.cookies.find(cookie => cookie.name === "public-root")!;
    assert.equal(publicCookie.secure, false);
    assert.equal(publicCookie.httpOnly, false);
    assert.equal(nativeRequests.at(-1)!.cookies.find(cookie => cookie.name === "finite-root")!.expires, cookieExpiry);
    const retainedScopes = await catalog.acquisitionHeaders("generated", selected.url, {});
    assert(retainedScopes.Cookie.includes("path-only=scoped") && retainedScopes.Cookie.includes("domain-only=scoped"));
    assert(retainedScopes.Cookie.includes("relative-root=bounded"));
    assert(!retainedScopes.Cookie.includes("removed=obsolete"), "Native removal must not resurrect a previous root cookie");
    assert(!(await catalog.acquisitionHeaders("generated", `${origin}/other`, {})).Cookie.includes("path-only=scoped"));
    const currentJar = (catalog as unknown as { sessions: Map<string, { clearances: Map<string, { jar: {
        setCookie(value: string, url: string): Promise<unknown>;
    } }> }> }).sessions.get("generated")!.clearances.get(origin)!.jar;
    await currentJar.setCookie("account=; Path=/; Max-Age=0", selected.url);
    requiredAccount = undefined;
    clearanceValue = "logged-out";
    await resolve();
    assert.equal(nativeRequests.at(-1)!.configuredSession, configuredSession);
    assert(!nativeRequests.at(-1)!.cookies.some(cookie => cookie.name === "account"),
        "A removed configured name must not fall back to the original header value");
    assert.equal(nativeProfiles.get("example.com")!.cookies.has("account"), false);
    assert(!/(?:^|;\s*)account=/.test((await catalog.acquisitionHeaders("generated", selected.url, {})).Cookie));
    requiredAccount = "retained";
    const beforeInvalid = nativeRequests.length;
    await settings.configure("generated", undefined, { cookie: "account=one; account=two" });
    await assert.rejects(resolve(), error => error instanceof ContentError && error.code === "source_session_invalid");
    assert.equal(nativeRequests.length, beforeInvalid, "Ambiguous configured cookie names must not be merged silently");
    await settings.configure("generated", undefined, { cookie: "account=new" });
    assert.equal((await catalog.acquisitionHeaders("generated", selected.url, {})).Cookie, "account=new");
    await resolve();
    assert.equal(nativeRequests.at(-1)!.cookies[0].value, "new");
    await settings.configure("generated", undefined, { cookie: "" });
    assert.equal((await catalog.acquisitionHeaders("generated", selected.url, {})).Cookie, undefined);

    scenario = "browser";
    await assert.rejects(resolve(), error => error instanceof ContentError && error.code === "browser_required");
    assert.equal(settings.health("generated")?.code, "browser_required");
    browserHealthy = true;
    await retry();
    assert.equal(settings.health("generated"), undefined);
    scenario = "torrent";
    healthy = false;
    await assert.rejects(resolve(), /valid torrent metadata/);
    assert.equal(settings.failedCheck("generated")?.method, "resolve");
    healthy = true;
    await retry();
    assert.equal(settings.health("generated"), undefined);
    scenario = "partial";
    await resolve();
    assert(settings.health("generated"));
    assert(!settings.health("generated")!.message.includes(secret));
    scenario = "http";
    await retry();
    assert.equal(settings.health("generated"), undefined);

    scenario = "cancel";
    const controller = new AbortController();
    const hanging = new Promise<void>(resolve => { notifyHang = resolve; });
    const canceled = resolveVariant(selected, catalog, controller.signal, browser as never, "");
    const rejection = assert.rejects(canceled, { name: "AbortError" });
    await hanging;
    controller.abort();
    await rejection;
    assert.equal(settings.health("generated"), undefined);
    await assert.rejects(resolveVariant({ ...selected, sourceId: "link", url: "http://127.0.0.1/private" },
        catalog, signal, browser as never, ""), /local network/);
    assert.equal(settings.health("link"), undefined);
    const healthRows = await readFile(join(directory, "library.sqlite-wal"));
    assert(!healthRows.includes(Buffer.from(secret)), "Private acquisition data reached SQLite diagnostics");
    console.log(JSON.stringify({ ok: true, persistedResolution: true, exactRetry: true, noInspectionSubstitution: true,
        interactiveRetry: true, crossOriginClearance: true, redirectClearance: true, configuredSessionContinuity: true,
        rotatedSessionCookieRetained: true, configuredFingerprintStable: true, scopedSeedsExcluded: true,
        scopedCookiesRetained: true, cookieAttributesPreserved: true, cookieExpiryPreserved: true, nativeRemovalHonored: true,
        authoritativeReplacement: true, configuredCookieDeletion: true, unknownExpiryProtected: true,
        browserFailure: true, torrentFailure: true, partialFailure: true, cancellation: true, ephemeralLinksExcluded: true,
        noAcquisitionSecrets: true }));
}
finally {
    catalog.close();
    server.stop(true);
    store.close();
    await rm(directory, { recursive: true, force: true });
}
