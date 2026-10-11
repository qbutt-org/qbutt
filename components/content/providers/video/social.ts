import type { Provider, Source, Work } from "../../contracts.ts";
import { assertExactVariant } from "../../references.ts";
import { json, publication } from "./common.ts";

// URL identity parsing follows stream-catcher's proven VK/Rutube parsers.
export function rutubeId(value: string): string | undefined {
    try {
        const url = new URL(value);
        if (!/^https?:$/.test(url.protocol) || !["rutube.ru", "www.rutube.ru"].includes(url.hostname)) return undefined;
        return url.pathname.match(/^\/(?:video|(?:play\/)?embed)\/([a-f0-9]{32})(?:\/|$)/i)?.[1]?.toLowerCase();
    }
    catch {
        return undefined;
    }
}

export function vkId(value: string): string | undefined {
    try {
        const url = new URL(value);
        const hostname = url.hostname.toLowerCase().replace(/^(?:www|m|new|vksport)\./, "");
        if (!/^https?:$/.test(url.protocol)
            || !["vk.com", "vk.ru", "vkvideo.com", "vkvideo.ru", "vkontakte.com", "vkontakte.ru"].includes(hostname)) return undefined;
        return (url.searchParams.get("z") ?? url.pathname.slice(1)).match(/^video(-?\d+_\d+)(?:\/|$)/)?.[1];
    }
    catch {
        return undefined;
    }
}

function matchesPublication(title: string, query: string, description?: string): boolean {
    const unwanted = /\b(?:trailer|teaser|reaction|review|AMV|highlights|promo|preview)\b|трейлер|тизер|реакци|обзор|нарезк|промо|превью/i;
    const promo = /(?:^|\n)\s*(?:анимированное\s+)?промо(?:\s|$)/i.test(description || "");
    if ((unwanted.test(title) || promo) && !unwanted.test(query)) return false;
    const normalize = (value: string) => value.toLocaleLowerCase().replace(/ё/g, "е");
    const words = normalize(query).match(/[\p{L}\p{N}]+/gu) ?? [];
    const normalized = normalize(title);
    return words.every(word => normalized.includes(word));
}

interface RutubeVideo {
    id: string;
    title: string;
    description?: string;
    thumbnail_url?: string;
    author?: { name: string };
    duration?: number;
    publication_ts?: string;
    is_deleted?: boolean;
    is_hidden?: boolean;
    is_paid?: boolean;
    is_locked?: boolean;
    category?: { id: number };
}

function rutubeWork(video: RutubeVideo): Work {
    const work = publication(rutubeProvider.source, video.id, video.title, `https://rutube.ru/video/${video.id}/`);
    work.variants[0]!.publicationKind = "upload";
    work.cover = video.thumbnail_url;
    work.description = video.description;
    work.variants[0]!.author = video.author?.name;
    work.variants[0]!.publishedAt = video.publication_ts;
    work.variants[0]!.duration = video.duration;
    if (video.duration) work.variants[0]!.notes = [{ text: `Длительность: ${Math.round(video.duration / 60)} мин.`, url: work.url! }];
    if (video.category?.id === 41) {
        work.category = "anime";
        const year = /\((?:аниме(?:-сериал)?,?\s*)?(19\d{2}|20\d{2})\)\s*$/i.exec(video.title)
            || /^Год\s*(?:производства|выпуска)?\s*:\s*(19\d{2}|20\d{2})\b/im.exec(video.description || "");
        work.year = year ? Number(year[1]) : undefined;
        const title = video.title.replace(/\s*\((?:аниме(?:-сериал)?,?\s*)?(?:19|20)\d{2}\)\s*$/i, "");
        const episode = /^(.+?)\s*[-–—]?\s*(\d+(?:\.\d+)?)\s*серия(?:\s*\[([^\]]+)\])?(?:\s*\/\s*(.+))?$/i.exec(title);
        if (episode && (video.duration || 0) >= 15 * 60) {
            work.aliases = [episode[1]!.trim(), ...episode[4] ? [episode[4].trim()] : []];
            work.variants[0]!.episode = String(Number(episode[2]));
            work.variants[0]!.translation = episode[3]?.trim()
                || /^Озвучка\s*[-–—:]\s*([^\n(]+)/im.exec(video.description || "")?.[1]?.trim();
            const season = /(?:season\s+(\d+)|(\d+)(?:st|nd|rd|th)?\s+season|(?:^|\s)(\d+)\s*сезон(?:\s|$)|\s(\d+)$)/i.exec(episode[1]!);
            work.variants[0]!.season = season?.slice(1).find(Boolean);
        }
    }
    return work;
}

export const rutubeProvider: Provider = {
    source: { id: "rutube", name: "Rutube", url: "https://rutube.ru", categories: ["movies", "tv", "anime"], enabled: true },
    async search(request, context) {
        if (!["all", "movies", "tv", "anime"].includes(request.category)) return [];
        const id = rutubeId(request.query);
        if (id) {
            return [rutubeWork(await json<RutubeVideo>(`https://rutube.ru/api/video/${id}/`, context))];
        }
        const response = await json<{ results: RutubeVideo[] }>(`https://rutube.ru/api/search/video/?query=${encodeURIComponent(request.query)}&page=1`, context);
        return response.results
            .filter(video => !video.is_deleted && !video.is_hidden && !video.is_paid && !video.is_locked)
            .filter(video => matchesPublication(video.title, request.query, video.description))
            .map(video => {
                const work = rutubeWork(video);
                return { ...work, category: request.category === "all" ? work.category : request.category };
            });
    },
    async inspect(variant, context) {
        const id = rutubeId(variant.url);
        if (!id) throw new Error("Некорректная ссылка Rutube.");
        return rutubeWork(await json<RutubeVideo>(`https://rutube.ru/api/video/${id}/`, context));
    },
    async resolve(variant, context) {
        const id = rutubeId(variant.url);
        if (!id) throw new Error("Некорректная ссылка Rutube.");
        const current = rutubeWork(await json<RutubeVideo>(`https://rutube.ru/api/video/${id}/`, context)).variants[0]!;
        assertExactVariant(variant, current);
        return { kind: "video", resolver: "yt-dlp", url: variant.url, title: current.title, translation: current.translation,
            selection: { episode: current.episode, season: current.season } };
    },
};

interface VkVideo {
    id: number;
    owner_id: number;
    title: string;
    description?: string;
    date?: number;
    image?: { url: string; width: number }[];
    duration?: number;
}

function vkWork(video: VkVideo, source: Source): Work {
    const id = `${video.owner_id}_${video.id}`;
    const work = publication(source, id, video.title, `https://vkvideo.ru/video${id}`);
    work.variants[0]!.publicationKind = "upload";
    work.description = video.description;
    work.cover = video.image?.toSorted((left, right) => right.width - left.width)[0]?.url;
    work.variants[0]!.publishedAt = video.date ? new Date(video.date * 1000).toISOString() : undefined;
    work.variants[0]!.duration = video.duration;
    return work;
}

export const vkProvider: Provider = {
    source: {
        id: "vkvideo", name: "VK Видео", url: "https://vkvideo.ru",
        categories: ["movies", "tv", "anime"], enabled: true,
        settings: [{ name: "accessToken", label: "Токен VK для поиска", type: "password" }],
    },
    async search(request, context) {
        if (!["all", "movies", "tv", "anime"].includes(request.category)) return [];
        const id = vkId(request.query);
        if (id) {
            const work = publication(vkProvider.source, id, `VK Видео ${id}`, `https://vkvideo.ru/video${id}`);
            work.variants[0]!.publicationKind = "upload";
            return [work];
        }
        if (!context.settings.accessToken) {
            throw new Error("Поиск VK требует токен аккаунта. Ссылки на конкретные видео открываются без настройки поиска.");
        }
        const parameters = new URLSearchParams({ v: "5.199", q: request.query, count: "100", adult: "0" });
        const result = await json<{ response?: { items: VkVideo[] }; error?: { error_code: number } }>(
            `https://api.vk.com/method/video.search?${parameters}`, context,
            { headers: { Authorization: `Bearer ${context.settings.accessToken}` } });
        if (!result.response) throw new Error(`VK не разрешил поиск (код ${result.error?.error_code ?? "неизвестен"}). Проверьте токен аккаунта.`);
        return result.response.items.filter(video => matchesPublication(video.title, request.query))
            .map(video => ({ ...vkWork(video, vkProvider.source), category: request.category === "all" ? "movies" : request.category }));
    },
    async resolve(variant) {
        if (!vkId(variant.url)) throw new Error("Некорректная ссылка VK Видео.");
        return { kind: "video", resolver: "yt-dlp", url: variant.url, title: variant.title, translation: variant.translation };
    },
};
