import type { Provider, ProviderContext, Source, Work } from "../../contracts.ts";
import { assertExactVariant } from "../../references.ts";
import { json } from "../http.ts";
import { publication } from "./common.ts";
import type { VkSearchVideo } from "../../browser/protocol.ts";
import { ContentError } from "../../errors.ts";

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

function matchesPublication(title: string, query: string, description?: string, duration?: number): boolean {
    const unwanted = /\b(?:trailer|teaser|reaction|review|AMV|highlights|promo|preview)\b|трейлер|тизер|реакци|обзор|нарезк|промо|превью/i;
    const promo = /(?:^|\n)\s*(?:анимированное\s+)?промо(?:\s|$)/i.test(description || "");
    const announcement = !!duration && duration <= 180
        && /(?:^|\n)\s*Статус\s*:\s*(?:запланирован|анонсирован|скоро)/i.test(description || "")
        && !/\d+\s*серия|\bS\d+E\d+\b/i.test(title);
    if ((unwanted.test(title) || promo || announcement) && !unwanted.test(query)) return false;
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
            if (/(?:^|\W)(?:на\s+русском|русск(?:ая|ой)\s+озвучк)/i.test(video.description || ""))
                work.variants[0]!.languages = ["ru"];
        }
    }
    return work;
}

async function rutubeQuality(work: Work, context: ProviderContext): Promise<Work> {
    const id = rutubeId(work.variants[0]!.url)!;
    const options = await json<{ video_balancer?: { m3u8?: string } }>(
        `https://rutube.ru/api/play/options/${id}/`, context);
    const address = options.video_balancer?.m3u8;
    if (!address) return work;
    const response = await context.fetch(address);
    if (!response.ok) throw new Error(`Rutube не предоставил данные качества (HTTP ${response.status}).`);
    const manifest = await response.text();
    if (!manifest.startsWith("#EXTM3U")) throw new Error("Rutube изменил формат данных качества.");
    const heights = [...manifest.matchAll(/\bRESOLUTION=\d+x(\d+)\b/g)].map(match => Number(match[1]));
    if (heights.length) work.variants[0]!.quality = `${Math.max(...heights)}p`;
    return work;
}

export const rutubeProvider: Provider = {
    source: { id: "rutube", name: "Rutube", url: "https://rutube.ru", categories: ["movies", "tv", "anime"], enabled: true },
    async search(request, context, emit) {
        if (!["all", "movies", "tv", "anime"].includes(request.category)) return [];
        const id = rutubeId(request.query);
        if (id) {
            const work = rutubeWork(await json<RutubeVideo>(`https://rutube.ru/api/video/${id}/`, context));
            emit?.(work);
            return [await rutubeQuality(work, context)];
        }
        const response = await json<{ results: RutubeVideo[] }>(`https://rutube.ru/api/search/video/?query=${encodeURIComponent(request.query)}&page=1`, context);
        const works = response.results
            .filter(video => !video.is_deleted && !video.is_hidden && !video.is_paid && !video.is_locked)
            .filter(video => matchesPublication(video.title, request.query, video.description, video.duration))
            .map(video => {
                const work = rutubeWork(video);
                return { ...work, category: request.category === "all" ? work.category : request.category };
            });
        for (const work of works) emit?.(work);
        for (let offset = 0; offset < works.length; offset += 4) {
            await Promise.all(works.slice(offset, offset + 4).map(async work => {
                try {
                    await rutubeQuality(work, context);
                    emit?.(work);
                }
                catch (error) {
                    context.signal.throwIfAborted();
                    context.reportError?.(error);
                }
            }));
        }
        return works;
    },
    async inspect(variant, context) {
        const id = rutubeId(variant.url);
        if (!id) throw new Error("Некорректная ссылка Rutube.");
        return rutubeQuality(rutubeWork(await json<RutubeVideo>(`https://rutube.ru/api/video/${id}/`, context)), context);
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

function vkWork(video: VkSearchVideo, source: Source): Work {
    const work = publication(source, video.id, video.title, video.url);
    work.variants[0]!.publicationKind = "upload";
    work.cover = video.cover;
    work.variants[0]!.author = video.author;
    work.variants[0]!.duration = video.duration;
    return work;
}

export const vkProvider: Provider = {
    source: {
        id: "vkvideo", name: "VK Видео", url: "https://vkvideo.ru",
        categories: ["movies", "tv", "anime"], enabled: true,
    },
    async search(request, context) {
        if (!["all", "movies", "tv", "anime"].includes(request.category)) return [];
        const id = vkId(request.query);
        if (id) {
            const work = publication(vkProvider.source, id, `VK Видео ${id}`, `https://vkvideo.ru/video${id}`);
            work.variants[0]!.publicationKind = "upload";
            return [work];
        }
        if (!context.searchVkVideos) throw new ContentError("browser_required", "Connect qcatch in Sources to search VK Video.", "https://vkvideo.ru/");
        const videos = await context.searchVkVideos(request.query.trim());
        return videos.filter(video => matchesPublication(video.title, request.query))
            .map(video => ({ ...vkWork(video, vkProvider.source), category: request.category === "all" ? "movies" : request.category }));
    },
    async resolve(variant) {
        if (!vkId(variant.url)) throw new Error("Некорректная ссылка VK Видео.");
        return { kind: "video", resolver: "yt-dlp", url: variant.url, title: variant.title, translation: variant.translation };
    },
};
