import type { ResolvedContent } from "./contracts";

export type AudioTrack = NonNullable<ResolvedContent["audioTrack"]>;
export interface AudioMetadata {
    title?: string;
    language?: string;
    codec?: string;
    channels?: number;
}

const languageAliases: Record<string, string> = {
    alb: "sqi", arm: "hye", baq: "eus", bur: "mya", chi: "zho", cze: "ces", dut: "nld",
    fre: "fra", geo: "kat", ger: "deu", gre: "ell", ice: "isl", mac: "mkd", may: "msa",
    per: "fas", rum: "ron", slo: "slk", tib: "bod", wel: "cym",
};

function language(value: string): string {
    const code = value.toLowerCase();
    return /^[a-z]{2,3}$/.test(code) ? new Intl.Locale(languageAliases[code] || code).language : code;
}

function fragment(actual: string, expected: string): boolean {
    const normalize = (value: string) => value.normalize("NFC").replace(/\s+/g, " ").trim().toLowerCase();
    const literal = normalize(expected).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    return !!literal && new RegExp(`(?:^|[^\\p{L}\\p{N}])${literal}(?=$|[^\\p{L}\\p{N}])`, "u").test(normalize(actual));
}

export function matchesAudio(actual: AudioMetadata, expected: AudioTrack): boolean {
    if (!actual.title || !fragment(actual.title, expected.title)
        || (expected.type && !fragment(actual.title, expected.type)))
        return false;
    return (!expected.language || (!!actual.language && language(actual.language) === language(expected.language)))
        && (!expected.codec || actual.codec?.toLowerCase() === expected.codec.toLowerCase())
        && (!expected.channels || actual.channels === expected.channels);
}
