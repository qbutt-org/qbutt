import type { Provider, ProviderContext, ResolvedContent, Variant, WorkDetails } from "../../contracts.ts";
import { publicUrl } from "../../net.ts";
import { json } from "../http.ts";
import { absoluteUrl, videoVariant } from "./common.ts";
import { resolveCvh } from "./cvh.ts";
import { resolveKodik } from "./kodik.ts";

const siteUrl = "https://yani.tv";
const apiUrl = "https://api.yani.tv";
const sessionRefreshes = new Map<string, Promise<Record<string, string>>>();

interface Video {
    video_id: number;
    data: { player: string; dubbing: string; player_id: number };
    number: string;
    duration?: number;
    date?: number;
    iframe_url: string;
}

interface Anime {
    anime_id: number;
    anime_url: string;
    title: string;
    description?: string;
    year?: number;
    poster?: { big?: string };
    other_titles?: string[];
    videos?: Video[];
    remote_ids?: { myanimelist_id?: number; shikimori_id?: number; kp_id?: number };
}

interface AksorVideo {
    id: string;
    anime_id: number;
    episode: string;
    studio_name: string;
    is_subtitle: boolean;
    qualities: Record<string, string | null>;
}

function headers(context: ProviderContext): Record<string, string> {
    const result: Record<string, string> = { Accept: "application/json" };
    if (context.settings.applicationToken) result["X-Application"] = context.settings.applicationToken;
    if (context.settings.accountToken) result.Authorization = `Yummy ${context.settings.accountToken}`;
    return result;
}

async function api<T>(path: string, context: ProviderContext): Promise<T> {
    const refreshed = Number(context.settings.accountTokenRefreshedAt);
    if (context.settings.accountToken && context.saveSettings
        && (!Number.isFinite(refreshed) || Date.now() - refreshed > 2 * 24 * 60 * 60 * 1000)) {
        const key = `${context.dataDir}\0${context.settings.accountToken}`;
        let refresh = sessionRefreshes.get(key);
        if (!refresh) {
            refresh = (async () => {
                const response = await json<{ response: { token: string } }>(`${apiUrl}/profile/token`, context,
                    { headers: headers(context) });
                if (!response.response?.token || typeof response.response.token !== "string") {
                    throw new Error("YummyAnime не обновил сессию. Подключите аккаунт заново.");
                }
                const changes = { accountToken: response.response.token, accountTokenRefreshedAt: String(Date.now()) };
                await context.saveSettings!(changes);
                return changes;
            })().finally(() => sessionRefreshes.delete(key));
            sessionRefreshes.set(key, refresh);
        }
        Object.assign(context.settings, await refresh);
    }
    return json<T>(`${apiUrl}${path}`, context, { headers: headers(context) });
}

function animeWork(anime: Anime): WorkDetails {
    const url = `${siteUrl}/catalog/item/${anime.anime_url}`;
    const catalog = videoVariant(yaniProvider.source, String(anime.anime_id), anime.title, url);
    const variants = (anime.videos ?? []).map(video => ({
        ...catalog,
        id: `yani:${anime.anime_id}:video:${video.video_id}`,
        episode: video.number,
        translation: video.data.dubbing.replace(/^озвучка\s*/i, ""),
        player: video.data.player.replace(/^плеер\s*/i, ""),
        publishedAt: video.date ? new Date(video.date * 1000).toISOString() : undefined,
        languages: /субтитры/i.test(video.data.dubbing) ? ["ja", "ru"] : ["ru"],
        translationType: /субтитры/i.test(video.data.dubbing) ? "subtitles" as const : "dub" as const,
        duration: video.duration,
    }));
    return {
        id: `yani:${anime.anime_id}`,
        title: anime.title,
        category: "anime",
        url,
        year: anime.year,
        description: anime.description,
        aliases: anime.other_titles,
        identifiers: {
            ...(anime.remote_ids?.myanimelist_id ? { mal: String(anime.remote_ids.myanimelist_id) } : {}),
            ...(anime.remote_ids?.shikimori_id ? { shikimori: String(anime.remote_ids.shikimori_id) } : {}),
            ...(anime.remote_ids?.kp_id ? { kinopoisk: String(anime.remote_ids.kp_id) } : {}),
        },
        cover: absoluteUrl(anime.poster?.big, siteUrl),
        variants: variants.length ? variants : [{ ...catalog, needsInspection: true }],
    };
}

export function yaniPublication(url: URL): Variant | undefined {
    const slug = /^\/catalog\/item\/([a-z\d]+(?:-[a-z\d]+)*)\/?$/.exec(url.pathname)?.[1];
    if (url.protocol !== "https:" || !["yani.tv", "ru.yummyani.me", "yummyani.me"].includes(url.hostname)
        || url.username || url.password || url.port || !slug) return undefined;
    return { ...videoVariant(yaniProvider.source, slug, "YummyAnime", `${siteUrl}/catalog/item/${slug}`), needsInspection: true };
}

async function animeDetails(variant: Variant, context: ProviderContext): Promise<Anime> {
    const id = variant.id.split(":")[1];
    if (!/^[a-z\d]+(?:-[a-z\d]+)*$/.test(id ?? "")) throw new Error("Не удалось определить тайтл YummyAnime.");
    const data = await api<{ response: Anime }>(`/anime/${id}?need_videos=true`, context);
    return data.response;
}

async function resolveAksor(anime: Anime, selected: Video, playerUrl: string,
    context: ProviderContext): Promise<ResolvedContent> {
    const player = new URL(playerUrl);
    const hash = /^\/video\/([a-f\d]{32})$/.exec(player.pathname)?.[1];
    if (player.origin !== "https://player.aksor.tv" || !hash || player.username || player.password) {
        throw new Error("Источник изменил адрес выбранного плеера Aksor.");
    }

    const video = await json<AksorVideo>(`https://player.aksor.tv/api/video/${hash}`, context,
        { headers: { Accept: "application/json", Referer: player.href } });
    const subtitles = /^субтитры(?:\s|$)/i.test(selected.data.dubbing);
    const studio = selected.data.dubbing.replace(/^(?:озвучка|субтитры)\s*/i, "")
        || (subtitles ? "subtitles" : "");
    const normalize = (value: string) => value.normalize("NFC").replace(/\s+/g, " ").trim().toLowerCase();
    if (video.id !== hash || video.anime_id !== anime.anime_id || video.episode !== selected.number
        || video.is_subtitle !== subtitles || typeof video.studio_name !== "string"
        || normalize(video.studio_name) !== normalize(studio)) {
        throw new Error("Aksor вернул другой тайтл, серию или озвучку. Обновите варианты тайтла.");
    }

    const quality = ["q4k", "q2k", "q1080", "q720", "q480", "q360"]
        .find(quality => typeof video.qualities?.[quality] === "string" && video.qualities[quality]!.length > 0);
    const address = quality ? video.qualities?.[quality] : undefined;
    if (!address) throw new Error("Aksor не предоставил видео выбранной серии и озвучки.");
    const media = await publicUrl(address);
    if (media.protocol !== "https:" || !media.pathname.endsWith(".mpd")) {
        throw new Error("Aksor изменил формат адреса выбранного видео.");
    }
    return {
        kind: "video",
        resolver: "direct",
        url: media.href,
        headers: { Referer: player.href },
        title: anime.title,
        translation: selected.data.dubbing.replace(/^озвучка\s*/i, ""),
        quality: ({ q4k: "4K", q2k: "2K", q1080: "1080p", q720: "720p", q480: "480p", q360: "360p" } as Record<string, string>)[quality!],
        selection: { episode: selected.number, player: "Aksor" },
    };
}

export const yaniProvider: Provider = {
    source: {
        id: "yani",
        name: "YummyAnime",
        url: siteUrl,
        categories: ["anime"],
        enabled: true,
        settings: [
            { name: "applicationToken", label: "Токен приложения", type: "password" },
            { name: "accountToken", label: "Токен аккаунта", type: "password" },
        ],
    },
    async search(request, context, emit) {
        if (!["all", "anime"].includes(request.category)) return [];
        const result = await api<{ response: Anime[] }>(`/anime?q=${encodeURIComponent(request.query)}&limit=40`, context);
        const releases = result.response.slice(0, 40);
        for (const anime of releases) emit?.(animeWork(anime));
        // Search returns only the Russian title. Verified alternate titles live
        // in the detail response; fetch metadata without the large video list.
        for (let offset = 0; offset < releases.length; offset += 4) {
            await Promise.all(releases.slice(offset, offset + 4).map(async anime => {
                try {
                    const metadata = await api<{ response: Anime }>(`/anime/${anime.anime_id}?need_videos=false`, context);
                    anime.other_titles = metadata.response.other_titles;
                    emit?.(animeWork(anime));
                }
                catch (error) {
                    context.signal.throwIfAborted();
                    context.reportError?.(error);
                }
            }));
        }
        return releases.map(animeWork);
    },
    async inspect(variant, context, emit) {
        const anime = await animeDetails(variant, context);
        const work = animeWork(anime);
        emit?.(work);
        const firstEpisodes = new Map<string, Variant>();
        for (const selected of work.variants) {
            if (selected.player !== "CVH" && selected.player !== "Aksor") continue;
            if (variant.episode && selected.id !== variant.id) continue;
            const key = JSON.stringify([selected.translation, selected.player]);
            const first = firstEpisodes.get(key);
            if (!first || Number(selected.episode) < Number(first.episode)) firstEpisodes.set(key, selected);
        }
        const inspectable = [...firstEpisodes.values()];
        // Publish every episode immediately, then verify a representative of
        // each voice/player within a small optional metadata budget. Quality
        // belongs only to the exact episode whose player confirmed it.
        const qualitySignal = AbortSignal.any([context.signal, AbortSignal.timeout(8_000)]);
        const qualityContext = { ...context, signal: qualitySignal,
            fetch: (url: string, init?: RequestInit) => context.fetch(url, { ...init,
                signal: init?.signal ? AbortSignal.any([qualitySignal, init.signal]) : qualitySignal }) };
        for (let offset = 0; offset < inspectable.length && !qualitySignal.aborted; offset += 4) {
            await Promise.all(inspectable.slice(offset, offset + 4).map(async selected => {
                try {
                    const video = anime.videos!.find(video => String(video.video_id) === selected.id.split(":")[3])!;
                    const url = absoluteUrl(video.iframe_url, siteUrl);
                    if (!url) return;
                    const resolved = selected.player === "Aksor" ? await resolveAksor(anime, video, url, qualityContext)
                        : await resolveCvh(url, { title: anime.title, malId: anime.remote_ids?.myanimelist_id,
                            episode: video.number, translation: selected.translation! }, qualityContext);
                    if (resolved.quality) {
                        selected.quality = resolved.quality;
                        emit?.(work);
                    }
                }
                catch (error) {
                    context.signal.throwIfAborted();
                    if (!qualitySignal.aborted) context.reportError?.(error);
                }
            }));
        }
        return work;
    },
    async resolve(variant, context) {
        const anime = await animeDetails(variant, context);
        const selected = anime.videos?.find(video => String(video.video_id) === variant.id.split(":")[3]);
        if (!selected) throw new Error("Выберите серию, озвучку и плеер в вариантах тайтла.");
        if (variant.episode !== selected.number
            || variant.translation !== selected.data.dubbing.replace(/^озвучка\s*/i, "")
            || variant.player !== selected.data.player.replace(/^плеер\s*/i, "")) {
            throw new Error("Источник изменил выбранную серию, озвучку или плеер. Обновите варианты тайтла.");
        }
        const url = absoluteUrl(selected.iframe_url, siteUrl);
        if (!url) throw new Error("Источник не предоставил адрес выбранного плеера.");
        const playerUrl = new URL(url);
        if (playerUrl.origin === "https://ru.yummyani.me" && playerUrl.pathname === "/iframeVK.html") {
            const ids = playerUrl.searchParams.getAll("id");
            const id = ids.length === 1 ? /^(?:video)?(-?\d+_\d+)$/.exec(ids[0]!)?.[1] : undefined;
            if (!id || playerUrl.username || playerUrl.password)
                throw new Error("Источник изменил ссылку выбранного видео VK. Обновите варианты тайтла.");
            return { kind: "video", resolver: "yt-dlp", url: `https://vkvideo.ru/video${id}`,
                title: anime.title, translation: variant.translation,
                selection: { episode: selected.number, player: variant.player } };
        }
        if (variant.player === "Aksor") return resolveAksor(anime, selected, url, context);
        if (variant.player === "Kodik") {
            return resolveKodik(url, {
                title: anime.title,
                episode: selected.number,
                translation: variant.translation,
                publicationUrl: variant.url,
            }, context);
        }
        if (variant.player === "CVH") {
            return resolveCvh(url, {
                title: anime.title,
                malId: anime.remote_ids?.myanimelist_id,
                episode: selected.number,
                translation: variant.translation,
            }, context);
        }
        return { kind: "video", resolver: "browser", url, headers: { Referer: variant.url },
            title: anime.title, translation: selected.data.dubbing.replace(/^озвучка\s*/i, "") };
    },
};
