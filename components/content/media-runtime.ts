import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat } from "node:fs/promises";
import { join, resolve } from "node:path";

import lock from "../../resources/media/component-lock.json";

export type MediaTool = "mpv" | "ffmpeg" | "ffprobe" | "yt-dlp";

const tools: Record<MediaTool, string> = {
    mpv: "mpv.exe",
    ffmpeg: "tools/ffmpeg.exe",
    ffprobe: "profile/tools/bin/ffprobe.exe",
    "yt-dlp": "tools/yt-dlp.exe",
};
const validated = new Map<string, Promise<void>>();

export function mediaRoot(appDir: string): string {
    const lab = process.env.QBUTT_LAB_PATHS === "1" && process.env.QBUTT_MEDIA_RUNTIME_DIR;
    return resolve(lab || join(appDir, "media"));
}

export async function mediaTool(appDir: string, name: MediaTool, signal: AbortSignal): Promise<string> {
    signal.throwIfAborted();
    const lab = process.env.QBUTT_LAB_PATHS === "1" && process.env.QBUTT_MEDIA_RUNTIME_DIR;
    const root = mediaRoot(appDir);
    const output = tools[name];
    const path = join(root, output);
    const key = lab ? `lab:${path}` : `bundle:${root}`;
    let validation = validated.get(key);
    if (!validation) {
        validation = (async () => {
            const files = lab ? { [output]: "" } : {
                ...lock.runtime.files,
                ...lock.bundledHashes,
            };
            for (const [output, expected] of Object.entries(files)) {
                const file = join(root, output);
                const info = await lstat(file).catch(error => {
                    if (error.code === "ENOENT") return null;
                    throw error;
                });
                if (!info?.isFile() || info.isSymbolicLink())
                    throw new Error(`The bundled media file ${output} is missing. Reinstall qbutt and try again.`);
                if (!lab) {
                    const hash = createHash("sha256");
                    for await (const chunk of createReadStream(file)) hash.update(chunk);
                    if (hash.digest("hex") !== expected)
                        throw new Error(`The bundled media file ${output} failed its integrity check. Reinstall qbutt and try again.`);
                }
            }
        })().catch(error => {
            validated.delete(key);
            throw error;
        });
        validated.set(key, validation);
    }
    await validation;
    signal.throwIfAborted();
    return path;
}
