import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { Catalog } from "../../components/content/catalog";
import type { Provider, ProviderContext, Variant, Work } from "../../components/content/contracts";
import { ContentError } from "../../components/content/errors";
import { hdrezkaProvider } from "../../components/content/providers/video/hdrezka";
import { rutubeProvider } from "../../components/content/providers/video/social";
import { yaniProvider } from "../../components/content/providers/video/yani";
import { SourceSettings } from "../../components/content/settings";
import { LibraryStore } from "../../components/content/store";

const directory = await mkdtemp(join(tmpdir(), "qbutt-progressive-search-"));
const store = await LibraryStore.open(directory);
const settings = new SourceSettings(store);
const cookieRequests: string[] = [];
let clearance: string | undefined;
const browser = { status: () => ({ ready: false, browsers: [] }), cookies: async (url: string) => {
    cookieRequests.push(url);
    return clearance ? [{ name: "fixture-clearance", value: clearance, domain: new URL(url).hostname,
        path: "/", secure: true, hostOnly: true }] : [];
} };
const catalog = new Catalog(directory, directory, settings, browser as never, {
    async check(url: string) { throw new ContentError("browser_check_required", "Generated persistent browser check", url); },
    close() {},
} as never);
const sessionContext = catalog.context.bind(catalog);
const signal = AbortSignal.timeout(20_000);
const title = "Generated Student Council";
const anime = { anime_id: 1, anime_url: "generated-student-council", title, year: 2026,
    other_titles: ["Generated Council"], remote_ids: { myanimelist_id: 1 } };
const upload = (id: string, name: string, duration: number, description = "") => ({
    id: id.repeat(32), title: name, duration, description, category: { id: 41 },
});
let releaseMetadata!: () => void;
const metadata = new Promise<void>(resolve => { releaseMetadata = resolve; });
let initialResult!: () => void;
const initial = new Promise<void>(resolve => { initialResult = resolve; });
let searchFinished = false;
let firstResultBeforeMetadata = false;
let callbackCount = 0;
const json = (value: unknown) => Response.json(value);
let hdrezkaPage = "";
const fixtureFetch: ProviderContext["fetch"] = async address => {
    const url = new URL(address);
    if (url.hostname === "hdrezka-home.tv")
        return new Response(hdrezkaPage, { status: 200, headers: { "Content-Type": "text/html; charset=utf-8" } });
    if (url.hostname === "api.yani.tv" && url.pathname === "/anime")
        return json({ response: [anime] });
    if (url.hostname === "api.yani.tv" && url.pathname === "/anime/1") {
        await metadata;
        return json({ response: anime });
    }
    if (url.pathname === "/api/search/video/") return json({ results: [
        upload("a", `${title} - 2 серия [OnWave]`, 1430),
        upload("b", title, 58, "Тип: TV Сериал\nСтатус: запланировано"),
        upload("c", `${title} - 1 серия`, 90, "Короткий полный эпизод"),
        upload("d", title, 90, "Полное короткометражное аниме"),
    ] });
    if (url.pathname.startsWith("/api/play/options/"))
        return json({ video_balancer: { m3u8: `https://fixture.example/${url.pathname.split("/")[4]}.m3u8` } });
    if (url.hostname === "fixture.example") return new Response(
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=1920x1080\n1080.m3u8\n");
    throw new Error(`Unexpected generated fixture request: ${url.hostname}${url.pathname}`);
};
catalog.context = async (_id, contextSignal) => ({
    dataDir: directory, settings: {}, signal: contextSignal, fetch: fixtureFetch,
});

function fixtureVariant(id: string, fields: Partial<Variant> = {}): Variant {
    return { id, sourceId: "generated", sourceName: "Generated legal fixture", title: "Generated", url: `https://example.com/${id}`,
        transport: "video", ...fields };
}

function fixtureWork(id: string, fields: Partial<Work> = {}): Work {
    return { id, title: "Berserk", category: "anime", variants: [fixtureVariant(id)], ...fields };
}

function collect(): { works: Map<string, Work>; emit(event: string, data: unknown): void } {
    const works = new Map<string, Work>();
    return { works, emit(event, data) {
        if (event === "result") works.set((data as Work).id, data as Work);
        if (event === "removeResult") works.delete((data as { id: string }).id);
    } };
}

try {
    catalog.providers.splice(0, catalog.providers.length, yaniProvider, rutubeProvider);
    const output = collect();
    const searching = catalog.search({ query: title, category: "anime", sources: ["yani", "rutube"], language: "ru-RU" }, signal,
        (event, data) => {
            output.emit(event, data);
            if (event === "result") {
                ++callbackCount;
                if (!searchFinished && !firstResultBeforeMetadata) {
                    firstResultBeforeMetadata = true;
                    initialResult();
                }
            }
        }).then(() => { searchFinished = true; });
    await Promise.race([initial, new Promise((_, reject) => setTimeout(() => reject(new Error("No incremental result")), 2000))]);
    assert(!searchFinished, "The generated metadata gate must still be pending after the first result");
    releaseMetadata();
    await searching;
    assert(firstResultBeforeMetadata);
    assert(![...output.works.values()].some(work => work.id === `rutube:${"b".repeat(32)}`), "Confirmed promotional preview leaked into ordinary search");
    assert([...output.works.values()].some(work => work.id === `rutube:${"c".repeat(32)}`), "A short full episode was excluded by duration alone");
    assert([...output.works.values()].some(work => work.id === `rutube:${"d".repeat(32)}`), "A short full work was excluded by duration alone");
    const grouped = output.works.get("yani:1")!;
    assert(grouped.variants.some(variant => variant.sourceId === "rutube" && variant.episode === "2" && variant.quality === "1080p"));

    const original = fixtureWork("catalog-1997", { year: 1997, identifiers: { mal: "33" }, aliases: ["Berserk"] });
    const remake = fixtureWork("catalog-2016", { year: 2016, identifiers: { mal: "32379" }, aliases: ["Berserk"] });
    const torrent = fixtureWork("torrent", { variants: [fixtureVariant("torrent", {
        title: "[Generated] Berserk - 01 [1080p]", transport: "torrent", infoHash: "1".repeat(40),
    })] });
    const generated: Provider = { source: { id: "generated", name: "Generated", url: "https://example.com", categories: ["anime"], enabled: true },
        async search(_request, _context, emit) {
            emit?.(original);
            emit?.(torrent);
            emit?.(remake);
            return [original, torrent, remake];
        } };
    catalog.providers.splice(0, catalog.providers.length, generated);
    const ambiguous = collect();
    await catalog.search({ query: "Berserk", category: "anime", sources: ["generated"] }, signal, ambiguous.emit);
    assert.equal(ambiguous.works.size, 3, "A later remake must undo a previously unique publication association");
    assert(ambiguous.works.get("torrent")?.variants.length === 1);
    assert(ambiguous.works.get(original.id)?.variants.every(variant => variant.id !== "torrent"));

    const seasons = ["1", "2", undefined].map((season, index) => fixtureWork(`season-${index}`, {
        title: "Generated Seasons", year: 2026,
        variants: [fixtureVariant(`season-${index}`, { episode: "2", season })],
    }));
    generated.search = async () => seasons;
    const separated = collect();
    await catalog.search({ query: seasons[0]!.title, category: "anime", sources: ["generated"] }, signal, separated.emit);
    assert.equal(separated.works.size, 3, "Conflicting or unproven seasons merged through an equal title and year");

    const releases = ["one", "two"].map((id, index) => fixtureWork(id, {
        title: "Generated Game", category: "games", variants: [fixtureVariant(id, {
            transport: "torrent", infoHash: "a".repeat(40), version: String(index + 1), edition: id,
        })],
    }));
    generated.source.categories = ["anime", "games"];
    generated.search = async () => releases;
    const exact = collect();
    await catalog.search({ query: releases[0]!.title, category: "games", sources: ["generated"] }, signal, exact.emit);
    assert.equal(exact.works.size, 1);
    assert.equal([...exact.works.values()][0]!.variants.length, 2, "Equal torrent bytes erased an exact edition/version publication");

    const ranked = fixtureWork("ranked", { title: "Generated Ranked", identifiers: { mal: "77" }, variants: [
        fixtureVariant("direct", { languages: ["RUS"], quality: "1080p", episode: "2", translation: "OnWave" }),
        fixtureVariant("seeded", { transport: "torrent", languages: ["ru"], quality: "1080p", seeders: 10000, season: "1", episode: "S01E02", translation: "Дубляж OnWave" }),
        fixtureVariant("scarce", { transport: "torrent", languages: ["ru"], quality: "1080p", seeders: 1 }),
        fixtureVariant("english", { languages: ["en"], quality: "2160p" }),
        fixtureVariant("sub", { languages: ["ja", "ru"], translation: "Субтитры | Kazoku Project" }),
        fixtureVariant("dub", { languages: ["ru"], translation: "Kazoku Project" }),
    ] });
    generated.search = async () => [ranked];
    const ranking = collect();
    await catalog.search({ query: ranked.title, category: "anime", sources: ["generated"], language: "ru-RU" }, signal, ranking.emit);
    const variants = ranking.works.get(ranked.id)!.variants;
    assert.equal(variants[0]!.id, "direct", "Direct video was penalized for not having torrent peers");
    assert(variants.findIndex(variant => variant.id === "english") > variants.findIndex(variant => variant.id === "seeded"));
    assert(variants.findIndex(variant => variant.id === "seeded") < variants.findIndex(variant => variant.id === "scarce"));
    assert.deepEqual(variants[0]!.languages, ["ru"]);
    assert.deepEqual(variants.find(variant => variant.id === "direct")!.facets, variants.find(variant => variant.id === "seeded")!.facets);
    assert.equal(JSON.parse(JSON.stringify(variants.find(variant => variant.id === "seeded"))).facets.season, "",
        "Canonical season clearing must survive JSON instead of falling back to the raw season in Qt");
    assert.equal(variants.find(variant => variant.id === "sub")!.facets?.translation, "Kazoku Project");
    assert.equal(variants.find(variant => variant.id === "sub")!.translation, "Субтитры | Kazoku Project");
    assert.equal(variants.find(variant => variant.id === "seeded")!.episode, "S01E02", "Canonical facets overwrote exact selection metadata");

    const catalogWork = fixtureWork("catalog", { title: "Generated Student Council", year: 2026,
        aliases: ["Generated Council", "Generated Council Alternative"] });
    const episodeTorrent = (id: string, title: string) => fixtureWork(id, { title,
        variants: [fixtureVariant(id, { title, transport: "torrent" })] });
    const releaseLabel = "(VOSTFR Multi Subs Generated Council Alternative Generated Council)";
    const episodeOne = episodeTorrent("release-one", `Translated Council S01E01 SUBFRENCH 1080p WEB DL AAC2 0 x264 ${releaseLabel}`);
    const episodeTwo = episodeTorrent("release-two", `Translated Council S01E02 UNCENSORED SUBFRENCH 1080p WEB DL AAC2 0 x264 ${releaseLabel}`);
    const derivative = episodeTorrent("derivative", "Other Work S01E01 (Generated Council Alternative OVA)");
    const otherSeason = episodeTorrent("season-two", `Translated Council S02E01 ${releaseLabel}`);
    const volume = episodeTorrent("volume", `Translated Council v01 (2026) (Digital) ${releaseLabel}`);
    const manga = fixtureWork("manga", { title: catalogWork.title, category: "manga", aliases: catalogWork.aliases,
        variants: [fixtureVariant("manga", { title: catalogWork.title, transport: "manga" })] });
    generated.search = async () => [catalogWork, episodeOne, episodeTwo, derivative, otherSeason, volume, manga];
    const annotated = collect();
    await catalog.search({ query: catalogWork.title, category: "all", sources: ["generated"] }, signal, annotated.emit);
    const associated = annotated.works.get(catalogWork.id)!;
    assert.equal(associated.variants.length, 3, "Verified aliases with language annotations must identify only the two episode torrents");
    for (const release of [episodeOne, episodeTwo]) {
        const variant = associated.variants.find(variant => variant.id === release.variants[0]!.id)!;
        assert.equal(variant.url, release.variants[0]!.url);
        assert.equal(variant.title, release.title);
        assert.equal(variant.episode, release === episodeOne ? "1" : "2");
    }
    for (const separate of [derivative, otherSeason, volume, manga])
        assert(annotated.works.has(separate.id), "Unconfirmed title metadata, seasons, volumes and manga must stay separate");
    const annotatedRemake = fixtureWork("remake", { title: catalogWork.title, aliases: catalogWork.aliases, year: 2025 });
    generated.search = async () => [catalogWork, annotatedRemake, episodeTwo];
    const annotatedAmbiguous = collect();
    await catalog.search({ query: catalogWork.title, category: "anime", sources: ["generated"] }, signal, annotatedAmbiguous.emit);
    assert.equal(annotatedAmbiguous.works.size, 3, "Language annotations must not resolve ambiguous remake identity");

    let inspections = 0;
    let inspectionState: "partial" | "ready" | "failed" = "partial";
    generated.inspect = async (variant, context) => {
        ++inspections;
        if (inspectionState === "failed") throw new Error("Publication HTTP 503");
        if (inspectionState === "partial") context.reportError?.(new Error("Optional editions HTTP 403 token=private-fixture"));
        const ready = { ...variant, needsInspection: false };
        return fixtureWork("inspected", { variants: inspectionState === "ready"
            ? [ready, { ...ready, id: `${variant.id}:recovered`, edition: "Recovered edition" }] : [ready] });
    };
    const pending = fixtureVariant("publication", { needsInspection: true });
    await catalog.inspect(pending, signal);
    assert(settings.health("generated")?.message.includes("403"));
    assert(!settings.health("generated")!.message.includes("private-fixture"));
    assert.equal(settings.failedCheck("generated")?.method, "inspect");
    generated.search = async () => [ranked];
    await catalog.search({ query: ranked.title, category: "anime", sources: ["generated"] }, signal, () => {});
    assert(settings.health("generated"), "An unrelated successful search cleared a failed publication inspection");
    await catalog.inspect(pending, signal);
    assert.equal(inspections, 1, "Revisiting the same exact publication must reuse the completed inspection");
    const partialRetry = await catalog.checkSource("generated", signal);
    assert.equal(inspections, 2, "Manual retry must re-run the failed publication instead of a generic search");
    assert.equal(partialRetry?.work.variants.length, 1);
    assert.deepEqual(partialRetry?.publication, pending);
    assert(settings.health("generated"));
    inspectionState = "ready";
    const recovered = await catalog.checkSource("generated", signal);
    assert.equal(inspections, 3);
    assert.deepEqual(recovered?.publication, pending, "Recovery must identify the exact retried publication");
    assert.equal(recovered?.work.variants.length, 2);
    assert.equal(recovered?.work.variants[1]?.id, "publication:recovered");
    assert.equal(recovered?.work.variants[1]?.url, pending.url);
    assert.equal(settings.health("generated"), undefined);
    assert.deepEqual(await catalog.inspect(pending, signal), recovered?.work);
    assert.equal(inspections, 3, "Recovery must replace the cached partial inspection");

    settings.recordHealth("generated", "Publication HTTP 503", { method: "inspect", variant: pending });
    inspectionState = "failed";
    assert.equal(await catalog.checkSource("generated", signal), undefined, "A failed retry must not publish recovered details");
    assert.equal(inspections, 4);
    assert(settings.health("generated")?.message.includes("503"));

    catalog.providers.push(hdrezkaProvider);
    const publicationUrl = "https://hdrezka-home.tv/animation/comedy/92943-generated-fixture.html";
    const publication = fixtureVariant("hdrezka:92943", { sourceId: "hdrezka", sourceName: "HDRezka",
        title, url: publicationUrl, needsInspection: true });
    const selected = fixtureVariant("ready-clean", { title });
    const retainedWork = fixtureWork("canonical", { title, variants: [selected, publication] });
    generated.search = async () => [retainedWork];
    await catalog.search({ query: title, category: "anime", sources: ["generated"] }, signal, () => {});
    const originalWork = structuredClone(catalog.latestWork(retainedWork));
    const challengeTitle = "Проверяем, что вы не бот!";
    const anubisPage = `<h1>${challengeTitle}</h1>
        <script id="anubis_challenge" type="application/json">{}</script>
        <script id="anubis-main" type="module" src="/.within.website/x/cmd/anubis/static/js/main.mjs"></script>`;
    const detailUpdates: Work[] = [];
    for (const document of [anubisPage,
        '<form id="challenge-form"></form><script src="/cdn-cgi/challenge-platform/h/g/orchestrate/chl_page/v1"></script>',
        '<script src="https://check.ddos-guard.net/check.js"></script>',
        '<form id="captcha-form"><input name="captcha"></form>',
        '<form action="/captcha/verify"><input name="answer"></form>']) {
        hdrezkaPage = document;
        await assert.rejects(catalog.inspect(publication, signal, details => detailUpdates.push(details)), error =>
            error instanceof ContentError && error.code === "browser_check_required" && error.url === publicationUrl);
    }
    hdrezkaPage = anubisPage;
    assert.equal(detailUpdates.length, 0, "An HTTP 200 browser challenge must never publish replacement metadata");
    const retained = catalog.latestWork(retainedWork);
    assert.deepEqual(retained, originalWork, "A browser check changed the latest canonical work");
    assert(retained?.variants.some(variant => variant.id === selected.id && variant.url === selected.url));
    assert(retained?.variants.some(variant => variant.id === publication.id && variant.url === publication.url && variant.needsInspection));
    assert.equal((await settings.describe(hdrezkaProvider.source)).enabled, true);
    assert.equal(settings.health("hdrezka")?.code, "browser_check_required");
    assert.equal(settings.health("hdrezka")?.url, publicationUrl);
    const persisted = await LibraryStore.open(directory);
    assert.equal(new SourceSettings(persisted).health("hdrezka")?.code, "browser_check_required");
    assert.equal(new SourceSettings(persisted).health("hdrezka")?.url, publicationUrl);
    persisted.close();
    await assert.rejects(hdrezkaProvider.resolve!({ ...publication, id: "hdrezka:92943:translation:238" },
        await catalog.context("hdrezka", signal)), error => error instanceof ContentError
        && error.code === "browser_check_required" && error.url === publicationUrl,
    "A challenged CDN request must offer the publication page rather than an AJAX endpoint");
    assert.equal(await catalog.checkSource("hdrezka", signal), undefined);

    const publicationPage = (name: string) => `<div class="b-post__title"><h1>${name}</h1></div>
        <div class="b-post__info"><table><tr><td>Перевод</td><td>Generated voice</td></tr></table></div>
        <script>initCDNMoviesEvents(92943, 238);</script>`;
    hdrezkaPage = publicationPage(title);
    const cleared = await catalog.checkSource("hdrezka", signal);
    assert.equal(cleared?.work.title, title);
    assert.deepEqual(cleared?.publication, publication);
    assert(cleared?.work.variants.every(variant => variant.url === publicationUrl && !variant.needsInspection));
    assert.equal(settings.health("hdrezka"), undefined);
    hdrezkaPage = publicationPage(challengeTitle + " Just a moment... DDoS-Guard cf-chl- captcha")
        + '<form action="/comments"><input name="captcha"></form><a href="/captcha">Captcha information</a>';
    const legitimate = await hdrezkaProvider.inspect!(publication, await catalog.context("hdrezka", signal));
    assert.equal(legitimate.title, challengeTitle + " Just a moment... DDoS-Guard cf-chl- captcha",
        "Challenge words in a legitimate title must not reject an actual publication page");
    hdrezkaPage = `<h1>${challengeTitle}</h1>`;
    settings.recordHealth("hdrezka", "Unknown page", { method: "inspect", variant: publication });
    assert.equal(await catalog.checkSource("hdrezka", signal), undefined);
    assert.equal(settings.health("hdrezka")?.code, "source_page_unavailable",
        "An unrecognized document must not become a successful generic-h1 fallback");

    clearance = "first";
    await sessionContext("hdrezka", signal, publicationUrl);
    assert.equal(cookieRequests.at(-1), publicationUrl);
    assert.equal((await catalog.acquisitionHeaders("hdrezka", publicationUrl, {})).Cookie, "fixture-clearance=first");
    clearance = "cleared";
    await sessionContext("hdrezka", signal, publicationUrl);
    assert.equal((await catalog.acquisitionHeaders("hdrezka", publicationUrl, {})).Cookie, "fixture-clearance=cleared",
        "A manual retry must refresh the browser session for the exact redirected publication domain");
    assert.equal((await catalog.acquisitionHeaders("hdrezka", hdrezkaProvider.source.url, {})).Cookie, undefined,
        "Publication clearance cookies must remain scoped to their original domain");

    settings.recordHealth("generated", "Persisted fixture failure");
    const reopened = await LibraryStore.open(directory);
    assert.equal(new SourceSettings(reopened).health("generated")?.message, "Persisted fixture failure");
    reopened.close();
    console.info(JSON.stringify({ ok: true, firstResultBeforeMetadata, callbackCount,
        finalWorks: output.works.size, ambiguousWorks: ambiguous.works.size,
        rankedIds: variants.map(variant => variant.id), inspections, persistedHealth: true,
        structuralBrowserCheck: true, exactPublicationRetry: true, browserCookieRefresh: true }));
}
finally {
    releaseMetadata();
    catalog.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
}
