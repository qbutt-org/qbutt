import { join } from "node:path";
import { mkdir } from "node:fs/promises";
import type { Event, ResolvedContent, Variant, WorkDetails } from "../../components/content/contracts.ts";
import { verifyLiveDownload } from "../content-online/download.ts";
import { debuggerSocket, observePlayer } from "./frames.ts";

const [lab, appDir, animeId = "10661", chosenIds = "687385", mode = "browser"] = Bun.argv.slice(2);
if (!lab || !appDir || !/^\d+$/.test(animeId) || !/^\d+(?:,\d+)*$/.test(chosenIds)
    || !["browser", "public"].includes(mode))
    throw new Error("Usage: bun live.ts <isolated-lab-directory> <portable-app-directory> [anime-id] [exact-video-ids] [browser|public]");
const videoIds = chosenIds.split(",");
const mediaRuntime = process.env.QBUTT_MEDIA_RUNTIME_DIR ?? "";
if (!mediaRuntime)
    throw new Error("Set QBUTT_MEDIA_RUNTIME_DIR to the separately prepared pinned lab media tools.");

const debugUrl = "http://127.0.0.1:43902";
const helper = Bun.spawn([join(lab, "qbutt-content.exe"), "--data-dir", join(lab, "profile"),
    "--download-dir", join(lab, "downloads"), "--app-dir", appDir], {
    cwd: lab, stdin: "pipe", stdout: "pipe", stderr: "ignore", windowsHide: true,
    env: { ...process.env, QBUTT_LAB_PATHS: "1", QBUTT_MEDIA_RUNTIME_DIR: mediaRuntime },
});
const requests = new Map<number, { events: Event[]; finish(events: Event[]): void; fail(error: Error): void }>();
let nextId = 0;
const read = (async () => {
    let buffered = "";
    const decoder = new TextDecoder();
    for await (const chunk of helper.stdout) {
        buffered += decoder.decode(chunk, { stream: true });
        let end: number;
        while ((end = buffered.indexOf("\n")) !== -1) {
            const event = JSON.parse(buffered.slice(0, end)) as Event;
            buffered = buffered.slice(end + 1);
            const pending = requests.get(event.id);
            if (!pending) continue;
            pending.events.push(event);
            if (event.event === "complete" || event.event === "error") {
                requests.delete(event.id);
                pending.finish(pending.events);
            }
        }
    }
    for (const pending of requests.values()) pending.fail(new Error("Isolated helper exited before replying."));
    requests.clear();
})();

function request(method: string, params: Record<string, unknown> = {}): Promise<Event[]> {
    const id = ++nextId;
    return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
            requests.delete(id);
            reject(new Error(`Helper request timed out: ${method}`));
        }, 70_000);
        requests.set(id, { events: [],
            finish(events) { clearTimeout(timeout); resolve(events); },
            fail(error) { clearTimeout(timeout); reject(error); },
        });
        helper.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
        helper.stdin.flush();
    });
}

const pause = (milliseconds: number) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const safeMessage = (value: unknown) => String(value ?? "").replace(/https?:\/\/\S+/g, "[URL]").slice(0, 1200);
let browser: ReturnType<typeof Bun.spawn> | undefined;

async function publicMedia(playerUrl: string): Promise<string> {
    const player = new URL(playerUrl);
    if (player.origin !== "https://player.aksor.tv" || !/^\/video\/[a-z\d]+$/i.test(player.pathname))
        throw new Error("The public acceptance requires the exact Aksor player URL.");
    const response = await fetch(new URL(player.pathname.replace("/video/", "/api/video/"), player.origin), {
        headers: { Accept: "application/json", Referer: player.href }, signal: AbortSignal.timeout(15_000),
    });
    if (!response.ok) throw new Error(`The independent selected Aksor API returned HTTP ${response.status}.`);
    const config = await response.json() as { anime_id: number; episode: string; studio_name: string;
        is_subtitle: boolean; qualities: Record<string, string | null> };
    if (String(config.anime_id) !== animeId || String(config.episode) !== "2"
        || config.studio_name !== "AniLibria" || config.is_subtitle !== false)
        throw new Error("The independent Aksor API returned a different anime, episode, voice or subtitle variant.");
    const quality = ["q4k", "q2k", "q1080", "q720", "q480", "q360"].find(key => config.qualities[key]);
    if (!quality) throw new Error("The independent selected API provided no quality.");
    const url = new URL(config.qualities[quality]!);
    if (url.protocol !== "https:" || !url.pathname.endsWith(".mpd"))
        throw new Error("The independent selected API did not provide a DASH media address.");
    return url.href;
}

async function targets(required = false): Promise<{ type: string; url: string; webSocketDebuggerUrl: string }[]> {
    try {
        const response = await fetch(`${debugUrl}/json/list`, { signal: AbortSignal.timeout(1000) });
        if (!response.ok) throw new Error("The isolated browser target snapshot failed.");
        const result = await response.json();
        if (!Array.isArray(result)) throw new Error("The isolated browser target snapshot was invalid.");
        return result;
    } catch (error) {
        if (required) throw error;
        return [];
    }
}

async function diagnose(target: { webSocketDebuggerUrl: string }, expectedUrl: string): Promise<unknown> {
    return new Promise((resolve) => {
        const socket = new WebSocket(target.webSocketDebuggerUrl);
        const done = (value?: unknown) => { clearTimeout(timeout); socket.close(); resolve(value); };
        const timeout = setTimeout(() => done(), 1000);
        socket.addEventListener("error", () => done(), { once: true });
        socket.addEventListener("open", () => socket.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: {
            returnByValue: true,
            expression: `JSON.stringify({origin:location.origin,title:document.title,text:document.body?.innerText?.slice(0,200),
                exactSelectedFrame:[...document.querySelectorAll('iframe[src]')].some(f=>f.name==='qbutt:player'&&f.src===${JSON.stringify(expectedUrl)}),
                frames:[...document.querySelectorAll('iframe[src]')].map(f=>{try{return new URL(f.src).origin}catch{return 'invalid'}}),
                media:[...document.querySelectorAll('video,audio')].map(m=>({ready:m.readyState,paused:m.paused,error:m.error?.code,
                    source:m.currentSrc?.startsWith('blob:')?'blob':m.currentSrc?new URL(m.currentSrc).origin:''})),
                playControls:[...document.querySelectorAll('button,[role=button],.play_button,.play-button,.vjs-big-play-button')].slice(0,12).map(b=>({class:b.className,text:b.innerText?.slice(0,50)})),
                failedResources:performance.getEntriesByType('resource').filter(r=>r.responseStatus>=400).slice(0,8).map(r=>({origin:new URL(r.name).origin,status:r.responseStatus}))})`,
        } })), { once: true });
        socket.addEventListener("message", (message) => {
            const reply = JSON.parse(String(message.data));
            if (reply.id === 1) {
                try { done(JSON.parse(reply.result?.result?.value)); } catch { done(); }
            }
        });
    });
}

async function decode(resolved: ResolvedContent): Promise<Record<string, unknown>> {
    if (!resolved.url || !resolved.proxyUrl) throw new Error("Actual helper omitted media URL or guarded proxy.");
    const headers = Object.entries(resolved.headers ?? {}).map(([key, value]) => `${key}: ${value}\r\n`).join("");
    const input = ["-http_proxy", resolved.proxyUrl, "-rw_timeout", "10000000", "-headers", headers,
        "-protocol_whitelist", "http,https,httpproxy,tcp,tls,crypto", "-i"];
    const args = [join(mediaRuntime, "tools", "ffmpeg.exe"), "-hide_banner", "-loglevel", "error", "-nostats",
        "-threads", "2", ...input, resolved.url];
    if (resolved.audioUrl) args.push(...input, resolved.audioUrl, "-map", "0:v:0", "-map", "1:a:0");
    args.push("-t", "3", "-progress", "pipe:1", "-f", "null", "-");
    const child = Bun.spawn(args, { cwd: mediaRuntime, stdout: "pipe", stderr: "pipe", windowsHide: true });
    const timeout = setTimeout(() => child.kill(), 30_000);
    const [progress, error, code] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
    clearTimeout(timeout);
    const frames = Math.max(0, ...[...progress.matchAll(/^frame=(\d+)$/gm)].map(match => Number(match[1])));
    return { ok: code === 0 && frames > 0, exitCode: code, frames, secondsLimit: 3,
        ...(code || !frames ? { error: safeMessage(error) } : {}) };
}

try {
    if (mode === "browser") {
        await mkdir(join(lab, "edge-profile"), { recursive: true });
        await request("configureSource", { id: "browser", settings: { browser: "edge" } });
        browser = Bun.spawn(["C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
            "--start-minimized", "--no-first-run", "--no-default-browser-check",
            "--remote-debugging-port=43902", `--user-data-dir=${join(lab, "edge-profile")}`,
            `--load-extension=${join(lab, "browser-extension", "chromium")}`, "about:blank"], {
            stdin: "ignore", stdout: "ignore", stderr: "ignore", windowsHide: true,
        });
        let ready = false;
        for (let attempt = 0; attempt < 100; attempt++) {
            const events = await request("browserStatus");
            const status = events.find(event => event.event === "browser")?.data as { ready?: boolean; userAgent?: string } | undefined;
            if (status?.ready) {
                if (!status.userAgent?.includes("Edg/")) throw new Error("Authenticated Edge user agent missing.");
                ready = true;
                break;
            }
            await pause(500);
        }
        if (!ready) throw new Error("Isolated Edge extension did not connect.");
    } else {
        const status = (await request("browserStatus")).find(event => event.event === "browser")?.data as { ready?: boolean };
        if (!status || status.ready) throw new Error("The public acceptance requires no connected browser extension.");
        console.log(JSON.stringify({ stage: "browser", connected: false, mode }));
    }

    const catalogVariant: Variant = { id: `yani:${animeId}`, sourceId: "yani", sourceName: "YummyAnime",
        title: "Anonymous upstream check", transport: "video", url: `https://yani.tv/catalog/item/${animeId}` };
    const inspected = await request("inspect", { variant: catalogVariant });
    const details = inspected.find(event => event.event === "details")?.data as WorkDetails | undefined;
    if (!details?.variants?.length) throw new Error(`Yani inspect failed: ${safeMessage(inspected.at(-1)?.data)}`);
    const metadata = await (await fetch(`https://api.yani.tv/anime/${animeId}?need_videos=true`, {
        headers: { Accept: "application/json" }, signal: AbortSignal.timeout(15_000),
    })).json() as { response: { videos: { video_id: number; iframe_url: string }[] } };
    const players = [...new Set(details.variants.map(variant => variant.player).filter(Boolean))];
    console.log(JSON.stringify({ stage: "catalog", animeId, title: details.title, variants: details.variants.length, players }));
    let failedSelection = false;
    for (const id of videoIds) {
        const selected = details.variants.find(variant => variant.id === `yani:${animeId}:video:${id}`);
        if (!selected || selected.episode !== "2" || selected.translation !== "AniLibria") {
            throw new Error(`Exact video ${id} does not identify the requested episode 2 / AniLibria.`);
        }
        const player = selected.player;
        const videoId = Number(selected.id.split(":")[3]);
        const expected = metadata.response.videos.find(video => video.video_id === videoId)?.iframe_url;
        if (!expected) throw new Error("Selected variant ID missing from live upstream metadata.");
        const expectedUrl = new URL(expected, "https://yani.tv").href;
        const expectedMedia = mode === "public" ? await publicMedia(expectedUrl) : undefined;
        const observed = new Set<string>();
        const diagnostics = new Map<string, unknown>();
        let exactFrame = false;
        let watching = mode === "browser";
        const watch = (async () => {
            while (watching) {
                for (const target of await targets()) {
                    if (target.type !== "page" || target.url === "about:blank") continue;
                    observed.add(target.url);
                    const diagnostic = await diagnose(target, expectedUrl) as { exactSelectedFrame?: boolean } | undefined;
                    if (diagnostic) {
                        diagnostics.set(target.url, diagnostic);
                        exactFrame ||= !!diagnostic.exactSelectedFrame;
                    }
                }
                await pause(350);
            }
        })();
        let stopObserving: () => Promise<unknown[]> = async () => [];
        let events: Event[];
        let playerDiagnostics: unknown[];
        try {
            if (mode === "browser") stopObserving = await observePlayer(debugUrl, expectedUrl);
            events = await request("resolve", { variant: selected });
        }
        finally {
            playerDiagnostics = await stopObserving();
            watching = false;
            await watch;
        }
        const failed = events.find(event => event.event === "error")?.data as { code?: string; message?: string; url?: string } | undefined;
        const resolved = events.find(event => event.event === "resolved")?.data as ResolvedContent | undefined;
        const evidence = { stage: "resolve", mode, variantId: selected.id, episode: selected.episode,
            translation: selected.translation, player, sourceOrigin: new URL(expectedUrl).origin,
            ...(mode === "browser" ? { exactSelectedPageObserved: observed.has(expectedUrl), exactSelectedFrameObserved: exactFrame }
                : { exactPublicMedia: resolved?.url === expectedMedia, direct: resolved?.resolver === "direct" }) };
        if (failed || !resolved?.url) {
            failedSelection = true;
            console.log(JSON.stringify({ ...evidence, ok: false, code: failed?.code, message: safeMessage(failed?.message),
                errorPreservesSelection: failed?.url === expectedUrl, diagnostics: [...diagnostics.values()], playerDiagnostics }));
            continue;
        }
        const identity = mode === "browser" ? exactFrame : resolved.url === expectedMedia && resolved.resolver === "direct";
        const decoded = await decode(resolved);
        failedSelection ||= !decoded.ok || !identity;
        console.log(JSON.stringify({ ...evidence, ok: decoded.ok && identity, mediaOrigin: new URL(resolved.url).origin,
            mediaPath: new URL(resolved.url).pathname, separateAudio: !!resolved.audioUrl, decode: decoded }));
        if (decoded.ok && identity) {
            console.log(JSON.stringify({ stage: "download", ...await verifyLiveDownload(request, lab, selected) }));
        }
    }
    if (mode === "browser") {
        const remaining = await targets(true);
        const captureTabsClosed = remaining.every(target => target.type !== "page" || target.url === "about:blank");
        failedSelection ||= !captureTabsClosed;
        console.log(JSON.stringify({ stage: "cleanup", remainingPages: remaining.filter(target => target.type === "page").length,
            captureTabsClosed }));
    }
    if (failedSelection) process.exitCode = 1;
} finally {
    if (browser) try {
        const socket = await debuggerSocket(debugUrl);
        socket.send(JSON.stringify({ id: 1, method: "Browser.close" }));
        await pause(300);
        socket.close();
    } catch { browser?.kill(); }
    helper.stdin.end();
    const timeout = setTimeout(() => helper.kill(), 5000);
    await helper.exited;
    clearTimeout(timeout);
    await read;
}
