import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { ProviderContext, ResolvedContent, Variant } from "./contracts";
import type { Catalog } from "./catalog";
import { publicUrl } from "./net";
import type { BrowserBridge } from "./browser";
import { mediaRuntime } from "./media-runtime";
import { captureSelectionId } from "./references";

interface ExtractedFormat {
    url?: string;
    vcodec?: string;
    acodec?: string;
    http_headers?: Record<string, string>;
}

interface ExtractedMedia extends ExtractedFormat {
    title?: string;
    requested_formats?: ExtractedFormat[];
    requested_downloads?: ExtractedFormat[];
}

export async function extractMedia(url: string, headers: Record<string, string>, appDir: string,
    signal: AbortSignal, proxyUrl: string): Promise<ResolvedContent> {
    const source = await publicUrl(url);
    const args = ["--ignore-config", "--no-warnings", "--no-playlist", "--dump-single-json", "--skip-download",
        "--socket-timeout", "20", "--retries", "1", "--format", "bestvideo*+bestaudio/best", "--proxy", proxyUrl];
    let cookies: string | undefined;
    const publicHeaders: Record<string, string> = {};
    for (const [name, value] of Object.entries(headers)) {
        if (!/^[\w-]+$/.test(name) || /[\r\n]/.test(value))
            throw new Error("The source supplied invalid media headers.");
        if (name.toLowerCase() === "authorization")
            throw new Error("Open this source through its browser player to use its signed-in session.");
        if (name.toLowerCase() === "cookie") {
            cookies = value;
            continue;
        }
        publicHeaders[name] = value;
        args.push("--add-headers", `${name}:${value}`);
    }
    args.push("--", url);

    const cookieFile = cookies ? join(tmpdir(), `qbutt-media-${randomUUID()}.cookies`) : undefined;
    if (cookieFile) {
        const lines = cookies!.split(";").map(cookie => {
            const separator = cookie.indexOf("=");
            if (separator < 1 || /[\t\r\n]/.test(cookie))
                throw new Error("The source supplied invalid session cookies.");
            return [source.hostname, "FALSE", "/", source.protocol === "https:" ? "TRUE" : "FALSE", "0",
                cookie.slice(0, separator).trim(), cookie.slice(separator + 1).trim()].join("\t");
        });
        await writeFile(cookieFile, "# Netscape HTTP Cookie File\n" + lines.join("\n") + "\n", { flag: "wx", mode: 0o600 });
        args.unshift("--cookies", cookieFile);
    }

    let metadata: ExtractedMedia;
    try {
        const runtime = await mediaRuntime(appDir, signal);
        metadata = await new Promise<ExtractedMedia>((resolve, reject) => {
            const child = spawn(runtime.tools["yt-dlp"], args, {
                cwd: runtime.runtimeDir, windowsHide: true, stdio: ["ignore", "pipe", "pipe"], signal,
            });
            const output: Buffer[] = [];
            let size = 0;
            const timeout = setTimeout(() => child.kill(), 60_000);
            child.stdout.on("data", chunk => {
                size += chunk.length;
                if (size > 8 * 1024 * 1024)
                    child.kill();
                else
                    output.push(chunk);
            });
            child.stderr.resume();
            child.on("error", reject);
            child.on("close", code => {
                clearTimeout(timeout);
                if (code !== 0) {
                    reject(new Error("This video could not be opened. The source may require a browser session."));
                    return;
                }
                try {
                    resolve(JSON.parse(Buffer.concat(output).toString("utf8")));
                }
                catch {
                    reject(new Error("The video source returned an incomplete stream."));
                }
            });
        });
    }
    finally {
        if (cookieFile)
            await unlink(cookieFile);
    }

    const formats = metadata.requested_formats || metadata.requested_downloads || [metadata];
    const video = formats.find(format => format.url && format.vcodec !== "none") || formats[0];
    const audio = formats.find(format => format.url && format.vcodec === "none" && format.acodec !== "none");
    if (!video?.url)
        throw new Error("This video has no playable stream.");

    await publicUrl(video.url);
    if (audio?.url)
        await publicUrl(audio.url);

    const mediaHeaders = Object.fromEntries(Object.entries({ ...publicHeaders,
        ...metadata.http_headers, ...video.http_headers })
        .filter(([name]) => !["cookie", "authorization"].includes(name.toLowerCase())));
    if (cookies && new URL(video.url).origin === source.origin)
        mediaHeaders.Cookie = cookies;

    return {
        kind: "video",
        resolver: "direct",
        url: video.url,
        audioUrl: audio?.url,
        title: metadata.title,
        headers: mediaHeaders,
    };
}

export async function resolveVariant(variant: Variant, catalog: Catalog,
    signal: AbortSignal, browser: BrowserBridge, proxyUrl: string,
    options: { interactive?: boolean } = {}): Promise<ResolvedContent> {
    return catalog.sourceOperation(variant.sourceId, signal, { method: "resolve", variant },
        context => resolveContent(variant, catalog, signal, browser, proxyUrl, context), options);
}

async function resolveContent(variant: Variant, catalog: Catalog, signal: AbortSignal,
    browser: BrowserBridge, proxyUrl: string, context: ProviderContext): Promise<ResolvedContent> {
    const provider = catalog.providers.find(item => item.source.id === variant.sourceId);
    let resolved: ResolvedContent | undefined;

    if (provider) {
        if (!provider.resolve && (variant.transport === "book" || variant.transport === "manga")) {
            await publicUrl(variant.url);
            return { kind: variant.transport, url: variant.url, title: variant.title };
        }
        if (!provider.resolve)
            throw new Error("This source cannot open the selected variant.");
        resolved = await provider.resolve(variant, context);
    }
    else if (variant.sourceId === "link") {
        const captureId = captureSelectionId(variant);
        const url = captureId ? variant.url
            : (await catalog.settings.values(`acquisition:${variant.id}`)).url || variant.magnet || variant.url;
        if (url.startsWith("magnet:"))
            return { kind: "torrent", url };

        await publicUrl(url);
        const directVideo = /\.(?:m3u8|mpd|mp4|mkv|webm)$/i.test(new URL(url).pathname);
        resolved = { kind: variant.transport, url,
            resolver: captureId ? "browser" : variant.transport === "video" && !directVideo ? "yt-dlp" : "direct",
            ...(captureId ? { selection: { captureId } } : {}),
        };
    }
    else {
        await catalog.engine.request("resolve", { variant, ...await catalog.browserSessions(signal, [variant.sourceId]) }, event => {
            if (event.event === "resolved")
                resolved = event.data as ResolvedContent;
        }, signal);
    }

    if (!resolved)
        throw new Error("The selected variant is unavailable.");

    if (resolved.resolver === "browser") {
        if (!resolved.url)
            throw new Error("The source did not supply a player address.");
        await publicUrl(resolved.url);
        const referrer = Object.entries(resolved.headers || {}).find(([name]) => name.toLowerCase() === "referer")?.[1];
        if (referrer)
            await publicUrl(referrer);
        resolved = { ...resolved, ...await browser.resolve(resolved.url, resolved.selection,
            { signal, kind: resolved.kind === "file" || resolved.kind === "torrent" ? resolved.kind : "video", referrer }) };
    }

    if (resolved.resolver === "yt-dlp") {
        if (!resolved.url)
            throw new Error("The source did not supply a video address.");
        return { ...resolved, ...await extractMedia(resolved.url, resolved.headers || {}, catalog.appDir, signal, proxyUrl), proxyUrl };
    }

    if (resolved.url && !resolved.url.startsWith("magnet:"))
        await publicUrl(resolved.url);
    if (resolved.audioUrl)
        await publicUrl(resolved.audioUrl);
    for (const subtitle of resolved.subtitles || [])
        await publicUrl(subtitle.url);

    if (resolved.kind === "torrent" && !resolved.torrentData && resolved.url && !resolved.url.startsWith("magnet:")) {
        const response = await context.fetch(resolved.url, { headers: resolved.headers });
        if (!response.ok)
            throw new Error(`The torrent source returned HTTP ${response.status}.`);
        const data = Buffer.from(await response.arrayBuffer());
        if (data.length > 8 * 1024 * 1024 || data[0] !== 0x64 || data.at(-1) !== 0x65)
            throw new Error("The source did not return valid torrent metadata.");
        return { kind: "torrent", title: resolved.title, torrentData: data.toString("base64") };
    }

    if ((resolved.kind === "file" || resolved.kind === "video") && resolved.url)
        resolved.headers = await catalog.acquisitionHeaders(variant.sourceId, resolved.url, resolved.headers || {});

    if (resolved.kind === "video")
        resolved.proxyUrl = proxyUrl;

    return resolved;
}
