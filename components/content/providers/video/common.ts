import { load } from "cheerio";
import type { ProviderContext, Source, Variant, Work } from "../../contracts.ts";

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

export async function json<T>(url: string, context: ProviderContext, init?: RequestInit): Promise<T> {
    const response = await context.fetch(url, init);
    if (!response.ok) {
        if ([401, 403, 420].includes(response.status)) {
            throw new Error(`Источник требует входа или проверки в браузере (HTTP ${response.status}).`);
        }
        throw new Error(`Источник вернул HTTP ${response.status}.`);
    }
    return response.json() as Promise<T>;
}

export async function html(url: string, context: ProviderContext, init?: RequestInit): Promise<ReturnType<typeof load>> {
    const response = await context.fetch(url, init);
    if (!response.ok) throw new Error(`Откройте источник в браузере и пройдите проверку (HTTP ${response.status}).`);
    const charset = response.headers.get("content-type")?.match(/charset=([^;\s]+)/i)?.[1] ?? "utf-8";
    const text = new TextDecoder(charset).decode(await response.arrayBuffer());
    if (/DDoS-Guard|Just a moment\.\.\.|cf-chl-/.test(text)) {
        throw new Error("Источник требует проверки в браузере.");
    }
    return load(text);
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
