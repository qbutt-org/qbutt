import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { zipSync, strToU8 } from "../../components/content/node_modules/fflate";
import { handleReaderRequest } from "../../components/content/reader/service";
import type { ProviderContext } from "../../components/content/contracts";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";

function require(value: unknown, message: string): asserts value {
    if (!value) throw new Error(message);
}

const root = await mkdtemp(join(tmpdir(), "qbutt-glossary-integration-"));
const requests: { blocks: { id: string; text: string }[]; glossary: Record<string, string> }[] = [];
const terms = { 璃央: "Рио", 星渡りの門: "Звёздные врата", 千景: "Тикагэ", 弥生: "Яёй", 月影隊: "Лунный отряд", 翠: "Мидори", 楓: "Каэдэ" };
let pause: { started: () => void; wait: Promise<void> } | undefined;
let invalidTerm = false;
const server = Bun.serve({
    hostname: "127.0.0.1", port: 0,
    async fetch(request) {
        require(request.headers.get("authorization") === "Bearer generated-glossary-fixture", "API authorization missing");
        require(!request.headers.has("cookie"), "Source cookies reached AI transport");
        const body = await request.json() as any;
        if (body.response_format.json_schema.schema.properties.regions) {
            return Response.json({ choices: [{ finish_reason: "stop", message: { content: JSON.stringify({
                regions: [{ id: "region-1", text: "璃央は猫を見ました。", x: 0.1, y: 0.2, width: 0.4, height: 0.3,
                    layout: { x: 0.1, y: 0.2, width: 0.4, height: 0.3 }, orientation: "horizontal", ink: "dark",
                    inkBoxes: [{ x: 0.1, y: 0.2, width: 0.4, height: 0.3 }] }],
            }) } }] });
        }
        require(body.response_format.json_schema.schema.required.includes("terms"), "Terms were not requested with the translation");
        const prompt = JSON.parse(body.messages[1].content);
        requests.push(prompt);
        const gate = pause;
        pause = undefined;
        if (gate) {
            gate.started();
            await gate.wait;
        }
        const source = prompt.blocks.map((block: { text: string }) => block.text).join("\n");
        const learned = invalidTerm ? [{ source: "存在しない名前", target: "Отсутствующее имя" }]
            : Object.entries(terms).filter(([term]) => source.includes(term)).map(([source, target]) => ({ source, target }));
        const value = {
            blocks: prompt.blocks.map((block: { id: string; text: string }) => {
                let text = block.text;
                for (const [source, target] of Object.entries(terms)) text = text.replaceAll(source, prompt.glossary[source] ?? target);
                return { id: block.id, text: `Перевод: ${text}` };
            }),
            terms: learned,
        };
        return Response.json({ choices: [{ finish_reason: "stop", message: { content: JSON.stringify(value) } }] });
    },
});
const context: ProviderContext = {
    appDir: process.env.QBUTT_CONTENT_APP_DIR,
    signal: new AbortController().signal,
    dataDir: join(root, "profile"),
    settings: { aiProvider: "openai-compatible", aiBaseUrl: `http://127.0.0.1:${server.port}/v1`,
        aiApiKey: "generated-glossary-fixture", aiModel: "glossary-fixture" },
    fetch: async () => { throw new Error("AI must use its own transport"); },
};
const request = (method: string, params: Record<string, unknown>) => handleReaderRequest(method, params, context);
const glossaryPath = (document: ReaderDocument) => join(context.dataDir, "reader", "glossaries", document.id + ".json");
const glossary = (document: ReaderDocument) => request("reader.glossary", { documentId: document.id }) as Promise<Record<string, string>>;
const save = (document: ReaderDocument, entries: Record<string, string>) => request("reader.glossary", { documentId: document.id, entries });

function holdNextTranslation(): { started: Promise<void>; release: () => void } {
    let release!: () => void;
    let started!: () => void;
    const wait = new Promise<void>(resolve => { release = resolve; });
    const beginning = new Promise<void>(resolve => { started = resolve; });
    pause = { started, wait };
    return { started: beginning, release };
}

try {
    const chapters = [
        "<p>璃央は星渡りの門に入りました。</p>",
        "<p>璃央は星渡りの門で待っています。</p>",
        "<p>璃央と月影隊は庭に入りました。</p>",
        "<p>弥生は庭に入りました。</p>",
        "<p>翠は庭に入りました。</p>",
        "<p>楓は庭に入りました。</p>",
    ];
    const archive: Record<string, Uint8Array> = {
        "META-INF/container.xml": strToU8('<container><rootfile full-path="book.opf"/></container>'),
        "book.opf": strToU8(`<package><metadata><title>Legal glossary fixture</title></metadata><manifest>${chapters.map((_, index) => `<item id="c${index}" href="c${index}.html"/>`).join("")}</manifest><spine>${chapters.map((_, index) => `<itemref idref="c${index}"/>`).join("")}</spine></package>`),
    };
    chapters.forEach((html, index) => { archive[`c${index}.html`] = strToU8(html); });
    const path = join(root, "story.epub");
    await writeFile(path, zipSync(archive));
    const document = await request("reader.local", { path }) as ReaderDocument;
    const pages = await Promise.all(document.chapters.map(chapter => request("reader.chapter", {
        documentId: document.id, chapterId: chapter.id,
    }) as Promise<ReaderPage>));

    await request("reader.translate", { page: pages[0] });
    require(Number(requests.length) === 1 && !Object.keys(requests[0]!.glossary).length, "Learning made an extra AI request");
    require((await glossary(document)).璃央 === "Рио", "First chapter did not learn its proper name");
    const firstState = await readFile(glossaryPath(document), "utf8");
    require(Object.keys(JSON.parse(firstState).manual).length === 0, "Learned names were marked as manual");
    await save(document, await glossary(document));
    require(await readFile(glossaryPath(document), "utf8") === firstState, "Identical Save changed glossary ownership");
    await request("reader.translate", { page: pages[0] });
    require(Number(requests.length) === 1, "Learned entries invalidated the completed stage");

    // A saved block survives a lost glossary write and stage publication.
    await writeFile(glossaryPath(document), JSON.stringify({ entries: {}, manual: {} }));
    const stages = join(context.dataDir, "reader", "translations");
    for (const name of await readdir(stages)) await rm(join(stages, name));
    await request("reader.translate", { page: pages[0] });
    require(Number(requests.length) === 1 && (await glossary(document)).璃央 === "Рио", "Block cache did not replay learned terms after interrupted persistence");

    await save(document, { ...await glossary(document), 無関係: "Посторонний термин" });
    await request("reader.translate", { page: pages[0] });
    require(Number(requests.length) === 1, "Unrelated manual entry invalidated unchanged blocks");
    await request("reader.translate", { page: pages[1] });
    require(Number(requests.length) === 2 && requests[1]!.glossary.璃央 === "Рио"
        && requests[1]!.glossary.星渡りの門 === "Звёздные врата", "Next chapter did not receive learned terms");

    await save(document, { ...await glossary(document), 璃央: "Лио" });
    const overridden = await request("reader.translate", { page: pages[1] }) as ReaderPage;
    require(overridden.html?.includes("Лио") && (await glossary(document)).璃央 === "Лио", "AI overwrote the user's name");
    const withoutGate = await glossary(document);
    delete withoutGate.星渡りの門;
    await save(document, withoutGate);
    await request("reader.translate", { page: pages[1] });
    require(!Object.hasOwn(await glossary(document), "星渡りの門"), "Deleted term was automatically added again");
    require(JSON.parse(await readFile(glossaryPath(document), "utf8")).manual.星渡りの門 === null, "Manual deletion did not leave a tombstone");

    const relevantRace = holdNextTranslation();
    const stale = request("reader.translate", { page: pages[2] }).then(() => false, () => true);
    await relevantRace.started;
    await save(document, { ...await glossary(document), 璃央: "Ручное имя" });
    relevantRace.release();
    require(await stale, "A translation using an obsolete manual glossary was published");
    require((await glossary(document)).璃央 === "Ручное имя", "Concurrent AI merge lost a manual edit");
    const current = await request("reader.translate", { page: pages[2] }) as ReaderPage;
    require(current.html?.includes("Ручное имя"), "Retry did not use the current manual glossary");

    const unrelatedRace = holdNextTranslation();
    const continuing = request("reader.translate", { page: pages[3] });
    await unrelatedRace.started;
    await save(document, { ...await glossary(document), 別の無関係: "Ещё один посторонний термин" });
    const beforeRelease = requests.length;
    unrelatedRace.release();
    await continuing;
    await request("reader.translate", { page: pages[3] });
    require(Number(requests.length) === beforeRelease && (await glossary(document)).弥生 === "Яёй", "Unrelated concurrent edit restarted AI or lost learned terms");

    const learningRace = holdNextTranslation();
    const firstLearning = request("reader.translate", { page: pages[4] });
    await learningRace.started;
    await request("reader.translate", { page: pages[5] });
    learningRace.release();
    await firstLearning;
    const combined = await glossary(document);
    require(combined.翠 === "Мидори" && combined.楓 === "Каэдэ", "Concurrent automatic merges lost a name");

    const parallelBooks: ReaderPage[] = [];
    for (const [index, name] of ["Имя в первой книге", "Имя во второй книге"].entries()) {
        const path = join(root, `separate-${index}.html`);
        await writeFile(path, `<!-- Separate original ${index} --><p>璃央は庭に入りました。</p>`);
        const separate = await request("reader.local", { path }) as ReaderDocument;
        await writeFile(glossaryPath(separate), JSON.stringify({ entries: { 璃央: name }, manual: {} }));
        parallelBooks.push(await request("reader.chapter", { documentId: separate.id, chapterId: separate.chapters[0]!.id }) as ReaderPage);
    }
    const separated = await Promise.all(parallelBooks.map(page => request("reader.translate", { page }) as Promise<ReaderPage>));
    require(separated[0]!.html?.includes("Имя в первой книге") && separated[1]!.html?.includes("Имя во второй книге"),
        "Different documents shared a block cache despite conflicting learned names");

    const legacyPath = join(root, "legacy.html");
    await writeFile(legacyPath, "<p>千景は璃央を待っています。</p>");
    const legacy = await request("reader.local", { path: legacyPath }) as ReaderDocument;
    const legacyPage = await request("reader.chapter", { documentId: legacy.id, chapterId: legacy.chapters[0]!.id }) as ReaderPage;
    await writeFile(glossaryPath(legacy), JSON.stringify({ 璃央: "Старое ручное имя" }));
    const legacyTranslation = await request("reader.translate", { page: legacyPage }) as ReaderPage;
    require(legacyTranslation.html?.includes("Старое ручное имя"), "Legacy glossary was not preserved");
    const migrated = JSON.parse(await readFile(glossaryPath(legacy), "utf8"));
    require(migrated.manual.璃央 === "Старое ручное имя" && migrated.entries.千景 === "Тикагэ"
        && !Object.hasOwn(migrated.manual, "千景"), "Legacy migration confused learned and manual terms");

    const image = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a0u8AAAAASUVORK5CYII="), value => value.charCodeAt(0));
    const comicPath = join(root, "parallel.cbz");
    await writeFile(comicPath, zipSync({ "1.png": image, "2.png": image }));
    const comic = await request("reader.local", { path: comicPath }) as ReaderDocument;
    const comicPage = await request("reader.chapter", { documentId: comic.id, chapterId: comic.chapters[0]!.id }) as ReaderPage;
    await Promise.all([0, 1].map(imageIndex => request("reader.translate", { page: comicPage, imageIndex })));
    const merged = await request("reader.translate", { page: comicPage }) as ReaderPage;
    require(merged.regions?.length === 2 && merged.translatedImages?.join(",") === "0,1", "Concurrent image stages lost translated pages");
    await Promise.all(merged.regions.map((region, index) => request("reader.correct", {
        documentId: comic.id, pageId: comicPage.id, regionId: region.id, text: `Ручная правка ${index}`,
    })));
    const corrected = await request("reader.translate", { page: comicPage }) as ReaderPage;
    require(corrected.regions?.every(region => region.translation.startsWith("Ручная правка")), "Concurrent corrections overwrote each other");

    const malformedPath = join(root, "malformed.html");
    await writeFile(malformedPath, "<p>千景は帰りました。</p>");
    const malformed = await request("reader.local", { path: malformedPath }) as ReaderDocument;
    const malformedPage = await request("reader.chapter", { documentId: malformed.id, chapterId: malformed.chapters[0]!.id }) as ReaderPage;
    invalidTerm = true;
    const rejected = await request("reader.translate", { page: malformedPage }).then(() => false, () => true);
    require(rejected && !Object.keys(await glossary(malformed)).length, "Hallucinated source term entered the glossary");
    console.info("PASS same-request learned glossary, next chapter, manual priority/tombstones, stage/block cache, interrupted persistence, legacy migration, concurrent edits/images/corrections, invalid-term rejection");
}
finally {
    pause = undefined;
    server.stop(true);
    await rm(root, { recursive: true, force: true });
}
