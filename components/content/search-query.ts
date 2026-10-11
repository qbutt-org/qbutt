export interface SearchQuery {
    title: string;
    words: string[];
    year?: number;
}

export function normalizeTitle(value: string): string {
    return value.toLocaleLowerCase().normalize("NFKD").replace(/\p{M}+/gu, "")
        .replace(/\b(?:[a-z]\.){2,}[a-z]?\.?/g, abbreviation => abbreviation.replace(/\./g, ""))
        .replace(/[\p{P}\p{S}]+/gu, " ")
        .replace(/\s+/g, " ").trim();
}

export function searchQuery(value: string): SearchQuery {
    const year = /^(.*\p{L}.*?)\s+\(?(19\d{2}|20\d{2})\)?[!.]?$/u.exec(value.trim());
    const title = year?.[1] ?? value.trim();
    return { title, words: normalizeTitle(title).split(" ").filter(Boolean), year: year ? Number(year[2]) : undefined };
}

export function queryYearMatches(query: SearchQuery, names: string[], year?: number): boolean {
    // A number can belong to a title, as in Blade Runner 2049, rather than
    // describe its release year. Preserve that literal title interpretation.
    return !query.year || year === query.year
        || names.some(name => normalizeTitle(name).split(" ").includes(String(query.year)));
}

export function queryMatches(query: SearchQuery, names: string[], year?: number): boolean {
    if (!queryYearMatches(query, names, year)) return false;
    return names.some(name => {
        const normalized = normalizeTitle(name);
        const words = normalized.split(" ");
        return query.words.every(word => {
            if (/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u.test(word)) {
                return normalized.includes(word);
            }
            return words.some(candidate => candidate === word
                || /^[\p{Script=Cyrillic}]{4,}$/u.test(word) && candidate.startsWith(word));
        });
    });
}
