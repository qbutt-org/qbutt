import { createGameProvider } from "./catalog.ts";

export const gamingProviders = [
    createGameProvider({ id: "freetp", name: "FreeTP", url: "https://freetp.org",
        searchTitle: "#dle-content .heading a", content: ".fullstory" }),
    createGameProvider({ id: "byrut", name: "ByRut", url: "https://byrutgame.org", compactAcronyms: true,
        searchTitle: "#dle-content a:has(.search_res_title)", content: "article.main_content" }),
    createGameProvider({ id: "igruha", name: "Torrent-Igruha", url: "https://torrent-igruha.org",
        searchTitle: "#dle-content h2 a, #dle-content .short-title a, #dle-content .shortstory h3 a", content: ".fullstory, .full-story, .full-news" }),
    createGameProvider({ id: "onlinefix", name: "Online-Fix", url: "https://online-fix.me", compactAcronyms: true,
        searchTitle: "#dle-content h2.title", content: ".full-story" }),
    createGameProvider({ id: "fitgirl", name: "FitGirl", url: "https://fitgirl-repacks.site", wordpress: true,
        searchTitle: ".entry-title a", content: ".entry-content" }),
    createGameProvider({ id: "dodi", name: "DODI", url: "https://dodi-repacks.site", wordpress: true,
        searchTitle: ".entry-title a", content: ".entry-content" }),
];
