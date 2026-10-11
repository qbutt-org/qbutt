import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { handleReaderRequest } from "../../components/content/reader/service";
import { contentId, type ReaderDocument, type ReaderPage } from "../../components/content/reader/types";
import type { ProviderContext } from "../../components/content/contracts";
import { cleanImageText } from "../../components/content/reader/image-processing";
import { mediaRuntime as resolveMediaRuntime } from "../../components/content/media-runtime";

function require(value: unknown, message: string): asserts value {
    if (!value) throw new Error(message);
}

const directory = resolve(process.env.QBUTT_READER_IMAGE_FIXTURE ?? "C:/Temp/qbutt-image-cleanup-qa");
const appDir = process.env.QBUTT_CONTENT_APP_DIR;
require(appDir, "Set QBUTT_CONTENT_APP_DIR to the application fixture");
process.env.QBUTT_LAB_PATHS = "1";
const { runtimeDir: mediaRuntime, tools: { ffmpeg } } = await resolveMediaRuntime(appDir, AbortSignal.timeout(30_000));
const input = join(directory, "vertical-art.png");
const recognition = JSON.parse(await readFile(join(directory, "ocr.json"), "utf8"));
let requests = 0;
let transparentFixture = false;
const server = Bun.serve({
    hostname: "127.0.0.1", port: 0,
    async fetch(request) {
        require(!request.headers.has("cookie"), "AI received source cookies");
        ++requests;
        const body = await request.json() as any;
        const value = body.response_format.json_schema.schema.properties.regions
            ? { regions: recognition.regions.map((region: Record<string, unknown>) =>
                transparentFixture ? { ...region, ink: "dark" } : region) } : {
            blocks: JSON.parse(body.messages[1].content).blocks.map((block: {id: string; text: string}) => ({
                id: block.id, text: block.text.includes("凛") ? "Обещание Рин живёт здесь." : "Вместе мы найдём новый мир.",
            })), terms: [{ source: "凛", target: "Рин" }],
        };
        return Response.json({ choices: [{ finish_reason: "stop", message: { content: JSON.stringify(value) } }] });
    },
});
const context: ProviderContext = {
    signal: new AbortController().signal, appDir, dataDir: join(directory, `compatible-profile-${crypto.randomUUID()}`),
    settings: { aiProvider: "openai-compatible", aiBaseUrl: `http://127.0.0.1:${server.port}/v1`,
        aiApiKey: "generated-image-fixture", aiModel: "fixture" },
    fetch: async () => { throw new Error("Image fixture should not access a source"); },
};
const request = (method: string, params: Record<string, unknown>) => handleReaderRequest(method, params, context);
async function decodeFixture(path: string, format = "rgb24"): Promise<Uint8Array> {
    const child = Bun.spawn({ cmd: [ffmpeg, "-v", "error", "-i", path,
        "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", format, "pipe:1"],
        cwd: mediaRuntime, stdout: "pipe", stderr: "pipe", windowsHide: true });
    const [bytes, diagnostics, exit] = await Promise.all([new Response(child.stdout).arrayBuffer(),
        new Response(child.stderr).text(), child.exited]);
    require(exit === 0, `Fixture decode failed: ${diagnostics}`);
    return new Uint8Array(bytes);
}

try {
    const originalHash = contentId(await readFile(input));
    const document = await request("reader.local", { path: input }) as ReaderDocument;
    const page = await request("reader.chapter", { documentId: document.id, chapterId: document.chapters[0]!.id }) as ReaderPage;
    const started = performance.now();
    const translated = await request("reader.translate", { page }) as ReaderPage;
    require(requests === 2 && translated.regions?.length === 3, "OCR and translation did not finish in two AI requests");
    const image = translated.images?.[0];
    require(image?.cleanedUrl && image.url === page.images?.[0]?.url && translated.original?.images?.[0]?.url === image.url,
        "Cleaned layer replaced the immutable original");
    require(image.width === 900 && image.height === 1000, "Image dimensions not preserved");
    require(translated.regions.every(region => region.layout && !Object.hasOwn(region, "inkBoxes")), "Public image layout leaked internal masks");
    const layer = JSON.parse(await readFile(join(image.cleanedUrl, "..", "layers.json"), "utf8"));
    const [original, clean, truth, mask, truthInk] = await Promise.all([
        decodeFixture(input), decodeFixture(image.cleanedUrl), decodeFixture(join(directory, "truth.png")),
        decodeFixture(layer.maskUrl), decodeFixture(join(directory, "truth-ink.png")),
    ]);
    let masked = 0;
    let ink = 0;
    let covered = 0;
    let outsideChanges = 0;
    let beforeError = 0;
    let afterError = 0;
    for (let index = 0; index < original.length; index += 3) {
        const selected = mask[index]! > 0;
        if (selected) ++masked;
        if (truthInk[index]! > 64) {
            ++ink;
            if (selected) ++covered;
        }
        for (let channel = 0; channel < 3; ++channel) {
            if (!selected && original[index + channel] !== clean[index + channel]) ++outsideChanges;
            if (selected) {
                beforeError += Math.abs(original[index + channel]! - truth[index + channel]!);
                afterError += Math.abs(clean[index + channel]! - truth[index + channel]!);
            }
        }
    }
    require(outsideChanges === 0, "Cleanup changed artwork outside the stroke mask");
    require(covered / ink > 0.98, "Cleanup missed visible text strokes");
    require(masked / 900000 < 0.04, "Cleanup filled rectangular text areas instead of stroke masks");
    require(afterError / beforeError < 0.1, "Cleanup did not restore the known illustration background");
    require((await request("reader.glossary", { documentId: document.id }) as Record<string,string>).凛 === "Рин", "Image translation did not learn a name");
    const layerTime = (await stat(image.cleanedUrl)).mtimeMs;
    const edited = await request("reader.correct", { documentId: document.id, pageId: page.id,
        regionId: translated.regions[2]!.id, text: "Рин сдержит обещание." }) as ReaderPage;
    const cached = await request("reader.translate", { page }) as ReaderPage;
    require(edited.regions?.[2]?.translation === "Рин сдержит обещание." && cached.regions?.[2]?.translation === "Рин сдержит обещание.", "Correction/cache lost editable text");
    require(requests === 2 && (await stat(image.cleanedUrl)).mtimeMs === layerTime, "Correction or cache reprocessed image/AI");
    require(contentId(await readFile(input)) === originalHash, "Original image was changed");

    transparentFixture = true;
    const transparentInput = join(directory, "transparent.png");
    const transparentHash = contentId(await readFile(transparentInput));
    const transparentDocument = await request("reader.local", { path: transparentInput }) as ReaderDocument;
    const transparentPage = await request("reader.chapter", { documentId: transparentDocument.id,
        chapterId: transparentDocument.chapters[0]!.id }) as ReaderPage;
    const transparentTranslation = await request("reader.translate", { page: transparentPage }) as ReaderPage;
    const transparentImage = transparentTranslation.images?.[0];
    require(transparentImage?.cleanedUrl, "Transparent image has no cleaned layer");
    const transparentLayer = JSON.parse(await readFile(join(transparentImage.cleanedUrl, "..", "layers.json"), "utf8"));
    const [rgba, cleanedRgba, transparentMask] = await Promise.all([
        decodeFixture(transparentInput, "rgba"), decodeFixture(transparentImage.cleanedUrl, "rgba"), decodeFixture(transparentLayer.maskUrl),
    ]);
    let outsideRgbaChanges = 0;
    let originalAlpha = 0;
    let cleanedAlpha = 0;
    for (let pixel = 0; pixel < rgba.length / 4; ++pixel) {
        originalAlpha += rgba[pixel * 4 + 3]!;
        cleanedAlpha += cleanedRgba[pixel * 4 + 3]!;
        for (let channel = 0; channel < 4; ++channel) {
            if (!transparentMask[pixel * 3] && rgba[pixel * 4 + channel] !== cleanedRgba[pixel * 4 + channel]) ++outsideRgbaChanges;
        }
    }
    require(outsideRgbaChanges === 0 && cleanedAlpha / originalAlpha < 0.03,
        "Transparent cleanup lost alpha or changed RGBA outside its mask");
    require(contentId(await readFile(transparentInput)) === transparentHash, "Transparent original changed");
    let codexReplay: { evidence: string; opaqueRgbaIdentical: boolean; maskIdentical: boolean } | undefined;
    if (process.env.QBUTT_READER_CODEX_EVIDENCE) {
        const path = resolve(process.env.QBUTT_READER_CODEX_EVIDENCE);
        const prior = JSON.parse(await readFile(path, "utf8"));
        require(prior.success && prior.ocr?.sourceRegions?.length && prior.ocr.cleanedUrl, "A successful real Codex image run is required for replay");
        const replay = await cleanImageText(prior.ocr.fixture, prior.ocr.sourceRegions, context);
        const [beforeRgba, afterRgba, beforeMask, afterMask] = await Promise.all([
            decodeFixture(prior.ocr.cleanedUrl, "rgba"), decodeFixture(replay.cleanedUrl, "rgba"),
            decodeFixture(prior.ocr.maskUrl), decodeFixture(replay.maskUrl),
        ]);
        require(Buffer.from(beforeRgba).equals(Buffer.from(afterRgba)), "RGBA processing changed the real Codex opaque pixels");
        require(Buffer.from(beforeMask).equals(Buffer.from(afterMask)), "RGBA processing changed the real Codex opaque mask");
        codexReplay = { evidence: path, opaqueRgbaIdentical: true, maskIdentical: true };
    }
    const evidence = { passed: true, fixture: input, originalHash, cleanedUrl: image.cleanedUrl, maskUrl: layer.maskUrl,
        requests, durationMs: Math.round(performance.now() - started), maskedPixels: masked, inkRecall: covered / ink,
        maskedFraction: masked / 900000, outsideChanges, residualErrorRatio: afterError / beforeError,
        regions: cached.regions, cacheAndCorrection: true, originalUnchanged: true,
        transparent: { input: transparentInput, cleanedUrl: transparentImage.cleanedUrl, outsideRgbaChanges,
            residualAlphaRatio: cleanedAlpha / originalAlpha, originalUnchanged: true }, codexReplay };
    await mkdir(directory, { recursive: true });
    await writeFile(join(directory, "compatible-evidence.json"), JSON.stringify(evidence, null, 2));
    console.info("PASS reversible image cleanup, known-background recovery, bounded stroke masks, layout, glossary, correction/cache", JSON.stringify(evidence));
}
finally {
    server.stop(true);
}
