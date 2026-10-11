import { readFile, mkdir, writeFile, stat } from "node:fs/promises";
import { basename, dirname, extname, join, posix, resolve, relative, isAbsolute } from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";
import { unzipSync } from "fflate";
import { load } from "cheerio";
import { sanitizeChapter } from "./html";
import { contentId, type ReaderDocument, type ReaderPage } from "./types";

export interface LocalDocument extends ReaderDocument {
    local: true;
    assetRoot: string;
    format: "epub" | "cbz" | "text" | "image";
    images?: string[];
}

const imageExtensions = new Set([".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp"]);

async function storeOriginal(path: string, bytes: Uint8Array): Promise<void> {
    await mkdir(dirname(path), { recursive: true });
    try {
        await writeFile(path, bytes, { flag: "wx" });
    }
    catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "EEXIST") {
            throw error;
        }
    }
}

function archivePath(path: string): string {
    const normalized = posix.normalize(path.replaceAll("\\", "/"));
    if (!normalized || normalized.startsWith("/") || normalized === ".." || normalized.startsWith("../") || normalized.includes(":")) {
        throw new Error("Архив содержит небезопасный путь.");
    }
    return normalized;
}

function inside(path: string, root: string): boolean {
    const difference = relative(resolve(root), resolve(path));
    return difference !== ".." && !difference.startsWith("..\\") && !difference.startsWith("../") && !isAbsolute(difference);
}

export async function openLocalDocument(path: string, dataDir: string): Promise<LocalDocument> {
    const extension = extname(path).toLowerCase();
    if (!imageExtensions.has(extension) && ![".epub", ".cbz", ".zip", ".txt", ".html", ".htm", ".fb2"].includes(extension)) {
        throw new Error("Поддерживаются EPUB, CBZ, TXT, HTML, FB2 и изображения.");
    }
    const file = await stat(path);
    if (!file.isFile() || file.size > 512 * 1024 * 1024) {
        throw new Error("Выберите книгу или архив размером до 512 МБ.");
    }
    const bytes = await readFile(path);
    const id = contentId(bytes);
    const root = join(dataDir, "reader", "local", id);
    const document: LocalDocument = { id, title: basename(path, extension), sourceId: "local", local: true,
        chapters: [], assetRoot: root, format: "text" };

    if (extension === ".epub" || extension === ".cbz" || extension === ".zip") {
        let total = 0;
        let count = 0;
        const entries = unzipSync(bytes, { filter(entry) {
            archivePath(entry.name);
            total += entry.originalSize;
            ++count;
            if (entry.originalSize > 64 * 1024 * 1024 || total > 1024 * 1024 * 1024 || count > 30_000) {
                throw new Error("Архив превышает допустимый объём распаковки.");
            }
            return !entry.name.endsWith("/");
        } });
        for (const [name, content] of Object.entries(entries)) {
            const destination = join(root, archivePath(name));
            await storeOriginal(destination, content);
        }
        if (extension !== ".epub") {
            const order = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
            document.format = "cbz";
            document.images = Object.keys(entries).filter(name => imageExtensions.has(extname(name).toLowerCase()))
                .sort(order.compare).map(name => join(root, archivePath(name)));
            if (!document.images.length) {
                throw new Error("В архиве нет изображений для чтения.");
            }
            document.chapters = [{ id, title: document.title, url: `qbutt-local://${id}/images` }];
            return document;
        }
        document.format = "epub";
        const containerBytes = entries["META-INF/container.xml"];
        if (!containerBytes) {
            throw new Error("В EPUB отсутствует META-INF/container.xml.");
        }
        const container = load(new TextDecoder().decode(containerBytes), { xml: true });
        const packagePath = archivePath(container("rootfile").first().attr("full-path") ?? "");
        const packageBytes = entries[packagePath];
        if (!packageBytes) {
            throw new Error("В EPUB отсутствует описание книги.");
        }
        const $ = load(new TextDecoder().decode(packageBytes), { xml: true });
        document.title = $("dc\\:title,title").first().text().trim() || document.title;
        const manifest = new Map<string, string>();
        $("manifest item").each((_, element) => {
            const itemId = $(element).attr("id");
            const href = $(element).attr("href");
            if (itemId && href) {
                manifest.set(itemId, archivePath(posix.join(posix.dirname(packagePath), decodeURIComponent(href.split("#")[0]!))));
            }
        });
        $("spine itemref").each((_, element) => {
            const name = manifest.get($(element).attr("idref") ?? "");
            if (!name || !entries[name] || $(element).attr("linear") === "no") {
                return;
            }
            const chapterHtml = load(new TextDecoder().decode(entries[name]));
            const title = chapterHtml("h1,h2,title").first().text().trim() || `Глава ${document.chapters.length + 1}`;
            document.chapters.push({ id: contentId(name), title, url: pathToFileURL(join(root, name)).href,
                number: document.chapters.length + 1 });
        });
        if (!document.chapters.length) {
            throw new Error("В EPUB нет доступных глав.");
        }
        return document;
    }

    const destination = join(root, "original" + extension);
    await storeOriginal(destination, bytes);
    document.format = imageExtensions.has(extension) ? "image" : "text";
    if (document.format === "image") {
        document.images = [destination];
    }
    document.chapters = [{ id, title: document.title, url: pathToFileURL(destination).href }];
    return document;
}

export async function readLocalChapter(document: LocalDocument, chapterId: string): Promise<ReaderPage> {
    const selected = document.chapters.find(chapter => chapter.id === chapterId);
    if (!selected) {
        throw new Error("Глава отсутствует в этой книге.");
    }
    const page: ReaderPage = { id: selected.id, documentId: document.id, title: selected.title, kind: "text", direction: "ltr" };
    if (document.images) {
        return { ...page, kind: "images", direction: "vertical", images: document.images.map((url, index) => ({ id: String(index), url })) };
    }
    const path = fileURLToPath(selected.url);
    if (!inside(path, document.assetRoot)) {
        throw new Error("Адрес главы выходит за пределы книги.");
    }
    const text = await readFile(path, "utf8");
    if (extname(path) === ".txt") {
        const $ = load("<body></body>");
        for (const paragraph of text.split(/\r?\n\s*\r?\n/)) {
            $("body").append($("<p>").text(paragraph));
        }
        page.html = $("body").html()!;
    }
    else {
        const content = document.format === "text" && extname(path) === ".fb2" ? fictionBookHtml(text) : text;
        const $ = load(sanitizeChapter(content, selected.url, true));
        $("img[src]").each((_, image) => {
            const url = $(image).attr("src")!;
            if (!url.startsWith("file:") || !inside(fileURLToPath(url), document.assetRoot)) {
                $(image).remove();
            }
        });
        page.html = $("body").html()!;
    }
    return page;
}

function fictionBookHtml(xml: string): string {
    const $ = load(xml, { xml: true });
    const paragraphs = $("body").html();
    if (!paragraphs) {
        throw new Error("В FB2 отсутствует текст книги.");
    }
    return paragraphs.replace(/<\/?section[^>]*>/g, "").replace(/<(\/?)(subtitle|title)>/g, "<$1h2>");
}
