import type { ProviderContext, Variant } from "../../components/content/contracts.ts";
import { yaniProvider } from "../../components/content/providers/video/yani.ts";

interface UpstreamVideo {
    video_id: number;
    number: string;
    iframe_url: string;
    data: { player: string; dubbing: string };
}

interface UpstreamAnime {
    title: string;
    videos: UpstreamVideo[];
}

const ids = Bun.argv.slice(2);
let failed = false;

for (const id of ids.length ? ids : ["10661", "166", "651"]) {
    if (!/^\d+$/.test(id))
        throw new Error("Usage: bun tests/content-online/yani.ts [anime-id ...]");

    let upstream: UpstreamAnime | undefined;
    const context: ProviderContext = {
        dataDir: "",
        settings: {},
        signal: AbortSignal.timeout(90_000),
        async fetch(url, init) {
            const response = await fetch(url, {
                ...init,
                signal: AbortSignal.any([this.signal, AbortSignal.timeout(20_000)]),
            });
            if (response.ok && new URL(url).pathname === `/anime/${id}`)
                upstream = (await response.clone().json() as { response: UpstreamAnime }).response;
            return response;
        },
    };

    try {
        const catalog: Variant = {
            id: `yani:${id}`,
            sourceId: "yani",
            sourceName: "YummyAnime",
            title: "Live catalog check",
            url: `https://yani.tv/catalog/item/${id}`,
            transport: "video",
        };
        const details = await yaniProvider.inspect!(catalog, context);
        if (!upstream?.videos.length || details.title !== upstream.title)
            throw new Error("Live source title or video list missing.");
        if (details.variants.length !== upstream.videos.length)
            throw new Error("Source episode, voice or player variants were dropped.");

        const videos = new Map(upstream.videos.map(video => [`yani:${id}:video:${video.video_id}`, video]));
        if (videos.size !== details.variants.length || new Set(details.variants.map(variant => variant.id)).size !== videos.size)
            throw new Error("Source video IDs changed or were duplicated.");

        for (const variant of details.variants) {
            const video = videos.get(variant.id);
            if (!video || variant.episode !== video.number || !variant.player || !variant.translation)
                throw new Error("An exact source choice lost its identity or label.");
            if (/^Субтитры/i.test(video.data.dubbing) && !/^Субтитры/i.test(variant.translation))
                throw new Error("Subtitle and dubbing choices became indistinguishable.");
            if (variant.downloadUrl || variant.url !== details.url)
                throw new Error("A temporary player address escaped into the stable catalog variant.");
        }

        const voices = new Set(upstream.videos.map(video => video.data.dubbing));
        if (new Set(details.variants.map(variant => variant.translation)).size !== voices.size)
            throw new Error("Distinct upstream dubbing and subtitle choices were merged.");

        const exact = details.variants.find(variant => variant.player === "Aksor"
            && variant.translation === "AniLibria" && variant.episode === "2");
        if (!exact)
            throw new Error("The explicitly chosen Aksor / AniLibria / episode 2 is unavailable.");
        const selected = [exact];
        if (id === "10661") {
            for (const translation of ["StudioBand", "Субтитры"]) {
                const variant = details.variants.find(variant => variant.player === "Aksor"
                    && variant.translation === translation && variant.episode === "2");
                if (!variant) throw new Error(`The explicit ${translation} / episode 2 selection is unavailable.`);
                selected.push(variant);
            }
        }
        const browser = details.variants.find(variant => variant.player !== "Aksor" && variant.translation === "Субтитры");
        if (browser) selected.push(browser);

        for (const variant of selected) {
            const expected = videos.get(variant.id)!;
            const resolved = await yaniProvider.resolve!(variant, context);
            const playerUrl = new URL(expected.iframe_url, "https://yani.tv");
            if (resolved.kind !== "video" || resolved.translation !== variant.translation || resolved.title !== details.title)
                throw new Error("Resolution changed the selected source video or its voice.");
            if (variant.player === "Aksor") {
                const response = await context.fetch(`https://player.aksor.tv/api/video/${playerUrl.pathname.split("/")[2]}`,
                    { headers: { Accept: "application/json", Referer: playerUrl.href } });
                if (!response.ok) throw new Error(`Independent Aksor API returned HTTP ${response.status}.`);
                const config = await response.json() as {
                    anime_id: number; episode: string; studio_name: string; is_subtitle: boolean;
                    qualities: Record<string, string | null>;
                };
                const maximum = ["q4k", "q2k", "q1080", "q720", "q480", "q360"]
                    .map(quality => config.qualities[quality]).find(Boolean);
                if (!maximum || resolved.resolver !== "direct" || resolved.url !== new URL(maximum).href
                    || resolved.headers?.Referer !== playerUrl.href || config.anime_id !== Number(id)
                    || config.episode !== variant.episode
                    || config.is_subtitle !== /^Субтитры/.test(variant.translation!)
                    || config.studio_name !== (variant.translation === "Субтитры" ? "subtitles" : variant.translation)
                    || resolved.selection?.episode !== variant.episode || resolved.selection?.player !== "Aksor")
                    throw new Error("Direct Aksor resolution changed the exact API identity or maximum available quality.");
            }
            else if (resolved.resolver !== "browser" || resolved.url !== playerUrl.href || resolved.headers?.Referer !== details.url) {
                throw new Error("The selected browser player changed.");
            }
        }

        for (const changed of [{ episode: "9999" }, { translation: "Another studio" }, { player: "Another player" }]) {
            let rejected = false;
            try { await yaniProvider.resolve!({ ...exact, ...changed }, context); }
            catch (error) { rejected = String(error).includes("Обновите варианты"); }
            if (!rejected) throw new Error("A stale exact source choice was silently resolved.");
        }

        console.log(JSON.stringify({
            source: "yani",
            id,
            title: details.title,
            variants: details.variants.length,
            voices: voices.size,
            players: [...new Set(details.variants.map(variant => variant.player))],
            selections: selected.map(({ id, episode, translation, player }) => ({ id, episode, translation, player })),
            directAksorIdentity: true,
            maximumAvailableQuality: true,
            staleSelectionRejected: true,
            status: "passed",
        }));
    }
    catch (error) {
        failed = true;
        console.log(JSON.stringify({ source: "yani", id, status: "failed",
            error: String(error).replace(/https?:\/\/\S+/g, "[source]") }));
    }
}

if (failed)
    process.exitCode = 1;
