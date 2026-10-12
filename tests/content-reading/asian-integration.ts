import assert from "node:assert/strict";
import { mkdtemp, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { load } from "../../components/content/node_modules/cheerio";
import { CookieJar } from "../../components/content/node_modules/tough-cookie";
import type { BrowserBridge } from "../../components/content/browser";
import { Catalog } from "../../components/content/catalog";
import type { ProviderContext, Work } from "../../components/content/contracts";
import { sourceFetch } from "../../components/content/net";
import { readingProviders } from "../../components/content/providers/reading";
import { originalReferences } from "../../components/content/providers/reading-originals";
import { loadAsianReadingChapter } from "../../components/content/providers/reading-asian";
import { handleReaderRequest } from "../../components/content/reader/service";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";
import { sameWork } from "../../components/content/results";
import { SourceSettings } from "../../components/content/settings";
import { LibraryStore } from "../../components/content/store";

const directory = await mkdtemp(join(tmpdir(), "qbutt-asian-reading-"));
const store = await LibraryStore.open(directory);
const browser = {
    async status() { return { selected: "auto", ready: false, browsers: [] }; },
    async cookies() { return []; },
    async close() {},
} as unknown as BrowserBridge;
const signal = AbortSignal.timeout(180_000);
const network = sourceFetch(signal, new CookieJar());
let networkCalls = 0;
const context: ProviderContext = {
    dataDir: directory,
    settings: {},
    signal,
    fetch(url, init) {
        ++networkCalls;
        return network(url, init);
    },
};
const offline = { ...context, fetch: async () => { throw new Error("Offline reading used the network"); } };

try {
    for (const expected of [
        { source: "jjwxc", query: "默读", id: "jjwxc:2771073", author: "priest", count: 25, script: /\p{Script=Han}/u },
        { source: "naver", query: "웹소설 작가 도전기", id: "naver:930259", author: "냑서", count: 15, script: /\p{Script=Hangul}/u },
    ]) {
        const provider = readingProviders.find(provider => provider.source.id === expected.source);
        assert(provider?.inspect, `${expected.source} is not registered for reading`);
        const results = await provider.search({ query: expected.query, category: "books" }, context);
        const work = results.find(work => work.id === expected.id);
        assert(work, `${expected.source} did not return the actual official publication`);
        assert.equal(work.title, expected.query);
        assert.equal(work.variants[0]!.author, expected.author);
        assert.equal(work.identifiers?.[expected.source], expected.id.split(":")[1]);
        for (const other of results.filter(other => other.id !== work.id && other.title === work.title)) {
            assert(!sameWork(work, other), "Unrelated equal-title publications were merged");
        }

        // Use the verified live publication link, rather than a title heuristic.
        const linked = originalReferences([{ source_link: work.url }], [work.title], "Local catalog title");
        const original = linked.variants[0]!;
        assert.equal(original.id, work.id);
        assert.equal(original.title, work.title);
        const details = await provider.inspect(original, context);
        assert.equal(details.id, work.id);
        assert.equal(details.chapters?.length, expected.count);
        assert(details.chapters.every((chapter, index, chapters) => index === 0 || chapter.number! > chapters[index - 1]!.number!),
            "Source pagination lost chapter order or repeated a chapter");
        if (expected.source === "jjwxc") {
            assert(details.notes?.some(note => note.text.includes("Платных глав")), "Paid chapter limitation was omitted");
            assert(details.chapters.every(chapter => !chapter.url.includes("/vip/")), "Paid chapters became readable variants");
        }

        const document = await handleReaderRequest("reader.open", { work: details, variant: original }, context) as ReaderDocument;
        assert.equal(document.chapters.length, expected.count);
        for (const selected of [document.chapters[0]!, document.chapters.at(-1)!]) {
            const page = await handleReaderRequest("reader.chapter", {
                documentId: document.id, chapterId: selected.id,
            }, context) as ReaderPage;
            assert.equal(page.kind, "text");
            assert(page.html && page.html.length > 1000 && expected.script.test(page.html), "Full original chapter is missing");
            assert(!page.html.includes("�"), "Original source encoding was corrupted");
            const $ = load(page.html);
            assert.equal($("script,iframe,form").length, 0, "Active site content reached the reader");
            for (const image of $("img").toArray()) {
                const url = $(image).attr("src")!;
                assert(url.startsWith("file:"), "Inline chapter image was not cached locally");
                assert((await stat(new URL(url))).size > 0, "Cached inline chapter image is empty");
            }
            const cached = await handleReaderRequest("reader.chapter", {
                documentId: document.id, chapterId: selected.id,
            }, offline) as ReaderPage;
            assert(cached.cached && cached.html === page.html, "Offline chapter differs from its official original");
        }
        console.info(`PASS ${expected.source}: actual search/author/identity → exact original link → ${expected.count} chapters → first/last full text → offline cache`);
    }

    const catalog = new Catalog(directory, directory, new SourceSettings(store), browser);
    const found = new Map<string, Work>();
    const errors: unknown[] = [];
    await catalog.search({
        query: "Мачеха главного героя - злодейка!", category: "books", sources: ["ranobelib", "jjwxc"],
    }, signal, (event, data) => {
        if (event === "result") {
            const work = data as Work;
            found.set(work.id, work);
        }
        else if (event === "removeResult") {
            found.delete((data as { id: string }).id);
        }
        else if (event === "source" && (data as { status: string }).status === "error") {
            errors.push(data);
        }
    });
    assert.deepEqual(errors, [], "An automatic original search failed");
    const translated = found.get("ranobelib:37956--chengwei-nan-zhu-de-fanpai-houma");
    assert(translated?.aliases?.some(title => title.includes("反派")), "Search did not enrich the exact localized title");
    assert(!translated.variants.some(variant => variant.sourceId === "jjwxc"), "A missing official link was silently invented");
    const candidate = found.get("jjwxc:4133211");
    assert(candidate?.title === "成为男主的反派后妈" && candidate.variants[0]?.author === "清越流歌",
        "One localized Catalog.search did not discover the official author publication");
    assert(!sameWork(translated, candidate), "A title candidate without shared identity was silently merged");
    const selected = await handleReaderRequest("reader.open", {
        work: candidate, variant: candidate.variants[0],
    }, context) as ReaderDocument;
    const originalPage = await handleReaderRequest("reader.chapter", {
        documentId: selected.id, chapterId: selected.chapters[0]!.id,
    }, context) as ReaderPage;
    assert(originalPage.html && originalPage.html.length > 1000 && /\p{Script=Han}/u.test(originalPage.html));
    console.info(`PASS one localized Catalog.search → native alias → separate official author candidate → ${selected.chapters.length} public chapters → original text`);

    if (process.argv.includes("--corpus")) {
        for (const expected of [
            { source: "syosetu", query: "猫", script: /[\p{Script=Hiragana}\p{Script=Katakana}]/u },
            { source: "kakuyomu", query: "猫", script: /[\p{Script=Hiragana}\p{Script=Katakana}]/u },
            { source: "royalroad", query: "mother of learning", script: /[A-Za-z]/ },
            { source: "linovel", query: "魔女", script: /\p{Script=Han}/u },
        ]) {
            const provider = readingProviders.find(provider => provider.source.id === expected.source)!;
            const results = await provider.search({ query: expected.query, category: "books" }, context);
            assert(results.length, `${expected.source} returned no actual original publications`);
            const work = results[0]!;
            const linked = originalReferences([{ source_link: work.url }], [work.title], "Localized title");
            const variant = linked.variants[0];
            assert(variant?.id === work.variants[0]!.id && variant.languages?.[0] === work.variants[0]!.languages?.[0],
                `${expected.source} original identity/language changed`);
            const document = await handleReaderRequest("reader.open", { work, variant }, context) as ReaderDocument;
            const page = await handleReaderRequest("reader.chapter", {
                documentId: document.id, chapterId: document.chapters[0]!.id,
            }, context) as ReaderPage;
            assert(page.html && expected.script.test(page.html), `${expected.source} original text was not preserved`);
            console.info(`PASS preserved ${expected.source} live source identity → original link → ${document.chapters.length} chapters → original text`);
        }
    }

    const before = networkCalls;
    for (const [sourceId, url] of [
        ["jjwxc", "https://wap.jjwxc.net/vip/2771073/26"],
        ["jjwxc", "https://wap.jjwxc.net.evil.invalid/book2/2771073/1"],
        ["naver", "https://novel.naver.com/webnovel/detail?novelId=930259&volumeNo=1"],
        ["naver", "https://novel.naver.com/challenge/detail?novelId=930259&novelId=1&volumeNo=1"],
    ]) {
        await assert.rejects(loadAsianReadingChapter(sourceId!, { id: "rejected", title: "Rejected publication", url: url! }, context),
            /Адрес главы не принадлежит/);
    }
    assert.equal(networkCalls, before, "Rejected chapter URLs reached the network");
    console.info("PASS paid/unsupported/foreign/ambiguous chapter URLs rejected before fetching");
}
finally {
    await browser.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
}
