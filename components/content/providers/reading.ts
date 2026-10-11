import { load } from "cheerio";
import type { Chapter, Provider, ProviderContext, Variant, Work } from "../contracts";
import { contentId, type ReaderPage } from "../reader/types";
import { sanitizeChapter } from "../reader/html";
import { asianReadingProviders, loadAsianReadingChapter } from "./reading-asian";
import { originalReferences } from "./reading-originals";
import { mangaDex, loadMangaDexChapter } from "./reading-mangadex";
import { nhentai, loadNhentaiChapter } from "./reading-nhentai";

type RecordData = Record<string, any>;

async function json(url: string, context: ProviderContext, headers?: Record<string, string>): Promise<RecordData> {
    const response = await context.fetch(url, { headers, signal: context.signal });
    if (!response.ok) {
        throw new Error(`Источник вернул HTTP ${response.status}.`);
    }
    return response.json();
}

async function html(url: string, context: ProviderContext): Promise<string> {
    const response = await context.fetch(url, { signal: context.signal });
    if (!response.ok) {
        throw new Error(`Источник вернул HTTP ${response.status}.`);
    }
    return response.text();
}

function item(sourceId: string, sourceName: string, title: string, url: string, id: string,
    category: "books" | "manga", cover?: string, description?: string, alternateTitles: string[] = []): Work {
    return {
        id: `${sourceId}:${id}`,
        title,
        aliases: [...new Set(alternateTitles.filter(value => value && value !== title))],
        category,
        url,
        cover,
        description,
        variants: [{ id: `${sourceId}:${id}`, sourceId, sourceName, title, url,
            transport: category === "manga" ? "manga" : "book",
            languages: [({ kakuyomu: "ja", syosetu: "ja", royalroad: "en", linovel: "zh",
                ranobehub: "ru", ranobelib: "ru", mangalib: "ru" } as Record<string, string>)[sourceId]!] }],
    };
}

function slug(variant: Variant): string {
    return variant.id.slice(variant.sourceId.length + 1);
}

function chapter(title: string, url: string, number?: number): Chapter {
    return { id: contentId(url), title, url, number };
}

const ranobeHub: Provider = {
    source: { id: "ranobehub", name: "RanobeHub", url: "https://ranobehub.org", categories: ["books"], enabled: true },
    async search(request, context) {
        const result = await json(`https://ranobehub.org/api/search?q=${encodeURIComponent(request.query)}&rh_client=modern`, context);
        return (result.books ?? []).map((book: RecordData) => item("ranobehub", "RanobeHub", book.title || book.originalTitle,
            `https://ranobehub.org/ranobe/${book.id}-${book.slug}`, `${book.id}-${book.slug}`, "books",
            book.posterUrl ? new URL(book.posterUrl, "https://ranobehub.org").href : undefined, book.description,
            [book.originalTitle]));
    },
    async inspect(variant, context) {
        const bookId = slug(variant).split("-")[0];
        const chapters: Chapter[] = [];
        let total = 1;
        for (let offset = 0; offset < total; offset += 250) {
            const result = await json(`https://ranobehub.org/api/books/${bookId}/chapters?limit=250&offset=${offset}`, context);
            total = result.total;
            for (const entry of result.items ?? []) {
                chapters.push({
                    ...chapter(entry.title, `https://ranobehub.org/api/chapters/${entry.id}`, chapters.length + 1),
                    volume: Number.isFinite(entry.volume) && entry.volume > 0 ? entry.volume : undefined,
                });
            }
            if (chapters.length > 30_000) {
                throw new Error("В произведении слишком много глав.");
            }
        }
        return { ...item("ranobehub", "RanobeHub", variant.title, variant.url, slug(variant), "books"), chapters };
    },
};

function libGroup(sourceId: string, sourceName: string, siteId: number, site: string, category: "books" | "manga"): Provider {
    const headers = { Accept: "application/json", "Site-Id": String(siteId), Referer: site, Origin: site };
    const typeSuffix = /\s+\((?:Novel|Новелла)\)$/iu;

    function metadata(book: RecordData, url: string, id: string): Work {
        const title = book.rus_name || book.eng_name || book.name;
        const titles = [book.name, book.eng_name, book.rus_name, title.replace(typeSuffix, ""), ...(book.otherNames ?? [])];
        const original = originalReferences(book.externalLinks ?? [], titles, title);
        const description = typeof book.summary === "string" ? book.summary
            : book.summary ? load(richTextHtml(book.summary, [])).text() : undefined;
        const work = item(sourceId, sourceName, title, url, id, category, book.cover?.default, description, titles);
        work.variants[0]!.author = book.authors?.map((author: RecordData) => author.name).filter(Boolean).join(", ") || undefined;
        work.variants.push(...original.variants);
        work.identifiers = original.identifiers;
        return work;
    }

    return {
        source: { id: sourceId, name: sourceName, url: site, categories: [category], enabled: true },
        async search(request, context, emit) {
            const result = await json(`https://api.cdnlibs.org/api/manga?site_id%5B0%5D=${siteId}&q=${encodeURIComponent(request.query)}`, context, headers);
            const works: Work[] = (result.data ?? []).map((book: RecordData) => {
                const id = book.slug_url || `${book.id}--${book.slug}`;
                return item(sourceId, sourceName, book.rus_name || book.eng_name || book.name,
                    `${site}/ru/${category === "books" ? "book" : "manga"}/${id}`, id, category, book.cover?.default,
                    undefined, [book.name, book.eng_name, book.rus_name]);
            });
            for (const work of works) emit?.(work);
            // The list endpoint rejects metadata fields. Enrich one exact
            // title only, without loading chapters.
            // Novel/Новелла is the catalog's type suffix; keep WN/LN editions.
            const searchTitle = (title: string) => title.normalize("NFKC").trim()
                .replace(typeSuffix, "").toLocaleLowerCase().replace(/\s+/g, " ");
            const query = searchTitle(request.query);
            const exact = works.filter(work => [work.title, ...work.aliases || []]
                .some(title => searchTitle(title) === query));
            if (exact.length === 1) {
                const work = exact[0]!;
                const id = slug(work.variants[0]!);
                try {
                    const details = await json(`https://api.cdnlibs.org/api/manga/${id}?fields[]=otherNames&fields[]=externalLinks&fields[]=authors`, context, headers);
                    const enriched = metadata(details.data, work.url!, id);
                    works[works.indexOf(work)] = enriched;
                    emit?.(enriched);
                }
                catch (error) {
                    context.signal.throwIfAborted();
                    context.reportError?.(error);
                }
            }
            return works;
        },
        async inspect(variant, context) {
            const publication = /^(.*):branch:(\d+)$/.exec(slug(variant));
            const id = publication?.[1] ?? slug(variant);
            const requestedBranch = publication ? Number(publication[2]) : undefined;
            const [details, listing] = await Promise.all([
                json(`https://api.cdnlibs.org/api/manga/${id}?fields[]=summary&fields[]=otherNames&fields[]=externalLinks&fields[]=authors`, context, headers),
                json(`https://api.cdnlibs.org/api/manga/${id}/chapters`, context, headers),
            ]);
            const book = details.data;
            const branches = new Map<number, Set<string>>();
            for (const entry of listing.data ?? []) {
                for (const branch of entry.branches ?? []) {
                    const branchId = branch.branch_id ?? 0;
                    const teams = branches.get(branchId) ?? new Set<string>();
                    for (const team of branch.teams ?? []) {
                        if (team.name) teams.add(team.name);
                    }
                    branches.set(branchId, teams);
                }
            }
            let defaultBranch = branches.keys().next().value;
            if (branches.size > 1) {
                const first = listing.data.find((entry: RecordData) => entry.branches?.length);
                // The API's default can differ from the first listed translation.
                // Resolve it once, then keep that publication for every chapter.
                const preferred = await json(`https://api.cdnlibs.org/api/manga/${id}/chapter?branch_id=0&number=${encodeURIComponent(first.number)}&volume=${encodeURIComponent(first.volume)}`, context, headers);
                defaultBranch = preferred.data.branch_id ?? 0;
                if (!branches.has(defaultBranch!)) {
                    throw new Error("Источник не вернул выбранную ветку перевода.");
                }
            }
            const selectedBranch = requestedBranch ?? defaultBranch;
            if (requestedBranch !== undefined && !branches.has(requestedBranch)) {
                throw new Error("Выбранная ветка перевода больше недоступна.");
            }
            const chapters: Chapter[] = [];
            for (const entry of listing.data ?? []) {
                if (!(entry.branches ?? []).some((branch: RecordData) => (branch.branch_id ?? 0) === selectedBranch)) continue;
                const url = `https://api.cdnlibs.org/api/manga/${id}/chapter?branch_id=${selectedBranch}&number=${encodeURIComponent(entry.number)}&volume=${encodeURIComponent(entry.volume)}`;
                const title = `Том ${entry.volume}, глава ${entry.number}${entry.name ? ` — ${entry.name}` : ""}`;
                chapters.push(chapter(title, url, entry.index));
            }
            const work = metadata(book, variant.url, id);
            const base = work.variants[0]!;
            const translations = [...branches].sort(([left], [right]) => Number(right === defaultBranch) - Number(left === defaultBranch))
                .map(([branchId, teams]) => ({ ...base,
                    id: branchId === 0 ? base.id : `${base.id}:branch:${branchId}`,
                    translation: [...teams].join(", ") || (branches.size > 1 ? `Перевод ${branchId}` : undefined),
                }));
            work.variants = [...translations, ...work.variants.slice(1)];
            return { ...work, chapters };
        },
    };
}

function kakuyomuState(document: string): RecordData[] {
    const $ = load(document);
    const data = JSON.parse($("script#__NEXT_DATA__").text() || "{}");
    return Object.values(data.props?.pageProps?.__APOLLO_STATE__ ?? {});
}

const kakuyomu: Provider = {
    source: { id: "kakuyomu", name: "Kakuyomu", url: "https://kakuyomu.jp", categories: ["books"], enabled: true },
    async search(request, context) {
        const state = kakuyomuState(await html(`https://kakuyomu.jp/search?q=${encodeURIComponent(request.query)}`, context));
        return state.filter(entry => entry?.__typename === "Work").map(work => item("kakuyomu", "Kakuyomu", work.title,
            `https://kakuyomu.jp/works/${work.id}`, work.id, "books", work.adminCoverImageUrl, work.introduction));
    },
    async inspect(variant, context) {
        const state = kakuyomuState(await html(variant.url, context));
        const work = state.find(entry => entry?.__typename === "Work" && entry.id === slug(variant));
        if (!work) {
            throw new Error("Kakuyomu не вернул сведения о произведении.");
        }
        const episodes = state.filter(entry => entry?.__typename === "Episode");
        return { ...item("kakuyomu", "Kakuyomu", work.title, variant.url, work.id, "books", work.adminCoverImageUrl, work.introduction),
            chapters: episodes.map((entry, index) => chapter(entry.title, `${variant.url}/episodes/${entry.id}`, index + 1)) };
    },
};

const syosetu: Provider = {
    source: { id: "syosetu", name: "小説家になろう", url: "https://syosetu.com", categories: ["books"], enabled: true },
    async search(request, context) {
        const $ = load(await html(`https://yomou.syosetu.com/search.php?word=${encodeURIComponent(request.query)}`, context));
        const works: Work[] = [];
        $(".searchkekka_box .novel_h a, .p-search-novel__title a").each((_, anchor) => {
            const url = $(anchor).attr("href");
            if (url?.startsWith("https://ncode.syosetu.com/")) {
                works.push(item("syosetu", "小説家になろう", $(anchor).text().trim(), url, new URL(url).pathname.split("/")[1]!, "books"));
            }
        });
        return works;
    },
    async inspect(variant, context) {
        const first = load(await html(variant.url, context));
        const title = first(".p-novel__title").first().text().trim() || variant.title;
        const chapters: Chapter[] = [];
        const last = new URL(first(".c-pager__item--last").attr("href") || variant.url, variant.url);
        const pageCount = Math.min(300, Number(last.searchParams.get("p")) || 1);
        for (let page = 1; page <= pageCount; ++page) {
            const $ = page === 1 ? first : load(await html(`${variant.url}?p=${page}`, context));
            $(".p-eplist__sublist a").each((_, anchor) => {
                const url = new URL($(anchor).attr("href")!, variant.url).href;
                chapters.push(chapter($(anchor).text().trim(), url, chapters.length + 1));
            });
        }
        if (!chapters.length && first(".p-novel__text").length) {
            chapters.push(chapter(title, variant.url, 1));
        }
        return { ...item("syosetu", "小説家になろう", title, variant.url, slug(variant), "books"), chapters };
    },
};

const royalRoad: Provider = {
    source: { id: "royalroad", name: "Royal Road", url: "https://www.royalroad.com", categories: ["books"], enabled: true },
    async search(request, context) {
        const site = "https://www.royalroad.com";
        const $ = load(await html(`${site}/fictions/search?page=1&title=${encodeURIComponent(request.query)}&globalFilters=true`, context));
        const works: Work[] = [];
        $(".fiction-list-item").each((_, entry) => {
            const anchor = $(entry).find("h2.fiction-title a");
            const path = anchor.attr("href");
            const id = path?.match(/^\/fiction\/(\d+)/)?.[1];
            if (path && id) {
                const cover = $(entry).find("figure img").attr("src");
                works.push(item("royalroad", "Royal Road", anchor.text().trim(), new URL(path, site).href, id, "books",
                    cover ? new URL(cover, site).href : undefined, $(entry).find(".fiction-description").text().trim()));
            }
        });
        return works;
    },
    async inspect(variant, context) {
        const document = await html(variant.url, context);
        const $ = load(document);
        const listing = document.match(/window\.chapters\s*=\s*(\[[\s\S]*?\]);/)?.[1];
        if (!listing) {
            throw new Error("Royal Road не вернул список глав.");
        }
        const chapters = (JSON.parse(listing) as RecordData[]).filter(entry => entry.isUnlocked && entry.visible)
            .sort((left, right) => left.order - right.order)
            .map(entry => chapter(entry.title, new URL(entry.url, variant.url).href, entry.order + 1));
        const cover = $("img.thumbnail").first().attr("src");
        return { ...item("royalroad", "Royal Road", $("h1").first().text().trim(), variant.url, slug(variant), "books",
            cover ? new URL(cover, variant.url).href : undefined, $(".description").first().text().trim()), chapters };
    },
};

const linovel: Provider = {
    source: { id: "linovel", name: "轻之文库 / Linovel", url: "https://www.linovel.net", categories: ["books"], enabled: true },
    async search(request, context) {
        const site = "https://www.linovel.net";
        const $ = load(await html(`${site}/search/?kw=${encodeURIComponent(request.query)}`, context));
        const works: Work[] = [];
        $(".rank-book-list a.search-book").each((_, entry) => {
            const path = $(entry).attr("href");
            const id = path?.match(/\/book\/(\d+)\.html/)?.[1];
            if (path && id) {
                const cover = $(entry).find("img").attr("src");
                works.push(item("linovel", "轻之文库 / Linovel", $(entry).find(".book-name").text().trim(),
                    new URL(path, site).href, id, "books", cover ? new URL(cover, site).href : undefined));
            }
        });
        return works;
    },
    async inspect(variant, context) {
        const $ = load(await html(variant.url, context));
        const chapters = $("div.chapter a").toArray().map((entry, index) => chapter($(entry).text().trim(),
            new URL($(entry).attr("href")!, variant.url).href, index + 1));
        const cover = $(".book-cover img").first().attr("src");
        return { ...item("linovel", "轻之文库 / Linovel", $(".book-title").first().text().trim(), variant.url, slug(variant), "books",
            cover ? new URL(cover, variant.url).href : undefined, $(".about-text").first().text().trim()), chapters };
    },
};

export const readingProviders: Provider[] = [ranobeHub,
    libGroup("ranobelib", "RanobeLib", 3, "https://ranobelib.me", "books"),
    libGroup("mangalib", "MangaLib", 1, "https://mangalib.me", "manga"),
    mangaDex, nhentai,
    kakuyomu, syosetu, royalRoad, linovel,
    ...asianReadingProviders];

export function readingPublication(address: URL): Variant | undefined {
    if (address.protocol !== "https:" || address.username || address.password || address.port || address.search) return;
    for (const provider of readingProviders) {
        if (address.hostname !== new URL(provider.source.url).hostname) continue;
        const sourceId = provider.source.id;
        const id = sourceId === "mangadex" ? /^\/title\/([a-f\d]{8}-(?:[a-f\d]{4}-){3}[a-f\d]{12})(?:\/[^/]+)?\/?$/i.exec(address.pathname)?.[1]
            : sourceId === "nhentai" ? /^\/g\/(\d+)\/?$/.exec(address.pathname)?.[1]
            : ["mangalib", "ranobelib"].includes(sourceId)
                ? /^\/ru\/(?:(?:manga|book)\/)?(\d+--[\w-]+)\/?$/.exec(address.pathname)?.[1] : undefined;
        if (!id) return;
        const category = provider.source.categories[0];
        const url = sourceId === "mangadex" ? `${provider.source.url}/title/${id}`
            : sourceId === "nhentai" ? `${provider.source.url}/g/${id}/`
            : `${provider.source.url}/ru/${category === "books" ? "book" : "manga"}/${id}`;
        return { id: `${sourceId}:${id}`, sourceId, sourceName: provider.source.name, title: "", url,
            transport: category === "books" ? "book" : "manga", needsInspection: true };
    }
}

export async function loadReadingChapter(sourceId: string, selected: Chapter, context: ProviderContext): Promise<Omit<ReaderPage, "id" | "documentId">> {
    if (sourceId === "mangadex") return loadMangaDexChapter(selected, context);
    if (sourceId === "nhentai") return loadNhentaiChapter(selected, context);
    if (sourceId === "jjwxc" || sourceId === "naver")
        return loadAsianReadingChapter(sourceId, selected, context);
    const url = new URL(selected.url);
    const expected = ({ ranobehub: "ranobehub.org", kakuyomu: "kakuyomu.jp", syosetu: "ncode.syosetu.com",
        ranobelib: "api.cdnlibs.org", mangalib: "api.cdnlibs.org", royalroad: "www.royalroad.com",
        linovel: "www.linovel.net" } as Record<string, string>)[sourceId];
    if (!expected || url.protocol !== "https:" || url.hostname !== expected) {
        throw new Error("Адрес главы не принадлежит выбранному источнику.");
    }
    if (sourceId === "mangalib" || sourceId === "ranobelib") {
        const site = sourceId === "mangalib" ? "https://mangalib.me" : "https://ranobelib.me";
        const headers = { Accept: "application/json", "Site-Id": sourceId === "mangalib" ? "1" : "3", Referer: site, Origin: site };
        const result = await json(url.href, context, headers);
        if (sourceId === "mangalib") {
            const constants = await json("https://api.cdnlibs.org/api/constants?fields[]=imageServers", context, headers);
            // The display server converts originals to AVIF. The source's download
            // server preserves JPEG/PNG files supported by the native reader and OCR.
            const server = constants.data.imageServers.find((entry: RecordData) => entry.id === "download" && entry.site_ids.includes(1));
            if (!server || !Array.isArray(result.data.pages) || !result.data.pages.length) {
                throw new Error("В этой главе нет доступных страниц.");
            }
            return { title: selected.title, kind: "images", direction: "rtl", images: result.data.pages.map((entry: RecordData) => ({
                id: String(entry.id), url: /^https?:/.test(entry.url) ? entry.url : server.url + "/" + entry.url.replace(/^\/+/, ""),
                width: entry.width, height: entry.height,
            })) };
        }
        const value = result.data.content;
        const content = typeof value === "string" ? value : richTextHtml(value, result.data.attachments ?? []);
        return { title: selected.title, kind: "text", html: sanitizeChapter(content, site), direction: "ltr" };
    }
    if (sourceId === "ranobehub") {
        const result = await json(url.href, context);
        return { title: result.chapter.title, kind: "text", html: sanitizeChapter(result.chapter.html, "https://ranobehub.org"), direction: "ltr" };
    }
    const $ = load(await html(url.href, context));
    if (sourceId === "linovel" && $(".fufei-app-download-hint").length) {
        throw new Error("Эта глава платная. Откройте её на сайте Linovel.");
    }
    if (sourceId === "royalroad") {
        // Royal Road embeds display:none classes in chapter text. Remove those
        // elements before discarding CSS so hidden junk never becomes readable.
        const css = $("style").map((_, style) => $(style).text()).toArray().join("\n");
        for (const hidden of css.matchAll(/\.([A-Za-z_][\w-]*)\s*\{[^}]*display\s*:\s*none\s*[;}]/g)) {
            $(`.${hidden[1]}`).remove();
        }
    }
    const content = sourceId === "kakuyomu" ? $(".widget-episodeBody").html()
        : sourceId === "royalroad" ? $(".chapter-content,.author-note-portlet").toArray().map(element => $(element).html()).join("<hr/>")
        : sourceId === "linovel" ? $(".article-text").html()
        : $(".p-novel__body .p-novel__text:not([class*=p-novel__text--])").html();
    if (!content?.trim()) {
        throw new Error("Источник не вернул текст главы.");
    }
    return { title: selected.title, kind: "text", html: sanitizeChapter(content, url.href), direction: "ltr" };
}

function richTextHtml(value: RecordData, attachments: RecordData[]): string {
    const $ = load("<body></body>");
    function append(nodes: RecordData[], parent: ReturnType<typeof $>): void {
        for (const node of nodes ?? []) {
            if (node.type === "text") {
                parent.append($("<span>").text(node.text ?? ""));
                continue;
            }
            if (node.type === "image") {
                for (const image of node.attrs?.images ?? []) {
                    const file = attachments.find(entry => entry.name === image.image || entry.id === image.image);
                    if (file?.url) {
                        parent.append($("<img>").attr("src", file.url));
                    }
                }
                continue;
            }
            const tag = ({ paragraph: "p", heading: "h2", hardBreak: "br", horizontalRule: "hr", blockquote: "blockquote",
                bulletList: "ul", orderedList: "ol", listItem: "li" } as Record<string, string>)[node.type] ?? "span";
            const element = $(`<${tag}>`);
            append(node.content ?? [], element);
            parent.append(element);
        }
    }
    append(value.content ?? [], $("body"));
    return $("body").html() ?? "";
}
