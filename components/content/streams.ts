import type { ProviderContext, ResolvedContent } from "./contracts";
import type { publicAddresses } from "./net";
import { NetworkProxy } from "./networkproxy";

// Native players use the helper's guarded Node transport for HLS resources.
// The helper retains one prepared video; downloads keep their original source.
export class PlaybackStreams {
    private generation = 0;
    private current?: {
        gateway: NetworkProxy;
        controller: AbortController;
        url: string;
        original: ResolvedContent;
    };

    constructor(private readonly network?: { addresses: typeof publicAddresses }) {}

    async close(): Promise<void> {
        ++this.generation;
        const current = this.current;
        this.current = undefined;
        current?.controller.abort();
        await current?.gateway.close();
    }

    original(resolved: ResolvedContent): ResolvedContent {
        const current = this.current;
        if (resolved.kind !== "video" || !current || resolved.url !== current.url)
            return resolved;
        return { ...current.original, audioTrack: resolved.audioTrack || current.original.audioTrack };
    }

    async prepare(resolved: ResolvedContent, context: ProviderContext): Promise<ResolvedContent> {
        if (resolved.kind !== "video")
            return resolved;
        context.signal.throwIfAborted();
        if (!resolved.url || !/\.m3u8$/i.test(new URL(resolved.url).pathname)) {
            await this.close();
            return resolved;
        }

        const generation = ++this.generation;
        const assertCurrent = () => {
            context.signal.throwIfAborted();
            if (generation !== this.generation)
                throw new DOMException("Playback preparation was replaced.", "AbortError");
        };
        const controller = new AbortController();
        const gateway = await NetworkProxy.start({ network: this.network, payload: {
            sourceUrl: resolved.url, headers: resolved.headers || {}, signal: controller.signal,
        } });
        const resources = new Map<string, string>();
        const origin = new URL(resolved.url).origin;
        let manifestCount = 0;
        const playlist = async (input: string, ancestors = new Set<string>()): Promise<string> => {
            assertCurrent();
            if (ancestors.has(input) || ancestors.size > 8)
                throw new Error("The video playlist is recursive or too large.");
            const cached = resources.get(input);
            if (cached)
                return cached;
            if (++manifestCount > 64)
                throw new Error("The video playlist contains too many manifests.");
            const headers = Object.fromEntries(Object.entries(resolved.headers || {})
                .filter(([name]) => new URL(input).origin === origin
                    || /^(?:user-agent|referer|origin|accept|accept-language)$/i.test(name)));
            const response = await context.fetch(input, { headers });
            const body = await response.text();
            if (!response.ok || Buffer.byteLength(body) > 4 * 1024 * 1024)
                throw new Error("The source could not provide this video playlist.");
            const lines = body.replace(/^\uFEFF/, "").split(/\r?\n/);
            if (lines[0] !== "#EXTM3U")
                throw new Error("The source returned an invalid video playlist.");
            const master = lines.some(line => line.startsWith("#EXT-X-STREAM-INF:"));
            if (!master && !lines.includes("#EXT-X-ENDLIST"))
                throw new Error("The source did not provide a complete video playlist.");

            const next = new Set([...ancestors, input]);
            const reference = async (address: string, nested: boolean): Promise<string> => {
                assertCurrent();
                const target = new URL(address, response.url).href;
                if (nested)
                    return playlist(target, next);
                let local = resources.get(target);
                if (!local) {
                    local = await gateway.resource(target);
                    resources.set(target, local);
                }
                return local;
            };
            for (let index = 1; index < lines.length; ++index) {
                const line = lines[index];
                if (line && !line.startsWith("#")) {
                    lines[index] = await reference(line, master);
                    continue;
                }
                const uri = /(?:^|,)URI="([^"]+)"/.exec(line.slice(line.indexOf(":") + 1));
                if (!uri)
                    continue;
                const nested = /^#EXT-X-(?:MEDIA|I-FRAME-STREAM-INF):/.test(line);
                lines[index] = line.replace(`URI="${uri[1]}"`, `URI="${await reference(uri[1], nested)}"`);
            }
            const local = gateway.manifest(lines.join("\n"));
            resources.set(input, local);
            return local;
        };

        try {
            const url = await playlist(resolved.url);
            const audioUrl = resolved.audioUrl
                ? await (/\.m3u8$/i.test(new URL(resolved.audioUrl).pathname)
                    ? playlist(resolved.audioUrl) : gateway.resource(resolved.audioUrl)) : undefined;
            const subtitles = resolved.subtitles ? await Promise.all(resolved.subtitles.map(async subtitle => ({
                ...subtitle, url: await (/\.m3u8$/i.test(new URL(subtitle.url).pathname)
                    ? playlist(subtitle.url) : gateway.resource(subtitle.url)),
            }))) : undefined;
            assertCurrent();
            const previous = this.current;
            this.current = { gateway, controller, url, original: resolved };
            previous?.controller.abort();
            await previous?.gateway.close();
            assertCurrent();
            return { ...resolved, url, audioUrl, subtitles, headers: {}, proxyUrl: gateway.url };
        }
        catch (error) {
            controller.abort();
            await gateway.close();
            throw error;
        }
    }
}
