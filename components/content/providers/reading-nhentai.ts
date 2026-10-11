import type { Chapter, Provider, ProviderContext, Variant, Work } from "../contracts";
import { contentId, type ReaderPage } from "../reader/types";

const site = "https://nhentai.net";
const api = `${site}/api/v2`;
type RecordData = Record<string, any>;

async function json(path: string, context: ProviderContext): Promise<RecordData> {
    const response = await context.fetch(`${api}${path}`, {
        headers: { Accept: "application/json", "User-Agent": "qbutt (https://github.com/qbutt-org/qbutt)" },
        signal: context.signal,
    });
    if (!response.ok) throw new Error(`nhentai returned HTTP ${response.status}.`);
    return response.json();
}

function mediaServer(value: unknown, kind: "image" | "thumb"): string {
    if (typeof value !== "string") throw new Error("nhentai did not return a media server.");
    const url = new URL(value);
    const host = kind === "image" ? /^i\d*\.nhentai\.net$/ : /^t\d*\.nhentai\.net$/;
    if (url.protocol !== "https:" || !host.test(url.hostname) || url.port || url.username || url.password
        || url.pathname !== "/" || url.search || url.hash) throw new Error("nhentai returned an unsupported media server.");
    return url.origin;
}

function imageUrl(server: string, path: unknown, mediaId: string): string {
    if (typeof path !== "string" || !new RegExp(`^galleries/${mediaId}/[\\w.-]+$`).test(path)
        || [".", ".."].includes(path.split("/").at(-1)!)) {
        throw new Error("nhentai returned an invalid image path.");
    }
    return `${server}/${path}`;
}

const languageCodes: Record<string, string> = {
    english: "en", japanese: "ja", chinese: "zh", russian: "ru", korean: "ko", spanish: "es",
    portuguese: "pt", french: "fr", german: "de", italian: "it", vietnamese: "vi", thai: "th",
    indonesian: "id", polish: "pl", turkish: "tr", ukrainian: "uk",
};

function metadata(gallery: RecordData, server: string): Work {
    const id = String(gallery.id);
    const mediaId = String(gallery.media_id);
    if (!/^\d+$/.test(id) || !/^\d+$/.test(mediaId)) throw new Error("nhentai returned an invalid gallery.");
    const title = gallery.title?.pretty || gallery.english_title;
    const aliases = [gallery.title?.english, gallery.title?.japanese, gallery.english_title, gallery.japanese_title]
        .filter((value): value is string => typeof value === "string" && Boolean(value) && value !== title);
    if (typeof title !== "string" || !title.trim()) throw new Error("nhentai did not return the gallery title.");
    const url = `${site}/g/${id}/`;
    const tags: RecordData[] = gallery.tags ?? [];
    const languages = [...new Set(tags.filter(tag => tag.type === "language").map(tag => languageCodes[tag.name]).filter(Boolean))];
    const cover = gallery.cover?.path || gallery.thumbnail;
    return {
        id: `nhentai:${id}`, title, aliases: [...new Set(aliases)], category: "manga", url,
        identifiers: { nhentai: id }, cover: cover ? imageUrl(server, cover, mediaId) : undefined,
        variants: [{ id: `nhentai:${id}`, sourceId: "nhentai", sourceName: "nhentai", title, url, transport: "manga",
            languages, needsInspection: !gallery.tags,
            author: tags.filter(tag => tag.type === "artist").map(tag => tag.name).join(", ") || undefined,
            translation: gallery.scanlator || undefined,
            publishedAt: gallery.upload_date ? new Date(gallery.upload_date * 1000).toISOString() : undefined }],
    };
}

function publication(variant: Variant): string {
    const id = /^nhentai:(\d+)$/.exec(variant.id)?.[1];
    const url = new URL(variant.url);
    if (!id || url.origin !== site || url.username || url.password
        || url.pathname !== `/g/${id}/` || url.search || url.hash) {
        throw new Error("The nhentai gallery does not match its source link.");
    }
    return id;
}

export const nhentai: Provider = {
    source: { id: "nhentai", name: "nhentai", url: site, categories: ["manga"], enabled: true },
    async search(request, context, emit) {
        const [result, cdn] = await Promise.all([
            json(`/search?query=${encodeURIComponent(request.query)}&page=1`, context), json("/cdn", context),
        ]);
        if (!Array.isArray(result.result)) throw new Error("nhentai did not return search results.");
        const server = mediaServer(cdn.thumb_servers?.[0], "thumb");
        const works = result.result.filter((gallery: RecordData) => !gallery.blacklisted).map((gallery: RecordData) => metadata(gallery, server));
        for (const work of works) emit?.(work);
        return works;
    },
    async inspect(variant, context) {
        const id = publication(variant);
        const [gallery, cdn] = await Promise.all([json(`/galleries/${id}`, context), json("/cdn", context)]);
        if (String(gallery.id) !== id) throw new Error("nhentai returned a different gallery.");
        const work = metadata(gallery, mediaServer(cdn.thumb_servers?.[0], "thumb"));
        const url = `${api}/galleries/${id}`;
        return { ...work, chapters: [{ id: contentId(url), title: work.title, number: 1, url }] };
    },
};

export async function loadNhentaiChapter(selected: Chapter, context: ProviderContext): Promise<Omit<ReaderPage, "id" | "documentId">> {
    const address = new URL(selected.url);
    const id = /^\/api\/v2\/galleries\/(\d+)$/.exec(address.pathname)?.[1];
    if (!id || address.origin !== site || address.username || address.password || address.search || address.hash) {
        throw new Error("This gallery does not belong to nhentai.");
    }
    const [gallery, cdn] = await Promise.all([json(`/galleries/${id}`, context), json("/cdn", context)]);
    const mediaId = String(gallery.media_id);
    if (String(gallery.id) !== id || !/^\d+$/.test(mediaId) || !Array.isArray(gallery.pages) || !gallery.pages.length
        || gallery.pages.length > 2000) throw new Error("nhentai did not return gallery pages.");
    const server = mediaServer(cdn.image_servers?.[0], "image");
    const pages: RecordData[] = [...gallery.pages].sort((left, right) => left.number - right.number);
    if (pages.some((page, index) => page.number !== index + 1)) throw new Error("nhentai returned an incomplete page sequence.");
    return { title: selected.title, kind: "images", direction: "rtl", images: pages.map(page => ({
        id: String(page.number), url: imageUrl(server, page.path, mediaId), width: page.width, height: page.height,
    })) };
}
