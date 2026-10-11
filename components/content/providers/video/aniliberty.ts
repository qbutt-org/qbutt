import type { Provider, ProviderContext, Variant, WorkDetails } from "../../contracts.ts";
import { json } from "../http.ts";
import { absoluteUrl, videoVariant } from "./common.ts";

const baseUrl = "https://anilibria.top";
const apiUrl = `${baseUrl}/api/v1`;

interface Episode {
    id: string;
    ordinal: number;
    name?: string;
    hls_480?: string;
    hls_720?: string;
    hls_1080?: string;
    rutube_id?: string;
}

interface Release {
    id: number;
    alias: string;
    name: { main: string; english?: string; alternative?: string };
    description?: string;
    year?: number;
    poster?: { src?: string };
    mal?: { id: number };
    shikimori?: { id: number };
    episodes?: Episode[];
    torrents?: {
        id: number;
        label: string;
        magnet: string;
        hash: string;
        size: number;
        seeders: number;
        quality?: { value: string };
        description?: string;
    }[];
}

const source = {
    id: "aniliberty",
    name: "AniLiberty",
    url: baseUrl,
    categories: ["anime"] as const,
    enabled: true,
};

function releaseWork(release: Release): WorkDetails {
    const url = `${baseUrl}/anime/releases/release/${release.alias}`;
    const catalog = videoVariant(anilibertyProvider.source, String(release.id), release.name.main, url);
    const variants: Variant[] = [];
    for (const episode of release.episodes ?? []) {
        for (const quality of [1080, 720, 480] as const) {
            const stream = episode[`hls_${quality}`];
            if (!stream) continue;
            const variant = videoVariant(anilibertyProvider.source,
                `${release.id}:episode:${episode.id}:${quality}`, release.name.main, url);
            variant.episode = String(episode.ordinal);
            variant.quality = `${quality}p`;
            variant.translation = "AniLiberty";
            variant.languages = ["ru"];
            const parameters = new URL(stream, baseUrl).searchParams;
            if (parameters.get("isWithVideoAds") === "1" || parameters.get("isWithVideoAdsAlways") === "1") {
                variant.adStatus = "reported";
                variant.notes = [{ text: "Источник пометил поток как содержащий видеорекламу.", url }];
            }
            variants.push(variant);
        }
        if (episode.rutube_id) {
            const variant = videoVariant(anilibertyProvider.source,
                `${release.id}:rutube:${episode.id}`, release.name.main, url);
            variant.episode = String(episode.ordinal);
            variant.translation = "AniLiberty";
            variant.player = "Rutube";
            variant.languages = ["ru"];
            variants.push(variant);
        }
    }
    for (const torrent of release.torrents ?? []) {
        variants.push({
            ...catalog,
            id: `${source.id}:${release.id}:torrent:${torrent.id}`,
            title: torrent.label,
            transport: "torrent",
            magnet: torrent.magnet,
            infoHash: torrent.hash,
            size: torrent.size,
            seeders: torrent.seeders,
            quality: torrent.quality?.value,
            episode: torrent.description,
            translation: "AniLiberty",
            languages: ["ru"],
        });
    }
    return {
        id: `${source.id}:${release.id}`,
        title: release.name.main,
        category: "anime",
        url,
        year: release.year,
        description: release.description,
        aliases: [release.name.english, release.name.alternative].filter((value): value is string => Boolean(value)),
        identifiers: {
            ...(release.mal?.id ? { mal: String(release.mal.id) } : {}),
            ...(release.shikimori?.id ? { shikimori: String(release.shikimori.id) } : {}),
        },
        cover: absoluteUrl(release.poster?.src, baseUrl),
        variants: variants.length ? variants : [{ ...catalog, needsInspection: true }],
    };
}

async function details(variant: Variant, context: ProviderContext): Promise<Release> {
    const id = variant.id.split(":")[1];
    if (!/^\d+$/.test(id ?? "")) throw new Error("Не удалось определить релиз AniLiberty.");
    return json<Release>(`${apiUrl}/anime/releases/${id}`, context);
}

export const anilibertyProvider: Provider = {
    source: { ...source, categories: [...source.categories] },
    async search(request, context) {
        if (!["all", "anime"].includes(request.category)) return [];
        const releases = await json<Release[]>(`${apiUrl}/app/search/releases?query=${encodeURIComponent(request.query)}`, context);
        return releases.map(releaseWork);
    },
    async inspect(variant, context) {
        return releaseWork(await details(variant, context));
    },
    async resolve(variant, context) {
        const release = await details(variant, context);
        const parts = variant.id.split(":");
        if (parts[2] === "torrent") {
            const torrent = release.torrents?.find(item => String(item.id) === parts[3]);
            if (!torrent) throw new Error("Раздача больше не доступна в релизе.");
            return { kind: "torrent", url: torrent.magnet, title: release.name.main };
        }
        const episode = release.episodes?.find(item => item.id === parts[3]);
        if (!episode) throw new Error("Выберите серию и качество в вариантах релиза.");
        if (parts[2] === "rutube" && episode.rutube_id) {
            return { kind: "video", resolver: "yt-dlp", url: `https://rutube.ru/video/${episode.rutube_id}/`, title: release.name.main, translation: "AniLiberty" };
        }
        const stream = episode[`hls_${parts[4]}` as keyof Episode];
        if (typeof stream !== "string" || !/^https?:\/\//.test(stream)) {
            throw new Error("Выбранное качество больше не доступно. Обновите варианты серии.");
        }
        return { kind: "video", resolver: "direct", url: stream, title: release.name.main, translation: "AniLiberty" };
    },
};
