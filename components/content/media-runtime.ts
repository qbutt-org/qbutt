import { execFile } from "node:child_process";
import { lstat } from "node:fs/promises";
import { isAbsolute, resolve } from "node:path";
import { promisify } from "node:util";

import { productExecutable, validateProductVersion } from "./products";
import { ContentError } from "./errors";

export type MediaTool = "mpv" | "ffmpeg" | "ffprobe" | "yt-dlp";

interface MediaRuntime {
    player: string;
    runtimeDir: string;
    tools: Record<MediaTool, string>;
}

const runtimes = new Map<string, Promise<MediaRuntime>>();

export async function mediaRuntime(appDir: string, signal: AbortSignal): Promise<MediaRuntime> {
    signal.throwIfAborted();
    const lab = process.env.QBUTT_LAB_PATHS === "1";
    const labPlayer = lab ? process.env.QBUTT_LAB_QPLAY : undefined;
    if (lab && (!labPlayer || !isAbsolute(labPlayer)))
        throw new ContentError("qplay_unavailable", "Set QBUTT_LAB_QPLAY to the absolute path of the verified qplay fixture.");
    const key = labPlayer ? `lab:${resolve(labPlayer)}` : resolve(appDir);
    let pending = runtimes.get(key);
    if (!pending) {
        pending = (async () => {
            const player = labPlayer ? resolve(labPlayer) : await productExecutable(appDir);
            if (!player) throw new ContentError("qplay_unavailable", "qplay is not installed. Reinstall qbutt or install qplay and try again.");
            const { stdout } = await promisify(execFile)(player, ["--runtime-info-json"], {
                windowsHide: true, timeout: 30_000, maxBuffer: 64 * 1024,
            });
            const info = JSON.parse(stdout);
            validateProductVersion(info);
            const tools: Record<MediaTool, string> = {
                mpv: info.tools?.mpv,
                ffmpeg: info.tools?.ffmpeg,
                ffprobe: info.tools?.ffprobe,
                "yt-dlp": info.tools?.ytDlp,
            };
            if (typeof info.runtimeDir !== "string" || !isAbsolute(info.runtimeDir))
                throw new ContentError("qplay_unavailable", "qplay returned an invalid runtime directory. Reinstall qplay and try again.");
            for (const path of Object.values(tools)) {
                if (typeof path !== "string" || !isAbsolute(path) || !(await lstat(path)).isFile())
                    throw new ContentError("qplay_unavailable", "qplay media tools are missing. Reinstall qplay and try again.");
            }
            return { player, runtimeDir: info.runtimeDir, tools };
        })().catch(error => {
            runtimes.delete(key);
            if (!(error instanceof ContentError))
                throw new ContentError("qplay_unavailable", "qplay could not start. Reinstall qplay or run the qbutt installer again.");
            throw error;
        });
        runtimes.set(key, pending);
    }
    const runtime = await pending;
    signal.throwIfAborted();
    return runtime;
}

export async function mediaTool(appDir: string, name: MediaTool, signal: AbortSignal): Promise<string> {
    return (await mediaRuntime(appDir, signal)).tools[name];
}
