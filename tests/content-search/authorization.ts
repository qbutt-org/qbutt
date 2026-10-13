import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { BrowserBridge } from "../../components/content/browser";
import { Catalog } from "../../components/content/catalog";
import type { Provider, Variant, Work } from "../../components/content/contracts";
import { ContentError } from "../../components/content/errors";
import { resolveVariant } from "../../components/content/media";
import { html } from "../../components/content/providers/http";
import { hdrezkaProvider } from "../../components/content/providers/video/hdrezka";
import { SourceSettings } from "../../components/content/settings";
import { openLink, share } from "../../components/content/sharing";
import { LibraryStore } from "../../components/content/store";

const directory = await mkdtemp(join(tmpdir(), "qbutt-source-authorization-"));
const origin = "https://example.com";
const title = "Generated HDRezka film";
const publication: Variant = {
    id: "hdrezka:1", sourceId: "hdrezka", sourceName: "HDRezka", title,
    url: `${origin}/films/fiction/1-generated-film.html`, transport: "video", needsInspection: true,
};
const requests: { path: string; manualSession: boolean }[] = [];
const server = Bun.serve({ port: 0, fetch(request) {
    const url = new URL(request.url);
    requests.push({ path: url.pathname, manualSession: request.headers.get("cookie") === "fixture-account=manual" });
    if (url.pathname === "/search/") {
        return new Response(`<div class="b-content__inline_item" data-id="1">
            <div class="b-content__inline_item-link"><a href="${publication.url}">${title}</a><div>2026</div></div>
            </div>`, { headers: { "Content-Type": "text/html" } });
    }
    if (url.pathname === new URL(publication.url).pathname) {
        return new Response(`<div class="b-post__title"><h1>${title}</h1></div>
            <div id="translators-list"><span data-translator_id="7">Generated voice</span></div>`,
            { headers: { "Content-Type": "text/html" } });
    }
    if (url.pathname === "/ajax/get_cdn_series/")
        return Response.json({ success: true, url: `[720p]${origin}/generated-legal.mp4` });
    if (url.pathname === "/free")
        return new Response(`<h1>${title}</h1>`, { headers: { "Content-Type": "text/html" } });
    return new Response("Unexpected fixture request", { status: 404 });
} });
const store = await LibraryStore.open(directory);
const settings = new SourceSettings(store);
let cookieRequests = 0;
const browser = {
    status: () => ({ ready: true, browsers: [], userAgent: "generated-qcatch/1" }),
    cookies: async () => {
        ++cookieRequests;
        return [{ name: "fixture-account", value: "automatic", domain: "example.com", path: "/", secure: true, hostOnly: true }];
    },
};
const catalog = new Catalog(directory, directory, settings, browser as unknown as BrowserBridge);
const freeProvider: Provider = {
    source: { id: "generated-free", name: "Generated free source", url: origin, enabled: true, categories: ["movies"] },
    async search(_request, context) {
        const $ = await html(`${origin}/free`, context);
        return [{ id: "generated-free:1", title: $("h1").text(), category: "movies", variants: [{
            id: "generated-free:1", sourceId: "generated-free", sourceName: "Generated free source",
            title: $("h1").text(), url: `${origin}/free`, transport: "video",
        }] }];
    },
};
catalog.providers.splice(0, catalog.providers.length, hdrezkaProvider, freeProvider);
const originalContext = catalog.context.bind(catalog);
catalog.context = async (id, signal, publicationUrl) => {
    const context = await originalContext(id, signal, publicationUrl);
    return { ...context, fetch: async (address, init) => {
        const target = new URL(address);
        assert.equal(target.origin, origin);
        const headers = new Headers(init?.headers);
        const response = await fetch(new URL(target.pathname + target.search, server.url), { ...init, headers, signal });
        Object.defineProperty(response, "url", { value: target.href });
        return response;
    } };
};
const signal = AbortSignal.timeout(30_000);
const unauthorized = (error: unknown) => error instanceof ContentError && error.code === "source_authorization_required";
const search = async (sources = ["hdrezka"]): Promise<Work[]> => {
    const works: Work[] = [];
    await catalog.search({ query: title, category: "movies", sources }, signal, (event, data) => {
        if (event === "result") works.push(data as Work);
    });
    return works;
};

try {
    await settings.configure("hdrezka", true, { baseUrl: origin });
    const described = await settings.describe(hdrezkaProvider.source);
    assert.equal(described.requiresLogin, true, "The source must expose its account requirement");
    assert.equal(described.enabled, true, "Missing authorization must preserve the saved source preference");
    assert.deepEqual(await search(), []);
    assert.equal(requests.length, 0, "Missing manual authorization must issue no HTTP requests");
    assert.equal(cookieRequests, 0, "Automatic browser cookies must not substitute for manual authorization");
    await assert.rejects(catalog.inspect(publication, signal), unauthorized);
    await assert.rejects(resolveVariant(publication, catalog, signal, browser as unknown as BrowserBridge, ""), unauthorized);
    assert.equal(requests.length, 0);

    await settings.configure("hdrezka", undefined, { cookie: " \t " });
    assert.deepEqual(await search(), []);
    await assert.rejects(catalog.inspect(publication, signal), unauthorized);
    assert.equal(requests.length, 0, "Whitespace is not account authorization");
    const independent = await search(["hdrezka", "generated-free"]);
    assert.equal(independent.length, 1);
    assert.equal(independent[0].variants[0].sourceId, "generated-free");
    assert.deepEqual(requests.map(request => request.path), ["/free"]);
    requests.length = 0;

    await settings.configure("hdrezka", undefined, { cookie: "fixture-account=manual" });
    const found = await search();
    assert.equal(found.length, 1);
    assert.equal(found[0].variants[0].sourceId, "hdrezka");
    const inspected = await catalog.inspect(found[0].variants[0], signal);
    const selected = inspected.variants[0];
    assert.equal(selected.translation, "Generated voice");
    const resolved = await resolveVariant(selected, catalog, signal, browser as unknown as BrowserBridge, "");
    assert.equal(resolved.url, `${origin}/generated-legal.mp4`);
    assert.equal(requests.length, 3, "Authorized search, inspection and resolution must use their actual source parsers");
    assert(requests.every(request => request.manualSession));
    store.remember(inspected, { variant: selected, favorite: true, lastOpened: new Date().toISOString() });
    const shared = openLink(share({ mode: "exact", work: inspected, variant: selected }));
    const beforeCache = requests.length;
    await catalog.inspect(found[0].variants[0], signal);
    assert.equal(requests.length, beforeCache, "The fixture must exercise an existing inspection cache");

    await settings.configure("hdrezka", undefined, { cookie: "" });
    requests.length = 0;
    const beforeCookies = cookieRequests;
    assert.deepEqual(await search(), []);
    await assert.rejects(catalog.inspect(found[0].variants[0], signal), unauthorized);
    assert.equal(settings.health("hdrezka")?.code, "source_authorization_required");
    assert.deepEqual(settings.failedCheck("hdrezka"), JSON.parse(JSON.stringify({ method: "inspect", variant: found[0].variants[0] })));
    const saved = store.entries("history")[0].variant!;
    await assert.rejects(resolveVariant(saved, catalog, signal, browser as unknown as BrowserBridge, ""), unauthorized);
    await assert.rejects(catalog.inspect(shared.variant!, signal), unauthorized);
    assert.equal(requests.length, 0, "Clearing authorization must block cached, saved and shared publication requests");
    assert.equal(cookieRequests, beforeCookies, "Removed authorization must not fall back to browser cookies");
    assert.equal((await settings.describe(hdrezkaProvider.source)).settings?.find(setting => setting.name === "cookie")?.configured, false);

    console.log(JSON.stringify({ passed: true, scenarios: ["absent-no-http", "automatic-session-not-authorization",
        "whitespace-no-http", "other-source-unchanged", "manual-session-search-inspect-resolve",
        "cached-inspect-requires-authorization", "history-resolve-requires-authorization", "shared-inspect-requires-authorization"] }));
}
finally {
    catalog.close();
    store.close();
    server.stop(true);
    await rm(directory, { recursive: true, force: true });
}
