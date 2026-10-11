import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Variant, Work } from "../../components/content/contracts";
import type { LibraryEntry, VariantFilters } from "../../components/content/store";
import { completed, ContentClient } from "./client";

const directory = await mkdtemp(join(tmpdir(), "qbutt-history-"));
let client = new ContentClient(directory);
const variants: Variant[] = Array.from({ length: 80 }, (_, index) => ({
    id: `episode:${index % 40}`, sourceId: index < 40 ? "yani" : "rutube",
    sourceName: index < 40 ? "YummyAnime" : "Rutube", title: `Publication episode ${index % 40}`,
    url: `https://fixture.example/${index}?token=history-private&episode=${index % 40}`,
    downloadUrl: "https://media.example/expiring?sig=history-private", transport: "video",
    episode: String(index % 40), translation: "Studio B", languages: ["ru"],
}));
const work: Work = { id: "canonical-series", title: "Original complete series", category: "anime",
    identifiers: { shikimori: "12345" }, variants };
const selected = variants[70]!;
selected.player = "CVH";
const filters: VariantFilters = { episode: { episode: "30", season: "" }, translation: "Studio B", language: "ru",
    source: { sourceId: selected.sourceId, player: "CVH", transport: "video" } };

async function remember(params: Record<string, unknown>): Promise<LibraryEntry> {
    const events = await client.request("remember", params, 10_000);
    completed(events);
    return events.at(-1)!.data as LibraryEntry;
}

async function history(): Promise<LibraryEntry[]> {
    const events = await client.request("library", { mode: "history" }, 10_000);
    completed(events);
    return events.filter(event => event.event === "library").flatMap(event => event.data as LibraryEntry[]);
}

try {
    await remember({ work, variant: selected,
        variantFilters: { ...filters, acquisitionUrl: "https://media.example/?token=history-private" }, opened: true });
    await client.close();
    client = new ContentClient(directory);
    let entries = await history();
    assert.equal(entries.length, 1);
    let entry = entries[0]!;
    assert.equal(entry.work.title, work.title);
    assert.equal(entry.work.variants.length, 80, "History must retain the entire known inventory, including source ID collisions");
    assert.equal(entry.variant?.sourceId, selected.sourceId);
    assert.equal(entry.variant?.id, selected.id, "History must retain the exact last played publication beyond the old 32-row limit");
    assert.deepEqual(entry.variantFilters, filters, "Episode, translation, source/player and language must survive process restart");
    assert(!JSON.stringify(entry).includes("history-private"));
    assert(entry.work.variants.every(variant => !variant.downloadUrl));
    assert(entry.work.variants.every(variant => new URL(variant.url).searchParams.has("episode")));

    await remember({ work: { ...work, id: "regrouped-series", variants: [variants[0]] },
        variant: variants[0], variantFilters: {}, favorite: true });
    entries = await history();
    assert.equal(entries.length, 1, "Regrouping must not create a second history row");
    entry = entries[0]!;
    assert.equal(entry.work.id, work.id);
    assert.equal(entry.work.variants.length, 80, "A later partial library update must not discard other known publications");
    assert.deepEqual(entry.variantFilters, filters);
    assert.equal(entry.variant?.sourceId, selected.sourceId);

    await remember({ work, variant: selected, variantFilters: {}, opened: true });
    await client.close();
    client = new ContentClient(directory);
    assert.deepEqual((await history())[0]!.variantFilters, {}, "All dropdown defaults must remain All after restart");
    await remember({ work, variant: selected, variantFilters: { language: "", translation: "" }, opened: true });
    await client.close();
    client = new ContentClient(directory);
    assert.deepEqual((await history())[0]!.variantFilters, { language: "", translation: "" },
        "Unknown language and translation must remain distinct from All");

    const missing: Work = { ...work, id: "missing-exact", title: "Different missing publication",
        identifiers: { shikimori: "67890" }, variants: [{ ...variants[0]!, id: "replacement", sourceId: "other" }] };
    const missingEntry = await remember({ work: missing, variant: { ...selected, sourceId: "disappeared" }, opened: true });
    assert.equal(missingEntry.variant?.sourceId, "disappeared");
    assert.equal(missingEntry.work.variants.length, 1);
    assert.equal(missingEntry.work.variants[0]!.sourceId, "other",
        "A missing exact publication must remain a bookmark, not become an available edition");

    const largeText = "Original legal fixture content. ".repeat(120_000);
    for (let index = 0; index < 5; ++index) {
        await remember({ work: { id: `large:${index}`, title: `Large library work ${index}`, category: "anime",
            description: largeText, identifiers: { fixture: String(index) }, variants: [{ ...variants[0]!,
                id: `large:${index}`, sourceId: "large", url: `https://fixture.example/large/${index}` }] }, opened: true });
    }
    const streamed = await client.request("library", { mode: "history" }, 10_000);
    completed(streamed);
    const libraryFrames = streamed.filter(event => event.event === "library");
    const streamedEntries = libraryFrames.flatMap(event => event.data as LibraryEntry[]);
    assert.equal(streamedEntries.length, 7);
    assert(Buffer.byteLength(JSON.stringify(streamedEntries)) > 16 * 1024 * 1024);
    assert(libraryFrames.every(event => Buffer.byteLength(JSON.stringify(event)) <= 16 * 1024 * 1024),
        "Large History must stream complete works within the existing IPC frame bound");

    const oversized: Work = { id: "oversized-union", title: "Oversized accumulated variants", category: "anime",
        identifiers: { fixture: "oversized" }, variants: [{ ...variants[0]!, id: "large-first", sourceId: "oversized",
            notes: [{ text: "x".repeat(9 * 1024 * 1024), url: "https://fixture.example/note" }] }] };
    await remember({ work: oversized, opened: true });
    const rejected = await client.request("remember", { work: { ...oversized,
        variants: [{ ...oversized.variants[0]!, id: "large-second" }] }, opened: true }, 10_000);
    assert.equal(rejected.at(-1)!.event, "error");
    assert.match((rejected.at(-1)!.data as { message: string }).message, /16 MiB.*Remove/);
    const retained = (await history()).find(entry => entry.work.id === oversized.id)!;
    assert.equal(retained.work.variants.length, 1, "Rejected oversized union must leave the previous valid inventory intact");
    assert.equal(retained.work.variants[0]!.id, "large-first");
    console.info(JSON.stringify({ ok: true, knownVariants: 80, processRestarts: 3,
        exactSourceIdentity: true, allAndUnknownFilters: true, sanitisedReferences: true,
        streamedHistoryBytes: Buffer.byteLength(JSON.stringify(streamedEntries)), oversizedUnionRejected: true }));
}
finally {
    await client.close();
    await rm(directory, { recursive: true, force: true });
}
