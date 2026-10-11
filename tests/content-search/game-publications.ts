import assert from "node:assert/strict";
import type { ProviderContext } from "../../components/content/contracts";
import { gamingProviders } from "../../components/content/providers/games";
import { openLink, share } from "../../components/content/sharing";

const provider = gamingProviders.find(provider => provider.source.id === "byrut")!;
const origin = "https://games-fixture.example";
const publication = `${origin}/generated-tactical-game.html`;
let scenario: "partial" | "empty" | "denied" | "unavailable" | "cancelled" = "partial";
let primaryVersion = "133804 / 1.5.2 / Build 24942528";
const requests: { path: string; method: string }[] = [];
const reported: unknown[] = [];

const server = Bun.serve({
    port: 0,
    async fetch(request) {
        const url = new URL(request.url);
        requests.push({ path: url.pathname, method: request.method });
        if (url.pathname === "/index.php")
            return new Response(`<div id="dle-content"><a href="${publication}"><span class="search_res_title">Generated Tactical Game</span></a></div>`);
        if (url.pathname === "/generated-tactical-game.html") {
            if (scenario === "denied" || scenario === "unavailable")
                return new Response("Unavailable", { status: scenario === "denied" ? 403 : 503 });
            if (scenario === "empty")
                return new Response('<article class="main_content"><h1>Generated Tactical Game</h1><p>No release links</p></article>');
            return new Response(`<article class="main_content" data-news-id="9"><h1>Generated Tactical Game</h1>
                <div class="subhnamever">v Build 99999</div>
                <a href="${origin}/primary.torrent">Скачать торрент</a>
                <div class="main-release-block">
                    <div class="info-basictor__item--version">Версия <strong>v ${primaryVersion}</strong></div>
                    <p class="not-actual">Более актуальная версия: v 133804</p>
                    <div class="block_down"><a href="${origin}/primary.torrent">Скачать .torrent</a></div>
                </div>
                <div id="other_repacks">
                    <div class="ajax-block" data-ajax="blocked"><span class="showtitle">Unavailable repack</span><div class="ajax-content"></div></div>
                    <div class="ajax-block" data-ajax="available"><span class="showtitle">Portable от Fixture</span>
                        <span class="show_ver">v 117216 + Digital Deluxe Edition</span><div class="ajax-content"></div></div>
                </div></article>`);
        }
        if (url.pathname === "/engine/ajax/controller.php") {
            if (scenario === "cancelled") {
                await Bun.sleep(200);
                return new Response("Cancelled operation");
            }
            const body = new URLSearchParams(await request.text());
            assert.equal(body.get("id"), "9");
            if (body.get("block") === "blocked")
                return new Response("error", { status: 403 });
            return new Response(`<p>Возможность сетевой игры.</p><a href="${origin}/deluxe.torrent">Скачать .torrent</a>`);
        }
        throw new Error(`Unexpected payload request: ${url.pathname}`);
    },
});

function context(signal = AbortSignal.timeout(10_000)): ProviderContext {
    return {
        signal, dataDir: "", settings: { baseUrl: origin },
        reportError(error) { reported.push(error); },
        fetch(url, init) {
            const target = new URL(url);
            return fetch(new URL(target.pathname + target.search, server.url), { ...init, signal });
        },
    };
}

try {
    const works = await provider.search({ query: "Generated Tactical Game", category: "games" }, context());
    assert.equal(works.length, 1);
    assert.equal(works[0]!.variants[0]!.needsInspection, true);
    assert.equal(requests.length, 1, "Global search must not fetch publication editions");
    const details = await provider.inspect!(works[0]!.variants[0]!, context());
    assert.equal(details.variants.length, 2, "An inaccessible alternative must not hide primary and accessible releases");
    assert(details.variants.every(variant => !variant.needsInspection));
    assert.equal(reported.length, 1);
    assert.match(String(reported[0]), /HTTP 403/);
    const primary = details.variants.find(variant => variant.downloadUrl?.endsWith("/primary.torrent"))!;
    const deluxe = details.variants.find(variant => variant.downloadUrl?.endsWith("/deluxe.torrent"))!;
    assert.equal(primary.version, "133804", "The header's newer alternative build must not replace the primary version");
    assert.equal(deluxe.version, "117216");
    assert.equal(deluxe.edition, "Digital Deluxe Edition");
    assert.equal(deluxe.cooperative, true);
    for (const variant of details.variants) {
        const exact = openLink(share({ mode: "exact", work: details, variant })).variant!;
        const resolved = await provider.resolve!(exact, context());
        assert.equal(resolved.url, variant.downloadUrl);
    }
    primaryVersion = "Build 24942528";
    const updated = await provider.inspect!(works[0]!.variants[0]!, context());
    assert.equal(updated.variants.length, 2, "Repeated links must not gain another version from the newer-release notice");
    assert.equal(updated.variants.find(variant => variant.downloadUrl === primary.downloadUrl)?.version, primaryVersion);
    await assert.rejects(provider.resolve!(primary, context()), /изменилась/,
        "An exact saved release must reject changed primary version metadata at the same URL");
    scenario = "empty";
    await assert.rejects(provider.inspect!(works[0]!.variants[0]!, context()), /ссылки на версии игры/,
        "A page without releases must not succeed with the original placeholder");
    for (const [failure, status] of [["denied", 403], ["unavailable", 503]] as const) {
        scenario = failure;
        await assert.rejects(provider.inspect!(works[0]!.variants[0]!, context()), new RegExp(`HTTP ${status}`));
    }
    scenario = "cancelled";
    const controller = new AbortController();
    const beforeCancellation = reported.length;
    const timer = setTimeout(() => controller.abort(), 30);
    try {
        await assert.rejects(provider.inspect!(works[0]!.variants[0]!, context(controller.signal)), { name: "AbortError" });
    }
    finally {
        clearTimeout(timer);
    }
    assert.equal(reported.length, beforeCancellation, "Cancellation must not become persisted source failure");
    assert(!requests.some(request => /\.torrent$/.test(request.path)), "The scenario must not download game payloads");
    console.log(JSON.stringify({ ok: true, partial403: true, availableEditions: 2, primaryVersion: primary.version,
        deluxeEdition: true, buildPrimary: true, exactShareResolution: true, emptyPageError: true, page403: true, page503: true,
        cancellation: true, noPayloadDownloads: true }));
}
finally {
    server.stop(true);
}
