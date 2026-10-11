import type { Variant, Work } from "../contracts";

interface OriginalSource {
    id: string;
    name: string;
    language: string;
    hosts: string[];
    publication(url: URL): { id: string; url: string } | undefined;
}

function pathPublication(url: URL, pattern: RegExp, canonical: (id: string) => string) {
    const id = pattern.exec(url.pathname)?.[1];
    return id ? { id, url: canonical(id) } : undefined;
}

const sources: OriginalSource[] = [
    {
        id: "syosetu", name: "小説家になろう", language: "ja", hosts: ["ncode.syosetu.com"],
        publication: url => pathPublication(url, /^\/(n[\da-z]+)\/?$/, id => `https://ncode.syosetu.com/${id}/`),
    },
    {
        id: "kakuyomu", name: "Kakuyomu", language: "ja", hosts: ["kakuyomu.jp"],
        publication: url => pathPublication(url, /^\/works\/(\d+)\/?$/, id => `https://kakuyomu.jp/works/${id}`),
    },
    {
        id: "royalroad", name: "Royal Road", language: "en", hosts: ["www.royalroad.com"],
        publication: url => pathPublication(url, /^\/fiction\/(\d+)(?:\/[^/]+)?\/?$/, id => `https://www.royalroad.com/fiction/${id}`),
    },
    {
        id: "linovel", name: "轻之文库 / Linovel", language: "zh", hosts: ["www.linovel.net"],
        publication: url => pathPublication(url, /^\/book\/(\d+)\.html$/, id => `https://www.linovel.net/book/${id}.html`),
    },
    {
        id: "jjwxc", name: "晋江文学城", language: "zh", hosts: ["www.jjwxc.net", "wap.jjwxc.net"],
        publication(url) {
            if (url.hostname === "wap.jjwxc.net") {
                return pathPublication(url, /^\/book2\/(\d+)\/?$/, id => `https://wap.jjwxc.net/book2/${id}`);
            }
            if (url.pathname !== "/onebook.php" || url.searchParams.getAll("novelid").length !== 1) {
                return undefined;
            }
            const id = url.searchParams.get("novelid")!;
            return /^\d+$/.test(id) ? { id, url: `https://wap.jjwxc.net/book2/${id}` } : undefined;
        },
    },
    {
        id: "naver", name: "NAVER 웹소설", language: "ko", hosts: ["novel.naver.com"],
        publication(url) {
            const league = /^\/(challenge|best)\/list(?:\.nhn)?$/.exec(url.pathname)?.[1];
            if (!league || url.searchParams.getAll("novelId").length !== 1) {
                return undefined;
            }
            const id = url.searchParams.get("novelId")!;
            return /^\d+$/.test(id) ? { id, url: `https://novel.naver.com/${league}/list?novelId=${id}` } : undefined;
        },
    },
];

export function readingOriginal(input: unknown): Variant | undefined {
    if (typeof input !== "string") {
        return undefined;
    }
    let url: URL;
    try {
        url = new URL(input);
    }
    catch {
        return undefined;
    }
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.port) {
        return undefined;
    }
    const source = sources.find(entry => entry.hosts.includes(url.hostname));
    const publication = source?.publication(url);
    if (!source || !publication) {
        return undefined;
    }
    return {
        id: `${source.id}:${publication.id}`,
        sourceId: source.id,
        sourceName: source.name,
        title: "",
        url: publication.url,
        transport: "book",
        languages: [source.language],
    };
}

export function originalReferences(links: { source_link?: unknown }[], titles: string[], title: string): Pick<Work, "variants" | "identifiers"> {
    const variants: Variant[] = [];
    const identifiers: Record<string, string> = {};
    for (const link of links) {
        const original = readingOriginal(link.source_link);
        if (!original || variants.some(variant => variant.id === original.id)) {
            continue;
        }
        const language = original.languages![0];
        let originalTitle: string | undefined;
        if (language === "ja") {
            originalTitle = titles.find(value => /[\p{Script=Hiragana}\p{Script=Katakana}]/u.test(value))
                ?? titles.find(value => /\p{Script=Han}/u.test(value));
        }
        else if (language === "zh") {
            originalTitle = titles.find(value => /\p{Script=Han}/u.test(value)
                && !/[\p{Script=Hiragana}\p{Script=Katakana}]/u.test(value));
        }
        else if (language === "ko") {
            originalTitle = titles.find(value => /\p{Script=Hangul}/u.test(value));
        }
        variants.push({ ...original, title: originalTitle || title });
        identifiers[original.sourceId] = original.id.slice(original.sourceId.length + 1);
    }
    return { variants, identifiers };
}
