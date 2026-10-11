import { spawn } from "node:child_process";
import { mkdir, readFile, rename, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { ProviderContext } from "../contracts";
import { mediaRuntime } from "../media-runtime";
import { contentId } from "./types";
import type { RecognizedRegion } from "./translation";

interface Raster {
    width: number;
    height: number;
    pixels: Uint8Array;
}

interface ImageLayer {
    cleanedUrl: string;
    maskUrl: string;
    width: number;
    height: number;
}

const MAX_PIXELS = 16_000_000;
const PIPELINE = 3;

async function ffmpeg(context: ProviderContext, args: string[], input?: Uint8Array): Promise<Uint8Array> {
    const appDir = context.appDir;
    if (!appDir) throw new Error("The bundled media tools are unavailable. Reinstall qbutt and try again.");
    const runtime = await mediaRuntime(appDir, context.signal);
    const signal = AbortSignal.any([context.signal, AbortSignal.timeout(30_000)]);
    return new Promise((resolve, reject) => {
        const child = spawn(runtime.tools.ffmpeg, ["-hide_banner", "-loglevel", "error", ...args],
            { cwd: runtime.runtimeDir, windowsHide: true, signal, stdio: ["pipe", "pipe", "pipe"] });
        const chunks: Buffer[] = [];
        let size = 0;
        let overflow = false;
        child.stdout.on("data", (chunk: Buffer) => {
            size += chunk.length;
            if (size > MAX_PIXELS * 4 + 4096) {
                overflow = true;
                child.kill();
            }
            else chunks.push(chunk);
        });
        // Drain diagnostics without exposing local filenames or retaining unbounded output.
        child.stderr.resume();
        child.stdin.on("error", () => {});
        child.on("error", () => reject(signal.aborted ? signal.reason
            : new Error("Не удалось запустить обработку изображения. Повторите попытку; оригинал сохранён.")));
        child.on("close", (code) => {
            if (signal.aborted) reject(signal.reason);
            else if (overflow) reject(new Error("Страница превышает предел обработки 16 мегапикселей. Оригинал сохранён."));
            else if (code !== 0) reject(new Error("Не удалось обработать изображение. Оригинал сохранён."));
            else resolve(Buffer.concat(chunks, size));
        });
        child.stdin.end(input);
    });
}

async function decode(path: string, context: ProviderContext): Promise<Raster> {
    const bytes = await ffmpeg(context, ["-max_pixels", String(MAX_PIXELS), "-i", path,
        "-frames:v", "1", "-pix_fmt", "rgba", "-f", "image2pipe", "-c:v", "pam", "pipe:1"]);
    const headerEnd = Buffer.from(bytes.buffer, bytes.byteOffset, Math.min(bytes.length, 4096)).indexOf("ENDHDR\n");
    const offset = headerEnd + 7;
    const header = new TextDecoder().decode(bytes.subarray(0, offset));
    const width = Number(/^WIDTH (\d+)$/m.exec(header)?.[1]);
    const height = Number(/^HEIGHT (\d+)$/m.exec(header)?.[1]);
    if (headerEnd < 0 || !header.startsWith("P7\n") || !/^DEPTH 4$/m.test(header) || !/^MAXVAL 255$/m.test(header)
        || !Number.isInteger(width) || !Number.isInteger(height) || width <= 0 || height <= 0
        || width * height > MAX_PIXELS || bytes.length - offset !== width * height * 4) {
        throw new Error("Некорректные размеры изображения. Оригинал сохранён.");
    }
    return { width, height, pixels: bytes.slice(offset) };
}

function luminance(pixels: Uint8Array, index: number): number {
    const alpha = pixels[index * 4 + 3]! / 255;
    const color = (pixels[index * 4]! * 77 + pixels[index * 4 + 1]! * 150 + pixels[index * 4 + 2]! * 29) / 256;
    return Math.floor(color * alpha + 255 * (1 - alpha));
}

async function inkMask(raster: Raster, regions: RecognizedRegion[], signal: AbortSignal): Promise<Uint8Array> {
    const { width, height, pixels } = raster;
    const mask = new Uint8Array(width * height);
    let boxes = 0;
    for (const region of regions) {
        for (const box of region.inkBoxes) {
            signal.throwIfAborted();
            const left = Math.max(0, Math.floor(box.x * width));
            const top = Math.max(0, Math.floor(box.y * height));
            const right = Math.min(width, Math.ceil((box.x + box.width) * width));
            const bottom = Math.min(height, Math.ceil((box.y + box.height) * height));
            const histogram = new Uint32Array(256);
            let samples = 0;
            for (let y = top; y < bottom; ++y) {
                for (let x = left; x < right; ++x) {
                    ++histogram[luminance(pixels, y * width + x)];
                    ++samples;
                }
            }
            // The lighter/darker quartile estimates the background inside a tight ink box.
            // A contrast threshold removes strokes, never the whole rectangle or balloon.
            let cumulative = 0;
            let background = 0;
            const percentile = samples * (region.ink === "dark" ? 0.8 : 0.2);
            for (; background < 255; ++background) {
                cumulative += histogram[background]!;
                if (cumulative >= percentile) break;
            }
            cumulative = 0;
            let ink = 0;
            const inkPercentile = samples * (region.ink === "dark" ? 0.02 : 0.98);
            for (; ink < 255; ++ink) {
                cumulative += histogram[ink]!;
                if (cumulative >= inkPercentile) break;
            }
            // Separate the actual ink peak from its local background. A fixed contrast
            // offset also selects weaker illustration lines underneath bright captions.
            const threshold = (background + ink) / 2;
            for (let y = top; y < bottom; ++y) {
                for (let x = left; x < right; ++x) {
                    const value = luminance(pixels, y * width + x);
                    if (region.ink === "dark" ? value >= threshold : value <= threshold) continue;
                    // Include antialiasing at the stroke edge, constrained to the OCR ink box.
                    for (let dy = -1; dy <= 1; ++dy) {
                        for (let dx = -1; dx <= 1; ++dx) {
                            const xx = x + dx;
                            const yy = y + dy;
                            if (xx >= left && xx < right && yy >= top && yy < bottom) mask[yy * width + xx] = 255;
                        }
                    }
                }
            }
            if (++boxes % 16 === 0) await Bun.sleep(0);
        }
    }
    return mask;
}

async function repairMaskedPixels(raster: Raster, mask: Uint8Array, signal: AbortSignal): Promise<Uint8Array> {
    const { width, height } = raster;
    const pixels = raster.pixels.slice();
    const known = new Uint8Array(mask.length);
    const queue = new Int32Array(mask.reduce((sum, pixel) => sum + Number(pixel > 0), 0));
    let head = 0;
    let tail = 0;
    for (let index = 0; index < mask.length; ++index) known[index] = mask[index] ? 0 : 1;
    const neighbours = (index: number) => {
        const x = index % width;
        return [x > 0 ? index - 1 : -1, x + 1 < width ? index + 1 : -1,
            index >= width ? index - width : -1, index + width < mask.length ? index + width : -1];
    };
    for (let index = 0; index < mask.length; ++index) {
        if (!known[index] && ((index % width > 0 && known[index - 1] === 1)
            || (index % width + 1 < width && known[index + 1] === 1)
            || (index >= width && known[index - width] === 1)
            || (index + width < mask.length && known[index + width] === 1))) {
            queue[tail++] = index;
            known[index] = 2;
        }
        if (index % 262144 === 0) {
            signal.throwIfAborted();
            await Bun.sleep(0);
        }
    }
    while (head < tail) {
        const index = queue[head++]!;
        const x = index % width;
        const y = Math.floor(index / width);
        const sums = [0, 0, 0, 0];
        let weights = 0;
        for (let dy = -3; dy <= 3; ++dy) {
            for (let dx = -3; dx <= 3; ++dx) {
                const xx = x + dx;
                const yy = y + dy;
                if ((!dx && !dy) || xx < 0 || yy < 0 || xx >= width || yy >= height) continue;
                const nearby = yy * width + xx;
                if (known[nearby] !== 1) continue;
                const weight = 1 / Math.pow(dx * dx + dy * dy, 1.5);
                weights += weight;
                for (let channel = 0; channel < 4; ++channel) sums[channel]! += pixels[nearby * 4 + channel]! * weight;
            }
        }
        if (!weights) throw new Error("Не удалось восстановить фон текста. Оригинал сохранён.");
        for (let channel = 0; channel < 4; ++channel) pixels[index * 4 + channel] = Math.round(sums[channel]! / weights);
        known[index] = 1;
        for (const next of neighbours(index)) {
            if (next >= 0 && known[next] === 0) {
                known[next] = 2;
                queue[tail++] = next;
            }
        }
        if (head % 8192 === 0) {
            signal.throwIfAborted();
            await Bun.sleep(0);
        }
    }
    signal.throwIfAborted();
    return pixels;
}

async function encode(path: string, raster: Raster, pixels: Uint8Array, gray: boolean, context: ProviderContext): Promise<void> {
    const bytes = await ffmpeg(context, ["-f", "rawvideo", "-pixel_format", gray ? "gray" : "rgba",
        "-video_size", `${raster.width}x${raster.height}`, "-i", "pipe:0", "-frames:v", "1",
        "-f", "image2pipe", "-c:v", "png", "pipe:1"], pixels);
    const temporary = `${path}.${crypto.randomUUID()}.tmp`;
    await writeFile(temporary, bytes);
    await rename(temporary, path);
}

export async function cleanImageText(path: string, regions: RecognizedRegion[], context: ProviderContext): Promise<ImageLayer> {
    const inkArea = regions.reduce((sum, region) => sum + region.inkBoxes.reduce((area, box) => area + box.width * box.height, 0), 0);
    if (inkArea > 2) {
        throw new Error("Распознавание вернуло слишком много перекрывающихся областей. Повторите перевод; оригинал сохранён.");
    }
    const original = await readFile(path);
    const key = contentId(JSON.stringify({ original: contentId(original),
        ink: regions.map(region => ({ ink: region.ink, boxes: region.inkBoxes })), pipeline: PIPELINE }));
    const directory = join(context.dataDir, "reader", "image-layers", key);
    const metadataPath = join(directory, "layers.json");
    try {
        const prior = JSON.parse(await readFile(metadataPath, "utf8")) as ImageLayer;
        if ((await stat(prior.cleanedUrl)).size && (await stat(prior.maskUrl)).size) return prior;
    }
    catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    const raster = await decode(path, context);
    context.signal.throwIfAborted();
    const mask = await inkMask(raster, regions, context.signal);
    const cleaned = await repairMaskedPixels(raster, mask, context.signal);
    const layer: ImageLayer = { cleanedUrl: join(directory, "cleaned.png"), maskUrl: join(directory, "mask.png"),
        width: raster.width, height: raster.height };
    await mkdir(directory, { recursive: true });
    await encode(layer.maskUrl, raster, mask, true, context);
    await encode(layer.cleanedUrl, raster, cleaned, false, context);
    context.signal.throwIfAborted();
    const temporary = `${metadataPath}.${crypto.randomUUID()}.tmp`;
    await writeFile(temporary, JSON.stringify(layer));
    await rename(temporary, metadataPath);
    return layer;
}
