import { load } from "cheerio";
import type { Provider, ProviderContext, Variant, WorkDetails } from "../../contracts.ts";
import { absoluteUrl, html, json, videoVariant } from "./common.ts";

interface CdnResponse {
    success: boolean;
    url?: string;
    episodes?: string;
    subtitle?: string;
    message?: string;
}

function sessionHeaders(context: ProviderContext, referer: string): Record<string, string> {
    const configured = context.settings.baseUrl || "https://hdrezka.ag";
    const sameOrigin = new URL(configured).origin === new URL(referer).origin;
    return {
        Referer: referer,
        ...(sameOrigin && context.settings.cookie ? { Cookie: context.settings.cookie } : {}),
    };
}

async function cdn(page: string, parameters: Record<string, string>, context: ProviderContext): Promise<CdnResponse> {
    return json<CdnResponse>(new URL("/ajax/get_cdn_series/", page).href, context, {
        method: "POST",
        headers: { ...sessionHeaders(context, page), "Content-Type": "application/x-www-form-urlencoded", "X-Requested-With": "XMLHttpRequest" },
        body: new URLSearchParams(parameters),
    });
}

function decodePlaylist(value: string): string {
    if (/^\[|^https?:\/\//.test(value)) return value;
    let encoded = value.replace(/^#h/, "");
    // These are the site's URL obfuscation salts, not encryption or DRM keys.
    const salts = ["IyMjI14hISMjIUBA", "QEBAQEAhIyMhXl5e", "JCQhIUAkJEBeIUAjJCRA", "JCQjISFAIyFAIyM=", "Xl5eIUAjIyEhIyM="];
    for (const salt of salts) encoded = encoded.replaceAll(`//_//${salt}`, "");
    if (encoded.includes("//_//")) throw new Error("Формат плеера изменился. Откройте выбранную версию в браузере.");
    return Buffer.from(encoded, "base64").toString("utf8");
}

function playlistEntries(value: string): { title: string; url: string }[] {
    return [...decodePlaylist(value).matchAll(/\[([^\]]+)\]([^\[]+)/g)]
        .flatMap(match => match[2]!.split(/\s+or\s+/).map(url => ({
            title: load(match[1]!).text().trim(),
            url: url.trim().replace(/,$/, ""),
        })))
        .filter(entry => /^https?:\/\//.test(entry.url));
}

export const hdrezkaProvider: Provider = {
    source: {
        id: "hdrezka", name: "HDRezka", url: "https://hdrezka.ag",
        categories: ["movies", "tv", "anime"], enabled: true,
        settings: [
            { name: "baseUrl", label: "Адрес сайта", type: "text", value: "https://hdrezka.ag" },
            { name: "cookie", label: "Сессия аккаунта", type: "password" },
        ],
    },
    async search(request, context) {
        if (!["all", "movies", "tv", "anime"].includes(request.category)) return [];
        const base = context.settings.baseUrl || hdrezkaProvider.source.url;
        const url = new URL("/search/", base);
        url.search = new URLSearchParams({ do: "search", subaction: "search", q: request.query }).toString();
        const $ = await html(url.href, context, { headers: sessionHeaders(context, base) });
        return $(".b-content__inline_item").toArray().flatMap(element => {
            const item = $(element);
            const link = item.find(".b-content__inline_item-link a").first();
            const page = absoluteUrl(link.attr("href"), base);
            const title = link.text().trim();
            const id = item.attr("data-id") ?? page?.match(/\/(\d+)-/)?.[1];
            if (!page || !title || !id) return [];
            const category = page.includes("/animation/") ? "anime" : page.includes("/series/") ? "tv" : "movies";
            if (request.category !== "all" && request.category !== category) return [];
            return [{ id: `hdrezka:${id}`, title, category, url: page,
                cover: absoluteUrl(item.find("img").attr("src"), base),
                year: Number(item.find(".b-content__inline_item-link div").text().match(/\b(19\d\d|20\d\d)\b/)?.[1]) || undefined,
                variants: [videoVariant(hdrezkaProvider.source, id, title, page)] } as WorkDetails];
        });
    },
    async inspect(variant, context) {
        const $ = await html(variant.url, context, { headers: sessionHeaders(context, variant.url) });
        const id = variant.id.split(":")[1]!;
        const title = $(".b-post__title h1").text().trim() || $("h1").first().text().trim();
        const initialization = $.html().match(/initCDN(Series|Movies)Events\(\d+,\s*(\d+)/);
        const series = initialization?.[1] === "Series" || $("[data-episode_id][data-season_id]").length > 0;
        const translators = $("#translators-list [data-translator_id]").toArray().map(item => ({
            id: $(item).attr("data-translator_id")!, name: $(item).text().trim(),
            ads: $(item).attr("data-ads") === "1", cam: $(item).attr("data-camrip") === "1",
        }));
        if (!translators.length && initialization?.[2]) {
            const information = $(".b-post__info tr").toArray().find(row => /перевод/i.test($(row).find("td").first().text()));
            translators.push({ id: initialization[2], name: information ? $(information).find("td").last().text().trim() : "Перевод источника",
                ads: false, cam: false });
        }
        const variants: Variant[] = [];
        for (const translator of translators) {
            const translatorId = translator.id;
            const translated: Variant = {
                ...videoVariant(hdrezkaProvider.source, `${id}:translation:${translatorId}`, title, variant.url),
                translation: translator.name,
                adStatus: translator.ads ? "reported" : "unknown",
                quality: translator.cam ? "CAM" : undefined,
            };
            if (!series) {
                variants.push(translated);
                continue;
            }
            const data = await cdn(variant.url, { action: "get_episodes", id, translator_id: translatorId }, context);
            if (!data.success || !data.episodes) {
                variants.push({ ...translated, id: `${translated.id}:series`,
                    notes: [{ text: "Источник не предоставил список серий для этого перевода.", url: variant.url }] });
                continue;
            }
            const episodes = load(data.episodes);
            for (const episode of episodes("[data-episode_id][data-season_id]").toArray()) {
                const season = episodes(episode).attr("data-season_id")!;
                const number = episodes(episode).attr("data-episode_id")!;
                variants.push({ ...translated, id: `${translated.id}:${season}:${number}`, episode: `S${season}E${number}` });
            }
        }
        return {
            id: `hdrezka:${id}`, title,
            category: variant.url.includes("/series/") ? "tv" : variant.url.includes("/animation/") ? "anime" : "movies",
            url: variant.url,
            cover: absoluteUrl($('meta[property="og:image"]').attr("content"), variant.url),
            description: $(".b-post__description_text").text().trim(),
            variants: variants.length ? variants : [variant],
        };
    },
    async resolve(variant, context) {
        const [, id, , translator, season, episode] = variant.id.split(":");
        if (!id || !translator) throw new Error("Выберите перевод в вариантах фильма.");
        if (season === "series") throw new Error("Источник не предоставил список серий. Подключите сессию источника и обновите варианты.");
        const data = await cdn(variant.url, { id, translator_id: translator,
            action: episode ? "get_stream" : "get_movie", ...(episode && season ? { season, episode } : {}) }, context);
        if (!data.success || !data.url) {
            return { kind: "video", resolver: "browser", url: variant.url, title: variant.title,
                translation: variant.translation,
                selection: { translationId: translator, season, episode } };
        }
        const streams = playlistEntries(data.url).sort((left, right) =>
            Number(right.title.match(/\d+/)?.[0] ?? 0) - Number(left.title.match(/\d+/)?.[0] ?? 0));
        const stream = streams[0];
        if (!stream) throw new Error("Выбранный плеер не предоставил прямой поток.");
        return { kind: "video", resolver: "direct", url: stream.url,
            title: variant.title, translation: variant.translation,
            headers: { Referer: variant.url },
            subtitles: data.subtitle ? playlistEntries(data.subtitle).map(entry => ({ title: entry.title, url: entry.url })) : undefined };
    },
};
