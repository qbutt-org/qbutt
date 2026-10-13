import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { zipSync } from "../../components/content/node_modules/fflate";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";
import { ContentClient, completed } from "../content-search/client";

const directory = await mkdtemp(join(tmpdir(), "qbutt-reader-ipc-"));
let client: ContentClient | undefined;

try {
    const textPath = join(directory, "original.txt");
    const comicPath = join(directory, "pages.cbz");
    await writeFile(textPath, "凛は庭に入りました。\n猫は窓辺で待っていました。\n");
    const image = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a0u8AAAAASUVORK5CYII="), value => value.charCodeAt(0));
    await writeFile(comicPath, zipSync({ "page10.png": image, "page2.png": image, "page1.png": image }));

    client = new ContentClient(directory);
    const textEvents = await client.request("reader.local", { path: textPath });
    completed(textEvents);
    const text = textEvents.at(-1)!.data as ReaderDocument;
    const chapterEvents = await client.request("reader.chapter", {
        documentId: text.id, chapterId: text.chapters[0]!.id,
    });
    completed(chapterEvents);
    const chapter = chapterEvents.at(-1)!.data as ReaderPage;
    assert(chapter.html?.includes("凛は庭に入りました。"), "Worker lost the original text");

    const comicEvents = await client.request("reader.local", { path: comicPath });
    completed(comicEvents);
    const comic = comicEvents.at(-1)!.data as ReaderDocument;
    const pageEvents = await client.request("reader.chapter", {
        documentId: comic.id, chapterId: comic.chapters[0]!.id,
    });
    completed(pageEvents);
    const pages = pageEvents.at(-1)!.data as ReaderPage;
    assert.equal(pages.images?.length, 3);
    assert.deepEqual(pages.images!.map(page => page.url.split(/[\\/]/).pop()),
        ["page1.png", "page2.png", "page10.png"]);

    const positions = [
        { documentId: text.id, chapterId: chapter.id, offset: 0.83 },
        { documentId: comic.id, chapterId: pages.id, imageIndex: 2, offset: 0.37 },
    ];
    const positionEvents = await Promise.all(positions.map(params => client!.request("reader.position", params)));
    for (const events of positionEvents) completed(events);
    await client.close();
    client = undefined;

    client = new ContentClient(directory);
    const restoredTextEvents = await client.request("reader.local", { path: textPath });
    completed(restoredTextEvents);
    const restoredText = restoredTextEvents.at(-1)!.data as ReaderDocument;
    assert.equal(restoredText.id, text.id);
    assert.deepEqual(restoredText.position, { chapterId: chapter.id, offset: 0.83 });
    const restoredComicEvents = await client.request("reader.local", { path: comicPath });
    completed(restoredComicEvents);
    const restoredComic = restoredComicEvents.at(-1)!.data as ReaderDocument;
    assert.equal(restoredComic.id, comic.id);
    assert.deepEqual(restoredComic.position, { chapterId: pages.id, imageIndex: 2, offset: 0.37 });
    const cachedEvents = await client.request("reader.chapter", { documentId: text.id, chapterId: chapter.id });
    completed(cachedEvents);
    const cached = cachedEvents.at(-1)!.data as ReaderPage;
    assert(cached.cached && cached.html === chapter.html, "Fresh worker changed the cached original");
    await client.close();
    client = undefined;
    console.info("PASS original reader IPC: text/CBZ, natural page order, confirmed positions + clean EOF, exact image/scroll restart, cached original");
}
finally {
    await client?.close();
    await rm(directory, { recursive: true, force: true });
}
