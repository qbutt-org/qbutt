import { mkdtemp, writeFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { zipSync, strToU8 } from "../../components/content/node_modules/fflate";
import { handleReaderRequest } from "../../components/content/reader/service";
import { readingProviders } from "../../components/content/providers/reading";
import type { ProviderContext } from "../../components/content/contracts";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";

function require(condition: unknown, message: string): asserts condition {
    if (!condition) {
        throw new Error(message);
    }
}

const root = await mkdtemp(join(tmpdir(), "qbutt-reading-integration-"));
const image = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a0u8AAAAASUVORK5CYII="), character => character.charCodeAt(0));
let networkCalls = 0;
const context: ProviderContext = {
    signal: new AbortController().signal,
    dataDir: join(root, "profile"),
    settings: {},
    async fetch(url, init) {
        ++networkCalls;
        return fetch(url, init);
    },
};

try {
    const epubPath = join(root, "日本語 test.epub");
    await writeFile(epubPath, zipSync({
        mimetype: strToU8("application/epub+zip"),
        "META-INF/container.xml": strToU8('<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>'),
        "OPS/book.opf": strToU8('<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>読書の練習</dc:title></metadata><manifest><item id="c1" href="text/one.xhtml"/><item id="c2" href="text/two.xhtml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>'),
        "OPS/text/one.xhtml": strToU8('<html><body><h1>第一章</h1><p><ruby>猫<rt>ねこ</rt></ruby>が来ました。</p><img src="../images/a.png"/><script>fetch("https://evil.invalid")</script><iframe src="https://evil.invalid"></iframe><img src="file:///C:/private.txt"/></body></html>'),
        "OPS/text/two.xhtml": strToU8('<html><body><h1>第二章</h1><p>凛は庭に入りました。</p></body></html>'),
        "OPS/images/a.png": image,
    }));
    const document = await handleReaderRequest("reader.local", { path: epubPath }, context) as ReaderDocument;
    require(document.title === "読書の練習" && document.chapters.length === 2, "EPUB metadata/spine failed");
    const page = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: document.chapters[0]!.id }, context) as ReaderPage;
    require(page.html?.includes("<ruby>") && page.html.includes("ねこ"), "Ruby was lost");
    require(!page.html?.includes("evil.invalid") && !page.html?.includes("private.txt"), "Active or external EPUB content survived");
    require(page.html?.includes("file:///") && page.html.includes("images/a.png"), "EPUB image resolution failed");
    await handleReaderRequest("reader.position", { documentId: document.id, chapterId: page.id, offset: 0.42 }, context);
    const reopened = await handleReaderRequest("reader.local", { path: epubPath }, context) as ReaderDocument;
    require(reopened.position?.offset === 0.42, "Reading position did not survive reopening");
    await handleReaderRequest("reader.glossary", { documentId: document.id, entries: { "凛": "Рин" } }, context);
    const glossary = await handleReaderRequest("reader.glossary", { documentId: document.id }, context) as Record<string, string>;
    require(glossary["凛"] === "Рин", "Glossary did not persist");
    let invalidGlossaryRejected = false;
    try {
        await handleReaderRequest("reader.glossary", { documentId: document.id, entries: 42 }, context);
    }
    catch {
        invalidGlossaryRejected = true;
    }
    require(invalidGlossaryRejected, "Invalid glossary accepted");
    const offline = { ...context, fetch: async () => { throw new Error("offline"); } };
    const cached = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: page.id }, offline) as ReaderPage;
    require(cached.cached && cached.html === page.html, "Offline chapter differs from saved original");

    const cbzPath = join(root, "pages.cbz");
    await writeFile(cbzPath, zipSync({ "page10.png": image, "page2.png": image, "page1.png": image }));
    const comic = await handleReaderRequest("reader.local", { path: cbzPath }, context) as ReaderDocument;
    const images = await handleReaderRequest("reader.chapter", { documentId: comic.id, chapterId: comic.chapters[0]!.id }, context) as ReaderPage;
    require(images.images?.map(entry => entry.url.split(/[\\/]/).pop()).join(",") === "page1.png,page2.png,page10.png", "CBZ natural ordering failed");
    for (const entry of images.images!) {
        require((await stat(entry.url)).size === image.length, "Cached image is incomplete");
    }
    await handleReaderRequest("reader.position", { documentId: comic.id, chapterId: images.id, imageIndex: 2 }, context);
    const reopenedComic = await handleReaderRequest("reader.local", { path: cbzPath }, context) as ReaderDocument;
    require(reopenedComic.position?.imageIndex === 2 && !Object.hasOwn(reopenedComic.position, "offset"),
        "Paged image position did not survive reopening");
    for (const position of [
        { imageIndex: -1 }, { imageIndex: 0.5 }, { imageIndex: Number.POSITIVE_INFINITY },
        {}, { offset: null }, { imageIndex: 2, offset: "invalid" },
    ]) {
        let invalidPositionRejected = false;
        try {
            await handleReaderRequest("reader.position", { documentId: comic.id, chapterId: images.id, ...position }, context);
        }
        catch {
            invalidPositionRejected = true;
        }
        require(invalidPositionRejected, "Empty or malformed reading position accepted");
    }

    const unsafePath = join(root, "unsafe.cbz");
    await writeFile(unsafePath, zipSync({ "../escape.png": image }));
    let rejected = false;
    try {
        await handleReaderRequest("reader.local", { path: unsafePath }, context);
    }
    catch {
        rejected = true;
    }
    require(rejected, "Unsafe ZIP entry accepted");
    require(networkCalls === 0, "Local reading unexpectedly used the network");
    console.info("PASS local EPUB/CBZ, ruby, image paths, sanitization, traversal rejection, offline cache, position, glossary");

    if (process.argv.includes("--live")) {
        for (const provider of readingProviders) {
            const query = ["kakuyomu", "syosetu"].includes(provider.source.id) ? "猫"
                : provider.source.id === "royalroad" ? "mother of learning" : provider.source.id === "linovel" ? "魔女" : "overlord";
            const results = await provider.search({ query, category: provider.source.categories[0]! }, context);
            require(results.length > 0, `${provider.source.id} search returned no works`);
            if (["ranobehub", "ranobelib", "mangalib"].includes(provider.source.id)) {
                require(results.some(work => work.aliases?.some(title => title.toLowerCase().includes("overlord"))),
                    `${provider.source.id} omitted the actual English title from its aliases`);
            }
            const variant = results[0]!.variants[0]!;
            const document = await handleReaderRequest("reader.open", { work: results[0], variant }, context) as ReaderDocument;
            require(document.chapters.length > 0, `${provider.source.id} returned no chapters`);
            if (provider.source.id !== "mangalib" || process.argv.includes("--live-images")) {
                const page = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: document.chapters[0]!.id }, context) as ReaderPage;
                require((page.html?.length ?? 0) > 20 || (page.images?.length ?? 0) > 0, `${provider.source.id} chapter has no content`);
                for (const image of page.images ?? []) {
                    require((await stat(image.url)).size > 0, "Online image is missing from the offline cache");
                }
                const offlinePage = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: page.id }, offline) as ReaderPage;
                require(offlinePage.cached && offlinePage.html === page.html && offlinePage.images?.length === page.images?.length,
                    `${provider.source.id} offline cache differs from original`);
            }
            console.info(`PASS ${provider.source.id}: ${results.length} works, ${document.chapters.length} chapters`);
        }
    }
}
finally {
    await rm(root, { recursive: true, force: true });
}
