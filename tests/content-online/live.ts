import { gamingProviders } from "../../components/content/providers/games/index.ts";
import { videoProviders } from "../../components/content/providers/video/index.ts";
import type { ProviderContext, WorkDetails } from "../../components/content/contracts.ts";

// Public, read-only integration: inspect catalog metadata, never game payloads.
const scenarios = [
    { source: "aniliberty", query: "Frieren", category: "anime" },
    { source: "yani", query: "Frieren", category: "anime" },
    { source: "rutube", query: "Blender", category: "movies" },
    { source: "hdrezka", query: "Интерстеллар", category: "all" },
    { source: "freetp", query: "Stardew", category: "games" },
    { source: "byrut", query: "Stardew", category: "games" },
    { source: "onlinefix", query: "Stardew", category: "games" },
    { source: "fitgirl", query: "Stardew", category: "games" },
] as const;

const arguments_ = process.argv.slice(2);
const selected = arguments_.filter(argument => !argument.startsWith("--"));
const query = arguments_.find(argument => argument.startsWith("--query="))?.slice("--query=".length);
const resolve = arguments_.includes("--resolve");
const providers = [...gamingProviders, ...videoProviders];
let failed = false;

for (const scenario of scenarios) {
    if (selected.length && !selected.includes(scenario.source)) continue;
    const provider = providers.find(candidate => candidate.source.id === scenario.source)!;
    const cookies = new Map<string, string>();
    const controller = new AbortController();
    const deadline = setTimeout(() => controller.abort(), 90000);
    const context: ProviderContext = {
        dataDir: "", settings: Object.fromEntries(provider.source.settings?.map(setting => [setting.name, String(setting.value ?? "")]) ?? []),
        signal: controller.signal,
        async fetch(url, init) {
            const headers = new Headers(init?.headers);
            const host = new URL(url).hostname;
            if (cookies.has(host)) headers.set("Cookie", cookies.get(host)!);
            const response = await fetch(url, { ...init, headers, signal: AbortSignal.any([controller.signal, AbortSignal.timeout(15000)]) });
            const returned = response.headers.getSetCookie();
            if (returned.length) cookies.set(host, returned.map(value => value.split(";")[0]).join("; "));
            return response;
        },
    };
    try {
        const works = await provider.search({ ...scenario, query: query ?? scenario.query }, context);
        if (!works.length) throw new Error("No catalog results");
        const initial = works[0]!;
        const details: WorkDetails = provider.inspect
            ? await provider.inspect(initial.variants[0]!, context) : initial;
        if (!details.title || !details.variants.length) throw new Error("Empty work details");
        if (new Set(details.variants.map(variant => variant.id)).size !== details.variants.length) throw new Error("Duplicate variant IDs");
        if (details.variants.some(variant => /[\ufffd]/.test(variant.title))) throw new Error("Incorrect source charset");
        if (details.variants.some(variant => /[?&](?:token|access_token|signature|expires)=/i.test(variant.url))) throw new Error("Transient token leaked into stable publication URL");
        if (scenario.source === "yani" && !query) {
            if (!initial.aliases?.some(alias => /\bFrieren\b/i.test(alias))) throw new Error("Verified English search aliases lost");
            if (details.variants.length < 10 || new Set(details.variants.map(variant => variant.translation)).size < 2) throw new Error("Translations or episodes lost");
            if (details.variants.some(variant => variant.downloadUrl)) throw new Error("Yani stored transient player URLs");
        }
        if (scenario.source === "freetp" && !query && !details.variants.some(variant => variant.publicationKind === "fix")) throw new Error("Separate multiplayer fix lost");
        if (scenario.source === "byrut" && !query) {
            if (works.length < 2) throw new Error("Ordinary/cooperative pages lost");
            if (details.variants.filter(variant => variant.transport === "torrent").length < 3) throw new Error("Lazy alternate torrents lost");
            if (!details.variants.some(variant => variant.publicationKind === "patch")) throw new Error("GOG patches lost");
        }
        if (scenario.source === "fitgirl" && !query && details.variants.filter(variant => variant.transport === "file").length < 3) {
            throw new Error("Publisher filehoster mirrors lost");
        }
        if (scenario.source === "onlinefix" && details.variants.some(variant => /Online-Fix (?:Hosters|Drive)/i.test(variant.title)
            && variant.publicationKind === "fix")) throw new Error("Source brand misclassified as a separate fix");
        const resolution = resolve ? await provider.resolve!(details.variants[0]!, context) : undefined;
        if (resolve && (!resolution?.url || !resolution.kind)) throw new Error("Empty resolution");
        if (scenario.source === "fitgirl" && resolve && /DataNodes/i.test(details.variants[0]!.title)
            && resolution?.resolver !== "browser") throw new Error("Filehoster HTML misrepresented as a direct archive");
        console.log(JSON.stringify({ source: scenario.source, query: query ?? scenario.query, works: works.length,
            title: details.title, variants: details.variants.length,
            translations: [...new Set(details.variants.map(variant => variant.translation).filter(Boolean))],
            authors: [...new Set(details.variants.map(variant => variant.author).filter(Boolean))],
            kinds: [...new Set(details.variants.map(variant => variant.publicationKind).filter(Boolean))],
            knownSizes: details.variants.filter(variant => variant.size !== undefined).length,
            resolution: resolution ? `${resolution.kind}/${resolution.resolver ?? "direct"}` : undefined,
            status: "passed" }));
    }
    catch (error) {
        failed = true;
        console.log(JSON.stringify({ source: provider.source.id, query: query ?? scenario.query, status: "failed",
            error: error instanceof Error ? error.message.replace(/https?:\/\/\S+/g, "[source]") : "Unknown source error" }));
    }
    finally {
        clearTimeout(deadline);
    }
}

// Sources requiring accounts must fail clearly with no account, not fabricate content.
for (const id of ["kinopub", "vkvideo"]) {
    if (selected.length && !selected.includes(id)) continue;
    const provider = providers.find(candidate => candidate.source.id === id)!;
    try {
        await provider.search({ query: "Sintel", category: "movies" }, {
            signal: new AbortController().signal, dataDir: "", settings: {},
            fetch: () => { throw new Error("Credential-free request escaped the auth boundary"); },
        });
        failed = true;
        console.log(JSON.stringify({ source: id, status: "failed", error: "Missing authentication not reported" }));
    }
    catch (error) {
        const message = error instanceof Error ? error.message : "";
        const passed = /аккаунт|токен/i.test(message);
        failed ||= !passed;
        console.log(JSON.stringify({ source: id, status: passed ? "auth-required" : "failed" }));
    }
}

if (failed) process.exitCode = 1;
