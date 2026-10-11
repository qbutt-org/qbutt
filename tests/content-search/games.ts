import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { ProviderContext, Variant, Work } from "../../components/content/contracts";
import { gamingProviders } from "../../components/content/providers/games";
import { assertExactVariant } from "../../components/content/references";
import { rankVariants, SearchResults } from "../../components/content/results";
import { openLink, share } from "../../components/content/sharing";

// Generated publication server exercises the complete source flow without
// downloading copyrighted payloads or relying on the availability of a site.
const directory = await mkdtemp(join(tmpdir(), "qbutt-games-integration-"));
const origin = "https://games-fixture.example";
let releaseVersion = "1.2.3";
let onlinefixSearches = 0;
const requests: { provider: string; path: string; query: string; method: string }[] = [];
const magnet = (suffix: string) => `magnet:?xt=urn:btih:${suffix.repeat(40)}&dn=Generated+Game`;

function publication(source: string, cooperative: boolean): string {
    const title = `Generated Game${cooperative ? " по сети" : ""}`;
    const comment = '<div id="comment-id-1"><b class="comm-author">Fixture author</b>'
        + '<div id="comm-id-1">Generated compatibility note</div></div>';
    if (source === "dodi") {
        return `<h1 class="entry-title">42- Generated Game (v${releaseVersion}) [DODI Repack]</h1>
            <div class="entry-content"><p>Game Version: v${releaseVersion}</p>
            <h3>Download Links</h3><p><strong>Torrent – <a href="${origin}/full.torrent">Click Here</a></strong></p>
            <p><strong>SwiftUploads – <a href="${origin}/full.zip">Click Here</a></strong></p>
            <h3>elamigos Updates</h3><p>Update v1.3.0 – <a href="${origin}/patch.zip">Click Here</a></p>
            <p>Soundtrack Collection – <a href="${origin}/bonus.zip">Click Here</a></p></div>${comment}`;
    }
    if (source === "fitgirl") {
        return `<h1 class="entry-title">Generated Game – v${releaseVersion}</h1><div class="entry-content">
            <h3>Download Mirrors (Torrent)</h3><ul>
            <li><a href="${origin}/tools/idm">IDM</a></li><li><a href="${magnet("a")}">magnet</a></li>
            <li><a href="${origin}/full.torrent">.torrent file only</a></li></ul>
            <h3>Download Mirrors (Direct Links)</h3><ul><li><a href="https://gofile.io/d/generated">Filehoster: GoFile</a></li></ul>
            </div>${comment}`;
    }
    if (source === "onlinefix") {
        return `<div class="full-story"><div class="full-story-header"><time>2025-02-26T17:28:58+03:00</time>
            <span class="author">0xdeadc0de</span><span class="views">2522298</span><h1>${title}</h1></div>
            <div class="full-story-content"><div>Generated site navigation</div><div itemprop="articleBody">
            <b>Информация об игре:</b> <i>Generated co-op puzzle adventure.</i><br>Generated installation instructions.
            <p>Версия игры: ${releaseVersion}</p>
            <p><a href="https://uploads.online-fix.me:2053/torrents/Generated/">Скачать Torrent</a></p>
            <p><a href="https://uploads.online-fix.me:2053/uploads/Generated/Fix%20Repair/">Скачать фикс</a></p>
            </div></div></div>${comment}`;
    }
    const links = `<p>Версия игры: ${releaseVersion}</p><p><a href="${origin}/full.torrent">Generated Game v${releaseVersion}.torrent</a></p>
        <p><a href="${origin}/fix.zip">Generated Game Multiplayer Fix v${releaseVersion}</a></p>`;
    if (source === "byrut") {
        return `<article class="main_content" data-news-id="1"><h1>${title}</h1>${links}
            <div id="other_repacks"><div class="ajax-block" data-ajax="other"><h3 class="showtitle">Other repack</h3>
            <div class="ajax-content"></div></div></div></article>${comment}`;
    }
    return `<div class="${source === "freetp" ? "fullstory" : "full-news"}"><h1>${title}</h1>${links}</div>${comment}`;
}

function searchPage(source: string): string {
    const titles = source === "byrut" ? ["Generated Game", "Generated Game по сети"] : ["Generated Game"];
    return `<div id="dle-content">${titles.map((title, index) => {
        const url = `${origin}/${source}/${index}.html`;
        if (source === "byrut")
            return `<a href="${url}"><span class="search_res_title">${title}</span></a>`;
        if (source === "freetp")
            return `<div class="heading"><a href="${url}">${title}</a></div>`;
        if (source === "onlinefix")
            return `<h2 class="title"><a href="${url}">${title}</a></h2>`;
        return `<h2 class="${source === "igruha" ? "short-title" : "entry-title"}"><a href="${url}">${title}</a></h2>`;
    }).join("")}</div>`;
}

const server = Bun.serve({
    port: 0,
    fetch(request) {
        const url = new URL(request.url);
        const provider = url.searchParams.get("provider")!;
        requests.push({ provider, path: url.pathname, query: url.searchParams.get("story") || url.searchParams.get("s") || "", method: request.method });
        if ((url.pathname === "/index.php" || url.searchParams.has("s")) && provider === "onlinefix" && !onlinefixSearches++)
            return new Response("Вы сможете воспользоваться поиском через 1 секунд.");
        if (url.pathname === "/index.php" || url.searchParams.has("s"))
            return new Response(searchPage(provider), { headers: { "Content-Type": "text/html; charset=utf-8" } });
        if (url.pathname.includes("controller.php"))
            return new Response(`<p>v1.1.0 · Previous release note</p><a href="${origin}/previous.torrent">Generated Game v1.1.0.torrent</a>`);
        if (url.pathname.endsWith(".html"))
            return new Response(publication(provider, url.pathname.endsWith("/1.html")), { headers: { "Content-Type": "text/html; charset=utf-8" } });
        if (url.pathname.endsWith("/torrents/Generated/"))
            return new Response('<a href="Generated.v1.2.3.torrent">Generated.v1.2.3.torrent</a>');
        if (url.pathname.endsWith("/Fix%20Repair/"))
            return new Response('<a href="Generated_Fix_Repair.rar">Generated_Fix_Repair.rar</a>');
        if (request.method === "HEAD")
            return new Response(null, { headers: { "Content-Type": "application/zip" } });
        throw new Error(`Unexpected fixture payload request: ${url.pathname}`);
    },
});

function context(source: string): ProviderContext {
    return {
        signal: AbortSignal.timeout(30_000), dataDir: directory, settings: { baseUrl: origin },
        fetch(url, init) {
            const target = new URL(url);
            const fixture = new URL(target.pathname + target.search, server.url);
            fixture.searchParams.set("provider", source);
            return fetch(fixture, init);
        },
    };
}

try {
    const current = new Map<string, Work>();
    const results = new SearchResults("Generated Game", (event, value) => {
        const work = value as Work;
        if (event === "result") current.set(work.id, work);
        if (event === "removeResult") current.delete(work.id);
    });
    for (const provider of gamingProviders) {
        const publications = await provider.search({ query: "G.E.N.", category: "games" }, context(provider.source.id));
        for (const work of publications) {
            assert(work.variants.every(variant => variant.needsInspection));
            assert.equal(openLink(share({ mode: "exact", work, variant: work.variants[0] })).variant?.needsInspection, true);
            results.add(work);
        }
    }
    assert.equal(current.size, 1, "All source publications must group under the generated game");
    const work = [...current.values()][0]!;
    assert.equal(work.variants.length, gamingProviders.length + 1, "ByRut's ordinary and coop publications must both survive");
    assert.equal(work.variants[0]!.cooperative, true, "Coop publication must be offered first");
    assert(requests.filter(request => request.query && ["byrut", "onlinefix"].includes(request.provider))
        .every(request => request.query === "GEN"), "Sources which break dotted game abbreviations must receive a compact query");
    assert(requests.filter(request => request.query && request.provider === "freetp")
        .every(request => request.query === "G.E.N."), "FreeTP requires the publication's dotted spelling");

    let inspected = 0;
    let resolved = 0;
    let selected: Variant | undefined;
    for (const publication of work.variants) {
        const provider = gamingProviders.find(provider => provider.source.id === publication.sourceId)!;
        const details = await provider.inspect!(publication, context(provider.source.id));
        assert(details.notes?.some(note => note.text.includes("Generated compatibility note")));
        assert(details.variants.every(variant => !variant.needsInspection && variant.id.includes(":link:")));
        assert(!details.variants.some(variant => /IDM/.test(variant.title)), "Download tools must not become game variants");
        if (provider.source.id === "byrut") {
            assert(details.variants.some(variant => variant.version === "1.1.0"));
            assert(details.variants.some(variant => variant.version === releaseVersion));
        }
        if (provider.source.id === "dodi") {
            assert.deepEqual(new Set(details.variants.map(variant => variant.publicationKind)), new Set(["full", "patch", "optional"]));
            assert(details.variants.filter(variant => variant.publicationKind === "full").every(variant => variant.version === releaseVersion),
                "A newer patch must not overwrite the base repack version");
        }
        if (provider.source.id === "onlinefix") {
            assert(details.description?.startsWith("Информация об игре: Generated co-op puzzle adventure.\nGenerated installation instructions."),
                "OnlineFix must display the article body with meaningful line breaks");
            assert(!/0xdeadc0de|2522298|2025-02-26|Generated site navigation/.test(details.description),
                "Publication metadata and site navigation must not become the game description");
        }
        inspected += details.variants.length;
        const ordered = rankVariants(details.variants);
        assert(ordered[0]!.publicationKind === "full", "Fixes and patches must not outrank the complete release");
        if (provider.source.id === "fitgirl")
            assert(ordered[0]!.magnet, "The ready magnet must precede an intermediary torrent page");
        for (const variant of details.variants) {
            const shared = openLink(share({ mode: "exact", work: details, variant })).variant!;
            assertExactVariant(variant, shared);
            const resolution = await provider.resolve!(shared, context(provider.source.id));
            assert.equal(resolution.url, variant.magnet || variant.downloadUrl);
            ++resolved;
        }
        if (publication.sourceId === "freetp")
            selected = details.variants.find(variant => variant.publicationKind === "full");
    }
    assert(selected);
    const provider = gamingProviders.find(provider => provider.source.id === selected.sourceId)!;
    const shared = openLink(share({ mode: "exact", work, variant: selected })).variant!;
    releaseVersion = "2.0.0";
    await assert.rejects(provider.resolve!(shared, context(provider.source.id)), /изменилась/,
        "An updated payload at the same publication and download URL must reject the old exact link");
    assert.throws(() => assertExactVariant(shared, { ...shared, version: undefined }), /изменилась/,
        "Losing a previously known version must not silently satisfy exact selection");

    const abbreviated = new SearchResults("R.E.P.O.", () => {});
    assert(abbreviated.add({ id: "generated:acronym", title: "REPO", category: "games", variants: [selected] }),
        "The source spelling without dots must match the original dotted query");
    let aggregated: Work | undefined;
    const releaseResults = new SearchResults("Generated Game", (event, value) => {
        if (event === "result") aggregated = value as Work;
    });
    const first: Variant = { ...selected, infoHash: "a".repeat(40),
        notes: [{ text: "First source note", url: `${origin}/first`, author: "First author" }] };
    const second: Variant = { ...first, id: "generated:second", sourceId: "generated-second", url: `${origin}/second`,
        notes: [{ text: "Second source note", url: `${origin}/second`, author: "Second author" }] };
    releaseResults.add({ id: "generated:first", title: "Generated Game", category: "games", variants: [first],
        notes: [{ text: "First publication comment", url: `${origin}/first#comment`, author: "First user" }] });
    releaseResults.add({ id: "generated:second", title: "Generated Game", category: "games", variants: [second],
        notes: [{ text: "Second publication comment", url: `${origin}/second#comment`, author: "Second user" }] });
    releaseResults.add({ id: "generated:new-version", title: "Generated Game", category: "games",
        variants: [{ ...second, id: "generated:new-version", version: "2.0.0", infoHash: "b".repeat(40) }] });
    assert.equal(aggregated?.variants.length, 2, "Distinct payload versions must survive hash deduplication");
    assert.equal(aggregated?.notes?.length, 2, "Publication comments must survive streamed work merging");
    const sharedPayload = aggregated!.variants.find(variant => variant.infoHash === first.infoHash)!;
    assert(sharedPayload.notes?.some(note => note.text === "First source note" && note.author === "First author"));
    assert(sharedPayload.notes?.some(note => note.text === "Second source note" && note.author === "Second author"));
    assert(requests.every(request => !request.path.endsWith(".torrent") && request.method !== "POST"
        || request.path.includes("controller.php")), "The scenario must never fetch game payloads");
    console.log(JSON.stringify({ ok: true, sources: gamingProviders.length, publications: work.variants.length,
        inspected, resolved, checks: ["search", "coop-priority", "all-publications", "AJAX-repacks", "source-notes",
            "publisher-mirrors", "base-patch-boundary", "onlinefix-description", "exact-share-roundtrip", "same-url-version-rejection", "missing-version-rejection"] }));
}
finally {
    server.stop(true);
    await rm(directory, { recursive: true, force: true });
}
