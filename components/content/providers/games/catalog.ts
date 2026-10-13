import { createHash } from "node:crypto";
import { setTimeout } from "node:timers/promises";
import type { CheerioAPI } from "cheerio";
import type { Provider, ProviderContext, Source, Variant, Work, WorkDetails } from "../../contracts.ts";
import { assertExactVariant } from "../../references.ts";
import { html } from "../http.ts";
import { expandFitgirlMirrors } from "./fitgirl.ts";

interface Catalog {
    id: string;
    name: string;
    url: string;
    searchTitle: string;
    content: string;
    wordpress?: boolean;
    compactAcronyms?: boolean;
}

const downloadHosts = [
    "1fichier.com", "gofile.io", "pixeldrain.com", "buzzheavier.com", "files.fm", "qiwi.gg",
    "workupload.com", "datanodes.to", "fuckingfast.co", "megaup.net", "multiup.io", "multiup.org",
    "drive.google.com", "disk.yandex.ru", "disk.yandex.com", "mega.nz", "vikingf1le.us",
    "fileditchstuff.me", "online-fix.me", "freetp.org",
];

function urlAt(value: string | undefined, base: string): string | undefined {
    if (!value) return undefined;
    try {
        const url = new URL(value, base);
        return /^(?:https?|magnet):$/.test(url.protocol) ? url.href : undefined;
    }
    catch {
        return undefined;
    }
}

async function page(url: string, context: ProviderContext, init?: RequestInit): Promise<CheerioAPI> {
    const sameOrigin = context.settings.baseUrl
        && new URL(context.settings.baseUrl).origin === new URL(url).origin;
    return html(url, context, {
        ...init,
        headers: { ...(sameOrigin && context.settings.cookie ? { Cookie: context.settings.cookie } : {}), ...init?.headers },
    });
}

function key(value: string): string {
    return createHash("sha256").update(value).digest("hex").slice(0, 20);
}

function cleanText(value: string): string {
    return value.replace(/\u00a0/g, " ").replace(/[\t ]+/g, " ").replace(/\n\s*\n\s*\n/g, "\n\n").trim();
}

function version(value: string): string | undefined {
    return value.match(/(?:\bv\s*|(?:Game\s+)?Version\s*:?\s*|верси[яи]\s*:?\s*)((?:Build|билд)\s*\d+|\d+(?:[.\-_]\d+)*(?:[.\-_]?[a-z]\d*(?![a-z]))?(?:\s*(?:Build|билд)\s*\d+)?)/i)?.[1]
        ?? value.match(/\b(?:stable|beta|alpha)[.\-_]\d+(?:[.\-_]\d+)*(?:[.\-_][a-z])?\b/i)?.[0]
        ?? value.match(/\b(?:Build|билд)\s*\d+\b/i)?.[0];
}

function author(value: string): string | undefined {
    const found = value.match(/(?:RePack\s+(?:от|by)|репак\s+от|Steam[- ]Rip\s+от|\bby)\s+([\p{L}\p{N}._-]+)/iu)?.[1]
        ?? value.match(/\b(FitGirl|Pioneer|SELEZEN|Online-Fix|DODI|Wanterlude)\b/i)?.[1];
    return found?.replace(/\.(?:torrent|zip|rar|7z)$/i, "");
}

function bytes(value: string): number | undefined {
    const match = value.match(/(?:^|[\s[(])(\d+(?:[.,]\d+)?)\s*(ТБ|ГБ|МБ|КБ|TB|GB|MB|KB)(?=$|[^\p{L}\p{N}])/iu);
    if (!match) return undefined;
    const unit = match[2]!.toUpperCase();
    const power = /^[TТ]/.test(unit) ? 4 : /^[GГ]/.test(unit) ? 3 : /^[MМ]/.test(unit) ? 2 : 1;
    return Math.round(Number(match[1]!.replace(",", ".")) * 1024 ** power);
}

function titleOf(value: string): string {
    return cleanText(value)
        .replace(/\s*\((?:Update\s+)?v\d.*$/i, "")
        .replace(/\s*\[DODI Repack\]\s*$/i, "")
        .replace(/\s*\+\s*Online (?:Multiplayer|Co-op)\s*$/i, "")
        .replace(/\s+(?:играть\s+)?по\s+сети(?:\s+и\s+интернету)?(?:\s+онлайн)?(?:\s*\/\s*ЛАН)?\s*$/i, "")
        .replace(/\s*[–—]\s*(?:[^–—]*\bEdition\b|v\d|Build\s*\d).*$/i, "")
        .replace(/:\s*(?:Digital\s+)?(?:Deluxe|Ultimate|Gold|Complete|Collector.?s|Premium|Soundtrack)\s+Edition\b.*$/i, "");
}

function kind(label: string): Variant["publicationKind"] {
    if (/Online[- ]Fix (?:Hosters|Drive)/i.test(label)) return undefined;
    label = label.replace(/\bOnline[- ]Fix\b/gi, "");
    if (/\b(?:fix|crack)\b|фикс/i.test(label) && !/\+\s*(?:LAN|Steam)?\s*Fix/i.test(label)) return "fix";
    if (/patch|update|патч|обновление/i.test(label)) return "patch";
    if (/optional|selective|bonus|goodie|soundtrack|\bmods\b|бонус|дополнител|саундтрек|\bмоды\b/i.test(label)) return "optional";
    if (/\.part\d+\.|часть\s*\d+|part\s*\d+/i.test(label)) return "archivePart";
    return "full";
}

function directTorrent(url: string, label: string): boolean {
    return url.startsWith("magnet:") || /\.torrent(?:[?#]|$)|[?&]do=download(?:&|$)/i.test(url)
        || (/\/getfile-\d+/.test(url) && /\.torrent\b/i.test(label));
}

function unwrapLink(value: string): string {
    const url = new URL(value);
    if (url.pathname === "/engine/go.php" && url.searchParams.has("url")) {
        const decoded = Buffer.from(url.searchParams.get("url")!, "base64").toString("utf8");
        return urlAt(decoded, value) ?? value;
    }
    return value;
}

// Extract literal JSON assignments without evaluating any source-owned script.
function jsonAssignment<T>(script: string, name: string): T | undefined {
    const assignment = new RegExp(`\\b(?:const|let|var)\\s+${name}\\s*=\\s*`).exec(script);
    if (!assignment) return undefined;
    const start = assignment.index + assignment[0].length;
    if (!"[{".includes(script[start] ?? "")) return undefined;
    let depth = 0;
    let quoted = false;
    let escaped = false;
    for (let index = start; index < script.length; ++index) {
        const character = script[index];
        if (quoted) {
            if (escaped) escaped = false;
            else if (character === "\\") escaped = true;
            else if (character === '"') quoted = false;
            continue;
        }
        if (character === '"') quoted = true;
        else if ("[{".includes(character!)) ++depth;
        else if ("]}".includes(character!) && --depth === 0) {
            try { return JSON.parse(script.slice(start, index + 1)) as T; }
            catch { return undefined; }
        }
    }
    return undefined;
}

async function expandByrut($: CheerioAPI, publication: string, context: ProviderContext): Promise<void> {
    const id = $("article[data-news-id]").attr("data-news-id");
    if (!id) return;
    for (const element of $("#other_repacks [data-ajax]").toArray()) {
        const block = $(element).attr("data-ajax")!;
        try {
            const url = new URL("/engine/ajax/controller.php?mod=ajaxsp", publication).href;
            const request = (session: ProviderContext) => page(url, session, {
                method: "POST", body: new URLSearchParams({ block, id }),
                headers: { "Content-Type": "application/x-www-form-urlencoded", "X-Requested-With": "XMLHttpRequest", Referer: publication },
            });
            const response = context.withSourceSession ? await context.withSourceSession(url, request) : await request(context);
            $(element).find(".ajax-content").html(response.html());
        }
        catch (error) {
            context.signal.throwIfAborted();
            context.reportError?.(error);
        }
    }
}

async function expandOnlinefix($: CheerioAPI, publication: string, context: ProviderContext): Promise<void> {
    for (const element of $('.full-story a[href*="uploads.online-fix.me:2053/"]').toArray()) {
        const link = $(element);
        const directory = urlAt(link.attr("href"), publication);
        if (!directory || !directory.endsWith("/")) continue;
        let found = false;
        const pending = [directory];
        const visited = new Set<string>();
        while (pending.length) {
            const current = pending.shift()!;
            if (visited.has(current)) continue;
            visited.add(current);
            if (visited.size > 40) throw new Error("Каталог файлов слишком велик. Откройте выбранную публикацию.");
            const listing = await page(current, context, { headers: { Referer: publication } });
            for (const file of listing('a[href]').toArray()) {
                const name = listing(file).text().trim();
                const url = urlAt(listing(file).attr("href"), current);
                if (!url || !url.startsWith(directory) || url === current) continue;
                if (url.endsWith("/")) {
                    pending.push(url);
                    continue;
                }
                if (!/\.(?:torrent|zip|rar|7z)$/i.test(name)) continue;
                const entry = $("<p></p>");
                const prefix = /Fix%20Repair|\/fix\//i.test(current) ? "Фикс: " : "";
                entry.append($("<a></a>").attr("href", url).text(`${prefix}${name}`));
                link.parent().after(entry);
                found = true;
            }
        }
        if (found) link.remove();
    }
}

function releaseVariants($: CheerioAPI, source: Source, publication: string, title: string, selector: string): Variant[] {
    const body = $(selector).first();
    const variants = new Map<string, Variant>();
    const overallVersion = source.id === "byrut" ? version(body.find(".main-release-block .info-basictor__item--version").first().text())
        : version($(".subhnamever, .subver_info, h1.entry-title").first().text());
    const cooperative = /по сети|совместное прохождение|кооператив|multiplayer|co-op/i.test(title);
    const primaryAuthor = source.id === "fitgirl" ? "FitGirl" : source.id === "onlinefix" ? "Online-Fix"
        : body.find('a[href*="/user/"]').first().text().trim() || undefined;

    function add(url: string, label: string, sectionText: string, options: {
        publicationKind?: Variant["publicationKind"];
        transport?: Variant["transport"];
        size?: number;
    } = {}): void {
        const torrent = directTorrent(url, label) || /\.torrent file only|^(?:1337x|RuTor|Tapochek)/i.test(label);
        const releaseVersion = version(label) ?? version(sectionText) ?? overallVersion;
        const edition = [label, sectionText, title].join("\n")
            .match(/\b(?:Digital\s+)?(?:Deluxe|Ultimate|Gold|Complete|Collector.?s|Premium|Soundtrack)\s+Edition\b/i)?.[0];
        const publicationKind = options.publicationKind ?? kind(label);
        const releaseCooperative = cooperative || /возможность сетевой игры|игра по сети|multiplayer|co-op/i.test(sectionText) || undefined;
        const id = `${source.id}:${key(publication)}:link:${key(JSON.stringify([url, releaseVersion, edition, publicationKind, releaseCooperative]))}`;
        if (variants.has(id)) return;
        const variant: Variant = {
            id, sourceId: source.id, sourceName: source.name, title: cleanText(label) || title,
            url: publication, transport: options.transport ?? (torrent ? "torrent" : "file"),
            publicationKind,
            version: releaseVersion,
            edition,
            author: author(label) ?? author(sectionText) ?? primaryAuthor,
            cooperative: releaseCooperative,
            size: options.size,
            notes: sectionText ? [{ text: cleanText(sectionText), url: publication, author: source.name }] : [],
        };
        if (url.startsWith("magnet:")) {
            variant.magnet = url;
            variant.infoHash = url.match(/xt=urn:btih:([a-z0-9]+)/i)?.[1]?.toLowerCase();
        }
        else variant.downloadUrl = url;
        variants.set(id, variant);
    }

    body.find("a[href]").each((_, element) => {
        const link = $(element);
        const raw = urlAt(link.attr("href"), publication);
        if (!raw) return;
        const url = raw.startsWith("magnet:") ? raw : unwrapLink(raw);
        const label = cleanText(link.text() || link.attr("download") || "");
        const hostname = url.startsWith("magnet:") ? "" : new URL(url).hostname;
        const isHost = downloadHosts.some(host => hostname === host || hostname.endsWith(`.${host}`))
            && /download|скачать|torrent|\.zip|\.rar|\/[ufd]\//i.test(label + " " + url);
        const mirrorHeading = source.id === "fitgirl"
            ? link.parents("ul").last().prevAll("h3").first().text()
            : "";
        const publisherMirror = link.attr("data-publisher-mirror") === "true"
            || source.id === "fitgirl" && /Download Mirrors \((?:Direct Links|Torrent)\)/i.test(mirrorHeading)
                && /^(?:Filehoster:|1337x|RuTor|Tapochek)|\.torrent file only/i.test(label);
        if (!directTorrent(url, label) && !/\/getfile-\d+/.test(url) && !isHost && !publisherMirror) return;
        if (/\/user\/|\/guides\/|\/programs\/|\/files\/progs\//.test(url)) return;
        const release = link.closest(".ajax-block");
        const container = release.length ? release : link.closest(".main-release-block, li, .quote, p, .download-block");
        const before = container.prev("strong, p, h3, h4").text();
        const section = cleanText([release.find(".showtitle").text(), release.find(".show_ver").text(),
            mirrorHeading, before, container.text()].join("\n"));
        add(url, [release.find(".showtitle").text(), label].filter(Boolean).join(", "), section, {
            publicationKind: link.attr("data-publication-kind") as Variant["publicationKind"],
            transport: link.attr("data-release-transport") as Variant["transport"],
            size: bytes(section),
        });
    });

    body.find("script").each((_, element) => {
        const script = $(element).text();
        const links = jsonAssignment<Record<string, Record<string, string[]>>>(script, "linksData");
        if (!links) return;
        const files = jsonAssignment<{ type: string; name: string; size?: string }[]>(script, "filesData") ?? [];
        const section = $(element).closest(".ajax-block");
        for (const [group, mirrors] of Object.entries(links)) {
            const groupFiles = files.filter(file => file.type === group);
            for (const [host, urls] of Object.entries(mirrors)) {
                for (const value of urls) {
                    const url = urlAt(value, publication);
                    if (!url) continue;
                    const label = [section.find(".showtitle").text(), group, host].filter(Boolean).join(", ");
                    const notes = [section.find(".show_ver").text(), ...groupFiles.map(file => `${file.name}${file.size ? ` (${file.size})` : ""}`)].join("\n");
                    add(url, label, notes, {
                        publicationKind: group === "patch" ? "patch" : group === "goodie" ? "optional" : "full",
                        size: groupFiles.length === 1 ? bytes(groupFiles[0]!.size ?? "") : undefined,
                    });
                }
            }
        }
    });
    return [...variants.values()];
}

export function createGameProvider(catalog: Catalog): Provider {
    const source: Source = {
        id: catalog.id, name: catalog.name, url: catalog.url, categories: ["games"], enabled: true,
        settings: [
            { name: "baseUrl", label: "Адрес сайта", type: "text", value: catalog.url },
            { name: "cookie", label: "Сессия аккаунта", type: "password" },
        ],
    };
    const provider: Provider = {
        source,
        async search(request, context) {
            if (!["all", "games"].includes(request.category)) return [];
            const base = context.settings.baseUrl || source.url;
            const search = new URL(catalog.wordpress ? "/" : "/index.php", base);
            const query = catalog.compactAcronyms
                ? request.query.replace(/\b(?:[a-z]\.){2,}[a-z]?\.?/gi, abbreviation => abbreviation.replace(/\./g, ""))
                : request.query;
            search.search = catalog.wordpress ? new URLSearchParams({ s: query }).toString()
                : new URLSearchParams({ do: "search", subaction: "search", story: query }).toString();
            let $ = await page(search.href, context);
            const searchCooldown = /Вы сможете воспользоваться поиском через (\d+) секунд/i;
            const cooldown = $.text().match(searchCooldown);
            if (cooldown && Number(cooldown[1]) <= 60) {
                await setTimeout(Number(cooldown[1]) * 1000, undefined, { signal: context.signal });
                $ = await page(search.href, context);
            }
            if (searchCooldown.test($.text()))
                throw new Error("Источник временно ограничил поиск. Повторите его чуть позже.");
            const works = new Map<string, Work>();
            $(catalog.searchTitle).each((_, element) => {
                const item = $(element);
                const link = item.is("a") ? item : item.find("a").first().length ? item.find("a").first() : item.closest("a");
                const url = urlAt(link.attr("href"), base);
                const title = cleanText(item.find(".search_res_title").text() || item.text());
                if (!url || !title || works.has(url)) return;
                const id = `${source.id}:${key(url)}`;
                const raw = cleanText(item.text());
                const variant: Variant = { id, sourceId: source.id, sourceName: source.name,
                    title, url, transport: "torrent", version: version(raw), needsInspection: true,
                    cooperative: /по сети|multiplayer|co-op/i.test(raw) || undefined };
                works.set(url, { id, title: titleOf(title), category: "games", url,
                    cover: urlAt(item.find("img").attr("src") ?? item.closest("article,.shortstory,.base").find("img").first().attr("src"), base),
                    variants: [variant] });
            });
            return [...works.values()];
        },
        async inspect(variant, context) {
            const $ = await page(variant.url, context);
            if (source.id === "byrut") await expandByrut($, variant.url, context);
            if (source.id === "onlinefix") await expandOnlinefix($, variant.url, context);
            if (source.id === "fitgirl") await expandFitgirlMirrors($, context);
            const body = $(catalog.content).first();
            if (!body.length) throw new Error("Структура страницы источника изменилась. Откройте исходную публикацию.");
            const title = cleanText($("h1.entry-title").first().text() || body.find("h1").first().text() || $("h1").last().text());
            const variants = releaseVariants($, source, variant.url, title, catalog.content);
            if (!variants.length)
                throw new Error("Не удалось найти ссылки на версии игры. Откройте публикацию источника и повторите попытку.");
            const description = (source.id === "onlinefix" ? body.find('[itemprop="articleBody"]').first() : body).clone();
            description.find("script, style, noindex, .comments, [id^='comment-id-'], .navigation").remove();
            if (source.id === "onlinefix")
                description.find("br").replaceWith("\n");
            const notes: NonNullable<WorkDetails["notes"]> = [];
            for (const element of $("[id^='comm-id-'], .comment-body .comment-content").toArray()) {
                const comment = $(element);
                const container = comment.closest("[id^='comment-id-'],.comment-body");
                const text = cleanText(comment.text());
                if (!text) continue;
                const authorName = container.find(".comm-author,.avatarbox b,.group.author,.comment-author .fn").first().text().trim();
                const id = container.attr("id") ?? comment.attr("id");
                notes.push({ text, url: `${variant.url.split("#")[0]}${id ? `#${id}` : "#comments"}`,
                    author: authorName || "Комментарий пользователя" });
            }
            for (const element of body.find('a[href*="fitgirl-repacks.site"],a[href*="online-fix.me/games/"],a[href*="/user/"]').toArray()) {
                const link = $(element);
                const url = urlAt(link.attr("href"), variant.url);
                const text = cleanText(link.text());
                if (url && text) notes.push({ text, url, author: source.name });
            }
            const details: WorkDetails = {
                id: `${source.id}:${key(variant.url)}`, title: titleOf(title), category: "games", url: variant.url,
                cover: urlAt($('meta[property="og:image"]').attr("content") ?? body.find("img").first().attr("src"), variant.url),
                description: cleanText(description.text()),
                notes,
                identifiers: $("article[data-appid]").attr("data-appid") ? { steam: $("article[data-appid]").attr("data-appid")! } : undefined,
                variants,
            };
            return details;
        },
        async resolve(variant, context) {
            if (!variant.id.includes(":link:")) throw new Error("Выберите сборку или отдельный файл в вариантах игры.");
            const details = await provider.inspect!(variant, context);
            const current = details.variants.find(candidate => candidate.id === variant.id);
            const url = current?.magnet ?? current?.downloadUrl;
            if (!current || !url) throw new Error("Выбранная ссылка изменилась. Обновите варианты игры.");
            assertExactVariant(variant, current);
            if (current.magnet) return { kind: "torrent", url, title: current.title };
            if (/\/getfile-\d+/.test(url)) {
                await page(url, context, { headers: { Referer: variant.url } });
                const id = new URL(url).pathname.match(/getfile-(\d+)/)![1]!;
                return { kind: current.transport, resolver: "direct",
                    url: new URL(`/engine/download.php?id=${id}&area=`, url).href,
                    headers: { Referer: url }, title: current.title };
            }
            if (directTorrent(url, current.title)) {
                return { kind: current.transport, resolver: "direct", url, headers: { Referer: variant.url }, title: current.title };
            }
            if (/\.(?:zip|rar|7z|exe|bin)(?:[?#]|$)/i.test(url)) {
                const response = await context.fetch(url, { method: "HEAD", headers: { Referer: variant.url } });
                const mime = response.headers.get("content-type")?.split(";")[0]?.trim() ?? "";
                if (response.ok && /^(?:application\/(?:octet-stream|zip|x-7z-compressed|x-rar-compressed|vnd\.rar|x-msdownload|force-download))$/i.test(mime)) {
                    return { kind: current.transport, resolver: "direct", url, headers: { Referer: variant.url }, title: current.title };
                }
            }
            return { kind: current.transport, resolver: "browser", url, headers: { Referer: variant.url }, title: current.title };
        },
    };
    return provider;
}
