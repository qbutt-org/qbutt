import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Variant, Work } from "../../components/content/contracts";
import { contentId } from "../../components/content/reader/types";
import { ContentClient, completed, searchWorks as works } from "./client";

const root = await mkdtemp(join(tmpdir(), "qbutt-catalog-integration-"));

const client = new ContentClient(root);
try {
    const games = works(await client.request("search", {
        query: "Schedule I", category: "games", sources: ["byrut", "freetp"],
    }));
    const matchingGames = games.filter(work => work.title === "Schedule I");
    assert.equal(matchingGames.length, 1, "The same game must have one work across publications and sources");
    const game = matchingGames[0]!;
    assert(game.variants.some(variant => variant.sourceId === "byrut"));
    assert(game.variants.some(variant => variant.sourceId === "freetp"));
    const byrut = game.variants.filter(variant => variant.sourceId === "byrut");
    assert(new Set(byrut.map(variant => variant.url)).size >= 2,
        "The public fixture must include two distinct ByRut publications for this game");
    const selected = game.variants.find(variant => variant.sourceId === "freetp")!;
    const retained = game.variants.filter(variant => variant.sourceId !== selected.sourceId || variant.url !== selected.url);
    const inspection = await client.request("inspect", { work: game, variant: selected });
    completed(inspection);
    const details = inspection.find(event => event.event === "details")?.data as Work | undefined;
    assert(details, "The publication must return details");
    assert.equal(details.id, game.id, "Publication inspection must preserve the grouped work identity");
    for (const previous of retained) {
        assert(details.variants.some(variant => variant.id === previous.id && variant.sourceId === previous.sourceId),
            "Inspection must retain other publications, including those from the selected source");
    }
    assert(details.variants.some(variant => variant.sourceId === selected.sourceId && variant.url === selected.url
        && variant.id.includes(":link:")), "Inspection must replace the publication placeholder with real releases");

    const books = works(await client.request("search", {
        query: "Overlord", category: "books", sources: ["ranobehub", "ranobelib"],
    }));
    const overlord = books.filter(work => work.title === "Повелитель");
    assert.deepEqual(overlord.map(work => work.id).sort(), ["ranobehub:206-overlord", "ranobelib:274343--povelitel"],
        "Equal book titles without a shared author or identifier must remain separate publications");
    assert(overlord.every(book => book.variants.every(variant => !variant.author)
        && !Object.values(book.identifiers || {}).some(Boolean)),
        "The public fixture must exercise book titles without known authors or identifiers");
    for (const book of overlord) {
        assert.equal(new Set(book.variants.map(variant => variant.sourceId)).size, 1,
            "An unverified equal-title publication absorbed another source");
    }
    const webNovel = books.find(work => work.title === "Повелитель (WN)");
    assert(webNovel, "An explicitly different WN edition must remain a separate work despite its Overlord alias");
    assert(overlord.every(book => book.id !== webNovel.id));

    const webNovelInspection = await client.request("inspect", { work: webNovel, variant: webNovel.variants[0] });
    completed(webNovelInspection);
    const identifiedWebNovel = webNovelInspection.find(event => event.event === "details")!.data as Work;
    const selectedBook = identifiedWebNovel.variants.find(variant => variant.id === "syosetu:n4402bd");
    assert(selectedBook && identifiedWebNovel.identifiers?.syosetu === "n4402bd",
        "The real WN catalog must provide its exact official original link");
    completed(await client.request("remember", { work: identifiedWebNovel, variant: selectedBook, favorite: true }));
    const singleSource = works(await client.request("search", {
        query: selectedBook.url, category: "books", sources: [selectedBook.sourceId],
    })).find(work => work.variants.some(variant => variant.id === selectedBook.id));
    assert(singleSource, "The exact official publication must also be available through its direct URL");
    assert.notEqual(singleSource.id, identifiedWebNovel.id, "The fixture must exercise different catalog work identities");
    completed(await client.request("remember", { work: singleSource, variant: selectedBook, favorite: true }));
    const library = await client.request("library", {});
    completed(library);
    assert.equal((library.find(event => event.event === "library")!.data as unknown[]).length, 1,
        "Saving the same exact variant through different source sets must keep one bookmark");

    const reader = await client.request("reader.open", { work: singleSource, variant: selectedBook });
    completed(reader);
    const document = reader.find(event => event.event === "complete")!.data as { workId: string };
    assert.equal(document.workId, identifiedWebNovel.id, "Reader progress must use the canonical bookmark identity");
    completed(await client.request("progress", { id: document.workId, chapterId: "catalog-fixture", offset: 0.5 }));
    const updatedLibrary = await client.request("library", {});
    completed(updatedLibrary);
    const entries = updatedLibrary.find(event => event.event === "library")!.data as { progress?: { offset?: number } }[];
    assert.equal(entries[0]!.progress?.offset, 0.5, "Reading through another source set must update the existing bookmark");

    const anime = works(await client.request("search", {
        query: "Провожающая в последний путь Фрирен", category: "anime", sources: ["yani"],
    }));
    const firstSeason = anime.find(work => work.id === "yani:10661");
    const secondSeason = anime.find(work => work.id === "yani:15084");
    assert(firstSeason && secondSeason, "Different seasons must remain separate works despite a shared Kinopoisk series ID");
    assert.equal(firstSeason.identifiers?.kinopoisk, secondSeason.identifiers?.kinopoisk,
        "The public fixture must exercise a shared series identifier");
    assert.notEqual(firstSeason.identifiers?.mal, secondSeason.identifiers?.mal,
        "The public fixture must provide different authoritative season identifiers");

    for (const season of [firstSeason, secondSeason]) {
        completed(await client.request("remember", { work: season, variant: season.variants[0], favorite: true }));
    }
    const savedSeasons = await client.request("library", {});
    completed(savedSeasons);
    assert.equal((savedSeasons.find(event => event.event === "library")!.data as unknown[]).length, 3,
        "Saving two independent seasons must preserve two bookmarks alongside the earlier book");

    const combinedAnime = await client.request("search", {
        query: "Провожающая в последний путь Фрирен", category: "anime", sources: ["aniliberty", "yani"],
    });
    const combinedSeason = works(combinedAnime).find(work => work.variants.some(variant => variant.id === firstSeason.id));
    assert(combinedSeason?.variants.some(variant => variant.sourceId === "aniliberty"),
        "The same season must group across the two public catalogs");
    const mergedUpdates = combinedAnime.filter(event => event.event === "result")
        .map(event => event.data as Work)
        .filter(work => work.variants.some(variant => variant.id === firstSeason.id)
            && work.variants.some(variant => variant.sourceId === "aniliberty"));
    assert(mergedUpdates.length > 1, "Real alias discovery must revisit the merged catalog item");
    for (const work of mergedUpdates) {
        for (const alias of firstSeason.aliases || []) {
            assert(work.aliases?.includes(alias), "Later catalog responses must retain earlier real alternate titles");
        }
        for (const [namespace, id] of Object.entries(firstSeason.identifiers || {})) {
            assert.equal(work.identifiers?.[namespace], id, "Later catalog responses must retain earlier external identifiers");
        }
    }

    const reZero = await client.request("search", {
        query: "Re:Zero", category: "books", sources: ["ranobehub", "ranobelib"],
    });
    const describedBook = reZero.filter(event => event.event === "result").map(event => event.data as Work)
        .find(work => work.id === "ranobehub:1790-re-zero-kioku-no-kairo");
    assert(describedBook?.description, "The public fixture must include an existing source description");
    const bookInspection = await client.request("inspect", { work: describedBook, variant: describedBook.variants[0] });
    completed(bookInspection);
    const inspectedBook = bookInspection.find(event => event.event === "details")!.data as Work;
    assert.equal(inspectedBook.id, describedBook.id, "Inspection must retain the exact publication identity");
    assert.equal(inspectedBook.description, describedBook.description,
        "An undefined field from source inspection must not erase known metadata");

    const narrowSeason = works(await client.request("search", {
        query: "Провожающая в последний путь Фрирен", category: "anime", sources: ["aniliberty"],
    })).find(work => work.id === "aniliberty:9542");
    assert(narrowSeason, "The same season must be independently available through the narrower catalog");
    completed(await client.request("remember", {
        work: narrowSeason, variant: narrowSeason.variants[0], favorite: true,
    }));
    const rememberedAnime = await client.request("library", {});
    completed(rememberedAnime);
    const rememberedEntries = rememberedAnime.find(event => event.event === "library")!.data as { work: Work }[];
    assert.equal(rememberedEntries.length, 3, "Saving through the narrower catalog must reuse the existing bookmark");
    const rememberedSeason = rememberedEntries.find(entry => entry.work.id === firstSeason.id)?.work;
    assert(rememberedSeason, "The canonical bookmark identity must remain stable");
    for (const alias of firstSeason.aliases || []) {
        assert(rememberedSeason.aliases?.includes(alias), "Remembering a narrower catalog must retain known alternate titles");
    }
    for (const [namespace, id] of Object.entries(firstSeason.identifiers || {})) {
        assert.equal(rememberedSeason.identifiers?.[namespace], id,
            "Remembering a narrower catalog must retain known external identifiers");
    }

    const offlineVariant: Variant = {
        id: "syosetu:qbutt-offline-fixture", sourceId: "syosetu", sourceName: "Generated fixture",
        title: "Generated offline reader fixture", transport: "book",
        url: "https://ncode.syosetu.com/qbutt-offline-fixture/",
    };
    const savedWork: Work = {
        id: "catalog:cached-reader-fixture", title: offlineVariant.title, category: "books", variants: [offlineVariant],
    };
    completed(await client.request("remember", { work: savedWork, variant: offlineVariant, favorite: true }));

    const cachedId = contentId(offlineVariant.id);
    const cacheDir = join(root, "profile", "reader", "documents");
    await mkdir(cacheDir, { recursive: true });
    await writeFile(join(cacheDir, `${cachedId}.json`), JSON.stringify({
        id: cachedId, title: offlineVariant.title, sourceId: offlineVariant.sourceId, workId: offlineVariant.id,
        chapters: [{ id: "fixture-chapter", title: "Generated chapter", url: offlineVariant.url }],
    }));
    const cachedReader = await client.request("reader.open", {
        work: { ...savedWork, id: offlineVariant.id }, variant: offlineVariant,
    });
    completed(cachedReader);
    const cachedDocument = cachedReader.find(event => event.event === "complete")!.data as { id: string; workId: string };
    assert.equal(cachedDocument.id, cachedId, "A real source failure must preserve the generated cached document");
    assert.equal(cachedDocument.workId, savedWork.id, "Cached reading must use the canonical bookmark identity");
    completed(await client.request("reader.position", {
        documentId: cachedDocument.id, chapterId: "fixture-chapter", offset: 0.75,
    }));
    completed(await client.request("progress", {
        id: cachedDocument.workId, chapterId: "fixture-chapter", offset: 0.75,
    }));
    const cachedLibrary = await client.request("library", {});
    completed(cachedLibrary);
    const cachedEntries = cachedLibrary.find(event => event.event === "library")!.data as {
        work: Work; progress?: { offset?: number };
    }[];
    assert.equal(cachedEntries.find(entry => entry.work.id === savedWork.id)?.progress?.offset, 0.75,
        "Reading cached chapters after a source failure must update the existing bookmark");

    console.log(JSON.stringify({ ok: true, gamePublications: game.variants.length,
        inspectedVariants: details.variants.length, bookVariants: identifiedWebNovel.variants.length,
        checks: ["cross-source-grouping", "same-source-publications", "inspect-preservation",
            "unverified-book-identity-boundary", "explicit-edition-boundary",
            "bookmark-publication-identity", "canonical-reader-progress", "season-identifier-boundary",
            "season-bookmark-boundary", "alias-discovery-metadata-preservation", "description-preservation",
            "bookmark-metadata-preservation", "cached-canonical-reader-progress"] }));
}
finally {
    await client.close();
    await rm(root, { recursive: true, force: true });
}
