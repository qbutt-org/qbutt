import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { link, mkdir, open, rm, stat, unlink, writeFile, type FileHandle } from "node:fs/promises";
import { request as httpRequest, type IncomingMessage } from "node:http";
import { request as httpsRequest } from "node:https";
import { basename, extname, join } from "node:path";
import { createBrotliDecompress, createGunzip, createInflate } from "node:zlib";

import { matchesAudio, type AudioMetadata } from "./audio";
import type { ResolvedContent, Variant, Work } from "./contracts";
import { downloadDash } from "./dash";
import { ContentError } from "./errors";
import { mediaRuntime } from "./media-runtime";
import { publicAddresses, safeError } from "./net";
import { NetworkProxy } from "./networkproxy";
import { persistentVariant, persistentWork } from "./references";
import type { LibraryStore } from "./store";

export interface DownloadJob {
    id: string;
    title: string;
    status: "queued" | "downloading" | "complete" | "cancelled" | "error";
    work?: Work;
    variant?: Variant;
    path?: string;
    progress?: number;
    message?: string;
    code?: string;
}

interface EnqueueRequest {
    work?: Work;
    variant?: Variant;
    resolved?: ResolvedContent;
    url?: string;
    title?: string;
    options?: Record<string, unknown>;
}

interface QueueOptions {
    dataDir: string;
    downloadDir: string;
    appDir: string;
    store: LibraryStore;
    resolve(variant: Variant, signal: AbortSignal, options?: Record<string, unknown>): Promise<ResolvedContent>;
    emit(event: "job" | "progress", data: DownloadJob): void;
    // Integration fixtures supply an in-process connector. Product requests and
    // environment variables cannot relax the public-address policy.
    network?: { addresses: typeof publicAddresses };
}

interface PendingDownload {
    request: EnqueueRequest;
    controller: AbortController;
}

const activeStatuses = new Set(["queued", "downloading"]);

function fileName(title: string, extension: string): string {
    let name = title.normalize("NFC").replace(/[<>:"/\\|?*\x00-\x1f]/g, " ")
        .replace(/\s+/g, " ").replace(/[. ]+$/g, "").slice(0, 150).trim();
    if (!name || /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(name))
        name = "Download";
    if (extname(name).toLowerCase() !== extension.toLowerCase())
        name += extension;
    return name;
}

function fileExtension(url: string, kind: ResolvedContent["kind"], contentType: string): string {
    const extension = extname(new URL(url).pathname).toLowerCase();
    if (/^\.[a-z0-9]{1,8}$/.test(extension) && ![".php", ".asp", ".aspx", ".html"].includes(extension))
        return extension;
    const formats: Record<string, string> = { "application/pdf": ".pdf", "application/epub+zip": ".epub",
        "application/zip": ".zip", "application/x-zip-compressed": ".zip", "text/plain": ".txt" };
    if (formats[contentType])
        return formats[contentType];
    return kind === "book" ? ".epub" : ".bin";
}

export class DownloadQueue {
    private readonly records = new Map<string, DownloadJob>();
    private readonly pending = new Map<string, PendingDownload>();
    private processing?: Promise<void>;
    private closed = false;
    private progressSentAt = 0;
    private readonly restored: Promise<void>;

    constructor(private readonly config: QueueOptions) {
        const interrupted: string[] = [];
        for (const saved of config.store.setting<DownloadJob[]>("downloadJobs", [])) {
            if (!/^[0-9a-f-]{36}$/i.test(saved.id))
                continue;
            const job: DownloadJob = { id: saved.id, title: saved.title, status: saved.status,
                work: saved.work, variant: saved.variant, path: saved.path, code: saved.code };
            if (activeStatuses.has(job.status)) {
                job.status = "error";
                job.message = "Download interrupted. Download the original variant again to retry.";
                interrupted.push(job.id);
            }
            if (job.status === "complete" && (!job.path || !existsSync(job.path))) {
                job.status = "error";
                job.message = "The downloaded file is no longer available.";
            }
            if (job.status === "complete")
                job.progress = 1;
            this.records.set(job.id, job);
        }
        this.persist();
        this.restored = Promise.all(interrupted.map(id => this.cleanup(id))).then(() => {});
    }

    jobs(): DownloadJob[] {
        return structuredClone([...this.records.values()]);
    }

    enqueue(request: EnqueueRequest): DownloadJob {
        if (this.closed)
            throw new Error("The download queue is closing.");
        if (request.variant?.transport === "torrent" || request.resolved?.kind === "torrent")
            throw new Error("Torrent downloads belong to the torrent session.");
        if (!request.variant && !request.resolved?.url && !request.url)
            throw new Error("The source did not supply a download address.");

        const variant = request.variant ? persistentVariant(request.variant) : undefined;
        const work = request.work ? persistentWork({ id: request.work.id, title: request.work.title,
            category: request.work.category, variants: variant ? [variant] : [] }) : undefined;
        const job: DownloadJob = { id: crypto.randomUUID(),
            title: request.title || request.work?.title || request.variant?.title || request.resolved?.title || "Download",
            status: "queued", work, variant };
        this.records.set(job.id, job);
        this.pending.set(job.id, { request, controller: new AbortController() });
        this.changed(job);
        queueMicrotask(() => this.start());
        return structuredClone(job);
    }

    cancel(id: string): void {
        const job = this.records.get(id);
        if (!job || !activeStatuses.has(job.status))
            return;
        this.pending.get(id)?.controller.abort(new DOMException("Cancelled", "AbortError"));
        job.status = "cancelled";
        job.message = "Download stopped.";
        this.changed(job);
    }

    async close(): Promise<void> {
        this.closed = true;
        for (const [id, pending] of this.pending) {
            pending.controller.abort(new DOMException("Interrupted", "AbortError"));
            const job = this.records.get(id)!;
            if (activeStatuses.has(job.status)) {
                job.status = "error";
                job.message = "Download interrupted. Download the original variant again to retry.";
                this.changed(job);
            }
        }
        await this.processing;
        await this.restored;
    }

    private persist(): void {
        this.config.store.setSetting("downloadJobs", [...this.records.values()].map(job => ({
            id: job.id, title: job.title, status: job.status, work: job.work, variant: job.variant, path: job.path, code: job.code,
        })));
    }

    private changed(job: DownloadJob): void {
        this.persist();
        this.config.emit("job", structuredClone(job));
        if (job.work) {
            const download = { status: job.status, path: job.path };
            if (this.config.store.get(job.work.id))
                this.config.store.update(job.work.id, { download });
            else
                this.config.store.remember(job.work, { variant: job.variant, download });
        }
    }

    private start(): void {
        if (this.processing || this.closed)
            return;
        this.processing = this.run().finally(() => {
            this.processing = undefined;
            if ([...this.records.values()].some(job => job.status === "queued"))
                this.start();
        });
    }

    private async run(): Promise<void> {
        await this.restored;
        for (const [id, pending] of this.pending) {
            const job = this.records.get(id)!;
            if (job.status !== "queued") {
                this.pending.delete(id);
                continue;
            }
            const signal = pending.controller.signal;
            try {
                job.status = "downloading";
                this.progressSentAt = 0;
                this.changed(job);
                const resolved = pending.request.variant
                    ? await this.config.resolve(pending.request.variant, signal, pending.request.options)
                    : pending.request.resolved || { kind: "file" as const, url: pending.request.url };
                signal.throwIfAborted();
                if (!resolved.url || resolved.kind === "torrent" || resolved.resolver === "browser")
                    throw new Error("This source cannot be saved as a file.");
                await mkdir(this.config.downloadDir, { recursive: true });
                const staging = join(this.config.dataDir, "downloads", id);
                const partial = join(this.config.downloadDir, `.qbutt-${id}.part`);
                const output = await open(partial, "wx");
                await output.close();

                let extension: string;
                if (resolved.kind === "video") {
                    await mkdir(join(this.config.dataDir, "downloads"), { recursive: true });
                    await mkdir(staging);
                    await this.video(job, resolved, staging, partial, signal);
                    extension = ".mkv";
                }
                else {
                    const received = await this.receive(resolved.url, resolved, partial, signal, (bytes, total) => {
                        this.progress(job, total ? bytes / total : undefined);
                    });
                    if (received.playlist || received.dash || received.contentType === "text/html")
                        throw new Error("This link returned a web page or playlist. Open its source to choose a downloadable file.");
                    extension = fileExtension(received.url.href, resolved.kind, received.contentType);
                }
                signal.throwIfAborted();
                if (!(await stat(partial)).size)
                    throw new Error("The source returned an empty file.");
                await this.publish(job, partial, fileName(job.title, extension), signal);
                this.changed(job);
            }
            catch (error) {
                if (job.status === "downloading") {
                    job.status = signal.aborted ? "cancelled" : "error";
                    job.message = signal.aborted ? "Download stopped." : safeError(error);
                    if (!signal.aborted && error instanceof ContentError)
                        job.code = error.code;
                    this.changed(job);
                }
            }
            finally {
                await this.cleanup(id);
                this.pending.delete(id);
            }
        }
    }

    private progress(job: DownloadJob, value?: number): void {
        if (job.status !== "downloading")
            return;
        const next = value === undefined ? undefined : Math.min(0.99, Math.max(0, value));
        if (Date.now() - this.progressSentAt < 200)
            return;
        if (next !== undefined && job.progress !== undefined && next - job.progress < 0.01)
            return;
        job.progress = next;
        this.progressSentAt = Date.now();
        this.config.emit("progress", { id: job.id, title: job.title, status: job.status, progress: next });
    }

    private async cleanup(id: string): Promise<void> {
        await rm(join(this.config.dataDir, "downloads", id), { recursive: true, force: true });
        await rm(join(this.config.downloadDir, `.qbutt-${id}.part`), { force: true });
    }

    private async publish(job: DownloadJob, partial: string, name: string, signal: AbortSignal): Promise<void> {
        const extension = extname(name);
        const stem = basename(name, extension);
        for (let index = 1; index <= 10000; ++index) {
            signal.throwIfAborted();
            const destination = join(this.config.downloadDir, index === 1 ? name : `${stem} (${index})${extension}`);
            try {
                // The completed part is closed and immutable before publication.
                // Creating its final directory entry is atomic and never replaces
                // an existing file; no writable staging shares the published inode.
                await link(partial, destination);
                if (signal.aborted) {
                    await unlink(destination);
                    signal.throwIfAborted();
                }
                // Commit the job in the same turn as publication. A later stop
                // must not turn a completed, visible file into a cancelled job.
                job.path = destination;
                job.status = "complete";
                job.progress = 1;
                job.message = undefined;
                return;
            }
            catch (error) {
                if ((error as NodeJS.ErrnoException).code !== "EEXIST")
                    throw error;
            }
        }
        throw new Error("Choose a different name for this download.");
    }

    private async remote(input: string, resolved: ResolvedContent, signal: AbortSignal): Promise<{ response: IncomingMessage; url: URL }> {
        let url = new URL(input);
        const credentialOrigin = new URL(resolved.url!).origin;
        for (let redirects = 0; redirects <= 5; ++redirects) {
            const addresses = await (this.config.network?.addresses || publicAddresses)(url);
            signal.throwIfAborted();
            const headers: Record<string, string> = { "User-Agent": "qbutt", "Accept-Encoding": "identity" };
            for (const [name, value] of Object.entries(resolved.headers || {})) {
                if (!/^[\w-]+$/.test(name) || /[\r\n]/.test(value))
                    throw new Error("The source supplied invalid download headers.");
                if (/^(?:host|connection|proxy-authorization|proxy-connection)$/i.test(name))
                    continue;
                if (url.origin !== credentialOrigin && !/^(?:user-agent|referer|origin|accept|accept-language)$/i.test(name))
                    continue;
                headers[name] = value;
            }
            const response = await new Promise<IncomingMessage>((resolve, reject) => {
                const send = url.protocol === "https:" ? httpsRequest : httpRequest;
                const connection = send(url, { headers, signal,
                    lookup(_host, options, callback) {
                        if (options.all)
                            callback(null, addresses);
                        else
                            callback(null, addresses[0].address, addresses[0].family);
                    },
                }, resolve);
                connection.setTimeout(30_000, () => connection.destroy(new Error("The download stopped responding.")));
                connection.on("error", reject);
                connection.end();
            });
            if ([301, 302, 303, 307, 308].includes(response.statusCode || 0)) {
                const location = response.headers.location;
                response.destroy();
                if (!location)
                    throw new Error("The source returned an incomplete redirect.");
                url = new URL(location, url);
                continue;
            }
            if ((response.statusCode || 0) < 200 || (response.statusCode || 0) >= 300) {
                response.destroy();
                throw new Error(`The source could not provide this file (HTTP ${response.statusCode}).`);
            }
            return { response, url };
        }
        throw new Error("The source redirected too many times.");
    }

    private async receive(input: string, resolved: ResolvedContent, destination: string, signal: AbortSignal,
        progress?: (bytes: number, total?: number) => void): Promise<{ url: URL; contentType: string; playlist?: string; dash?: string }> {
        const { response, url } = await this.remote(input, resolved, signal);
        const contentType = String(response.headers["content-type"] || "").split(";")[0].trim().toLowerCase();
        const playlist = /\.m3u8$/i.test(url.pathname) || /(?:mpegurl|vnd\.apple\.mpegurl)/i.test(contentType);
        const dash = /\.mpd$/i.test(url.pathname) || contentType === "application/dash+xml";
        const encoding = response.headers["content-encoding"];
        const decoder = encoding === "gzip" ? createGunzip() : encoding === "br" ? createBrotliDecompress()
            : encoding === "deflate" ? createInflate() : undefined;
        if (encoding && encoding !== "identity" && !decoder) {
            response.destroy();
            throw new Error("The source returned an unsupported file encoding.");
        }
        const payload = decoder ? response.pipe(decoder) : response;
        if (decoder)
            response.on("error", error => decoder.destroy(error));
        const total = decoder ? undefined : Number(response.headers["content-length"]) || undefined;
        let bytes = 0;
        const chunks: Buffer[] = [];
        let file: FileHandle | undefined;
        try {
            if (!playlist && !dash)
                file = await open(destination, "w");
            for await (const chunk of payload) {
                signal.throwIfAborted();
                bytes += chunk.length;
                if (playlist || dash) {
                    if (bytes > 4 * 1024 * 1024)
                        throw new Error("The video playlist is too large.");
                    chunks.push(chunk);
                }
                else
                    await file!.writeFile(chunk);
                progress?.(bytes, total);
            }
            if (total !== undefined && bytes !== total)
                throw new Error("The source returned an incomplete file.");
            const manifest = playlist || dash ? Buffer.concat(chunks).toString("utf8") : undefined;
            return { url, contentType, playlist: playlist ? manifest : undefined, dash: dash ? manifest : undefined };
        }
        finally {
            response.destroy();
            decoder?.destroy();
            await file?.close();
        }
    }

    private async video(job: DownloadJob, resolved: ResolvedContent, staging: string, partial: string,
        signal: AbortSignal): Promise<void> {
        const resources = new Map<string, string>();
        const renditions = new Map<string, AudioMetadata>();
        let gateway: NetworkProxy | undefined;
        const materialize = async (input: string, ancestors = new Set<string>()): Promise<string> => {
            if (ancestors.has(input) || ancestors.size > 8 || resources.size >= 10000)
                throw new Error("The video playlist is recursive or too large.");
            if (resources.has(input))
                return resources.get(input)!;
            const local = join(staging, `stream-${resources.size}.bin`);
            resources.set(input, local);
            const received = await this.receive(input, resolved, local, signal, () => this.progress(job));
            if (received.dash) {
                gateway ||= await NetworkProxy.start({ network: this.config.network,
                    payload: { sourceUrl: resolved.url!, headers: resolved.headers || {}, signal } });
                const manifest = await downloadDash(received.dash, received.url.href,
                    (url, ids) => gateway!.resource(url, ids));
                const playlist = local.replace(/\.bin$/, ".mpd");
                await writeFile(playlist, manifest);
                resources.set(input, playlist);
                return playlist;
            }
            if (!received.playlist)
                return local;
            const nextAncestors = new Set([...ancestors, input]);
            let lines = received.playlist.replace(/^\uFEFF/, "").split(/\r?\n/);
            if (lines[0] !== "#EXTM3U")
                throw new Error("The source returned an invalid video playlist.");
            const variants = lines.flatMap((line, index) => line.startsWith("#EXT-X-STREAM-INF:")
                ? [{ index, bandwidth: Number(/(?:^|,)BANDWIDTH=(\d+)/.exec(line.slice(18))?.[1]) || 0 }] : []);
            let selectedAudio: AudioMetadata | undefined;
            if (variants.length) {
                const selected = variants.sort((a, b) => b.bandwidth - a.bandwidth)[0];
                const audio = /AUDIO="([^"]+)"/.exec(lines[selected.index])?.[1];
                const audioTracks = lines.filter(line => line.startsWith("#EXT-X-MEDIA:") && line.includes("TYPE=AUDIO")
                    && (!audio || line.includes(`GROUP-ID="${audio}"`)));
                let chosen: string | undefined;
                if (resolved.audioTrack && audioTracks.length) {
                    const matches = audioTracks.filter(line => matchesAudio({
                        title: /(?:^|,)NAME="([^"]*)"/.exec(line.slice(13))?.[1],
                        language: /(?:^|,)LANGUAGE="([^"]*)"/.exec(line.slice(13))?.[1],
                    }, { ...resolved.audioTrack!, codec: undefined, channels: undefined }));
                    if (matches.length !== 1)
                        throw new Error("The selected voice cannot be identified uniquely in this stream. Choose another voice or player.");
                    chosen = matches[0];
                    selectedAudio = { title: /(?:^|,)NAME="([^"]*)"/.exec(chosen.slice(13))?.[1],
                        language: /(?:^|,)LANGUAGE="([^"]*)"/.exec(chosen.slice(13))?.[1] };
                }
                else
                    chosen = audioTracks.find(line => line.includes("DEFAULT=YES")) || audioTracks[0];
                lines = ["#EXTM3U", ...(chosen ? [chosen] : []), lines[selected.index], lines[selected.index + 1]];
            }
            else if (!lines.includes("#EXT-X-ENDLIST"))
                throw new Error("A live stream cannot be saved as a complete video.");
            const rewritten: string[] = [];
            for (const line of lines) {
                if (!line.trim())
                    continue;
                if (!line.startsWith("#")) {
                    rewritten.push(basename(await materialize(new URL(line, received.url).href, nextAncestors)));
                    continue;
                }
                let rewrittenLine = line;
                for (const match of [...line.matchAll(/URI="([^"]+)"/g)]) {
                    const path = await materialize(new URL(match[1], received.url).href, nextAncestors);
                    rewrittenLine = rewrittenLine.replace(match[0], `URI="${basename(path)}"`);
                }
                if (/URI=(?!")/.test(rewrittenLine))
                    throw new Error("The source returned an unsupported video reference.");
                rewritten.push(rewrittenLine);
            }
            const playlist = local.replace(/\.bin$/, ".m3u8");
            await writeFile(playlist, rewritten.join("\n") + "\n");
            resources.set(input, playlist);
            if (selectedAudio)
                renditions.set(playlist, selectedAudio);
            return playlist;
        };

        try {
            const video = await materialize(resolved.url!);
            const audio = resolved.audioUrl ? await materialize(resolved.audioUrl) : undefined;
            const subtitles: { path: string; language?: string; title: string }[] = [];
            for (const subtitle of resolved.subtitles || []) {
                subtitles.push({ path: await materialize(subtitle.url), language: subtitle.language, title: subtitle.title });
            }
            const inputs = [video, ...(audio ? [audio] : []), ...subtitles.map(item => item.path)];
            const protocols = gateway ? "file,http,tcp,crypto,data" : "file,crypto,data";
            const hlsOptions = ["-allowed_extensions", "ALL", "-allowed_segment_extensions", "ALL", "-extension_picky", "0"];
            const environment = gateway ? { ...Object.fromEntries(Object.entries(process.env)
                .filter(([name]) => !["no_proxy", "http_proxy"].includes(name.toLowerCase()))),
                http_proxy: gateway.url, no_proxy: "" } : process.env;
            let selectedStream: number | undefined;
            if (resolved.audioTrack) {
                const input = audio || video;
                const runtime = await mediaRuntime(this.config.appDir, signal);
                const streams = await new Promise<{ index: number; codec_type: string; codec_name?: string;
                    channels?: number; tags?: { title?: string; language?: string } }[]>((resolve, reject) => {
                    const child = spawn(runtime.tools.ffprobe, ["-v", "error",
                        "-protocol_whitelist", protocols, ...(input.endsWith(".m3u8") ? hlsOptions : []),
                        "-show_streams", "-of", "json", input], {
                        cwd: runtime.runtimeDir,
                        windowsHide: true, stdio: ["ignore", "pipe", "pipe"], signal, env: environment,
                    });
                    const timeout = setTimeout(() => child.kill(), 30000);
                    let output = "";
                    let childError: Error | undefined;
                    child.stdout.setEncoding("utf8");
                    child.stdout.on("data", chunk => {
                        output += chunk;
                        if (output.length > 2 * 1024 * 1024)
                            child.kill();
                    });
                    child.stderr.resume();
                    child.on("error", error => { childError = error; });
                    child.on("close", code => {
                        clearTimeout(timeout);
                        if (childError || code !== 0) {
                            reject(childError || new Error("The selected voice could not be verified. Choose another voice or player."));
                            return;
                        }
                        try {
                            const metadata = JSON.parse(output);
                            if (!Array.isArray(metadata.streams))
                                throw new Error("Missing streams");
                            resolve(metadata.streams);
                        }
                        catch {
                            reject(new Error("The stream returned incomplete audio information."));
                        }
                    });
                });
                const rendition = renditions.get(input);
                const matches = streams.filter(stream => stream.codec_type === "audio" && matchesAudio({
                    title: rendition?.title || stream.tags?.title,
                    language: stream.tags?.language || rendition?.language,
                    codec: stream.codec_name, channels: stream.channels,
                }, resolved.audioTrack!));
                if (matches.length !== 1)
                    throw new Error("The selected voice cannot be identified uniquely in this stream. Choose another voice or player.");
                selectedStream = matches[0].index;
            }
            const args = ["-nostdin", "-hide_banner", "-loglevel", "error", "-y"];
            for (const input of inputs) {
                args.push("-protocol_whitelist", protocols);
                if (input.endsWith(".m3u8"))
                    args.push(...hlsOptions);
                args.push("-i", input);
            }
            args.push("-map", "0:v?", "-map", selectedStream !== undefined
                ? `${audio ? 1 : 0}:${selectedStream}` : audio ? "1:a:0" : "0:a?");
            if (selectedStream !== undefined)
                args.push("-disposition:a:0", "default");
            for (let index = 0; index < subtitles.length; ++index) {
                args.push("-map", `${index + (audio ? 2 : 1)}:s:0`);
                if (subtitles[index].language)
                    args.push(`-metadata:s:s:${index}`, `language=${subtitles[index].language}`);
                args.push(`-metadata:s:s:${index}`, `title=${subtitles[index].title}`);
            }
            args.push("-c", "copy", "-c:s", "srt", "-f", "matroska", partial);
            const runtime = await mediaRuntime(this.config.appDir, signal);
            await new Promise<void>((resolve, reject) => {
                const child = spawn(runtime.tools.ffmpeg, args, {
                    cwd: runtime.runtimeDir,
                    windowsHide: true, stdio: ["ignore", "ignore", "pipe"], signal,
                    env: environment,
                });
                let diagnostic = "";
                child.stderr.setEncoding("utf8");
                child.stderr.on("data", chunk => { diagnostic = (diagnostic + chunk).slice(-2000); });
                let childError: Error | undefined;
                child.on("error", error => { childError = error; });
                child.on("close", code => {
                    if (childError)
                        reject(childError);
                    else if (code === 0)
                        resolve();
                    else
                        reject(new Error(diagnostic ? `This video could not be saved: ${safeError(new Error(diagnostic))}` : "This video could not be saved."));
                });
            });
        }
        finally {
            await gateway?.close();
        }
    }
}
