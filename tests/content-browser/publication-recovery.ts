import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Catalog } from "../../components/content/catalog";
import type { BrowserBridge } from "../../components/content/browser";
import type { Provider, Variant } from "../../components/content/contracts";
import { ContentError } from "../../components/content/errors";
import { html } from "../../components/content/providers/http";
import { SourceSettings } from "../../components/content/settings";
import { LibraryStore } from "../../components/content/store";
import { WebViewSessions } from "../../components/content/webview";

const origin = "https://example.com";
const title = "Generated publication";
const directory = await mkdtemp(join(tmpdir(), "qbutt-webview-recovery-"));
const helperPath = join(directory, "generated-helper.ts");
const logPath = join(directory, "requests.jsonl");
await writeFile(helperPath, String.raw`
import { appendFile } from "node:fs/promises";
import { request as httpRequest } from "node:http";
let buffered = "", finished = false, ignoresCancellation = false;
const reply = value => {
    if (finished) return;
    finished = true;
    process.stdout.write(JSON.stringify(value) + "\n", () => process.exit(0));
};
async function serve(request) {
    if (request.method === "cancel") {
        if (!ignoresCancellation) reply({ id: request.id, error: { code: "canceled" } });
        return;
    }
    ignoresCancellation = request.url.includes("stubborn");
    const endpoint = new URL(request.proxy.url);
    const guarded = await new Promise((resolve, reject) => {
        const outgoing = httpRequest({ hostname: endpoint.hostname, port: endpoint.port,
            path: "http://127.0.0.1/private", headers: { "Proxy-Authorization": "Basic " + Buffer.from(request.proxy.username + ":" + request.proxy.password).toString("base64") } }, response => {
            response.resume();
            response.on("end", () => resolve(response.statusCode === 403));
        });
        outgoing.on("error", reject);
        outgoing.end();
    });
    await appendFile(process.argv[2], JSON.stringify({ url: request.url, mode: request.mode,
        profileDirectory: request.profileDirectory, configuredSession: request.configuredSession,
        authoritativeCookies: request.authoritativeCookies,
        guarded, pid: process.pid }) + "\n");
    if (request.url.includes("stall") || ignoresCancellation) return;
    if (request.url.includes("slow")) await Bun.sleep(300);
    if (request.url.includes("runtime")) { reply({ id: request.id, error: { code: "webview_runtime_required" } }); return; }
    if (request.url.includes("human") && request.mode !== "interactive") {
        reply({ id: request.id, error: { code: "browser_check_required" } }); return;
    }
    if (request.url.includes("oversized")) { process.stdout.write("x".repeat(300000)); return; }
    const domain = request.url.includes("wrong-domain") ? "unrelated.example" : new URL(request.url).hostname;
    reply({ id: request.id, session: { url: request.url, userAgent: "generated-webview/1", cookies: [
        { name: "fixture-session", value: "cleared", domain, path: "/", secure: true, httpOnly: true,
            hostOnly: true, expires: Date.now() / 1000 + 300 }
    ] } });
}
process.stdin.setEncoding("utf8");
process.stdin.on("data", chunk => {
    buffered += chunk;
    let end;
    while ((end = buffered.indexOf("\n")) >= 0) {
        const line = buffered.slice(0, end);
        buffered = buffered.slice(end + 1);
        void serve(JSON.parse(line));
    }
});
`);
const webview = new WebViewSessions(directory, directory, [process.execPath, helperPath, logPath, "--stdio"]);
const requests: { path: string; cookie: string; userAgent: string }[] = [];
let persistent = false;
const challenge = `<h1>Generated browser check</h1><script id="anubis_challenge" type="application/json">{}</script>
    <script id="anubis-main" src="/.within.website/x/cmd/anubis/static/js/main.mjs"></script>`;
const server = Bun.serve({ port: 0, fetch(request) {
    const url = new URL(request.url);
    const cookie = request.headers.get("cookie") || "";
    const userAgent = request.headers.get("user-agent") || "";
    requests.push({ path: url.pathname, cookie, userAgent });
    if (url.pathname.includes("unavailable")) return new Response("Unavailable", { status: 503 });
    return new Response(!persistent && cookie === "fixture-session=cleared" && userAgent === "generated-webview/1"
        ? `<h1>${title}</h1>` : challenge, { headers: { "Content-Type": "text/html" } });
} });
const store = await LibraryStore.open(directory);
const settings = new SourceSettings(store);
let browserCookie = "old";
const browser = { status: () => ({ ready: false, browsers: [], userAgent: "generated-qcatch/1" }),
    cookies: async () => [{ name: "fixture-session", value: browserCookie, domain: "example.com", path: "/", secure: true, hostOnly: true }] };
const catalog = new Catalog(directory, directory, settings, browser as unknown as BrowserBridge, webview);
const publication = (name: string): Variant => ({ id: `generated:${name}`, sourceId: "generated",
    sourceName: "Generated source", title, url: `${origin}/${name}`, transport: "video", needsInspection: true });
const provider: Provider = {
    source: { id: "generated", name: "Generated source", url: origin, enabled: true, categories: ["anime"] },
    async search(_request, context) {
        const $ = await html(`${origin}/search`, context);
        return [{ id: "generated:work", title: $("h1").text(), category: "anime", variants: [publication("search")] }];
    },
    async inspect(variant, context) {
        const $ = await html(variant.url, context);
        return { id: variant.id, title: $("h1").text(), category: "anime", url: variant.url,
            variants: [{ ...variant, needsInspection: false, episode: "2", translation: "Generated voice" }] };
    },
};
catalog.providers.splice(0, catalog.providers.length, provider);
catalog.engine.request = async () => {};
const sessionContext = catalog.context.bind(catalog);
catalog.context = async (id, signal, publicationUrl) => {
    const context = await sessionContext(id, signal, publicationUrl);
    return { ...context, fetch: async (address, init) => {
        const target = new URL(address);
        assert.equal(target.origin, origin);
        const headers = await catalog.acquisitionHeaders(id, target.href, Object.fromEntries(new Headers(init?.headers)));
        headers["User-Agent"] ||= browser.status().userAgent;
        const response = await fetch(new URL(target.pathname, server.url), { ...init, headers, signal });
        Object.defineProperty(response, "url", { value: target.href });
        return response;
    } };
};
const signal = AbortSignal.timeout(25_000);
const logs = async (): Promise<{ url: string; mode: string; profileDirectory: string; configuredSession: string;
    authoritativeCookies?: boolean; guarded: boolean; pid: number }[]> =>
    (await readFile(logPath, "utf8").catch(() => "")).trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
function expire() {
    const session = (catalog as unknown as { sessions: Map<string, { clearances: Map<string, { expiresAt: number }> }> }).sessions.get("generated");
    const clearance = session?.clearances.get(origin);
    if (clearance) clearance.expiresAt = 0;
    settings.recordHealth("generated", undefined, settings.failedCheck("generated"));
    requests.length = 0;
}
async function waitForRequest(count: number) {
    for (let attempt = 0; attempt < 100; ++attempt) {
        if ((await logs()).length >= count) return;
        await Bun.sleep(20);
    }
    throw new Error("Generated helper did not receive its request");
}
async function waitForExit(pid: number) {
    for (let attempt = 0; attempt < 100; ++attempt) {
        try { process.kill(pid, 0); }
        catch { return; }
        await Bun.sleep(20);
    }
    throw new Error("Generated helper did not exit after cancellation");
}

try {
    let results = 0;
    await catalog.search({ query: title, category: "anime", sources: ["generated"] }, signal, event => { if (event === "result") ++results; });
    assert.equal(results, 1);
    assert.equal((await logs())[0].mode, "silent", "Background search must never open a visible WebView");
    assert.equal(requests.length, 2);
    assert.equal(settings.health("generated"), undefined);
    browserCookie = "changed";
    await catalog.context("generated", signal, publication("search").url);
    const retained = await catalog.acquisitionHeaders("generated", publication("search").url, { cookie: "stale", "User-Agent": "stale" });
    assert.equal(retained.Cookie, "fixture-session=cleared", "qcatch refresh must not erase WebView clearance");
    assert.equal(retained["User-Agent"], "generated-webview/1");
    assert.equal((await catalog.acquisitionHeaders("generated", "https://unrelated.example/", {})).Cookie, undefined);

    expire();
    const before = (await logs()).length;
    const [first, second] = await Promise.all([catalog.inspect(publication("first"), signal), catalog.inspect(publication("second"), signal)]);
    assert.equal((await logs()).length, before + 1, "Concurrent publications on one origin must coalesce clearance");
    assert.equal(first.variants[0].url, publication("first").url);
    assert.equal(second.variants[0].url, publication("second").url);
    assert(requests.filter(request => request.cookie === "fixture-session=cleared").every(request => request.userAgent === "generated-webview/1"));

    expire();
    persistent = true;
    await assert.rejects(catalog.inspect(publication("persistent"), signal), error => error instanceof ContentError && error.code === "browser_check_required");
    assert.equal(requests.length, 2, "A persistent source challenge must stop after one operation retry");
    persistent = false;
    expire();
    const human = publication("human");
    await assert.rejects(catalog.inspect(human, signal), error => error instanceof ContentError && error.code === "browser_check_required");
    const recovered = await catalog.checkSource("generated", signal, { interactive: true });
    assert.equal(recovered?.publication.url, human.url);
    assert.equal(settings.health("generated"), undefined);
    const humanRequests = (await logs()).filter(request => request.url === human.url);
    assert.deepEqual(humanRequests.map(request => request.mode), ["silent", "interactive"]);
    assert.equal(humanRequests[0].profileDirectory, humanRequests[1].profileDirectory);

    expire();
    await assert.rejects(catalog.inspect(publication("runtime"), signal), error => error instanceof ContentError && error.code === "webview_runtime_required");
    expire();
    const beforeUnavailable = (await logs()).length;
    await assert.rejects(catalog.inspect(publication("unavailable"), signal), /HTTP 503/);
    assert.equal((await logs()).length, beforeUnavailable);

    const beforeLease = (await logs()).length;
    const lease = new AbortController();
    const canceledLease = webview.check(`${origin}/slow-one`, lease.signal);
    const rejectedLease = assert.rejects(canceledLease, { name: "AbortError" });
    const shared = webview.check(`${origin}/slow-two`, signal);
    await waitForRequest(beforeLease + 1);
    lease.abort();
    await rejectedLease;
    await shared;
    assert.equal((await logs()).length, beforeLease + 1);

    const seed = { name: "account", value: "first", domain: "example.com", path: "/", secure: true, httpOnly: true, hostOnly: true };
    const beforeConfigured = (await logs()).length;
    await Promise.all([
        webview.check(`${origin}/slow-configured-first`, signal, false, [seed]),
        webview.check(`${origin}/configured-second`, signal, false, [{ ...seed, value: "second" }]),
    ]);
    const configured = (await logs()).slice(beforeConfigured);
    assert.equal(configured.length, 2, "Different configured sessions must not share clearance cookies");
    assert.notEqual(configured[0].configuredSession, configured[1].configuredSession);
    assert.equal(configured[0].profileDirectory, configured[1].profileDirectory, "Configuration changes must reuse one origin profile");
    const authoritative = { configuredSession: configured[0].configuredSession, authoritativeCookies: true };
    const beforeRotation = (await logs()).length;
    await Promise.all([
        webview.check(`${origin}/slow-rotated-first`, signal, false, [{ ...seed, value: "refreshed" }], authoritative),
        webview.check(`${origin}/rotated-second`, signal, false, [{ ...seed, value: "newer" }], authoritative),
    ]);
    const rotated = (await logs()).slice(beforeRotation);
    assert.equal(rotated.length, 2, "Different active snapshots must serialize under the same configured fingerprint");
    assert(rotated.every(request => request.authoritativeCookies));
    assert.equal((await logs()).at(-1)!.configuredSession, configured[0].configuredSession,
        "Effective cookie rotation must preserve the caller's configured-session fingerprint");
    const beforeIdentical = (await logs()).length;
    const anotherSeed = { ...seed, name: "another" };
    const retainedCookie = { name: "relative", domain: "example.com", path: "/", hostOnly: true };
    const anotherRetained = { ...retainedCookie, name: "another-relative" };
    await Promise.all([
        webview.check(`${origin}/slow-identical-first`, signal, false, [seed, anotherSeed],
            { ...authoritative, retainedCookies: [retainedCookie, anotherRetained] }),
        webview.check(`${origin}/identical-second`, signal, false, [anotherSeed, seed],
            { ...authoritative, retainedCookies: [anotherRetained, retainedCookie] }),
    ]);
    assert.equal((await logs()).length, beforeIdentical + 1, "Identical active snapshots must coalesce regardless of cookie order");

    expire();
    const controller = new AbortController();
    const beforeCancel = (await logs()).length;
    const canceled = catalog.inspect(publication("stall"), controller.signal);
    const rejected = assert.rejects(canceled, { name: "AbortError" });
    await waitForRequest(beforeCancel + 1);
    controller.abort();
    await rejected;
    assert.equal(requests.length, 1);
    assert.equal(settings.health("generated"), undefined);

    await assert.rejects(webview.check(`${origin}/wrong-domain`, signal), error => error instanceof ContentError && error.code === "webview_failed");
    await assert.rejects(webview.check(`${origin}/oversized`, signal), error => error instanceof ContentError && error.code === "webview_failed");
    const stubbornController = new AbortController();
    const stubbornCount = (await logs()).length;
    const stubborn = webview.check(`${origin}/stubborn`, stubbornController.signal);
    const stubbornRejected = assert.rejects(stubborn, { name: "AbortError" });
    await waitForRequest(stubbornCount + 1);
    const stubbornPid = (await logs()).at(-1)!.pid;
    stubbornController.abort();
    await stubbornRejected;
    await waitForExit(stubbornPid);
    const beforeClose = (await logs()).length;
    const closingFirst = assert.rejects(webview.check(`${origin}/slow-closing-first`, signal), { name: "AbortError" });
    const closingSecond = assert.rejects(webview.check(`${origin}/closing-second`, signal, false, [seed]), { name: "AbortError" });
    await waitForRequest(beforeClose + 1);
    const closingPid = (await logs()).at(-1)!.pid;
    webview.close();
    await Promise.all([closingFirst, closingSecond]);
    await waitForExit(closingPid);
    assert.equal((await logs()).length, beforeClose + 1, "Closing the session must not start a queued helper");
    await assert.rejects(webview.check(`${origin}/after-close`, signal), { name: "AbortError" });
    assert((await logs()).every(request => request.guarded));
    console.log(JSON.stringify({ ok: true, automaticSearch: true, exactPublicationRetry: true, cookieAndUserAgentPair: true,
        qcatchRefreshIsolation: true, originScope: true, expiry: true, coalescing: true, sharedCancellation: true,
        manualSameProfile: true, configuredSessionSerialization: true, activeSnapshotSerialization: true,
        identicalSnapshotCoalescing: true, missingRuntime: true, ordinaryFailure: true, canceledCheck: true,
        authenticatedProxy: true, wrongDomainRejected: true, oversizedRejected: true, forcedChildCleanup: true, queuedShutdown: true }));
}
finally {
    webview.close();
    server.stop(true);
    catalog.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
}
