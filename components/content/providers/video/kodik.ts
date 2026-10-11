import type { ProviderContext, ResolvedContent } from "../../contracts.ts";
import { defaultUserAgent, publicUrl } from "../../net.ts";
import { html, json } from "../http.ts";

interface VideoSources {
    links?: Record<string, { src: string; type: string }[]>;
}

function playerString(script: string, name: string): string {
    const literal = script.match(new RegExp(`(?:^|\\n)\\s*var ${name} = ("(?:[^"\\\\]|\\\\.)*");`))?.[1];
    if (!literal) throw new Error("Kodik изменил данные выбранного плеера. Откройте источник и повторите выбор.");
    return JSON.parse(literal);
}

export async function resolveKodik(playerUrl: string,
    selected: { title: string; episode: string; translation: string; publicationUrl: string },
    context: ProviderContext): Promise<ResolvedContent> {
    const player = new URL(playerUrl);
    const route = /^\/(season|serial|seria|video)\/(\d+)\/([a-f\d]{32})\/720p$/.exec(player.pathname);
    if (player.origin !== "https://kodikplayer.com" || player.username || player.password || !route
        || !/^\d+$/.test(selected.episode)) {
        throw new Error("Источник изменил адрес выбранной серии Kodik. Обновите варианты тайтла.");
    }
    const playlist = route[1] === "season" || route[1] === "serial";
    if (((playlist || player.searchParams.has("episode"))
            && player.searchParams.get("episode") !== selected.episode)
        || (route[1] === "video" && selected.episode !== "1")) {
        throw new Error("Источник изменил выбранную серию Kodik. Обновите варианты тайтла.");
    }

    // The ordinary season page exposes the selected episode's own ID and hash.
    // Keeping its selector lets us verify the episode instead of trusting the
    // first video in a series or another translation supplied as a substitute.
    player.searchParams.delete("only_episode");
    player.searchParams.delete("only_season");
    const headers = { Referer: player.href, "User-Agent": defaultUserAgent };
    const page = await html(player.href, context,
        { headers: { ...headers, Referer: selected.publicationUrl } });
    const script = page("script:not([src])").map((_index, element) => page(element).text()).get().join("\n");
    const normalize = (value: string) => value.normalize("NFC").replace(/\s+/g, " ").trim().toLowerCase();
    const translation = playerString(script, "translationTitle").replace(/^(.*)\.Subtitles$/i, "Субтитры $1");
    if (normalize(translation) !== normalize(selected.translation)) {
        throw new Error("Kodik вернул другую озвучку. Обновите варианты тайтла.");
    }

    const video = Object.fromEntries([...script.matchAll(/\bvInfo\.(type|hash|id)\s*=\s*'([a-z\d]+)';/g)]
        .map(match => [match[1], match[2]]));
    if (video.type !== (route[1] === "video" ? "video" : "seria")
        || !/^\d+$/.test(video.id ?? "") || !/^[a-f\d]{32}$/.test(video.hash ?? "")) {
        throw new Error("Kodik не предоставил выбранное видео.");
    }
    if (playlist) {
        const episodes = page(".serial-series-box select option")
            .filter((_index, element) => page(element).attr("value") === selected.episode);
        if (episodes.length !== 1 || episodes.attr("selected") === undefined
            || episodes.attr("data-other-translation") !== "false"
            || episodes.attr("data-id") !== video.id || episodes.attr("data-hash") !== video.hash) {
            throw new Error("Kodik не подтвердил выбранную серию и озвучку. Обновите варианты тайтла.");
        }
    } else if (video.id !== route[2] || video.hash !== route[3]) {
        throw new Error("Kodik вернул другое видео.");
    }

    const form = new URLSearchParams();
    for (const name of ["domain", "d_sign", "pd", "pd_sign", "ref", "ref_sign"]) {
        form.set(name === "domain" ? "d" : name, playerString(script, name));
    }
    form.set("type", video.type);
    form.set("id", video.id);
    form.set("hash", video.hash);
    form.set("info", "{}");
    form.set("cdn_is_working", "false");
    const sources = await json<VideoSources>("https://kodikplayer.com/ftor", context, {
        method: "POST",
        body: form,
        headers: { ...headers, "X-Requested-With": "XMLHttpRequest" },
    });
    const quality = Object.keys(sources.links ?? {}).filter(value => /^\d{3,4}$/.test(value))
        .sort((left, right) => Number(right) - Number(left))[0];
    const choices = sources.links?.[quality ?? ""];
    if (choices?.length !== 1 || typeof choices[0].src !== "string"
        || !/mpegurl/i.test(choices[0].type)) {
        throw new Error("Kodik не предоставил однозначный поток максимального качества выбранной серии.");
    }

    // This is the public player's URL encoding, not encrypted media. Its own
    // shipped script rotates letters by 18, then decodes the Base64 address.
    const encoded = choices[0].src;
    const address = encoded.includes("//") ? encoded : atob(encoded.replace(/[a-zA-Z]/g, letter => {
        const shifted = letter.charCodeAt(0) + 18;
        return String.fromCharCode(shifted <= (letter <= "Z" ? 90 : 122) ? shifted : shifted - 26);
    }));
    const advertised = new URL(address, player);
    if (advertised.protocol !== "https:") {
        throw new Error("Kodik изменил формат адреса выбранного видео.");
    }

    // The advertised CDN address redirects to the file's serving node. Resolve
    // that ordinary public redirect with the guarded source client so native
    // players receive the real manifest address and its relative segments.
    const response = await context.fetch(advertised.href, { headers });
    const manifest = await response.text();
    if (!response.ok || !manifest.startsWith("#EXTM3U")
        || !/^#EXT(?:INF:|-X-STREAM-INF:)/m.test(manifest)) {
        throw new Error(`Kodik не предоставил плейлист выбранного видео (HTTP ${response.status}).`);
    }
    const media = await publicUrl(response.url);
    if (media.protocol !== "https:") {
        throw new Error("Kodik изменил формат адреса выбранного видео.");
    }

    return {
        kind: "video",
        resolver: "direct",
        url: media.href,
        headers,
        title: selected.title,
        translation: selected.translation,
        selection: { episode: selected.episode, player: "Kodik" },
    };
}
