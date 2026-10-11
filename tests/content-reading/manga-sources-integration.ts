import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ProviderContext, Work } from "../../components/content/contracts";
import { CookieJar } from "../../components/content/node_modules/tough-cookie";
import { sourceFetch } from "../../components/content/net";
import { loadReadingChapter, readingProviders, readingPublication } from "../../components/content/providers/reading";
import { handleReaderRequest } from "../../components/content/reader/service";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";
import { openLink, share } from "../../components/content/sharing";

function require(value: unknown, message: string): asserts value {
    if (!value) throw new Error(message);
}

const directory = await mkdtemp(join(tmpdir(), "qbutt-manga-sources-"));
const image = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a0u8AAAAASUVORK5CYII="), value => value.charCodeAt(0));
const mangaId = "11111111-1111-4111-8111-111111111111";
const groupId = "22222222-2222-4222-8222-222222222222";
const otherGroupId = "33333333-3333-4333-8333-333333333333";
const chapterIds = ["44444444-4444-4444-8444-444444444444", "55555555-5555-4555-8555-555555555555", "66666666-6666-4666-8666-666666666666"];
const hash = "a".repeat(32);
let unavailableGroup = false;
let invalidPath: false | "traversal" | "dot" = false;
let enrichmentFails = false;
const reportedErrors: unknown[] = [];
const manga = { id: mangaId, attributes: { title: { en: "Reader fixture" }, altTitles: [{ ru: "Проверка чтения" }],
    availableTranslatedLanguages: ["en", "ru"], year: 2026 }, relationships: [] };
const gallery = { id: 123, media_id: "456", title: { pretty: "Reader fixture", english: "Reader fixture", japanese: "読書" },
    cover: { path: "galleries/456/cover.webp" }, upload_date: 1_700_000_000, tags: [{ type: "language", name: "english" }],
    pages: [{ number: 2, path: "galleries/456/2.png", width: 1, height: 1 }, { number: 1, path: "galleries/456/1.png", width: 1, height: 1 }] };
const context: ProviderContext = {
    dataDir: directory, settings: {}, signal: AbortSignal.timeout(120_000),
    reportError(error) { reportedErrors.push(error); },
    async fetch(input, init) {
        const url = new URL(input);
        const response = (data: unknown, status = 200) => Response.json(data, { status });
        if (url.hostname === "api.mangadex.org") {
            if (url.pathname === "/manga") return response({ result: "ok", data: [manga] });
            if (url.pathname === `/manga/${mangaId}`) return response({ result: "ok", data: manga });
            if (url.pathname.endsWith("/feed")) {
                return response({ result: "ok", total: 3, data: chapterIds.map((id, index) => ({ id,
                    attributes: { volume: "1", chapter: String(index === 2 ? 1 : index + 1), pages: 1,
                        translatedLanguage: "ru", publishAt: "2026-01-01T00:00:00Z" },
                    relationships: [{ id: index === 2 ? otherGroupId : groupId, type: "scanlation_group",
                        attributes: { name: index === 2 ? "Other group" : "Reader group" } }],
                })).filter(entry => !unavailableGroup || entry.relationships[0]!.id !== groupId) });
            }
            if (url.pathname.startsWith("/at-home/server/")) return response({ result: "ok", baseUrl: "https://uploads.mangadex.org/session-prefix",
                chapter: { hash, data: [invalidPath === "dot" ? ".." : invalidPath ? "../outside.png" : "fixture.png"] } });
        }
        if (url.hostname === "uploads.mangadex.org") {
            require(url.pathname === `/session-prefix/data/${hash}/fixture.png`, "MangaDex lost the server's access prefix");
            return new Response(image, { headers: { "Content-Type": "image/png" } });
        }
        if (url.hostname === "nhentai.net") {
            if (url.pathname.endsWith("/cdn")) return response({ image_servers: ["https://i1.nhentai.net"], thumb_servers: ["https://t1.nhentai.net"] });
            if (url.pathname.endsWith("/search")) return response({ result: [{ id: 123, media_id: "456", english_title: "Reader fixture", thumbnail: "galleries/456/thumb.webp" }] });
            if (url.pathname.endsWith("/galleries/123")) return response(invalidPath ? { ...gallery,
                pages: [{ number: 1, path: invalidPath === "dot" ? "galleries/456/.." : "https://foreign.invalid/1.png" }] } : gallery);
        }
        if (url.hostname === "i1.nhentai.net") {
            require(new Headers(init?.headers).get("Referer") === "https://nhentai.net/", "nhentai reader omitted its source referer");
            return new Response(image, { headers: { "Content-Type": "image/png" } });
        }
        if (url.hostname === "api.cdnlibs.org") {
            const book = { id: 9, slug_url: "9--reader-fixture", name: "Reader fixture", rus_name: "Проверка чтения" };
            if (url.pathname === "/api/manga") return response({ data: [book] });
            if (enrichmentFails) return response({}, 503);
            return response({ data: book });
        }
        throw new Error(`Unexpected fixture request to ${url.hostname}${url.pathname}`);
    },
};

try {
    for (const [sourceId, query] of [["mangadex", "Проверка чтения"], ["nhentai", "Reader fixture"]]) {
        const provider = readingProviders.find(provider => provider.source.id === sourceId)!;
        const emitted: Work[] = [];
        const works = await provider.search({ query: query!, category: "manga", language: "ru" }, context, work => emitted.push(work));
        require(works.length === 1 && emitted.length === 1, `${sourceId} did not emit its first result`);
        const details = await provider.inspect!(works[0]!.variants[0]!, context);
        const variant = sourceId === "mangadex" ? details.variants.find(variant => variant.translation === "Reader group")! : details.variants[0]!;
        require(variant.languages?.length === 1 && !variant.languages.includes("translated"), "Metadata invented or confused language tags");
        const link = share({ mode: "exact", work: details, variant });
        const imported = openLink(link);
        require(imported.variant?.id === variant.id && !link.includes("session-prefix"), "Exact share lost publication identity or included acquisition data");
        const document = await handleReaderRequest("reader.open", { work: imported.work, variant: imported.variant }, context) as ReaderDocument;
        require(document.chapters.length === (sourceId === "mangadex" ? 2 : 1), "Reader mixed translation groups or lost chapters");
        const page = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: document.chapters[0]!.id }, context) as ReaderPage;
        require(page.kind === "images" && page.images?.length === (sourceId === "mangadex" ? 1 : 2), "Reader lost fixture pages");
        for (const cached of page.images!) require((await stat(cached.url)).size === image.byteLength && Buffer.from(await readFile(cached.url)).equals(Buffer.from(image)), "Cached image bytes differ");
        await handleReaderRequest("reader.position", { documentId: document.id, chapterId: page.id, imageIndex: 0 }, context);
        const reopened = await handleReaderRequest("reader.open", { work: imported.work, variant: imported.variant }, context) as ReaderDocument;
        require(reopened.position?.chapterId === page.id, "Reading position did not survive exact reopening");
        const offline = { ...context, fetch: async () => { throw new Error("Offline reader cannot access the network"); } };
        const cached = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: page.id }, offline) as ReaderPage;
        require(cached.cached && cached.images?.length === page.images.length, "Offline page cache failed");
        for (const invalid of ["traversal", "dot"] as const) {
            invalidPath = invalid;
            let rejected = false;
            try { await loadReadingChapter(sourceId!, document.chapters[0]!, context); } catch { rejected = true; }
            require(rejected, `${sourceId} accepted an unsafe page path`);
        }
        invalidPath = false;
        if (sourceId === "mangadex") {
            unavailableGroup = true;
            let missingRejected = false;
            try { await provider.inspect!(variant, context); } catch { missingRejected = true; }
            require(missingRejected, "Missing MangaDex group silently switched translations");
            unavailableGroup = false;
        }
    }
    const lib = readingProviders.find(provider => provider.source.id === "mangalib")!;
    enrichmentFails = true;
    const emitted: Work[] = [];
    const partial = await lib.search({ query: "Проверка чтения", category: "manga" }, context, work => emitted.push(work));
    require(partial.length === 1 && emitted.length === 1 && reportedErrors.length === 1, "Optional enrichment lost a valid result or hid the source failure");
    const recognized = [
        [`https://mangadex.org/title/${mangaId}/reader-fixture`, `mangadex:${mangaId}`],
        ["https://nhentai.net/g/123/", "nhentai:123"],
        ["https://mangalib.me/ru/9--reader-fixture", "mangalib:9--reader-fixture"],
    ];
    for (const [url, id] of recognized) require(readingPublication(new URL(url!))?.id === id, "Public reader URL identity failed");
    for (const url of ["https://nhentai.net.evil.invalid/g/123/", "https://nhentai.net/g/123/?token=private", "https://user:password@nhentai.net/g/123/"])
        require(!readingPublication(new URL(url)), "Reader URL accepted a foreign host or credentials");
    console.info("PASS manga search → progressive result → exact language/group → share/reopen → generated image bytes/offline cache; unsafe paths and missing translation rejected; optional enrichment preserves valid result and reports health");

    if (process.argv.includes("--live-metadata")) {
        const signal = AbortSignal.timeout(120_000);
        const live: ProviderContext = { dataDir: directory, settings: {}, signal, fetch: sourceFetch(signal, new CookieJar()) };
        const dex = readingProviders.find(provider => provider.source.id === "mangadex")!;
        for (const query of ["Ore No Kokan Wa Bishoujo Datta No Ka", "мой член оказался девушкой"]) {
            const started = performance.now();
            const works = await dex.search({ query, category: "manga", language: "ru" }, live);
            const work = works.find(work => work.identifiers?.mangadex === "cf923481-1e96-4959-b3c1-9518718a4cc2");
            require(work, "Live MangaDex did not find the requested manga");
            const details = await dex.inspect!(work.variants[0]!, live);
            require(details.variants.some(variant => variant.languages?.includes("ru")) && details.chapters?.length, "Live MangaDex omitted real RU translations/chapters");
            console.info(JSON.stringify({ source: "MangaDex", query, milliseconds: Math.round(performance.now() - started), variants: details.variants.length, chapters: details.chapters.length }));
        }
        const adult = readingProviders.find(provider => provider.source.id === "nhentai")!;
        const result = await adult.search({ query: "qbutt-generated-qa-no-gallery-6f1864f0", category: "manga" }, live);
        require(result.length === 0, "nhentai did not respect the impossible metadata query");
        const link = readingPublication(new URL("https://nhentai.net/g/686751/"))!;
        const details = await adult.inspect!(link, live);
        const cover = await live.fetch(details.cover!, { method: "HEAD", headers: { Referer: "https://nhentai.net/" } });
        require(cover.ok && cover.headers.get("Content-Type")?.startsWith("image/"), "nhentai reported an unusable cover path");
        await cover.body?.cancel();
        const found = await adult.search({ query: `"${details.title}"`, category: "manga" }, live);
        require(found.some(work => work.id === details.id), "nhentai anonymous search omitted the inspected gallery");
        const page = await loadReadingChapter("nhentai", details.chapters![0]!, live);
        require(page.images?.length, "nhentai anonymous gallery omitted image metadata");
        const head = await live.fetch(page.images[0]!.url, { method: "HEAD", headers: { Referer: "https://nhentai.net/" } });
        require(head.ok && head.headers.get("Content-Type")?.startsWith("image/"), "nhentai CDN did not serve its reported image path");
        await head.body?.cancel();
        console.info(JSON.stringify({ source: "nhentai", search: "anonymous positive and empty", gallery: "anonymous metadata",
            pageCount: page.images.length, coverHead: cover.status, imageHead: head.status, imageBodiesRequested: 0 }));
    }
}
finally {
    await rm(directory, { recursive: true, force: true });
}
