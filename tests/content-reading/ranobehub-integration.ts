import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { ProviderContext, Variant } from "../../components/content/contracts";
import { readingProviders } from "../../components/content/providers/reading";
import { handleReaderRequest } from "../../components/content/reader/service";
import { contentId, type ReaderDocument, type ReaderPage } from "../../components/content/reader/types";

const root = await mkdtemp(join(tmpdir(), "qbutt-ranobehub-chapters-"));
console.log(JSON.stringify({ fixtureRoot: root }));
const dataDir = join(root, "profile");
const entries = Array.from({ length: 252 }, (_, index) => {
    const volume = index < 125 ? 1 : index < 251 ? 2 : undefined;
    const number = volume === 2 ? index - 124 : index + 1;
    return { id: 700000 + index, volume, number,
        title: number === 1 ? "Пролог" : number === 2 ? "Глава 1" : `Generated chapter ${number}` };
});
let includeVolumes = false;
const offsets: number[] = [];
const fetchedChapters: number[] = [];
const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(request) {
        const url = new URL(request.url);
        if (url.pathname === "/api/books/900001/chapters") {
            assert.equal(url.searchParams.get("limit"), "250");
            const offset = Number(url.searchParams.get("offset"));
            assert([0, 250].includes(offset), "Provider requested an unexpected chapter page");
            offsets.push(offset);
            const items = entries.slice(offset, offset + 250).map(entry => {
                const { volume, ...chapter } = entry;
                return includeVolumes ? entry : chapter;
            });
            return Response.json({ total: entries.length, items });
        }
        const chapterId = /^\/api\/chapters\/(\d+)$/.exec(url.pathname)?.[1];
        const chapter = entries.find(entry => String(entry.id) === chapterId);
        if (chapter) {
            fetchedChapters.push(chapter.id);
            return Response.json({ chapter: { title: chapter.title,
                html: `<p>Generated legal chapter ${chapter.id}, volume ${chapter.volume}.</p>` } });
        }
        return new Response("Unknown generated fixture route", { status: 404 });
    },
});
const context: ProviderContext = {
    signal: AbortSignal.timeout(30000),
    dataDir,
    settings: {},
    fetch(input, init) {
        const url = new URL(input);
        assert.equal(url.origin, "https://ranobehub.org", "The fixture attempted another source");
        return fetch(`http://127.0.0.1:${server.port}${url.pathname}${url.search}`, init);
    },
};
const variant: Variant = { id: "ranobehub:900001-generated-book", sourceId: "ranobehub",
    sourceName: "RanobeHub", title: "Generated book with repeated chapter titles", transport: "book",
    url: "https://ranobehub.org/ranobe/900001-generated-book" };
const provider = readingProviders.find(entry => entry.source.id === variant.sourceId)!;
let passed = false;

try {
    // The first opening stores a real reader document without volume metadata,
    // representing a profile created before the provider retained that field.
    const legacy = await handleReaderRequest("reader.open", { variant }, context) as ReaderDocument;
    assert.equal(legacy.chapters.length, entries.length);
    assert(legacy.chapters.every(chapter => chapter.volume === undefined));
    const selected = legacy.chapters[125]!;
    assert.equal(selected.title, legacy.chapters[0]!.title);
    assert.notEqual(selected.id, legacy.chapters[0]!.id);
    const page = await handleReaderRequest("reader.chapter", {
        documentId: legacy.id, chapterId: selected.id,
    }, context) as ReaderPage;
    assert(page.html?.includes(`Generated legal chapter ${entries[125]!.id}, volume 2.`));
    assert.deepEqual(fetchedChapters, [entries[125]!.id]);
    await handleReaderRequest("reader.position", {
        documentId: legacy.id, chapterId: selected.id, offset: 0.42,
    }, context);

    includeVolumes = true;
    offsets.length = 0;
    const details = await provider.inspect!(variant, context);
    assert.deepEqual(offsets, [0, 250]);
    assert.deepEqual(details.chapters!.map(chapter => ({ id: chapter.id, url: chapter.url, title: chapter.title })),
        entries.map(entry => {
            const url = `https://ranobehub.org/api/chapters/${entry.id}`;
            return { id: contentId(url), url, title: entry.title };
        }), "Volume metadata changed existing chapter identities, source titles or order");
    assert.deepEqual(details.chapters!.map(chapter => chapter.volume), entries.map(entry => entry.volume));
    assert.equal(details.chapters![251]!.volume, undefined, "An unknown volume was invented");

    const reopened = await handleReaderRequest("reader.open", { variant }, context) as ReaderDocument;
    assert.equal(reopened.id, legacy.id);
    assert.deepEqual(reopened.chapters.map(chapter => chapter.id), legacy.chapters.map(chapter => chapter.id));
    assert.deepEqual(reopened.position, { chapterId: selected.id, offset: 0.42 });
    const resumed = reopened.chapters.find(chapter => chapter.id === reopened.position!.chapterId)!;
    assert.equal(resumed.url, selected.url);
    assert.equal(resumed.volume, 2, "Resume selected the same title from another volume");
    const offline = { ...context, fetch: async () => { throw new Error("Fixture source is offline"); } };
    const cachedDocument = await handleReaderRequest("reader.open", { variant }, offline) as ReaderDocument;
    assert.deepEqual(cachedDocument.chapters, JSON.parse(JSON.stringify(reopened.chapters)));
    assert.deepEqual(cachedDocument.position, reopened.position);
    const cachedPage = await handleReaderRequest("reader.chapter", {
        documentId: cachedDocument.id, chapterId: cachedDocument.position!.chapterId,
    }, offline) as ReaderPage;
    assert.equal(cachedPage.cached, true);
    assert.equal(cachedPage.html, page.html);
    assert.deepEqual(fetchedChapters, [entries[125]!.id], "Metadata enrichment fetched or substituted the saved chapter");

    const result = { status: "passed", chapters: entries.length, paginationOffsets: [0, 250],
        repeatedPrologueAndFirstChapterAcrossVolumes: true, unknownVolumePreserved: true,
        urlDerivedIdsAndOrderPreserved: true, legacyProfileResumeVolume: resumed.volume,
        savedChapterId: resumed.id, savedOffset: reopened.position!.offset,
        originalCachedChapterPreserved: true, enrichedMetadataAvailableOffline: true };
    await writeFile(join(root, "result.json"), JSON.stringify(result, null, 2));
    passed = true;
    console.log(JSON.stringify({ ...result, evidence: join(root, "result.json") }));
}
finally {
    server.stop(true);
    if (passed)
        await rm(dataDir, { recursive: true, force: true });
}
