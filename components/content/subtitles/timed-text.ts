export type TimedTextFormat = "ass" | "ssa" | "srt" | "vtt";

export interface TimedTextFragment {
    readonly id: string;
    readonly text: string;
    readonly sourceStart: number;
    readonly sourceEnd: number;
}

export interface TimedTextCue {
    readonly id: string;
    readonly startMs: number;
    readonly endMs: number;
    readonly text: string;
    readonly raw: string;
    readonly sourceStart: number;
    readonly sourceEnd: number;
    readonly fragments: readonly TimedTextFragment[];
}

export interface TimedTextDocument {
    readonly format: TimedTextFormat;
    readonly source: string;
    readonly cues: readonly TimedTextCue[];
}

interface SourceLine {
    text: string;
    start: number;
    end: number;
}

const maxSourceBytes = 8 * 1024 * 1024;
const maxCues = 50_000;
const maxFragments = 200_000;
const maxCueCharacters = 64 * 1024;

function sourceLines(source: string): SourceLine[] {
    const lines: SourceLine[] = [];
    for (const match of source.matchAll(/([^\r\n]*)(?:\r\n|\r|\n|$)/g)) {
        if (match.index === source.length) break;
        if (lines.length >= 250_000) throw new Error("Subtitle has too many lines");
        lines.push({ text: match[1]!, start: match.index, end: match.index + match[1]!.length });
    }
    return lines;
}

function subtitleFormat(source: string, hint?: string): TimedTextFormat {
    if (hint) {
        const normalized = hint.toLowerCase().replace(/^\./, "");
        if (normalized === "ass" || normalized === "ssa" || normalized === "srt" || normalized === "vtt") return normalized;
        if (normalized === "subrip") return "srt";
        if (normalized === "webvtt" || normalized === "text/vtt") return "vtt";
        throw new Error(`Unsupported text subtitle format: ${hint}`);
    }
    if (/^\uFEFF?WEBVTT(?:[ \t]|\r|\n|$)/.test(source)) return "vtt";
    if (/^[ \t\uFEFF]*\[Events\][ \t]*$/im.test(source)) {
        return /^[ \t\uFEFF]*(?:ScriptType[ \t]*:[ \t]*v4\.00\+|\[V4\+ Styles\])/im.test(source) ? "ass" : "ssa";
    }
    if (/^[ \t\uFEFF]*\d{1,3}:\d{2}:\d{2},\d{3}[ \t]+-->/m.test(source)) return "srt";
    throw new Error("Unsupported text subtitle format");
}

function timestamp(value: string, format: TimedTextFormat): number {
    const pattern = format === "ass" || format === "ssa"
        ? /^(\d{1,3}):([0-5]\d):([0-5]\d)\.(\d{1,3})$/
        : format === "srt"
            ? /^(\d{1,3}):([0-5]\d):([0-5]\d),(\d{3})$/
            : /^(?:(\d{2,3}):)?([0-5]\d):([0-5]\d)\.(\d{3})$/;
    const match = value.trim().match(pattern);
    if (!match) throw new Error(`Invalid ${format.toUpperCase()} subtitle timestamp`);
    return ((Number(match[1] ?? 0) * 60 + Number(match[2])) * 60 + Number(match[3])) * 1000
        + Number(match[4]!.padEnd(3, "0"));
}

function decodeEntities(value: string): string {
    const entities: Record<string, string> = {
        amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: "\u00a0", lrm: "\u200e", rlm: "\u200f",
    };
    return value.replace(/&(#x[\da-f]+|#\d+|amp|lt|gt|quot|apos|nbsp|lrm|rlm);/gi, (original, entity: string) => {
        if (!entity.startsWith("#")) return entities[entity.toLowerCase()] ?? original;
        const code = entity[1]?.toLowerCase() === "x" ? parseInt(entity.slice(2), 16) : Number(entity.slice(1));
        return code > 0 && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff)
            ? String.fromCodePoint(code) : original;
    });
}

function textFragments(source: string, start: number, end: number, cueId: string, format: TimedTextFormat,
    budget: { fragments: number }): TimedTextFragment[] {
    const fragments: TimedTextFragment[] = [];
    const ass = format === "ass" || format === "ssa";
    const text = source.slice(start, end);
    const controls = ass ? /\{|\\[Nnh{}]|\r\n|\r|\n/g
        : /\{(?=\\)|<\/?[a-z](?:[^<>"'\r\n]|"[^"<\r\n]*"|'[^'<\r\n]*')*>|<\d{2,3}:[\d:.]+>|\r\n|\r|\n/gi;
    const lastBrace = text.lastIndexOf("}");
    let cursor = 0;
    let drawing = false;

    function append(until: number): void {
        if (drawing) return;
        const raw = text.slice(cursor, until);
        const leading = raw.length - raw.trimStart().length;
        const trailing = raw.trimEnd().length;
        if (trailing <= leading) return;
        if (++budget.fragments > maxFragments) throw new Error("Subtitle has too many text fragments");
        const value = raw.slice(leading, trailing);
        fragments.push({
            id: `${cueId}:${fragments.length + 1}`,
            text: ass ? value : decodeEntities(value),
            sourceStart: start + cursor + leading,
            sourceEnd: start + cursor + trailing,
        });
    }

    let match: RegExpExecArray | null;
    while ((match = controls.exec(text))) {
        const override = match[0] === "{";
        if (override) {
            if ((!ass && format !== "srt") || match.index > lastBrace) continue;
            controls.lastIndex = text.indexOf("}", match.index) + 1;
        }
        append(match.index);
        if (ass && override) {
            for (const tag of text.slice(match.index, controls.lastIndex).matchAll(/\\p(?!os|bo)[ \t]*([+-]?\d*)/g)) {
                drawing = Number(tag[1]) > 0;
            }
        }
        cursor = controls.lastIndex;
    }
    append(text.length);
    return fragments;
}

function addCue(cues: TimedTextCue[], source: string, format: TimedTextFormat,
    start: number, end: number, textStart: number, textEnd: number, startMs: number, endMs: number,
    budget: { fragments: number }): void {
    if (cues.length >= maxCues) throw new Error("Subtitle has too many cues");
    if (end - start > maxCueCharacters) throw new Error("Subtitle cue is too large");
    if (endMs < startMs) throw new Error("Subtitle cue ends before it starts");
    const id = `cue-${cues.length + 1}`;
    const fragments = textFragments(source, textStart, textEnd, id, format, budget);
    cues.push({
        id, startMs, endMs,
        text: fragments.map(fragment => fragment.text).join(" "),
        raw: source.slice(start, end),
        sourceStart: start,
        sourceEnd: end,
        fragments,
    });
}

function assCues(source: string, lines: SourceLine[], format: TimedTextFormat, budget: { fragments: number }): TimedTextCue[] {
    const cues: TimedTextCue[] = [];
    let events = false;
    let foundEvents = false;
    let fields: string[] = [];
    for (const line of lines) {
        const section = line.text.trim().match(/^\[([^\]]+)\]$/);
        if (section) {
            events = section[1]!.toLowerCase() === "events";
            foundEvents ||= events;
            fields = [];
            continue;
        }
        if (!events) continue;
        const fieldFormat = line.text.match(/^\s*Format\s*:\s*(.*)$/i);
        if (fieldFormat) {
            fields = fieldFormat[1]!.split(",").map(field => field.trim().toLowerCase());
            if (fields.length > 64 || fields.some(field => !field) || !fields.includes("start") || !fields.includes("end") || !fields.includes("text")
                || new Set(fields).size !== fields.length) throw new Error("Invalid ASS event field format");
            continue;
        }
        const dialogue = line.text.match(/^\s*Dialogue\s*:[ \t]*/i);
        if (!dialogue) continue;
        if (!fields.length) throw new Error("ASS dialogue is missing its event field format");

        // Text can contain commas. Other fields are read from the two ends of the record.
        const textIndex = fields.indexOf("text");
        const ranges: { start: number; end: number }[] = [];
        let left = dialogue[0].length;
        let right = line.text.length;
        for (let index = 0; index < textIndex; ++index) {
            const comma = line.text.indexOf(",", left);
            if (comma < 0) throw new Error("Incomplete ASS dialogue");
            ranges[index] = { start: left, end: comma };
            left = comma + 1;
        }
        for (let index = fields.length - 1; index > textIndex; --index) {
            const comma = line.text.lastIndexOf(",", right - 1);
            if (comma < left) throw new Error("Incomplete ASS dialogue");
            ranges[index] = { start: comma + 1, end: right };
            right = comma;
        }
        ranges[textIndex] = { start: left, end: right };
        const startRange = ranges[fields.indexOf("start")]!;
        const endRange = ranges[fields.indexOf("end")]!;
        addCue(cues, source, format, line.start, line.end, line.start + left, line.start + right,
            timestamp(line.text.slice(startRange.start, startRange.end), format),
            timestamp(line.text.slice(endRange.start, endRange.end), format), budget);
    }
    if (!foundEvents) throw new Error("ASS subtitles are missing the Events section");
    return cues;
}

function blockCues(source: string, lines: SourceLine[], format: "srt" | "vtt", budget: { fragments: number }): TimedTextCue[] {
    const cues: TimedTextCue[] = [];
    let cursor = 0;
    if (format === "vtt") {
        if (!/^\uFEFF?WEBVTT(?:[ \t].*)?$/.test(lines[0]?.text ?? "")) throw new Error("Invalid WebVTT header");
        while (cursor < lines.length && lines[cursor]!.text.trim()) ++cursor;
    }
    while (cursor < lines.length) {
        if (!lines[cursor]!.text.trim()) {
            ++cursor;
            continue;
        }
        const first = cursor;
        while (cursor < lines.length && lines[cursor]!.text.trim()) ++cursor;
        const last = cursor - 1;
        if (format === "vtt" && /^(?:NOTE(?:[ \t]|$)|STYLE$|REGION$)/.test(lines[first]!.text)) continue;
        const timingIndex = lines[first]!.text.includes("-->") ? first : first + 1;
        const timing = timingIndex <= last
            ? lines[timingIndex]!.text.match(/^\s*([\d:.,]+)[ \t]+-->[ \t]+([\d:.,]+)(?:[ \t].*)?$/) : null;
        if (!timing || (format === "srt" && timingIndex > first && !/^\uFEFF?\d+$/.test(lines[first]!.text.trim()))) {
            throw new Error(`Invalid ${format.toUpperCase()} subtitle cue`);
        }
        const textStart = timingIndex < last ? lines[timingIndex + 1]!.start : lines[timingIndex]!.end;
        addCue(cues, source, format, lines[first]!.start, lines[last]!.end, textStart, lines[last]!.end,
            timestamp(timing[1]!, format), timestamp(timing[2]!, format), budget);
    }
    return cues;
}

export function parseTimedText(source: string, formatHint?: string): TimedTextDocument {
    if (source.length > maxSourceBytes || Buffer.byteLength(source, "utf8") > maxSourceBytes) throw new Error("Subtitle exceeds the 8 MiB text limit");
    if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(source)) throw new Error("Binary subtitles cannot be translated as text");
    const format = subtitleFormat(source, formatHint);
    const lines = sourceLines(source);
    const budget = { fragments: 0 };
    const cues = format === "ass" || format === "ssa" ? assCues(source, lines, format, budget) : blockCues(source, lines, format, budget);
    return { format, source, cues };
}

export function renderTimedText(document: TimedTextDocument,
    translations: ReadonlyMap<string, string> | Readonly<Record<string, string>>): string {
    const output: string[] = [];
    let cursor = 0;
    let outputBytes = Buffer.byteLength(document.source, "utf8");
    for (const cue of document.cues) {
        let cueCharacters = cue.raw.length;
        for (const fragment of cue.fragments) {
            const translated = translations instanceof Map ? translations.get(fragment.id)
                : Object.hasOwn(translations, fragment.id) ? (translations as Readonly<Record<string, string>>)[fragment.id] : undefined;
            if (translated === undefined) continue;
            if (typeof translated !== "string" || translated.length > maxCueCharacters) throw new Error("Invalid subtitle translation");
            let safe = translated.replace(/[\r\n\t]+/g, " ").replace(/[\u0000-\u001f\u007f]/g, "").trim();
            if (!safe) throw new Error("Subtitle translation is empty");
            // SRT/VTT players also convert text to ASS internally. Keep translated prose literal there too.
            safe = safe.replace(/\\/g, "＼").replace(/\{/g, "｛").replace(/\}/g, "｝");
            if (document.format === "vtt") {
                safe = safe.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
            }
            else if (document.format === "srt") {
                // FFmpeg's SubRip decoder does not decode HTML entities.
                safe = safe.replace(/</g, "＜").replace(/>/g, "＞");
            }
            outputBytes += Buffer.byteLength(safe, "utf8") - Buffer.byteLength(document.source.slice(fragment.sourceStart, fragment.sourceEnd), "utf8");
            cueCharacters += safe.length - (fragment.sourceEnd - fragment.sourceStart);
            if (cueCharacters > maxCueCharacters) throw new Error("Translated subtitle cue is too large");
            if (outputBytes > maxSourceBytes) throw new Error("Translated subtitle exceeds the 8 MiB text limit");
            output.push(document.source.slice(cursor, fragment.sourceStart), safe);
            cursor = fragment.sourceEnd;
        }
    }
    output.push(document.source.slice(cursor));
    return output.join("");
}
