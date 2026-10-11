import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Catalog } from "../../components/content/catalog";
import type { Variant } from "../../components/content/contracts";
import { ContentError } from "../../components/content/errors";
import { gamingProviders } from "../../components/content/providers/games";
import { SourceSettings } from "../../components/content/settings";
import { LibraryStore } from "../../components/content/store";

const encode = createRequire(new URL("../../components/content/package.json", import.meta.url))("iconv-lite").encode;
const directory = await mkdtemp(join(tmpdir(), "qbutt-game-recovery-"));
const origin = "https://example.com";
const title = "Сгенерированная игра";
const configuredCookie = "generated-account=private-fixture-account";
const provider = gamingProviders.find(provider => provider.source.id === "byrut")!;
let scenario = "manual";
let clearance = "";
const requests: { path: string; query: string }[] = [];
const nativeRequests: { url: string; interactive: boolean }[] = [];
const challenges: Record<string, string> = {
    manual: '<form id="captcha-form"><input name="captcha"></form>',
    anubis: '<script id="anubis_challenge" type="application/json">{}</script>'
        + '<script id="anubis-main" src="/.within.website/x/cmd/anubis/static/js/main.mjs"></script>',
    cloudflare: '<form id="challenge-form"></form><script src="/cdn-cgi/challenge-platform/scripts/jsd/main.js"></script>',
    ddosguard: '<script src="https://check.ddos-guard.net/check.js"></script>',
};
const server = Bun.serve({ port: 0, async fetch(request) {
    const url = new URL(request.url);
    const cookie = request.headers.get("cookie") || "";
    const userAgent = request.headers.get("user-agent") || "";
    const body = await request.text();
    requests.push({ path: url.pathname, query: url.search });
    if (url.pathname === "/redirect-publication")
        return Response.redirect(new URL("/verification-target", server.url), 302);
    const authorized = clearance && cookie.includes(`generated-clearance=${clearance}`)
        && userAgent === "generated-native/1";
    const optionalAjax = scenario.startsWith("ajax");
    if (scenario === "persistent" || !optionalAjax && !["plain-text", "post-charset"].includes(scenario) && !authorized) {
        return new Response(challenges[scenario] || challenges.manual, {
            status: scenario === "http403" ? 403 : 200,
            headers: { "Content-Type": "text/html; charset=utf-8" },
        });
    }
    if (url.pathname === "/index.php") {
        return new Response(`<div id="dle-content"><a href="${origin}/search-publication">
            <span class="search_res_title">${title}</span></a></div>`);
    }
    if (url.pathname === "/engine/ajax/controller.php") {
        assert.equal(request.method, "POST");
        assert.equal(url.searchParams.get("mod"), "ajaxsp");
        const posted = new URLSearchParams(body);
        assert.equal(posted.get("id"), "9");
        assert.equal(posted.get("block"), "generated-alternative");
        const publication = requests.findLast(request => request.path !== "/engine/ajax/controller.php")!.path;
        assert.equal(request.headers.get("referer"), `${origin}${publication}`);
        assert.equal(request.headers.get("content-type"), "application/x-www-form-urlencoded");
        assert.equal(request.headers.get("x-requested-with"), "XMLHttpRequest");
        if (scenario === "post-charset")
            assert(cookie.includes(configuredCookie));
        if (optionalAjax && (!authorized || scenario === "ajax-persistent"))
            return new Response(challenges.manual, { headers: { "Content-Type": "text/html" } });
        return new Response(encode(`<p>Версия v 2.0.0</p><a href="${origin}/alternative.torrent">Скачать торрент</a>`, "windows-1251"),
            { headers: { "Content-Type": "text/html; charset=windows-1251" } });
    }
    assert(!url.pathname.endsWith(".torrent"), "The fixture must not download a game payload.");
    const extra = scenario === "post-charset" || optionalAjax
        ? '<div id="other_repacks"><div class="ajax-block" data-ajax="generated-alternative"><span class="showtitle">Portable</span>'
            + '<div class="ajax-content"></div></div></div>'
        : "";
    const description = scenario === "plain-text" ? "DDoS-Guard, Just a moment... and cf-chl- are discussed in this article." : "";
    const page = `<article class="main_content"${extra ? ' data-news-id="9"' : ""}><h1>${title}</h1>
        <p>${description}</p><a href="${origin}/primary.torrent">Скачать торрент</a>${extra}</article>`;
    return new Response(scenario === "post-charset" ? encode(page, "windows-1251") : page,
        { headers: { "Content-Type": `text/html; charset=${scenario === "post-charset" ? "windows-1251" : "utf-8"}` } });
} });
const store = await LibraryStore.open(directory);
const settings = new SourceSettings(store);
const browser = { status: () => ({ ready: false, browsers: [], userAgent: "generated-qcatch/1" }), cookies: async () => [] };
const native = { async check(url: string, _signal: AbortSignal, interactive = false) {
    nativeRequests.push({ url, interactive });
    if (scenario === "human" && !interactive)
        throw new ContentError("browser_check_required", "Generated manual check", url);
    clearance = `fixture-${nativeRequests.length}`;
    return { url, userAgent: "generated-native/1", cookies: [
        { name: "generated-clearance", value: clearance, domain: "example.com", path: "/", secure: true, httpOnly: true, hostOnly: true },
    ] };
}, close() {} };
const catalog = new Catalog(directory, directory, settings, browser as never, native as never);
catalog.providers.splice(0, catalog.providers.length, provider);
catalog.engine.request = async () => {};
const originalContext = catalog.context.bind(catalog);
catalog.context = async (id, signal, publicationUrl) => {
    const context = await originalContext(id, signal, publicationUrl);
    return { ...context, fetch: async (address, init) => {
        const target = new URL(address);
        assert.equal(target.origin, origin);
        const headers = await catalog.acquisitionHeaders(id, target.href, Object.fromEntries(new Headers(init?.headers)));
        headers["User-Agent"] ||= "generated-qcatch/1";
        const response = await fetch(new URL(target.pathname + target.search, server.url), { ...init, headers, signal });
        const final = new URL(response.url);
        Object.defineProperty(response, "url", { value: new URL(final.pathname + final.search, origin).href });
        return response;
    } };
};
const signal = AbortSignal.timeout(40_000);
function publication(path: string): Variant {
    return { id: `byrut:generated-${path}`, sourceId: "byrut", sourceName: "ByRut", title,
        url: `${origin}/${path}`, transport: "torrent", needsInspection: true };
}

try {
    await settings.configure("byrut", undefined, { baseUrl: origin });
    for (const kind of ["manual", "anubis", "cloudflare", "ddosguard", "http403"]) {
        scenario = kind;
        clearance = "";
        const selected = publication(kind);
        const beforeNative = nativeRequests.length;
        const beforeRequests = requests.length;
        const result = await catalog.inspect(selected, signal);
        assert.equal(nativeRequests.length, beforeNative + 1);
        assert.equal(nativeRequests.at(-1)!.url, selected.url);
        assert.equal(nativeRequests.at(-1)!.interactive, false);
        assert.deepEqual(requests.slice(beforeRequests).map(request => request.path), [`/${kind}`, `/${kind}`]);
        assert.equal(result.title, title);
        assert.equal(result.url, selected.url);
        assert.equal(result.variants.length, 1);
        assert.equal(result.variants[0]!.url, selected.url);
        assert.equal(result.variants[0]!.needsInspection, undefined);
        assert.equal(result.variants[0]!.downloadUrl, `${origin}/primary.torrent`);
        assert.equal(settings.health("byrut"), undefined);
    }

    scenario = "redirect";
    clearance = "";
    const redirect = publication("redirect-publication");
    const beforeRedirect = requests.length;
    const redirected = await catalog.inspect(redirect, signal);
    assert.equal(nativeRequests.at(-1)!.url, `${origin}/verification-target`);
    assert.deepEqual(requests.slice(beforeRedirect).map(request => request.path),
        ["/redirect-publication", "/verification-target", "/redirect-publication", "/verification-target"]);
    assert.equal(redirected.url, redirect.url);
    assert.equal(redirected.variants[0]!.url, redirect.url);

    scenario = "plain-text";
    const beforePlain = nativeRequests.length;
    const ordinary = await catalog.inspect(publication("plain-text"), signal);
    assert.equal(ordinary.title, title);
    assert(ordinary.description?.includes("DDoS-Guard"));
    assert.equal(nativeRequests.length, beforePlain, "Ordinary text mentions must not trigger a browser check.");

    scenario = "persistent";
    clearance = "";
    const failed = publication("persistent");
    const beforeFailure = requests.length;
    const beforeFailedNative = nativeRequests.length;
    await assert.rejects(catalog.inspect(failed, signal), error => error instanceof ContentError && error.code === "browser_check_required");
    assert.equal(requests.length, beforeFailure + 2);
    assert.equal(nativeRequests.length, beforeFailedNative + 1);
    assert.equal(settings.health("byrut")?.code, "browser_check_required");
    const reopened = await LibraryStore.open(directory);
    const saved = new SourceSettings(reopened).failedCheck("byrut");
    assert.equal(saved?.method, "inspect");
    assert(saved?.method === "inspect" && saved.variant.id === failed.id && saved.variant.url === failed.url);
    reopened.close();
    scenario = "plain-text";
    const retry = await catalog.checkSource("byrut", signal);
    assert.equal(retry?.publication.id, failed.id);
    assert.equal(retry?.work.url, failed.url);
    assert.equal(settings.health("byrut"), undefined);

    scenario = "human";
    clearance = "";
    const human = publication("human");
    await assert.rejects(catalog.inspect(human, signal), error => error instanceof ContentError && error.code === "browser_check_required");
    const recovered = await catalog.checkSource("byrut", signal, { interactive: true });
    assert.equal(recovered?.work.url, human.url);
    assert.deepEqual(nativeRequests.filter(request => request.url === human.url).map(request => request.interactive), [false, true]);

    for (const kind of ["ajax", "ajax-persistent"]) {
        scenario = kind;
        clearance = "";
        const selected = publication(kind);
        const beforeAjax = requests.length;
        const beforeNative = nativeRequests.length;
        const result = await catalog.inspect(selected, signal);
        assert.equal(nativeRequests.length, beforeNative + 1);
        assert.equal(nativeRequests.at(-1)!.url, `${origin}/engine/ajax/controller.php?mod=ajaxsp`);
        assert.deepEqual(requests.slice(beforeAjax).map(request => request.path),
            [`/${kind}`, "/engine/ajax/controller.php", "/engine/ajax/controller.php"],
            "Only the failed optional POST request must be retried.");
        assert.equal(result.url, selected.url);
        assert.equal(result.variants.length, kind === "ajax" ? 2 : 1);
        if (kind === "ajax")
            assert.equal(settings.health("byrut"), undefined);
        else {
            assert.equal(settings.health("byrut")?.code, "browser_check_required");
            const failed = settings.failedCheck("byrut");
            assert(failed?.method === "inspect" && failed.variant.url === selected.url);
        }
    }
    scenario = "ajax";
    await catalog.checkSource("byrut", signal);
    assert.equal(settings.health("byrut"), undefined);

    scenario = "post-charset";
    await settings.configure("byrut", undefined, { cookie: configuredCookie });
    const beforePost = nativeRequests.length;
    const post = await catalog.inspect(publication("post-charset"), signal);
    assert.equal(post.title, title);
    assert.equal(post.variants.length, 2);
    assert(post.variants.some(variant => variant.downloadUrl === `${origin}/alternative.torrent` && variant.version === "2.0.0"));
    assert.equal(nativeRequests.length, beforePost);
    const searchContext = await catalog.context("byrut", signal);
    const found = await provider.search({ query: title, category: "games" }, searchContext);
    assert.equal(found[0]?.title, title);
    assert(requests.some(request => request.path === "/index.php" && new URLSearchParams(request.query).get("story") === title));
    assert.equal(settings.health("byrut"), undefined);
    const storage = await readFile(join(directory, "library.sqlite-wal"));
    assert(!storage.includes(Buffer.from("private-fixture-account")), "Private cookies reached SQLite diagnostics.");
    assert(!requests.some(request => request.path.endsWith(".torrent")));
    console.log(JSON.stringify({ ok: true, realGameProvider: true, realHttp: true, persistedSqliteFailure: true,
        structuralChallenges: 5, exactPublicationRetry: true, boundedRetry: true, actualResponseUrl: true,
        legitimateTextMention: true, manualCallbackRetry: true, optionalAjaxRetry: true, persistentAjaxKeepsPrimary: true,
        postHeaders: true, charset: "windows-1251",
        configuredCookieScope: true, noPayloadDownloads: true, noPrivateDiagnostics: true }));
}
finally {
    catalog.close();
    server.stop(true);
    store.close();
    await rm(directory, { recursive: true, force: true });
}
