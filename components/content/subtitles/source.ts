import { lstat, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import type { ProviderContext } from "../contracts";
import { mediaRoot, mediaTool } from "../media-runtime";
import { NetworkProxy } from "../networkproxy";
import { contentId } from "../reader/types";

export interface SubtitleRequest {
    identity: string;
    url: string;
    headers?: Record<string, string>;
    track: { id: number; ffIndex?: number; codec?: string; language?: string; url?: string };
    position: number;
    startTime?: number;
    targetLanguage: string;
}

const textLimit = 8 * 1024 * 1024;

export function subtitleRequest(params: Record<string, unknown>): SubtitleRequest {
    const request = params as unknown as SubtitleRequest;
    if (typeof request.identity !== "string" || !request.identity || request.identity.length > 2048
        || typeof request.url !== "string" || request.url.length > 16384
        || !request.track || !Number.isInteger(request.track.id) || request.track.id < 0
        || (request.track.ffIndex !== undefined && (!Number.isInteger(request.track.ffIndex)
            || request.track.ffIndex < 0 || request.track.ffIndex > 512))
        || (request.track.url !== undefined && (typeof request.track.url !== "string" || request.track.url.length > 16384))
        || !Number.isFinite(request.position) || request.position < 0 || request.position > 7 * 86400
        || (request.startTime !== undefined && (!Number.isFinite(request.startTime) || Math.abs(request.startTime) > 1e9))
        || (request.targetLanguage !== undefined && request.targetLanguage !== "ru")) {
        throw new Error("The subtitle request is incomplete.");
    }
    for (const value of [request.track.codec, request.track.language]) {
        if (value !== undefined && (typeof value !== "string" || value.length > 128))
            throw new Error("The subtitle track is invalid.");
    }
    if (request.headers && (typeof request.headers !== "object" || Array.isArray(request.headers)
        || Object.keys(request.headers).length > 64 || Object.entries(request.headers).some(([name, value]) =>
            !/^[\w-]+$/.test(name) || typeof value !== "string" || value.length > 16384 || /[\r\n]/.test(value)))) {
        throw new Error("The source supplied invalid subtitle headers.");
    }
    return { ...request, targetLanguage: "ru" };
}

function ownedStream(url: URL): boolean {
    return url.protocol === "http:" && url.hostname === "127.0.0.1" && Number(url.port) > 0
        && (/^\/media\/[a-f\d]{32}\//i.test(url.pathname) || /^\/_qbutt\/[a-f\d]{48}\//i.test(url.pathname));
}

async function inputAddress(input: string): Promise<{ url: URL; fingerprint: string }> {
    const url = new URL(input);
    if (url.username || url.password)
        throw new Error("Subtitle addresses cannot contain credentials.");
    if (url.protocol === "file:") {
        const path = fileURLToPath(url);
        const file = await lstat(path);
        if (!file.isFile() || file.isSymbolicLink())
            throw new Error("The subtitle source is not a regular file.");
        return { url, fingerprint: `${file.size}:${file.mtimeMs}` };
    }
    if (url.protocol !== "http:" && url.protocol !== "https:")
        throw new Error("This subtitle address is unsupported.");
    return { url, fingerprint: "" };
}

export async function subtitleSourceKey(request: SubtitleRequest): Promise<string> {
    const media = await inputAddress(request.url);
    const external = request.track.url ? await inputAddress(request.track.url) : undefined;
    // The torrent publication and file index survive a regenerated private stream token.
    const mediaIdentity = /^torrent:[a-f\d]{40,64}:\d+$/i.test(request.identity) && ownedStream(media.url)
        ? request.identity : media.url.href;
    return contentId(JSON.stringify({ identity: request.identity, media: mediaIdentity,
        fingerprint: media.fingerprint, external: external?.url.href, externalFingerprint: external?.fingerprint,
        track: { id: request.track.id, ffIndex: request.track.ffIndex, codec: request.track.codec },
        startTime: request.track.url ? 0 : request.startTime || 0, pipeline: 2 }));
}

async function scopedInput(input: string, request: SubtitleRequest, signal: AbortSignal) {
    const { url } = await inputAddress(input);
    if (url.protocol === "file:" || ownedStream(url))
        return { url, close: async () => {} };

    // Reuse the media payload proxy: pinned public addresses and origin-scoped headers
    // remain owned by one component, including redirects and range requests.
    const source = new URL(request.url);
    const headers = Object.fromEntries(Object.entries(request.headers || {}).filter(([name]) =>
        url.origin === source.origin || /^(?:user-agent|referer|origin|accept|accept-language)$/i.test(name)));
    const proxy = await NetworkProxy.start({ payload: { sourceUrl: url.href, headers, signal } });
    try {
        return { url: new URL(await proxy.resource(url.href)), close: () => proxy.close() };
    }
    catch (error) {
        await proxy.close();
        throw error;
    }
}

function decodeSubtitle(bytes: Uint8Array): string {
    const encoding = bytes[0] === 0xff && bytes[1] === 0xfe ? "utf-16le"
        : bytes[0] === 0xfe && bytes[1] === 0xff ? "utf-16be" : "utf-8";
    return new TextDecoder(encoding, { fatal: true }).decode(bytes);
}

async function boundedText(response: Response, signal: AbortSignal): Promise<string> {
    if (!response.ok || !response.body)
        throw new Error(`The subtitle source returned HTTP ${response.status}.`);
    if (Number(response.headers.get("content-length")) > textLimit) {
        await response.body.cancel();
        throw new Error("The subtitle file is too large.");
    }
    const chunks: Uint8Array[] = [];
    let size = 0;
    const reader = response.body.getReader();
    try {
        while (true) {
            signal.throwIfAborted();
            const { done, value } = await reader.read();
            if (done)
                break;
            size += value.byteLength;
            if (size > textLimit)
                throw new Error("The subtitle file is too large.");
            chunks.push(value);
        }
    }
    finally {
        await reader.cancel().catch(() => {});
        reader.releaseLock();
    }
    return decodeSubtitle(Buffer.concat(chunks));
}

export async function loadSubtitleSource(request: SubtitleRequest, start: number, end: number,
    context: ProviderContext): Promise<{ text: string; formatHint: string; complete: boolean }> {
    if (request.track.url) {
        const signal = AbortSignal.any([context.signal, AbortSignal.timeout(25_000)]);
        const input = await scopedInput(request.track.url, request, signal);
        try {
            let text: string;
            if (input.url.protocol === "file:") {
                const file = await lstat(fileURLToPath(input.url));
                if (file.size > textLimit)
                    throw new Error("The subtitle file is too large.");
                text = decodeSubtitle(await readFile(fileURLToPath(input.url)));
            }
            else {
                text = await boundedText(await fetch(input.url, { signal, redirect: "error" }), signal);
            }
            return { text, formatHint: new URL(request.track.url).pathname.split(".").at(-1) || "", complete: true };
        }
        finally {
            await input.close();
        }
    }
    if (request.track.ffIndex === undefined)
        throw new Error("The player did not identify the embedded subtitle stream.");

    if (!context.appDir)
        throw new Error("The bundled subtitle tools could not be located.");
    const ffmpeg = await mediaTool(context.appDir, "ffmpeg", context.signal);
    const signal = AbortSignal.any([context.signal, AbortSignal.timeout(45_000)]);
    const input = await scopedInput(request.url, request, signal);
    try {
        const origin = request.startTime || 0;
        const args = [ffmpeg, "-nostdin", "-hide_banner", "-v", "error",
            "-protocol_whitelist", input.url.protocol === "file:" ? "file,pipe" : "http,https,tcp,tls,crypto,pipe",
            "-format_whitelist", "matroska,webm,mov,mp4,mpegts,avi,ogg,flv,mpeg,mxf,asf,rm",
            "-rw_timeout", "10000000", "-seek_timestamp", "1", "-ss", String(start + origin), "-copyts", "-i",
            input.url.protocol === "file:" ? fileURLToPath(input.url) : input.url.href,
            "-to", String(end + origin), "-output_ts_offset", String(-origin), "-avoid_negative_ts", "disabled",
            "-map", `0:${request.track.ffIndex}`, "-vn", "-an", "-c:s", "ass", "-f", "ass", "pipe:1"];
        const child = Bun.spawn(args, {
            cwd: mediaRoot(context.appDir), stdin: "ignore", stdout: "pipe", stderr: "pipe", windowsHide: true,
        });
        const abort = () => child.kill();
        signal.addEventListener("abort", abort, { once: true });
        try {
            signal.throwIfAborted();
            const [text, , exit] = await Promise.all([
                boundedText(new Response(child.stdout), signal),
                boundedText(new Response(child.stderr), signal),
                child.exited,
            ]);
            signal.throwIfAborted();
            if (exit !== 0)
                throw new Error("The embedded subtitle stream could not be read. Playback can continue with the original track.");
            return { text, formatHint: "ass", complete: false };
        }
        finally {
            signal.removeEventListener("abort", abort);
            child.kill();
            await child.exited;
        }
    }
    finally {
        await input.close();
    }
}
