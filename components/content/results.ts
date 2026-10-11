import type { Variant, Work } from "./contracts";
import { normalizeTitle as identity, queryMatches, queryYearMatches, searchQuery, type SearchQuery } from "./search-query";

function magnetHash(variant: Variant): string | undefined {
    if (/^[a-f\d]{40}$/i.test(variant.infoHash || ""))
        return `btih:${variant.infoHash!.toLowerCase()}`;
    if (/^[a-f\d]{64}$/i.test(variant.infoHash || ""))
        return `btmh:1220${variant.infoHash!.toLowerCase()}`;

    try {
        const hash = new URL(variant.magnet || "").searchParams.getAll("xt")
            .find(value => /^urn:bt(?:ih|mh):/i.test(value));
        if (!hash)
            return undefined;
        const value = hash.replace(/^urn:/i, "").toLowerCase();
        if (/^btih:[a-f\d]{40}$/.test(value) || /^btmh:1220[a-f\d]{64}$/.test(value))
            return value;
        if (/^btih:[a-z2-7]{32}$/.test(value)) {
            const alphabet = "abcdefghijklmnopqrstuvwxyz234567";
            let bits = 0;
            let accumulator = 0;
            const bytes: number[] = [];
            for (const letter of value.slice(5)) {
                accumulator = (accumulator << 5) | alphabet.indexOf(letter);
                bits += 5;
                if (bits >= 8) {
                    bits -= 8;
                    bytes.push((accumulator >>> bits) & 255);
                }
            }
            return `btih:${Buffer.from(bytes).toString("hex")}`;
        }
        return undefined;
    }
    catch {
        return undefined;
    }
}

function variantKey(variant: Variant): string {
    return magnetHash(variant) || `${variant.sourceId}:${variant.id}`;
}

function isUserUpload(work: Work): boolean {
    return work.variants.every(variant => variant.publicationKind === "upload");
}

function matchesSeason(titles: string[], season: string | undefined): boolean {
    if (!season) return true;
    const counters = titles.flatMap(title => {
        const counter = /(?:season (\d+)|(\d+)(?:st|nd|rd|th)? season|(\d+) сезон| (\d+)$)/.exec(title);
        return counter ? [counter.slice(1).find(Boolean)!] : [];
    });
    return counters.includes(season) || season === "1" && !counters.length;
}

function animeTorrentEpisode(variant: Variant, work: Work): Pick<Variant, "episode" | "season"> | undefined {
    const year = /[([]((?:19|20)\d{2})[)\]]/.exec(variant.title)?.[1];
    if (year && work.year && Number(year) !== work.year) return undefined;
    if (/\b(?:batch|complete)\b|\b\d{1,3}(?:\s*[-~–—]\s*\d{1,3}|\.\d{1,2})\b|\bS\d{1,2}E\d{1,3}\s*[-~–—]\s*(?:S\d{1,2})?E?\d{1,3}\b/i.test(variant.title))
        return undefined;
    const title = variant.title.replace(/^\[[^\]]+\]\s*/, "");
    const name = ` ${identity(title)} `;
    const alternatives = title.split(/[()[\]{},|\/]/).map(identity);
    const titles = [work.title, ...work.aliases || []].map(identity);
    const explicit = /\bS(\d{1,2})E(\d{1,3})(?:v\d+)?\b/i.exec(title);
    const season = explicit ? String(Number(explicit[1])) : undefined;
    if (!matchesSeason(titles, season)) return undefined;
    const aliases = (explicit
        ? titles.map(title => title.replace(/(?: season \d+| \d+(?:st|nd|rd|th)? season| \d+ сезон| \d+)$/, ""))
        : titles)
        .filter(alias => alias.length >= 5);
    const matched = aliases.filter(alias => {
        const offset = name.indexOf(` ${alias} `);
        const alternate = alternatives.some(label => label === alias || label.startsWith(alias + " ")
            && /^(?:multi subs?|english (?:sub|audio))(?: |$)/.test(label.slice(alias.length + 1)));
        return offset !== -1 && (alternate
            || !/[\p{Script=Latin}\p{Script=Cyrillic}]/u.test(name.slice(0, offset)));
    });
    if (!matched.length) return undefined;

    if (explicit) return { episode: String(Number(explicit[2])), season };

    for (const alias of matched) {
        const suffix = name.slice(name.indexOf(` ${alias} `) + alias.length + 1);
        // Numeric episode suffixes can follow a delimited alternate CJK title.
        // Resolution, codec and release-group numbers are not episode labels.
        const episode = /^\s+(?:[^\p{Script=Latin}\p{Script=Cyrillic}\d]+\s+)?(\d{2,3})(?:v\d+)?(?:\s|$)/u.exec(suffix);
        if (episode) return { episode: String(Number(episode[1])) };
    }
    return undefined;
}

function catalogVariants(publication: Work, catalog: Work): Variant[] {
    return publication.variants.map(variant => {
        const episode = catalog.category === "anime" && variant.transport === "torrent"
            ? animeTorrentEpisode(variant, catalog) : undefined;
        return episode ? { ...variant, ...episode } : variant;
    });
}

function matchingCatalog(publication: Work, catalog: Work[]): Work | undefined {
    const episodeUpload = publication.category === "anime" && isUserUpload(publication)
        && publication.variants.every(variant => variant.transport === "video" && variant.episode
            && (variant.duration || 0) >= 15 * 60);
    if (!episodeUpload && !publication.variants.every(variant => variant.transport === "torrent"))
        return undefined;

    const names = publication.variants.map(variant => identity(variant.title));
    const matches = catalog.filter(work => {
        if ((publication.category !== "all" && work.category !== publication.category)
            || (publication.year && work.year && publication.year !== work.year))
            return false;
        if (["books", "manga"].includes(work.category))
            return sameWork(publication, work);

        if (episodeUpload) {
            const titles = [work.title, ...work.aliases || []].map(identity);
            if (publication.variants.some(variant => !matchesSeason(titles, variant.season)))
                return false;
            if (publication.aliases?.some(alias => titles.includes(identity(alias))))
                return true;

            // A copied full synopsis can identify an alternate release title.
            // An explicit season label needs an exact title match above.
            if (publication.variants.some(variant => variant.season))
                return false;
            const synopsis = work.description?.normalize("NFC").replace(/\s+/g, " ").trim();
            const description = publication.description?.normalize("NFC").replace(/\s+/g, " ").trim();
            return !!synopsis && synopsis.length >= 200 && !!description && description.startsWith(synopsis);
        }

        if (work.category === "anime")
            return publication.variants.some(variant => animeTorrentEpisode(variant, work));

        return [work.title, ...work.aliases || []].some(alias => {
            const key = identity(alias);
            if (key.length < 5)
                return false;

            return names.some(name => name === key || name.startsWith(key + " "));
        });
    });

    return matches.length === 1 ? matches[0] : undefined;
}

export function sameWork(left: Work, right: Work): boolean {
    if (left.id === right.id)
        return true;

    if (left.variants.some(a => right.variants.some(b => a.sourceId === b.sourceId && a.id === b.id)))
        return true;

    if (left.variants.some(a => right.variants.some(b => {
        const hash = magnetHash(a);
        return hash && hash === magnetHash(b);
    })))
        return true;

    if ((left.category !== "all" && right.category !== "all" && left.category !== right.category)
        || (left.year && right.year && left.year !== right.year))
        return false;
    for (const [namespace, value] of Object.entries(left.identifiers || {})) {
        if (value && right.identifiers?.[namespace] && right.identifiers[namespace] !== value)
            return false;
    }

    for (const [namespace, value] of Object.entries(left.identifiers || {})) {
        if (value && right.identifiers?.[namespace] === value)
            return true;
    }

    // An uploader's label identifies its publication, not a catalog work.
    // Exact publication IDs, hashes and catalog identifiers above still match.
    if (isUserUpload(left) || isUserUpload(right))
        return false;

    if (left.category === "all" || left.category !== right.category)
        return false;

    if (["books", "manga"].includes(left.category)) {
        const authors = (work: Work) => work.variants.flatMap(variant => variant.author ? [identity(variant.author)] : []);
        const leftAuthors = authors(left);
        const rightAuthors = authors(right);
        if (!leftAuthors.length || !rightAuthors.length
            || !leftAuthors.some(author => rightAuthors.includes(author)))
            return false;
    }

    const title = identity(left.title);
    return title.length >= 3 && title === identity(right.title);
}

function mergeNotes(...groups: (Variant["notes"] | undefined)[]): NonNullable<Variant["notes"]> {
    return [...new Map(groups.flatMap(notes => notes || [])
        .map(note => [JSON.stringify([note.text, note.url, note.author]), note])).values()];
}

export function mergeWorkMetadata(current: Work | undefined, incoming: Work): Work {
    return {
        ...current,
        ...Object.fromEntries(Object.entries(incoming).filter(([, value]) => value !== undefined)),
        aliases: [...new Set([...current?.aliases || [], ...incoming.aliases || []])],
        identifiers: { ...current?.identifiers, ...incoming.identifiers },
        notes: mergeNotes(current?.notes, incoming.notes),
    } as Work;
}

export function rankVariants(variants: Variant[]): Variant[] {
    const score = (variant: Variant) => {
        let value = 0;
        if (variant.publicationKind === "patch" || variant.publicationKind === "fix")
            value -= 100;
        if (variant.adStatus === "reported")
            value -= 80;
        if (/\b(?:cam|hdcam|telesync|tsrip)\b/i.test(variant.quality || variant.title))
            value -= 60;
        if (variant.publicationKind === "full")
            value += 30;
        if (variant.cooperative)
            value += 15;
        if (variant.magnet)
            value += 5;
        if (variant.adStatus === "clean")
            value += 10;
        if (/\b(?:web[ .-]?dl|bluray|blu[ .-]?ray|remux)\b/i.test(variant.quality || variant.title))
            value += 10;
        if (variant.seeders !== undefined && variant.seeders > 0)
            value += Math.min(10, Math.log2(variant.seeders + 1));
        return value;
    };

    return [...variants].sort((left, right) => score(right) - score(left));
}

export class SearchResults {
    private readonly works = new Map<string, Work>();
    private readonly queries: SearchQuery[];

    constructor(query: string, private readonly emit: (event: string, data: unknown) => void) {
        this.queries = [searchQuery(query)];
    }

    addQuery(query: string): void {
        const parsed = searchQuery(query);
        if (parsed.words.length && !this.queries.some(previous => identity(previous.title) === identity(parsed.title)))
            this.queries.push(parsed);
    }

    add(incoming: Work): boolean {
        if (!incoming.id || !incoming.title || !incoming.variants?.length)
            return false;

        const names = [incoming.title, ...incoming.aliases || [], ...incoming.variants.map(variant => variant.title)];
        if (!queryYearMatches(this.queries[0]!, names, incoming.year)
            || !this.queries.some(query => queryMatches(query, names, incoming.year)))
            return false;

        const incomingCatalog = !isUserUpload(incoming)
            && incoming.variants.some(variant => variant.transport !== "torrent");
        const target = [...this.works.values()].find(work => sameWork(work, incoming));

        let work = incoming;
        if (target) {
            work = {
                ...(incomingCatalog ? mergeWorkMetadata(target, incoming) : target),
                notes: mergeNotes(target.notes, incoming.notes),
                variants: [...target.variants, ...incoming.variants],
            };
            if (target.id !== work.id) {
                this.works.delete(target.id);
                this.emit("removeResult", { id: target.id });
            }
        }

        this.store(work);
        return true;
    }

    associatePublications(): void {
        // Wait for every catalog and alias round before resolving a publication.
        // A later season must not make an earlier irreversible match ambiguous.
        const current = [...this.works.values()];
        const catalogs = current.filter(work => !isUserUpload(work)
            && work.variants.some(variant => variant.transport !== "torrent"));
        for (const publication of current) {
            const catalog = matchingCatalog(publication, catalogs);
            if (!catalog || catalog.id === publication.id) continue;
            const target = this.works.get(catalog.id)!;
            this.works.delete(publication.id);
            this.emit("removeResult", { id: publication.id });
            this.store({ ...target, notes: mergeNotes(target.notes, publication.notes),
                variants: [...target.variants, ...catalogVariants(publication, catalog)] });
        }
    }

    private store(work: Work): void {
        // Preserve the actual publications for equal hashes: one variant carries
        // alternatives in its notes, rather than losing source attribution.
        const variants = new Map<string, Variant>();
        for (const variant of work.variants) {
            const key = variantKey(variant);
            const previous = variants.get(key);
            if (previous) {
                previous.notes = mergeNotes(previous.notes, variant.notes, previous.url !== variant.url
                    ? [{ text: variant.sourceName, url: variant.url }] : []);
            }
            else
                variants.set(key, { ...variant });
        }
        work = { ...work, variants: rankVariants([...variants.values()]) };
        this.works.set(work.id, work);
        this.emit("result", { ...work, relevance: this.relevance(work) });
    }

    private relevance(work: Work): number {
        const query = this.queries[0]!;
        const names = [work.title, ...work.aliases || []];
        const title = identity(query.title);
        const literal = query.year ? `${title} ${query.year}` : title;
        if (!isUserUpload(work)) {
            if (names.some(name => [title, literal].includes(identity(name)))) return 2;
            return Number(queryMatches(query, names, work.year));
        }

        // A short upload with an exact movie label can still be a clip.
        // Duration affects ordering only; it does not establish work identity.
        if (!work.variants.some(variant => (variant.duration || 0) >= 40 * 60)) return 0;
        if (names.some(name => [title, literal].includes(identity(name)))) return 1;

        // Release metadata distinguishes alternate movie titles from derivatives.
        // Compare a literal numbered title before parsing any bare release year.
        return Number(names.some(name => {
            let releaseYear = false;
            let exactTitle = false;
            for (const label of name.split(/\s*[|/]\s*/)) {
                const metadata = /\s*[([][^()[\]]*(?:19|20)\d{2}[^()[\]]*[)\]]\s*$/.exec(label);
                const plain = metadata ? label.slice(0, metadata.index) : label;
                if (metadata) releaseYear = true;
                if ([title, literal].includes(identity(plain))) {
                    exactTitle = true;
                    continue;
                }
                const parsed = searchQuery(plain);
                if (parsed.year && identity(parsed.title) === title) {
                    releaseYear = true;
                    exactTitle = true;
                }
            }
            return releaseYear && exactTitle;
        }));
    }

    titleAliases(query: string): string[] {
        const parsed = searchQuery(query);
        const exact = identity(parsed.title);
        const aliases = new Map<string, { title: string; original: boolean }>();
        const catalogs = [...this.works.values()].filter(work => !isUserUpload(work));
        const exactMatches = catalogs.filter(work => [work.title, ...work.aliases || []].some(name => identity(name) === exact));
        const candidates = exactMatches.length ? exactMatches : catalogs.length === 1 ? catalogs : [];
        for (const work of candidates) {
            const names = [work.title, ...work.aliases || []];
            for (const name of names) {
                const key = identity(name);
                if (key !== exact) {
                    aliases.set(key, { title: name, original: ["books", "manga"].includes(work.category)
                        && /[\p{Script=Han}\p{Script=Hangul}\p{Script=Hiragana}\p{Script=Katakana}]/u.test(name) });
                }
            }
        }
        return [...aliases.values()].sort((left, right) => Number(right.original) - Number(left.original))
            .slice(0, 3).map(alias => alias.title);
    }

    get size(): number {
        return this.works.size;
    }
}
