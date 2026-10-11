import type { Chapter, Provider, ProviderContext, Variant, Work, WorkDetails } from "../contracts";
import { contentId, type ReaderPage } from "../reader/types";

const site = "https://mangadex.org";
const api = "https://api.mangadex.org";
const uuid = "[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12}";
const publicationPattern = new RegExp(`^mangadex:(${uuid})(?::([a-z]{2}(?:-[a-z]{2})?):((?:${uuid})(?:\\+(?:${uuid}))*|none))?$`, "i");
const chapterPattern = new RegExp(`^/chapter/(${uuid})$`, "i");
const ratings = "contentRating[]=safe&contentRating[]=suggestive&contentRating[]=erotica&contentRating[]=pornographic";

interface Entity {
    id: string;
    attributes: Record<string, any>;
    relationships?: { id: string; type: string; attributes?: Record<string, any> }[];
}

async function json(path: string, context: ProviderContext): Promise<any> {
    const response = await context.fetch(`${api}${path}`, { headers: { Accept: "application/json" }, signal: context.signal });
    if (!response.ok) throw new Error(`MangaDex returned HTTP ${response.status}.`);
    const result = await response.json();
    if (result.result !== "ok") throw new Error("MangaDex did not return the requested publication.");
    return result;
}

function metadata(manga: Entity, language = "en"): Work {
    const values = manga.attributes;
    const names: Record<string, string>[] = [values.title, ...values.altTitles ?? []];
    const title = names.map(name => name?.[language]).find(Boolean)
        || values.title?.en || Object.values(values.title ?? {})[0] as string;
    if (!title) throw new Error("MangaDex did not return the title.");
    const url = `${site}/title/${manga.id}`;
    const cover = manga.relationships?.find(entry => entry.type === "cover_art")?.attributes?.fileName;
    const languages: string[] = (values.availableTranslatedLanguages ?? []).filter((value: unknown) => typeof value === "string");
    languages.sort((left, right) => Number(right === language) - Number(left === language));
    return {
        id: `mangadex:${manga.id}`, title, category: "manga", url,
        aliases: [...new Set(names.flatMap(name => Object.values(name ?? {})).filter(name => name !== title))],
        description: values.description?.[language] || values.description?.en,
        year: typeof values.year === "number" ? values.year : undefined,
        identifiers: { mangadex: manga.id },
        cover: cover ? `https://uploads.mangadex.org/covers/${manga.id}/${encodeURIComponent(cover)}.256.jpg` : undefined,
        variants: [{ id: `mangadex:${manga.id}`, sourceId: "mangadex", sourceName: "MangaDex", title, url,
            transport: "manga", languages,
            needsInspection: true }],
    };
}

function publication(variant: Variant): { id: string; branch?: string } {
    const match = publicationPattern.exec(variant.id);
    const url = new URL(variant.url);
    if (!match || url.origin !== site || url.username || url.password
        || url.pathname !== `/title/${match[1]}` || url.search || url.hash) {
        throw new Error("The MangaDex publication does not match its source link.");
    }
    return { id: match[1]!, branch: match[2] ? `${match[2]}:${match[3]}` : undefined };
}

function branchFor(chapter: Entity): { key: string; language: string; translation?: string } {
    const groups = (chapter.relationships ?? []).filter(entry => entry.type === "scanlation_group").sort((left, right) => left.id.localeCompare(right.id));
    const language = chapter.attributes.translatedLanguage as string;
    return { key: `${language}:${groups.map(group => group.id).join("+") || "none"}`, language,
        translation: groups.map(group => group.attributes?.name).filter(Boolean).join(", ") || undefined };
}

async function inspect(variant: Variant, context: ProviderContext): Promise<WorkDetails> {
    const selected = publication(variant);
    const details = await json(`/manga/${selected.id}?includes[]=cover_art`, context);
    const work = metadata(details.data, variant.languages?.[0]);
    const branches = new Map<string, { variant: Variant; chapters: Map<string, Entity> }>();
    let offset = 0;
    let total = 1;
    while (offset < total) {
        const listing = await json(`/manga/${selected.id}/feed?limit=500&offset=${offset}&includes[]=scanlation_group&order[volume]=asc&order[chapter]=asc&${ratings}`, context);
        total = listing.total;
        if (!Array.isArray(listing.data) || !listing.data.length) break;
        for (const entry of listing.data as Entity[]) {
            if (!entry.attributes.pages || entry.attributes.externalUrl) continue;
            const branch = branchFor(entry);
            const current = branches.get(branch.key) ?? { variant: { ...work.variants[0]!,
                id: `${work.id}:${branch.key}`, languages: [branch.language], translation: branch.translation,
                needsInspection: false }, chapters: new Map<string, Entity>() };
            // Multiple uploads of one numbered chapter remain within their exact
            // translation group. Keep the newest published upload in that group.
            const key = entry.attributes.chapter === null ? entry.id : `${entry.attributes.volume ?? ""}:${entry.attributes.chapter}`;
            const prior = current.chapters.get(key);
            if (!prior || (entry.attributes.publishAt ?? "") > (prior.attributes.publishAt ?? "")) current.chapters.set(key, entry);
            branches.set(branch.key, current);
        }
        offset += listing.data.length;
        if (offset > 30_000) throw new Error("This MangaDex title has too many chapter uploads.");
    }
    const available = [...branches.entries()].sort(([, left], [, right]) => {
        const preferred = variant.languages?.[0] || "en";
        return Number(right.variant.languages?.[0] === preferred) - Number(left.variant.languages?.[0] === preferred)
            || right.chapters.size - left.chapters.size || left.variant.id.localeCompare(right.variant.id);
    });
    const chosen = selected.branch ? branches.get(selected.branch) : available[0]?.[1];
    if (!chosen) throw new Error("The selected MangaDex translation has no available chapters.");
    const entries = [...chosen.chapters.values()].sort((left, right) =>
        (left.attributes.volume == null ? Infinity : Number(left.attributes.volume))
            - (right.attributes.volume == null ? Infinity : Number(right.attributes.volume))
        || Number(left.attributes.chapter || 0) - Number(right.attributes.chapter || 0)
        || (left.attributes.publishAt || "").localeCompare(right.attributes.publishAt || ""));
    const russian = chosen.variant.languages?.[0] === "ru";
    const chapters = entries.map((entry, index) => {
        const values = entry.attributes;
        const label = values.chapter ? `${russian ? "Глава" : "Chapter"} ${values.chapter}`
            : values.title || (russian ? "Ваншот" : "One-shot");
        const url = `${api}/chapter/${entry.id}`;
        return { id: contentId(url), url, number: index + 1,
            title: `${values.volume ? `${russian ? "Том" : "Volume"} ${values.volume}, ` : ""}${label}${values.title && values.chapter ? ` — ${values.title}` : ""}` };
    });
    return { ...work, variants: available.map(([, value]) => value.variant), chapters };
}

export const mangaDex: Provider = {
    source: { id: "mangadex", name: "MangaDex", url: site, categories: ["manga"], enabled: true },
    async search(request, context, emit) {
        const result = await json(`/manga?title=${encodeURIComponent(request.query)}&limit=50&includes[]=cover_art&${ratings}`, context);
        const works = (result.data ?? []).map((entry: Entity) => metadata(entry, request.language || "en"));
        for (const work of works) emit?.(work);
        return works;
    },
    inspect,
};

export async function loadMangaDexChapter(selected: Chapter, context: ProviderContext): Promise<Omit<ReaderPage, "id" | "documentId">> {
    const address = new URL(selected.url);
    const id = chapterPattern.exec(address.pathname)?.[1];
    if (!id || address.origin !== api || address.username || address.password || address.search || address.hash) {
        throw new Error("This chapter does not belong to MangaDex.");
    }
    const result = await json(`/at-home/server/${id}`, context);
    const base = new URL(result.baseUrl);
    const hash = result.chapter?.hash;
    const files = result.chapter?.data;
    if (base.protocol !== "https:" || base.username || base.password || !/^[a-f0-9]{32}$/i.test(hash)
        || !Array.isArray(files) || !files.length || files.length > 2000
        || files.some((file: unknown) => typeof file !== "string" || !/^[\w.-]+$/.test(file) || file === "." || file === "..")) {
        throw new Error("MangaDex did not return valid chapter images.");
    }
    return { title: selected.title, kind: "images", direction: "rtl", images: files.map((file: string, index: number) => {
        const url = new URL(base);
        url.pathname = `${url.pathname.replace(/\/$/, "")}/data/${hash}/${file}`;
        return { id: String(index + 1), url: url.href };
    }) };
}
