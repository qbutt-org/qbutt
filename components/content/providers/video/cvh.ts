import type { ProviderContext, ResolvedContent } from "../../contracts.ts";
import { defaultUserAgent, publicUrl } from "../../net.ts";
import { json } from "./common.ts";

interface Playlist {
    isSerial: boolean;
    items?: {
        vkId: string;
        episode?: number;
        voiceStudio: string;
        voiceType: string;
    }[];
}

interface Video {
    unitedVideoId: number;
    sources?: Record<string, string>;
}

export async function resolveCvh(playerUrl: string,
    selected: { title: string; malId?: number; episode: string; translation: string },
    context: ProviderContext): Promise<ResolvedContent> {
    const player = new URL(playerUrl);
    const titleId = player.searchParams.get("anime_id");
    const episode = player.searchParams.get("episode");
    const voice = player.searchParams.get("dubbing_code");
    if (player.origin !== "https://ru.yummyani.me"
        || player.pathname !== "/iframeCVH.html" || player.username || player.password
        || !titleId || !/^\d+$/.test(titleId) || Number(titleId) !== selected.malId
        || episode !== selected.episode || !/^\d+$/.test(episode)
        || !voice || !/^[a-z\d_-]+$/i.test(voice)) {
        throw new Error("Источник изменил адрес выбранной серии CVH. Обновите варианты тайтла.");
    }

    const apiUrl = "https://plapi.cdnvideohub.com/api/v1/player/sv";
    const headers = { Referer: player.href, "User-Agent": defaultUserAgent };
    // The public YummyAnime player identifies its publisher and title catalog
    // with these values; voice and episode always come from the selected item.
    const query = new URLSearchParams({ pub: "745", aggr: "mali", id: titleId, vstu: voice, ep: episode });
    const playlist = await json<Playlist>(`${apiUrl}/playlist?${query}`, context, { headers });
    const normalize = (value: string) => value.normalize("NFC").replace(/\s+/g, " ").trim().toLowerCase();
    const choices = (playlist.items ?? []).filter(item => {
        const matchesEpisode = playlist.isSerial === true ? String(item.episode) === episode
            : playlist.isSerial === false && episode === "1" && item.episode === undefined;
        if (!matchesEpisode || typeof item.voiceStudio !== "string") return false;
        const prefix = ["Дубляж", "Субтитры"].includes(item.voiceType) ? `${item.voiceType} ` : "";
        return normalize(prefix + item.voiceStudio) === normalize(selected.translation);
    });
    if (choices.length !== 1 || !/^\d+$/.test(choices[0].vkId)) {
        throw new Error("CVH не подтвердил выбранную серию и озвучку. Обновите варианты тайтла.");
    }

    const video = await json<Video>(`${apiUrl}/video/${choices[0].vkId}`, context, { headers });
    if (String(video.unitedVideoId) !== choices[0].vkId) {
        throw new Error("CVH вернул другое видео выбранной серии.");
    }
    const address = ["mpeg4kUrl", "mpeg2kUrl", "mpegQhdUrl", "mpegFullHdUrl", "mpegHighUrl",
        "mpegMediumUrl", "mpegLowUrl", "mpegLowestUrl", "mpegTinyUrl"]
        .map(quality => video.sources?.[quality]).find(value => typeof value === "string" && value.length > 0);
    if (!address) throw new Error("CVH не предоставил видео выбранной серии и озвучки.");
    const media = await publicUrl(address);
    if (media.protocol !== "https:") throw new Error("CVH вернул неподдерживаемый адрес видео.");

    return {
        kind: "video",
        resolver: "direct",
        url: media.href,
        headers,
        title: selected.title,
        translation: selected.translation,
        selection: { episode, player: "CVH" },
    };
}
