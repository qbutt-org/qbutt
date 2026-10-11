import type { Provider, ProviderContext, WorkDetails } from "../../contracts.ts";
import { json, videoVariant } from "./common.ts";

interface KinoAudio {
    id: number;
    lang?: string;
    codec?: string;
    channels?: number;
    author?: { title?: string };
    type?: { title?: string };
}

interface KinoVideo {
    number?: number;
    title: string;
    audios?: KinoAudio[];
    subtitles?: { lang: string; url: string }[];
    files?: { quality: string; codec?: string; url?: { hls4?: string; hls?: string; http?: string } }[];
}

interface KinoItem {
    id: number;
    title: string;
    type: string;
    year?: number;
    plot?: string;
    posters?: { big?: string };
    advert?: boolean;
    videos?: KinoVideo[];
    seasons?: { number: number; episodes: KinoVideo[] }[];
}

async function api<T>(path: string, context: ProviderContext): Promise<T> {
    if (!context.settings.accessToken) throw new Error("Подключите аккаунт KinoPub в настройках источника.");
    const base = context.settings.apiUrl || "https://api.service-kp.com/v1/";
    return json<T>(new URL(path, base.endsWith("/") ? base : `${base}/`).href, context, {
        headers: { Authorization: `Bearer ${context.settings.accessToken}`, Accept: "application/json" },
    });
}

function itemWork(item: KinoItem): WorkDetails {
    const url = `https://kino.pub/item/view/${item.id}`;
    const catalog = videoVariant(kinopubProvider.source, String(item.id), item.title, url);
    catalog.adStatus = item.advert === true ? "reported" : "unknown";
    const videos = item.seasons?.flatMap(season => season.episodes.map((video, index) => ({
        video, season: season.number, number: video.number ?? index + 1,
    }))) ?? item.videos?.map((video, index) => ({ video, season: 0, number: video.number ?? index + 1 })) ?? [];
    const variants = videos.flatMap(({ video, season, number }) => {
        const audios = video.audios?.filter(audio => Number.isSafeInteger(audio.id) && audio.id > 0
            && (audio.author?.title || audio.type?.title)) ?? [];
        return (video.files ?? []).flatMap(file => (audios.length ? audios : [undefined]).map(audio => ({
            ...catalog,
            id: `kinopub:${item.id}:${season}:${number}:${encodeURIComponent(file.quality)}:${encodeURIComponent(file.codec ?? "")}`
                + (audio ? `:audio:${audio.id}` : ""),
            episode: season ? `S${season}E${number}` : videos.length > 1 ? String(number) : undefined,
            quality: file.quality,
            player: file.codec?.toUpperCase(),
            languages: audio?.lang ? [audio.lang] : [],
            translation: audio ? [audio.author?.title, audio.type?.title, audio.lang?.toUpperCase(),
                audio.codec?.toUpperCase(), audio.channels ? `${audio.channels} ch` : undefined]
                .filter(Boolean).join(", ") : undefined,
        })));
    });
    return {
        id: `kinopub:${item.id}`, title: item.title,
        category: /serial/.test(item.type) ? "tv" : "movies",
        url, year: item.year, description: item.plot, cover: item.posters?.big,
        variants: variants.length ? variants : [catalog],
    };
}

export const kinopubProvider: Provider = {
    source: {
        id: "kinopub", name: "KinoPub", url: "https://kino.pub",
        categories: ["movies", "tv", "anime"], enabled: true, requiresLogin: true,
        settings: [
            { name: "accessToken", label: "Токен аккаунта", type: "password" },
            { name: "apiUrl", label: "Адрес API", type: "text", value: "https://api.service-kp.com/v1/" },
        ],
    },
    async search(request, context) {
        if (!["all", "movies", "tv", "anime"].includes(request.category)) return [];
        const type = request.category === "tv" ? "&type=serial" : request.category === "movies" ? "&type=movie" : "";
        const result = await api<{ items: KinoItem[] }>(`items/search?q=${encodeURIComponent(request.query)}&perpage=40${type}`, context);
        return result.items.map(itemWork);
    },
    async inspect(variant, context) {
        const id = variant.id.split(":")[1];
        if (!/^\d+$/.test(id ?? "")) throw new Error("Не удалось определить фильм KinoPub.");
        const result = await api<{ item: KinoItem }>(`items/${id}`, context);
        return itemWork(result.item);
    },
    async resolve(variant, context) {
        const [, id, season, episode, quality, codec, selection, audioId] = variant.id.split(":");
        if (!quality || codec === undefined) throw new Error("Выберите серию, озвучку и качество в вариантах KinoPub.");
        const { item } = await api<{ item: KinoItem }>(`items/${id}`, context);
        const video = Number(season)
            ? item.seasons?.find(candidate => candidate.number === Number(season))?.episodes
                .find((candidate, index) => (candidate.number ?? index + 1) === Number(episode))
            : item.videos?.find((candidate, index) => (candidate.number ?? index + 1) === Number(episode));
        const file = video?.files?.find(candidate => candidate.quality === decodeURIComponent(quality)
            && (candidate.codec ?? "") === decodeURIComponent(codec));
        const audio = selection === "audio" ? video?.audios?.find(candidate => String(candidate.id) === audioId) : undefined;
        if (selection && (!audio || !(audio.author?.title || audio.type?.title)))
            throw new Error("Выбранная озвучка KinoPub больше не доступна. Выберите другую озвучку.");

        const url = file?.url?.hls4 ?? file?.url?.hls ?? file?.url?.http;
        if (!url || !/^https?:\/\//.test(url)) throw new Error("Выбранный поток KinoPub больше не доступен.");
        return { kind: "video", resolver: "direct", url, title: item.title,
            translation: variant.translation,
            audioTrack: audio ? { title: audio.author?.title || audio.type!.title!, type: audio.type?.title, language: audio.lang,
                codec: audio.codec, channels: audio.channels } : undefined,
            subtitles: video?.subtitles?.map(subtitle => ({ title: subtitle.lang, language: subtitle.lang, url: subtitle.url })) };
    },
};
