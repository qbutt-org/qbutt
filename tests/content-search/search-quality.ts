import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Work } from "../../components/content/contracts";
import { ContentClient, completed, searchWorks } from "./client";

type RankedWork = Work & { relevance?: number };
const directory = await mkdtemp(join(tmpdir(), "qbutt-search-quality-integration-"));
const client = new ContentClient(directory);
const matrix: { query: string; count: number; ids: string[] }[] = [];

async function search(query: string, category: "anime" | "movies", sources: string[]): Promise<RankedWork[]> {
    const events = await client.request("search", { query, category, sources });
    assert.deepEqual(events.filter(event => event.event === "source" && (event.data as { status: string }).status === "error"), [],
        "The public sources must complete the live search");
    const works = searchWorks(events);
    const publications = new Set<string>();
    for (const work of works) {
        for (const variant of work.variants) {
            const key = `${variant.sourceId}:${variant.id}`;
            assert(!publications.has(key), "Alias discovery duplicated an exact publication within or across works");
            publications.add(key);
        }
    }
    matrix.push({ query, count: works.length, ids: works.slice(0, category === "anime" ? works.length : 3).map(work => work.id) });
    return works;
}

try {
    const sources = ["aniliberty", "yani"];
    const full = await search("Даже у студсовета есть косяки!", "anime", sources);
    const original = full.find(work => work.identifiers?.mal === "61578");
    assert(original?.id === "yani:20398" && original.year, "The full title must find its real catalog identity and year");
    assert.equal(original.relevance, 2);
    for (const query of ["студсовет косяки", "косяки студсовет", "студсовета косяки",
        "Seitokai ni mo Ana wa Aru!", "В студсовете тоже есть дырка!", `студсовет косяки ${original.year}`]) {
        const found = await search(query, "anime", sources);
        assert.deepEqual(found.map(work => work.id), [original.id], "A title fragment, alias, word order or release year changed the entity");
        assert.equal(found[0]!.identifiers?.mal, original.identifiers?.mal);
        assert((found[0]!.relevance || 0) > 0, "The actual title match received the derivative rank");
    }
    const wrongYear = await search(`студсовет косяки ${original.year - 1}`, "anime", sources);
    assert(!wrongYear.some(work => work.id === original.id), "An explicit conflicting release year was ignored");

    const seasons = await search("Провожающая в последний путь Фрирен", "anime", sources);
    const first = seasons.find(work => work.identifiers?.mal === "52991");
    const second = seasons.find(work => work.identifiers?.mal === "59978");
    assert(first && second && first.id !== second.id, "Different authoritative season IDs were merged");
    assert.equal(first.identifiers?.kinopoisk, second.identifiers?.kinopoisk,
        "The real fixture must exercise a shared series identifier");
    for (const season of [first, second]) {
        assert.equal(seasons.filter(work => work.identifiers?.mal === season.identifiers?.mal).length, 1);
        assert(season.variants.some(variant => variant.sourceId === "aniliberty")
            && season.variants.some(variant => variant.sourceId === "yani"), "The same exact season did not group across catalogs");
    }
    const selected = first.variants.find(variant => variant.sourceId === "yani")!;
    const inspection = await client.request("inspect", { work: first, variant: selected });
    completed(inspection);
    const details = inspection.find(event => event.event === "details")!.data as Work;
    assert.equal(details.id, first.id, "Inspecting voices changed the grouped work identity");
    assert(details.variants.some(variant => variant.sourceId === "aniliberty"), "Voice inspection lost the other exact catalog");
    const voices = details.variants.filter(variant => variant.sourceId === "yani");
    assert(new Set(voices.map(variant => variant.translation)).size > 1, "The real fixture must expose different voices");
    assert(new Set(voices.map(variant => variant.episode)).size > 1, "The real fixture must expose different episodes");
    assert.equal(new Set(details.variants.map(variant => variant.id)).size, details.variants.length,
        "Inspection duplicated an exact voice/episode publication");
    assert(voices.every(variant => variant.id.startsWith("yani:10661:video:") && variant.episode && variant.player),
        "Voice/episode selection lost its declared source identity");
    const currentYear = await search(`Провожающая в последний путь Фрирен ${second.year}`, "anime", sources);
    assert(currentYear.some(work => work.identifiers?.mal === "59978")
        && !currentYear.some(work => work.identifiers?.mal === "52991"), "A release year selected the wrong season");

    const movies = await search("Интерстеллар", "movies", ["rutube"]);
    const trailer = /трейлер|тизер|\b(?:trailer|teaser)\b/i;
    assert(movies.length && movies.every(work => !trailer.test(work.title)), "Default movie search returned trailers");
    const ranked = [...movies].sort((left, right) => (right.relevance || 0) - (left.relevance || 0));
    assert((ranked[0]!.relevance || 0) > 0
        && ranked[0]!.variants.some(variant => (variant.duration || 0) >= 40 * 60), "A short clip ranked above full movie publications");
    const derivatives = movies.filter(work => /музык|саундтрек|оркестр|отрывок|сцена/i.test(work.title));
    assert(derivatives.length && derivatives.every(work => (work.relevance || 0) === 0), "A music or clip label received the full movie rank");
    assert(movies.every(work => work.variants.length === 1 && work.variants[0]!.publicationKind === "upload"),
        "Independent user uploads were silently merged by title");
    const numberedTitle = await search("Бегущий по лезвию 2049", "movies", ["rutube"]);
    assert(numberedTitle.length && numberedTitle.every(work => /\b2049\b/.test(work.title)),
        "A number belonging to the movie title was mistaken for its release year");
    const numberedDerivatives = numberedTitle.filter(work => /lonely Blues|Apocalyptic autumn/i.test(work.title));
    assert(numberedDerivatives.length && numberedDerivatives.every(work => !work.relevance),
        "The title number was treated as release metadata on a long derivative upload");
    const numberedYear = await search("Бегущий по лезвию 2049 2017", "movies", ["rutube"]);
    assert(numberedYear.length && numberedYear.some(work => (work.relevance || 0) > 0),
        "A numbered title lost its movie rank when the actual release year was supplied");
    assert(numberedYear.every(work => /\b2049\b/.test(work.title) && /\b2017\b/.test(work.title)),
        "A numbered title or explicit release year was ignored");
    const conflictingYear = await search("Бегущий по лезвию 2049 2014", "movies", ["rutube"]);
    assert(!conflictingYear.some(work => numberedYear.some(actual => actual.id === work.id)),
        "A conflicting release year returned the same movie publication");
    const explicitTrailers = await search("Интерстеллар трейлер", "movies", ["rutube"]);
    assert(explicitTrailers.some(work => trailer.test(work.title)), "An explicit trailer request was suppressed");

    console.info(JSON.stringify({ ok: true, matrix, voices: voices.length, moviePublications: movies.length,
        firstMovie: { id: ranked[0]!.id, title: ranked[0]!.title, relevance: ranked[0]!.relevance,
            duration: ranked[0]!.variants[0]!.duration }, derivativePublications: derivatives.length,
        explicitTrailers: explicitTrailers.length }));
}
finally {
    await client.close();
    await rm(directory, { recursive: true, force: true });
}
