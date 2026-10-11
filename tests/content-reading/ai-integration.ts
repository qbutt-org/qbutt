import { copyFile, mkdir, mkdtemp, writeFile, readFile, rm, stat } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";
import { zipSync, strToU8 } from "../../components/content/node_modules/fflate";
import { handleReaderRequest } from "../../components/content/reader/service";
import { contentId, type ReaderDocument, type ReaderPage } from "../../components/content/reader/types";
import type { ProviderContext } from "../../components/content/contracts";
import type { RecognizedRegion } from "../../components/content/reader/translation";

function require(condition: unknown, message: string): asserts condition {
    if (!condition) {
        throw new Error(message);
    }
}

async function authorizationHash(): Promise<string> {
    const path = join(process.env.CODEX_HOME || join(homedir(), ".codex"), "auth.json");
    const hash = new Bun.CryptoHasher("sha256");
    for await (const chunk of Bun.file(path).stream()) hash.update(chunk);
    return hash.digest("hex");
}

async function rgb(path: string, appDir: string): Promise<Uint8Array> {
    const child = Bun.spawn({ cmd: [join(appDir, "media", "tools", "ffmpeg.exe"), "-v", "error", "-i", path,
        "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"],
        stdout: "pipe", stderr: "ignore", windowsHide: true, signal: AbortSignal.timeout(30_000) });
    const [bytes, exit] = await Promise.all([new Response(child.stdout).arrayBuffer(), child.exited]);
    require(exit === 0, "Could not decode image evidence");
    return new Uint8Array(bytes);
}

async function imageMetrics(originalPath: string, cleanedPath: string, maskPath: string, appDir: string): Promise<Record<string, number> | undefined> {
    const directory = dirname(originalPath);
    const truthPath = join(directory, "truth.png");
    const inkPath = join(directory, "truth-ink.png");
    if (!await Bun.file(truthPath).exists() || !await Bun.file(inkPath).exists()) return undefined;
    const [original, cleaned, mask, truth, truthInk] = await Promise.all([
        rgb(originalPath, appDir), rgb(cleanedPath, appDir), rgb(maskPath, appDir), rgb(truthPath, appDir), rgb(inkPath, appDir),
    ]);
    require(original.length === cleaned.length && original.length === mask.length
        && original.length === truth.length && original.length === truthInk.length, "Image evidence dimensions differ");
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
            if (!selected && original[index + channel] !== cleaned[index + channel]) ++outsideChanges;
            beforeError += Math.abs(original[index + channel]! - truth[index + channel]!);
            afterError += Math.abs(cleaned[index + channel]! - truth[index + channel]!);
        }
    }
    return { maskedPixels: masked, maskedFraction: masked / (original.length / 3), inkRecall: covered / ink,
        outsideChanges, residualErrorRatio: afterError / beforeError };
}

const authorizationBefore = await authorizationHash();
const root = await mkdtemp(join(tmpdir(), "qbutt-reading-ai-integration-"));
const phases: string[] = [];
const checks: string[] = [];
const evidence: Record<string, unknown> = { version: 1, startedAt: new Date().toISOString(), success: false, checks };
const context: ProviderContext = {
    signal: new AbortController().signal,
    appDir: process.env.QBUTT_CONTENT_APP_DIR,
    dataDir: join(root, "profile"),
    settings: { aiProvider: "auto", aiModel: "anthropic/saved-external-model" },
    fetch,
};
const request = (method: string, params: Record<string, unknown>) => handleReaderRequest(method, params, context,
    (_, data) => {
        if (typeof data === "object" && data !== null && "phase" in data) {
            phases.push(String(data.phase));
        }
    });

try {
    const path = join(root, "original.html");
    const original = "<h1>庭の朝</h1><p>凛は猫に挨拶しました。</p><p>猫は庭で待っています。</p>";
    await writeFile(path, original, "utf8");
    const document = await request("reader.local", { path }) as ReaderDocument;
    const page = await request("reader.chapter", { documentId: document.id, chapterId: document.chapters[0]!.id }) as ReaderPage;
    await request("reader.glossary", { documentId: document.id, entries: { 凛: "Рин" } });
    const translated = await request("reader.translate", { page }) as ReaderPage;
    require(translated.provider === "codex" && translated.html?.includes("Рин") && /[А-Яа-я]/.test(translated.html), "Real Codex translation/glossary failed");
    require(translated.original?.html === page.html, "Translation changed the saved original");
    const phaseCount = phases.length;
    const cached = await request("reader.translate", { page }) as ReaderPage;
    require(cached.cached && phases.length === phaseCount, "Cached translation started another AI request");
    const glossary = await request("reader.glossary", { documentId: document.id }) as Record<string, string>;
    await request("reader.glossary", { documentId: document.id, entries: { ...glossary, 無関係: "Посторонний термин" } });
    const reusedBlocks = await request("reader.translate", { page }) as ReaderPage;
    require(reusedBlocks.html === translated.html && phases.length === phaseCount, "Unrelated glossary entry invalidated translated blocks");
    require(await readFile(path, "utf8") === original, "Reader changed the user's original file");
    evidence.translation = { provider: translated.provider, model: translated.model, original, translated: translated.html,
        glossary, originalUnchanged: true, cachedWithoutAi: true, unrelatedManualEditReusedBlocks: true };
    checks.push("real-translation", "manual-glossary", "original-preserved", "stage-and-block-cache");
    console.info("PASS real Codex translation, glossary, untouched original, offline stage/block caches");

    const storyPath = join(root, "learned-names.epub");
    await writeFile(storyPath, zipSync({
        "META-INF/container.xml": strToU8('<container><rootfile full-path="book.opf"/></container>'),
        "book.opf": strToU8('<package><metadata><title>Legal original story</title></metadata><manifest><item id="c1" href="one.html"/><item id="c2" href="two.html"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>'),
        "one.html": strToU8('<h1>月城璃央</h1><p>月城璃央は庭で待っています。月城璃央は静かに本を開きました。</p>'),
        "two.html": strToU8('<h1>月城璃央</h1><p>月城璃央は庭に入りました。月城璃央は猫に挨拶しました。</p>'),
    }));
    const story = await request("reader.local", { path: storyPath }) as ReaderDocument;
    const firstChapter = await request("reader.chapter", { documentId: story.id, chapterId: story.chapters[0]!.id }) as ReaderPage;
    const beforeLearning = phases.filter(phase => phase === "translating").length;
    const firstTranslation = await request("reader.translate", { page: firstChapter }) as ReaderPage;
    const learned = await request("reader.glossary", { documentId: story.id }) as Record<string, string>;
    const name = Object.entries(learned).find(([source]) => source.includes("璃央"));
    require(name?.[1] && /[А-Яа-я]/.test(name[1]), "Real Codex did not learn the recurring proper name");
    const secondChapter = await request("reader.chapter", { documentId: story.id, chapterId: story.chapters[1]!.id }) as ReaderPage;
    const nextTranslation = await request("reader.translate", { page: secondChapter }) as ReaderPage;
    require(nextTranslation.html?.includes(name[1]), "Next chapter did not use the learned name");
    const learnedCount = phases.length;
    await request("reader.translate", { page: firstChapter });
    require(phases.length === learnedCount && phases.filter(phase => phase === "translating").length - beforeLearning === 2,
        "Learning issued extra requests or invalidated a completed chapter");
    evidence.learning = { provider: firstTranslation.provider, model: firstTranslation.model, glossary: learned,
        chapters: [{ original: firstChapter.html, translated: firstTranslation.html },
            { original: secondChapter.html, translated: nextTranslation.html }], requests: 2, cachedWithoutAi: true };
    checks.push("automatic-provider-with-saved-external-model", "learned-name", "next-chapter-consistency", "one-request-per-chapter");
    console.info("PASS real Codex automatic provider with saved external model, learned proper name, next chapter consistency, one request per chapter");

    const imagePath = process.argv[2];
    if (imagePath) {
        const before = contentId(await readFile(imagePath));
        const comic = await request("reader.local", { path: imagePath }) as ReaderDocument;
        const image = await request("reader.chapter", { documentId: comic.id, chapterId: comic.chapters[0]!.id }) as ReaderPage;
        const recognized = await request("reader.translate", { page: image }) as ReaderPage;
        const sourceRecognition = JSON.parse(await readFile(join(context.dataDir, "reader", "ocr", before + ".layout-v2.json"), "utf8")) as {
            regions: RecognizedRegion[];
        };
        const regions = sourceRecognition.regions;
        const layerImage = recognized.images?.[0];
        require(layerImage?.cleanedUrl && layerImage.url === image.images?.[0]?.url, "OCR did not retain a separate cleaned layer");
        const layerTime = (await stat(layerImage.cleanedUrl)).mtimeMs;
        const layer = JSON.parse(await readFile(join(dirname(layerImage.cleanedUrl), "layers.json"), "utf8")) as {
            maskUrl: string; maskedPixels: number;
        };
        const ocrEvidence: Record<string, unknown> = { fixture: resolve(imagePath), sourceSha256: before,
            provider: recognized.provider, model: recognized.model, sourceRegions: regions, regions: recognized.regions,
            width: layerImage.width, height: layerImage.height, maskedPixels: layer.maskedPixels };
        evidence.ocr = ocrEvidence;
        if (process.env.QBUTT_READER_EVIDENCE) {
            const path = resolve(process.env.QBUTT_READER_EVIDENCE);
            const directory = dirname(path);
            const stem = basename(path, extname(path));
            const cleanedUrl = join(directory, stem + ".cleaned.png");
            const maskUrl = join(directory, stem + ".mask.png");
            await mkdir(directory, { recursive: true });
            await Promise.all([copyFile(layerImage.cleanedUrl, cleanedUrl), copyFile(layer.maskUrl, maskUrl)]);
            ocrEvidence.cleanedUrl = cleanedUrl;
            ocrEvidence.maskUrl = maskUrl;
        }
        if (context.appDir) {
            const metrics = await imageMetrics(imagePath, layerImage.cleanedUrl, layer.maskUrl, context.appDir);
            ocrEvidence.metrics = metrics;
            if (metrics) require(metrics.outsideChanges === 0, "Cleanup changed artwork outside its stroke mask");
        }
        require((recognized.regions?.length ?? 0) > 0, "Real OCR returned no regions; pass an image with readable text");
        require(recognized.regions!.every(region => region.x >= 0 && region.y >= 0 && region.x + region.width <= 1.001
            && region.y + region.height <= 1.001 && region.text && region.translation), "OCR geometry/translation failed");
        require(regions.every(region => region.layout && region.inkBoxes?.length > 0
            && ["horizontal", "vertical"].includes(region.orientation) && ["dark", "light"].includes(region.ink)),
        "OCR did not return usable layout and source-ink metadata");
        if (process.argv.includes("--vertical-jp")) {
            require(recognized.regions![0]!.text.replace(/\s/g, "") === "まだ知らない世界がある。"
                && recognized.regions![1]?.text.replace(/\s/g, "") === "一緒なら、きっと見つけられる。"
                && recognized.regions![0]!.x > recognized.regions![1]!.x, "Vertical Japanese transcription or right-to-left balloon order failed");
            require(regions[0]!.orientation === "vertical" && regions[1]!.orientation === "horizontal",
                "OCR did not identify the original vertical and horizontal writing directions");
            if (ocrEvidence.metrics) {
                require(regions[2]?.text.replace(/\s/g, "") === "凛との約束はここにある。" && regions[2].ink === "light",
                    "Light text over the illustrated background was not recognized correctly");
                checks.push("light-text-over-artwork");
            }
            checks.push("vertical-japanese-transcription", "right-to-left-balloon-order", "writing-directions");
            console.info("VERTICAL_JP_REGIONS", JSON.stringify(recognized.regions));
        }
        const regionId = recognized.regions![0]!.id;
        const edited = await request("reader.correct", { documentId: comic.id, pageId: image.id, regionId, text: "Исправленный перевод" }) as ReaderPage;
        require(edited.regions?.[0]?.translation === "Исправленный перевод", "OCR correction did not apply");
        const phasesAfterOcr = phases.filter(phase => phase === "recognizing" || phase === "translating").length;
        const imageGlossary = await request("reader.glossary", { documentId: comic.id }) as Record<string, string>;
        await request("reader.glossary", { documentId: comic.id, entries: { ...imageGlossary, 無関係: "Посторонний термин" } });
        const recomposed = await request("reader.translate", { page: image }) as ReaderPage;
        require(recomposed.regions?.find(region => region.id === regionId)?.translation === "Исправленный перевод", "Glossary edit lost the manual layer");
        require(phases.filter(phase => phase === "recognizing" || phase === "translating").length === phasesAfterOcr,
            "Glossary recomposition repeated OCR or unchanged translation");
        require((await stat(layerImage.cleanedUrl)).mtimeMs === layerTime, "Glossary recomposition repeated image processing");
        require(contentId(await readFile(imagePath)) === before, "OCR changed the original image");
        Object.assign(ocrEvidence, { correctedRegions: recomposed.regions, originalUnchanged: true,
            cachedWithoutAi: true, imageProcessingCacheReused: true });
        checks.push("vision-ocr", "layout-and-source-ink-metadata", "cached-manual-correction", "image-preserved");
        console.info("PASS real vision OCR, normalized regions, translation, editable cached layer, untouched image");
    }
    evidence.success = true;
}
catch (error) {
    evidence.error = error instanceof Error ? error.message : "Integration failed";
    throw error;
}
finally {
    evidence.completedAt = new Date().toISOString();
    evidence.authSha256Equal = await authorizationHash().then(hash => hash === authorizationBefore, () => false);
    try {
        if (process.env.QBUTT_READER_EVIDENCE) {
            const path = resolve(process.env.QBUTT_READER_EVIDENCE);
            await mkdir(dirname(path), { recursive: true });
            await writeFile(path, JSON.stringify(evidence, null, 2), "utf8");
            console.info("READER_EVIDENCE", path);
        }
        console.info("AUTH_SHA256_EQUAL", evidence.authSha256Equal);
    }
    finally {
        await rm(root, { recursive: true, force: true });
    }
}
