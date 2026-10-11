import { load } from "cheerio";
import iconv from "iconv-lite";
import type { Chapter, Provider, ProviderContext, Variant, Work } from "../contracts";
import { sanitizeChapter } from "../reader/html";
import { contentId, type ReaderPage } from "../reader/types";
import { readingOriginal } from "./reading-originals";

async function document(url: string, context: ProviderContext, encoding = "utf-8"): Promise<string> {
    const response = await context.fetch(url, { signal: context.signal });
    if (!response.ok) {
        throw new Error(`Источник вернул HTTP ${response.status}.`);
    }
    return new TextDecoder(encoding, { fatal: true }).decode(await response.arrayBuffer());
}

function publication(url: string, title: string, author?: string, cover?: string, description?: string): Work {
    const variant = readingOriginal(url);
    if (!variant) {
        throw new Error("Источник вернул неизвестный адрес произведения.");
    }
    return {
        id: variant.id,
        title,
        category: "books",
        url: variant.url,
        cover,
        description,
        identifiers: { [variant.sourceId]: variant.id.slice(variant.sourceId.length + 1) },
        variants: [{ ...variant, title, author }],
    };
}

function selectedPublication(variant: Variant): Variant {
    const original = readingOriginal(variant.url);
    if (!original || original.id !== variant.id || original.sourceId !== variant.sourceId) {
        throw new Error("Адрес произведения не принадлежит выбранному источнику.");
    }
    return original;
}

const jjwxc: Provider = {
    source: { id: "jjwxc", name: "晋江文学城", url: "https://wap.jjwxc.net", categories: ["books"], enabled: true },
    async search(request, context) {
        // The official mobile form submits its GB18030 page encoding.
        const query = [...iconv.encode(request.query, "gb18030")]
            .map(byte => `%${byte.toString(16).padStart(2, "0")}`).join("");
        const $ = load(await document(`https://wap.jjwxc.net/search?version=1&kw=${query}&t=1`, context, "gb18030"));
        const works: Work[] = [];
        $("a.track[href^='/book2/']").each((_, anchor) => {
            const url = new URL($(anchor).attr("href")!, "https://wap.jjwxc.net").href;
            const author = $(anchor).parent().find("a[href^='/wapauthor/']").text().trim();
            works.push(publication(url, $(anchor).text().trim(), author));
        });
        return works;
    },
    async inspect(variant, context) {
        const original = selectedPublication(variant);
        const $ = load(await document(`${original.url}?more=0&whole=1`, context, "gb18030"));
        const title = $("[data-novelname]").first().attr("data-novelname");
        if (!title) {
            throw new Error("晋江文学城 не вернул сведения о произведении.");
        }
        const id = original.id.slice("jjwxc:".length);
        const chapters: Chapter[] = [];
        $("#chapter_list_box a").each((_, anchor) => {
            const path = $(anchor).attr("href")!;
            const match = /^\/book2\/(\d+)\/(\d+)$/.exec(path);
            if (match?.[1] === id) {
                const url = new URL(path, original.url).href;
                chapters.push({ id: contentId(url), title: $(anchor).text().trim(), url, number: Number(match[2]) });
            }
        });
        if (chapters.length > 30_000) {
            throw new Error("В произведении слишком много глав.");
        }
        const paid = $("#chapter_list_box a[href^='/vip/']").length;
        const introduction = $("#novelintro_whole").text().trim() || $("#novelintro").text().trim();
        return {
            ...publication(original.url, title, $(".authorname-content a").first().text().trim(), undefined, introduction),
            chapters: chapters.sort((left, right) => left.number! - right.number!),
            notes: paid ? [{ text: `Показаны бесплатные главы. Платных глав на сайте: ${paid}.`, url: original.url }] : undefined,
        };
    },
};

const naver: Provider = {
    source: { id: "naver", name: "NAVER 웹소설", url: "https://novel.naver.com", categories: ["books"], enabled: true },
    async search(request, context) {
        const site = "https://novel.naver.com";
        const $ = load(await document(`${site}/search?keyword=${encodeURIComponent(request.query)}&section=all&target=novel&page=1`, context));
        const works: Work[] = [];
        $(".cont_sub .default_list a.link").each((_, anchor) => {
            const path = $(anchor).attr("href");
            if (!path || !/^\/(challenge|best)\/list\?/.test(path)) {
                return;
            }
            const title = $(anchor).find(".title").text().trim();
            const cover = $(anchor).find("img").attr("src");
            works.push(publication(new URL(path, site).href, title, $(anchor).find(".author").text().trim(),
                cover ? new URL(cover, site).href : undefined));
        });
        return works;
    },
    async inspect(variant, context) {
        const original = selectedPublication(variant);
        const first = load(await document(original.url, context));
        const title = first(".section_area_info h2.title").text().trim();
        if (!title) {
            throw new Error("NAVER не вернул сведения о произведении.");
        }
        const chapters = new Map<string, Chapter>();
        let pageCount = 1;
        for (let page = 1; page <= pageCount; ++page) {
            const $ = page === 1 ? first : load(await document(`${original.url}&page=${page}`, context));
            $(".default_paging a").each((_, anchor) => {
                const url = new URL($(anchor).attr("href")!, original.url);
                pageCount = Math.max(pageCount, Number(url.searchParams.get("page")) || 1);
            });
            if (pageCount > 3000) {
                throw new Error("В произведении слишком много страниц списка глав.");
            }
            $(".league_num a.list_item").each((_, anchor) => {
                const url = new URL($(anchor).attr("href")!, original.url);
                if (url.pathname !== new URL(original.url).pathname.replace("/list", "/detail")
                    || url.searchParams.get("novelId") !== original.id.slice("naver:".length)) {
                    return;
                }
                const number = Number(url.searchParams.get("volumeNo"));
                if (!Number.isInteger(number) || number < 1) {
                    return;
                }
                const title = $(anchor).find(".subj").text().trim();
                chapters.set(url.href, { id: contentId(url.href), title, url: url.href, number });
            });
        }
        const cover = first(".section_area_info .thumbnail img").attr("src");
        const author = first(".section_area_info a[href*='target=author']").first().text().trim();
        return {
            ...publication(original.url, title, author, cover ? new URL(cover, original.url).href : undefined,
                first("#summaryText").text().trim()),
            chapters: [...chapters.values()].sort((left, right) => left.number! - right.number!),
        };
    },
};

export const asianReadingProviders: Provider[] = [jjwxc, naver];

export async function loadAsianReadingChapter(sourceId: string, selected: Chapter, context: ProviderContext): Promise<Omit<ReaderPage, "id" | "documentId">> {
    const url = new URL(selected.url);
    const supported = sourceId === "jjwxc"
        ? url.hostname === "wap.jjwxc.net" && /^\/book2\/\d+\/\d+$/.test(url.pathname) && !url.search
        : sourceId === "naver" && url.hostname === "novel.naver.com" && /^\/(challenge|best)\/detail$/.test(url.pathname)
            && url.searchParams.getAll("novelId").length === 1 && /^\d+$/.test(url.searchParams.get("novelId") || "")
            && url.searchParams.getAll("volumeNo").length === 1 && /^\d+$/.test(url.searchParams.get("volumeNo") || "");
    if (!supported || url.protocol !== "https:" || url.username || url.password || url.port) {
        throw new Error("Адрес главы не принадлежит выбранному источнику.");
    }
    const $ = load(await document(url.href, context, sourceId === "jjwxc" ? "gb18030" : "utf-8"));
    const content = sourceId === "jjwxc" ? $(".content_ul > li").first().html() : $(".detail_view_content").html();
    if (!content?.trim()) {
        throw new Error("Источник не вернул публичный текст главы. Откройте её на сайте.");
    }
    return { title: selected.title, kind: "text", html: sanitizeChapter(content, url.href), direction: "ltr" };
}
