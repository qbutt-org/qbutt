import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { parseTimedText, renderTimedText, type TimedTextDocument, type TimedTextFormat } from "../../components/content/subtitles/timed-text";

function require(condition: unknown, message: string): asserts condition {
    if (!condition) throw new Error(message);
}

const ass = String.raw`[Script Info]
Title: Generated legal translation fixture
ScriptType: v4.00+
PlayResX: 640
PlayResY: 360
WrapStyle: 0

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,24,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1
Style: Sign,Arial,20,&H0000FFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,1,0,7,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Comment: 0,0:00:00.00,0:00:07.00,Default,Creator,0,0,0,,This comment remains untranslated.
Dialogue: 1,0:00:01.25,0:00:04.50,Default,凛,0010,0020,0030,,{\i1}Hello, world!{\i0}\N{\k20}One{\k30} song
Dialogue: 3,0:00:02.00,0:00:05.00,Sign,Actor,0,0,0,,{\an7\pos(20,20)}{\p1}m 0 0 l 40 0 40 20 0 20{\p}Sign
Dialogue: 0,0:00:04.00,0:00:06.00,Default,,0,0,0,,Overlap\hhere
`;

const ssa = String.raw`[Script Info]
ScriptType: v4.00

[V4 Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, TertiaryColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, AlphaLevel, Encoding
Style: Default,Arial,24,16777215,255,0,0,0,0,1,2,0,2,10,10,10,0,1

[Events]
Format: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: Marked=0,0:00:01.25,0:00:04.50,Default,,0,0,0,,{\b1}Older style{\b0}, kept
Dialogue: Marked=0,0:00:02.00,0:00:05.00,Default,,0,0,0,,Second line
`;

const srt = '\uFEFF17\r\n00:00:01,250 --> 00:00:04,500\r\n{\\an8}<i>Hello &amp; goodbye</i>\r\nSecond <b>line</b>.\r\n\r\n23\r\n00:00:02,000 --> 00:00:05,000\r\n<font color="#ff0000">Overlapping</font> cue\r\n';
const vtt = `\uFEFFWEBVTT generated fixture
Kind: captions
Language: en

STYLE
::cue(.sign) { color: lime; }

REGION
id:lower
width:80%
lines:3

NOTE this block contains timing-like data, never translated
00:01.000 --> 00:02.000
Private generated note.

speaker-one
00:01.250 --> 00:04.500 line:80% position:50% size:80% align:middle
<v Rin><c.sign>Hello &amp; goodbye</c></v>
Second <i>line</i>.

speaker-two
00:02.000 --> 00:05.000 region:lower vertical:rl
<lang en>Overlapping <00:03.000>cue &lt;literal&gt;</lang>
`;

const directory = await mkdtemp(join(tmpdir(), "qbutt-subtitle-formats-"));
const ffmpeg = process.env.QBUTT_SUBTITLE_FFMPEG ?? "ffmpeg";
const ffprobe = process.env.QBUTT_SUBTITLE_FFPROBE ?? "ffprobe";
const evidence: Record<string, unknown> = { fixtureDirectory: directory, checks: [] };
const checks = evidence.checks as string[];

async function command(args: string[]): Promise<string> {
    const process = Bun.spawn(args, { cwd: directory, stdout: "pipe", stderr: "pipe", windowsHide: true });
    const deadline = setTimeout(() => process.kill(), 30_000);
    try {
        const [stdout, stderr, exit] = await Promise.all([
            new Response(process.stdout).text(), new Response(process.stderr).text(), process.exited,
        ]);
        require(exit === 0, `${args[0]} exited ${exit}: ${stderr}`);
        return stdout;
    }
    finally {
        clearTimeout(deadline);
    }
}

function metadata(document: TimedTextDocument): string {
    const fragments = document.cues.flatMap(cue => cue.fragments);
    let result = "";
    let cursor = 0;
    for (const fragment of fragments) {
        result += document.source.slice(cursor, fragment.sourceStart) + "[HUMAN TEXT]";
        cursor = fragment.sourceEnd;
    }
    return result + document.source.slice(cursor);
}

function reject(action: () => unknown, description: string): void {
    let failed = false;
    try { action(); }
    catch { failed = true; }
    require(failed, `Accepted ${description}`);
}

async function packets(filename: string): Promise<unknown[]> {
    const result = JSON.parse(await command([ffprobe, "-v", "error", "-show_packets", "-show_entries", "packet=pts_time,duration_time", "-of", "json", filename]));
    return result.packets;
}

try {
    for (const [format, source] of Object.entries({ ass, ssa, srt, vtt }) as [TimedTextFormat, string][]) {
        const original = parseTimedText(source, format);
        const originalHash = createHash("sha256").update(source).digest("hex");
        require(original.cues.length === (format === "ass" ? 3 : 2), `${format}: incorrect cue count`);
        require(original.cues[0]!.startMs === 1250 && original.cues[0]!.endMs === 4500, `${format}: exact timing lost`);
        require(original.cues[1]!.startMs < original.cues[0]!.endMs, `${format}: overlap lost`);
        require(renderTimedText(original, new Map()) === source, `${format}: unchanged render changed source bytes`);
        require(parseTimedText(source).format === format, `${format}: format detection failed`);
        require(original.cues.every(cue => cue.raw === source.slice(cue.sourceStart, cue.sourceEnd)), `${format}: raw cue record differs`);

        const translations = new Map(original.cues.flatMap(cue => cue.fragments.map(fragment => [fragment.id, `Перевод ${fragment.text}`] as const)));
        const translated = renderTimedText(original, translations);
        const parsed = parseTimedText(translated, format);
        require(metadata(parsed) === metadata(original), `${format}: timing, styles, tags, line breaks, metadata or cue order changed`);
        require(parsed.cues.every((cue, index) => cue.startMs === original.cues[index]!.startMs && cue.endMs === original.cues[index]!.endMs), `${format}: translated timing differs`);
        require(parsed.cues.flatMap(cue => cue.fragments).every(fragment => fragment.text.startsWith("Перевод ")), `${format}: translated text was lost`);
        require(createHash("sha256").update(original.source).digest("hex") === originalHash, `${format}: original mutated`);

        const originalPath = `original.${format}`;
        const translatedPath = `translated.${format}`;
        await writeFile(join(directory, originalPath), source);
        await writeFile(join(directory, translatedPath), translated);
        const sourcePackets = await packets(originalPath);
        const translatedPackets = await packets(translatedPath);
        require(sourcePackets.length === original.cues.length, `${format}: FFmpeg did not read all original cues`);
        require(JSON.stringify(sourcePackets) === JSON.stringify(translatedPackets), `${format}: FFmpeg packet timing/duration changed`);
        if (format === "srt" || format === "vtt") {
            const decodedPath = `decoded-${format}.ass`;
            await command([ffmpeg, "-v", "error", "-y", "-i", translatedPath, "-c:s", "ass", decodedPath]);
            const decoded = parseTimedText(await readFile(join(directory, decodedPath), "utf8"), "ass");
            require(decoded.cues[0]!.text.includes("Перевод Hello & goodbye"), `${format}: FFmpeg displayed entity escapes instead of translated prose`);
        }
        await command([ffmpeg, "-v", "error", "-f", "lavfi", "-i", "color=black:s=640x360:r=1:d=7", "-vf", `subtitles=${translatedPath}`, "-frames:v", "7", "-f", "null", "-"]);
        require(await readFile(join(directory, originalPath), "utf8") === source, `${format}: original file changed`);

        evidence[format] = { cues: original.cues.length, fragments: original.cues.flatMap(cue => cue.fragments).length,
            originalSha256: originalHash, packetTiming: translatedPackets, metadataIdentical: true, renderedFrames: 7 };
        checks.push(`${format}: exact metadata, overlapping cues, immutable original, FFmpeg packet timing and 7 decoded frames`);
        console.info(`PASS ${checks.at(-1)}`);
    }

    const parsedAss = parseTimedText(ass, "ass");
    require(!parsedAss.cues.flatMap(cue => cue.fragments).some(fragment => fragment.text.includes("m 0 0")), "ASS drawing commands became translation input");
    require(parsedAss.cues[1]!.fragments[0]!.text === "Sign", "ASS implicit drawing reset lost following human text");
    require(parsedAss.cues[0]!.fragments.map(fragment => fragment.text).join("|") === "Hello, world!|One|song", "ASS karaoke and comma boundaries changed");
    const shiftedFields = String.raw`[Events]
Format: End, Text, Start, Style, Layer, Name, MarginL, MarginR, MarginV, Effect
Dialogue: 0:00:04.50,{\i1}Hello, world!{\i0},0:00:01.25,Sign,9,Rin,0010,0020,0030,scroll up;10
`;
    const flexible = parseTimedText(shiftedFields, "ass");
    require(flexible.cues[0]!.startMs === 1250 && flexible.cues[0]!.fragments[0]!.text === "Hello, world!", "ASS flexible field order/text commas failed");
    require(renderTimedText(flexible, { "cue-1:1": "Привет, мир!" }).includes(",0:00:01.25,Sign,9,Rin,0010,0020,0030,scroll up;10"), "ASS suffix fields changed");
    require(parseTimedText(vtt).cues[0]!.fragments[0]!.text === "Hello & goodbye", "VTT entities not decoded for translation");
    require(!parseTimedText(vtt).cues.flatMap(cue => cue.fragments).some(fragment => fragment.text.includes("Private generated note")), "VTT NOTE became translation input");
    require(parseTimedText(srt.replace(/\r\n/g, "\r"), "srt").cues.length === 2, "CR-only source lost cue boundaries");
    const angleText = parseTimedText('1\n00:00:01,000 --> 00:00:02,000\n2 < 3 and <font face="A>B">human</font>', "srt");
    require(angleText.cues[0]!.fragments.map(fragment => fragment.text).join("|") === "2 < 3 and|human", "Literal comparison or quoted tag attribute became control/human text");
    require(metadata(parseTimedText(renderTimedText(angleText, { "cue-1:1": "Два меньше трёх и", "cue-1:2": "человек" }), "srt")) === metadata(angleText), "Quoted tag attribute changed");
    const braceText = parseTimedText(String.raw`[Events]
Format: Start, End, Text
Dialogue: 0:00:01.00,0:00:02.00,\{human\}
`, "ass");
    require(braceText.cues[0]!.fragments[0]!.text === "human", "Escaped ASS literal braces hid visible text");
    const unclosedAss = parseTimedText("[Events]\nFormat: Start, End, Text\nDialogue: 0:00:01.00,0:00:02.00," + "{".repeat(60_000), "ass");
    require(unclosedAss.cues[0]!.fragments[0]!.text.length === 60_000, "Unclosed literal braces lost human text");

    const injection = String.raw`{\p1}\N<script>injected</script>` + "\r\n\r\n00:99:99,999 --> 00:99:99,999";
    for (const [format, source] of Object.entries({ ass, srt, vtt }) as [TimedTextFormat, string][]) {
        const document = parseTimedText(source, format);
        const fragment = document.cues[0]!.fragments[0]!;
        const translated = renderTimedText(document, new Map([[fragment.id, injection]]));
        require(metadata(parseTimedText(translated, format)) === metadata(document), `${format}: translation injected controls or a cue`);
        if (format !== "ass") require(!translated.includes("<script>injected</script>"), `${format}: translation injected HTML`);
    }
    checks.push("ASS reordered fields/comma payloads, karaoke/drawing, WebVTT voice/entities/NOTE, CR-only input, inline injection rejection");
    console.info(`PASS ${checks.at(-1)}`);

    reject(() => parseTimedText("PG\0\0\u0001", "sup"), "image subtitles as text");
    reject(() => parseTimedText("not subtitles", "hdmv_pgs_subtitle"), "unsupported bitmap codec");
    reject(() => parseTimedText("\0" + ass, "ass"), "binary source");
    reject(() => parseTimedText("x".repeat(8 * 1024 * 1024 + 1), "srt"), "oversized source");
    reject(() => parseTimedText("[Events]\nDialogue: 0,0:00:00.00,0:00:01.00,Hello", "ass"), "ASS dialogue without field format");
    reject(() => parseTimedText("1\n00:00:02,000 --> 00:00:01,000\nWrong", "srt"), "reversed cue timing");
    reject(() => parseTimedText("1\n00:00:01,000 --> 00:00:02,000\n" + "x".repeat(65_536), "srt"), "oversized cue");
    reject(() => parseTimedText("1\n00:00:01,000 --> 00:00:02,000\nText\n\n".repeat(50_001), "srt"), "excessive cue count");
    reject(() => parseTimedText(("[Events]\nFormat: Start, End, Text\n")
        + ("Dialogue: 0:00:01.00,0:00:02.00," + String.raw`{\i1}x`.repeat(10_000) + "\n").repeat(21), "ass"), "excessive text fragment count");
    reject(() => renderTimedText(parsedAss, { "cue-1:1": "\n\0\t" }), "empty translated text");
    checks.push("unsupported image/binary formats, source/cue bounds, invalid timing/ASS layout and empty translation rejected");
    console.info(`PASS ${checks.at(-1)}`);

    if (process.env.QBUTT_SUBTITLE_FORMAT_EVIDENCE) {
        await writeFile(process.env.QBUTT_SUBTITLE_FORMAT_EVIDENCE, JSON.stringify(evidence, null, 2) + "\n");
        console.info(`Evidence: ${process.env.QBUTT_SUBTITLE_FORMAT_EVIDENCE}`);
    }
}
finally {
    if (!process.env.QBUTT_SUBTITLE_FORMAT_EVIDENCE) await rm(directory, { recursive: true, force: true });
}
