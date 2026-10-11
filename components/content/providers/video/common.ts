import type { Source, Variant, Work } from "../../contracts.ts";

export function absoluteUrl(value: string | undefined | null, base: string): string | undefined {
    if (!value) return undefined;
    try {
        const url = new URL(value, base);
        return /^https?:$/.test(url.protocol) ? url.href : undefined;
    }
    catch {
        return undefined;
    }
}

export function videoVariant(source: Source, id: string, title: string, url: string): Variant {
    return {
        id: `${source.id}:${id}`,
        sourceId: source.id,
        sourceName: source.name,
        title,
        url,
        transport: "video",
        adStatus: "unknown",
    };
}

export function publication(source: Source, id: string, title: string, url: string): Work {
    return {
        id: `${source.id}:${id}`,
        title,
        category: "movies",
        url,
        variants: [videoVariant(source, id, title, url)],
    };
}
