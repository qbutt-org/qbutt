import { MAX_MESSAGE_BYTES, NATIVE_HOST, publicUrl, type BrowserCommand, type BrowserId, type BrowserResponse } from "../../content/browser/protocol.ts";
import { bestHlsVariant, hlsIsEncrypted, parseHlsMaster } from "./hls.ts";
import { classifyMedia, type Capture } from "./media.ts";

interface Frame {
    url: string;
    id?: number;
    status?: number;
    ambiguous?: boolean;
}

interface Probe {
    tabId: number;
    page: Frame;
    selected: Frame;
    resources: Map<string, Capture>;
    headers: Map<string, Record<string, string>>;
    controller: AbortController;
    ready: boolean;
}

const probes = new Map<number, Probe>();
const operations = new Map<number, AbortController>();
let port: chrome.runtime.Port | undefined;
const delay = (milliseconds: number, signal?: AbortSignal) => new Promise<void>((resolve, reject) => {
    signal?.throwIfAborted();
    const timer = setTimeout(() => { signal?.removeEventListener("abort", abort); resolve(); }, milliseconds);
    const abort = () => { clearTimeout(timer); reject(signal?.reason ?? new Error("Отменено")); };
    signal?.addEventListener("abort", abort, { once: true });
});

class SourceError extends Error {
    constructor(readonly code: string, message: string, readonly url?: string) { super(message); }
}

async function boundedText(response: Response): Promise<string> {
    const limit = 512 * 1024;
    if (Number(response.headers.get("content-length") ?? 0) > limit) throw new Error("Ответ источника слишком большой.");
    const reader = response.body?.getReader();
    if (!reader) return "";
    const decoder = new TextDecoder();
    let size = 0;
    let text = "";
    try {
        while (true) {
            const chunk = await reader.read();
            if (chunk.done) return text + decoder.decode();
            size += chunk.value.byteLength;
            if (size > limit) throw new Error("Ответ источника слишком большой.");
            text += decoder.decode(chunk.value, { stream: true });
        }
    } finally { await reader.cancel().catch(() => undefined); }
}

function browserId(): BrowserId {
    if (/Firefox\//.test(navigator.userAgent)) return "firefox";
    return /Edg\//.test(navigator.userAgent) ? "edge" : "chrome";
}

function connect(): void {
    if (port) return;
    try {
        const connection = chrome.runtime.connectNative(NATIVE_HOST);
        port = connection;
        connection.onDisconnect.addListener(() => {
            void chrome.runtime.lastError;
            if (port !== connection) return;
            port = undefined;
            for (const controller of operations.values()) controller.abort();
            operations.clear();
        });
        connection.onMessage.addListener((command) => void dispatch(command, connection));
        connection.postMessage({ type: "hello", browser: browserId(), version: 1, userAgent: navigator.userAgent });
    } catch { port = undefined; }
}

async function cookies(url: string) {
    return (await chrome.cookies.getAll({ url })).filter((cookie) => !cookie.partitionKey).map((cookie) => ({
        name: cookie.name,
        value: cookie.value,
        domain: cookie.domain,
        path: cookie.path,
        secure: cookie.secure,
        hostOnly: cookie.hostOnly,
        ...(cookie.expirationDate === undefined ? {} : { expirationDate: cookie.expirationDate }),
    }));
}

async function browserFetch(command: BrowserCommand, signal: AbortSignal) {
    const response = await fetch(publicUrl(command.url!), { ...command.init, credentials: "include", signal });
    publicUrl(response.url);
    const body = await boundedText(response);
    if ([401, 403, 429].includes(response.status) || /<title>\s*(?:just a moment|attention required)|cf-chl-|challenge-platform|id=["']captcha/i.test(body)) {
        throw new SourceError("verification_required", "Откройте источник в браузере, завершите вход или проверку и повторите действие.", command.url);
    }
    return { status: response.status, url: response.url, headers: Object.fromEntries(response.headers), body };
}

async function loaded(tabId: number, signal: AbortSignal): Promise<void> {
    for (let attempt = 0; attempt < 80; attempt++) {
        signal.throwIfAborted();
        if ((await chrome.tabs.get(tabId)).status === "complete") return;
        // A source page may keep an advertising frame loading indefinitely.
        // Its own DOM is sufficient for ordinary document navigation.
        try { if ((await commandTab(tabId, { type: "ready" }))?.ready) return; }
        catch { /* The new document has not received its content script yet. */ }
        await delay(100, signal);
    }
    throw new Error("Плеер не загрузился.");
}

async function commandTab(tabId: number, message: unknown, frameId = 0): Promise<any> {
    return chrome.tabs.sendMessage(tabId, message, { frameId });
}

async function backgroundTab(url: string): Promise<chrome.tabs.Tab> {
    const windows = (await chrome.windows.getAll({ windowTypes: ["normal"] })).filter((window) => !window.incognito);
    const window = windows.find((candidate) => candidate.focused) ?? windows[0];
    if (window?.id === undefined) throw new SourceError("browser_required", "Откройте обычное окно выбранного браузера и повторите действие.");
    return chrome.tabs.create({ url, active: false, windowId: window.id });
}

async function commandFrame(probe: Probe, message: unknown): Promise<any> {
    if (probe.selected.id === undefined || probe.selected.ambiguous) {
        throw new SourceError("selection_unavailable", "Источник не открыл выбранный плеер однозначно.");
    }
    return commandTab(probe.tabId, message, probe.selected.id);
}

async function mountFrame(probe: Probe, url: string, sourceUrl: string): Promise<void> {
    probe.selected = { url };
    probe.resources.clear();
    probe.headers.clear();
    // Authorize only the requested child before document_start. Its initial
    // manifest request may finish before the player's DOM is ready.
    probe.ready = true;
    const mounted = await commandTab(probe.tabId, { type: "frame:mount", url });
    if (mounted?.error) throw new SourceError(mounted.code, mounted.error, sourceUrl);
    for (let attempt = 0; attempt < 80; attempt++) {
        probe.controller.signal.throwIfAborted();
        if (probe.selected.id !== undefined) {
            const state = await commandFrame(probe, { type: "ready" }).catch((error) => {
                if (error instanceof SourceError) throw error;
                return undefined;
            });
            if (state?.url && state.url !== url) throw new SourceError("selected_content_changed", "Источник перенаправил выбранный плеер на другую страницу.", sourceUrl);
            if (state?.ready) {
                await confirmSelection(probe, sourceUrl);
                return;
            }
        }
        await delay(100, probe.controller.signal);
    }
    throw new SourceError("selection_unavailable", "Выбранный плеер не загрузился в источнике.", sourceUrl);
}

async function navigateFromPage(probe: Probe, url: string, sourceUrl: string): Promise<void> {
    const previousUrl = (await chrome.tabs.get(probe.tabId)).url;
    probe.page.url = url;
    probe.page.status = undefined;
    const navigation = await commandTab(probe.tabId, { type: "navigate", url });
    if (navigation?.error) throw new SourceError(navigation.code, navigation.error, sourceUrl);
    let navigated = false;
    for (let attempt = 0; attempt < 80; attempt++) {
        if ((await chrome.tabs.get(probe.tabId)).url !== previousUrl) { navigated = true; break; }
        await delay(100, probe.controller.signal);
    }
    if (!navigated) throw new SourceError("selection_unavailable", "Источник не открыл выбранный плеер.", sourceUrl);
    await loaded(probe.tabId, probe.controller.signal);
    await confirmSelection(probe, sourceUrl);
}

async function fileMetadata(url: string, sourceUrl: string, kind: "file" | "torrent", signal: AbortSignal) {
    let response = await fetch(publicUrl(url), { method: "HEAD", credentials: "include", signal });
    if (response.status === 405 || response.status === 501) {
        response = await fetch(publicUrl(url), { headers: { Range: "bytes=0-0" }, credentials: "include", signal });
    }
    try {
        publicUrl(response.url);
        if (!response.ok) return undefined;
        const type = response.headers.get("content-type") ?? "";
        const disposition = response.headers.get("content-disposition") ?? "";
        const torrent = /application\/x-bittorrent/i.test(type) || /filename[^;]*\.torrent(?:["';]|$)/i.test(disposition);
        const downloadable = torrent || /attachment/i.test(disposition)
            || /^(?:application\/(?:octet-stream|zip|x-rar|vnd\.rar|x-7z-compressed|x-msdownload|x-iso9660-image)|video\/|audio\/)/i.test(type);
        if (!downloadable || kind === "torrent" && !torrent) return undefined;
        const session = await cookies(response.url);
        const headers: Record<string, string> = { Referer: sourceUrl, "User-Agent": navigator.userAgent };
        if (session.length) headers.Cookie = session.map((cookie) => `${cookie.name}=${cookie.value}`).join("; ");
        return { kind: torrent ? "torrent" : "file", resolver: "direct", url: response.url, headers };
    } finally { await response.body?.cancel().catch(() => undefined); }
}

async function resolveFile(command: BrowserCommand, controller: AbortController) {
    const sourceUrl = publicUrl(command.url!);
    const kind = command.kind === "torrent" ? "torrent" : "file";
    const direct = await fileMetadata(sourceUrl, sourceUrl, kind, controller.signal);
    if (direct) return { ...direct, selection: command.selection ?? {} };
    const response = await fetch(sourceUrl, { credentials: "include", signal: controller.signal });
    if (!response.ok) throw new SourceError([401, 403, 429].includes(response.status) ? "verification_required" : "source_unavailable",
        `Выбранный источник вернул HTTP ${response.status}. Откройте источник и выберите доступное зеркало.`, sourceUrl);
    publicUrl(response.url);
    const body = await boundedText(response);
    const tab = await backgroundTab(chrome.runtime.getURL("parser.html"));
    if (tab.id === undefined) throw new Error("Не удалось открыть источник в фоне.");
    try {
        await loaded(tab.id, controller.signal);
        const links = await chrome.runtime.sendMessage({ type: "file:parse", url: response.url, body }) as { urls: string[]; verification?: boolean };
        if (links?.verification) throw new SourceError("verification_required", "Завершите вход или проверку на выбранном источнике и повторите действие.", sourceUrl);
        if (!Array.isArray(links?.urls) || !links.urls.length) {
            throw new SourceError("file_unavailable", "Источник не отдал ссылку на файл. Откройте выбранное зеркало и повторите действие.", sourceUrl);
        }
        if (links.urls.length !== 1) {
            throw new SourceError("selection_unavailable", "Откройте выбранное зеркало и выберите точный файл для загрузки.", sourceUrl);
        }
        const file = await fileMetadata(links.urls[0]!, sourceUrl, kind, controller.signal);
        if (!file) throw new SourceError("file_unavailable", "Источник не отдал доступный файл. Откройте выбранное зеркало и повторите действие.", sourceUrl);
        return { ...file, selection: command.selection ?? {} };
    } finally { await chrome.tabs.remove(tab.id).catch(() => undefined); }
}

async function hlsPlaylist(probe: Probe, url: string, visited = new Set<string>()): Promise<{
    url: string;
    audioUrl?: string;
    subtitles?: { title: string; url: string; language?: string }[];
    family: string[];
    master: boolean;
}> {
    publicUrl(url);
    if (visited.has(url) || visited.size >= 4) throw new Error("Плеер вернул зацикленный HLS-манифест.");
    visited.add(url);
    let body = probe.resources.get(url)?.body;
    let finalUrl = url;
    if (!body) {
        const response = await fetch(url, { credentials: "include", signal: probe.controller.signal });
        if (!response.ok) throw new SourceError([401, 403, 429].includes(response.status) ? "verification_required" : "selection_unavailable",
            "Выбранное качество видео недоступно. Обновите источник и повторите действие.");
        finalUrl = publicUrl(response.url);
        body = await boundedText(response);
    }
    if (!/^#EXTM3U/m.test(body)) throw new Error("Плеер не отдал HLS-манифест выбранного качества.");
    if (hlsIsEncrypted(body)) throw new Error("Зашифрованный поток не поддерживается.");
    const master = parseHlsMaster(body, finalUrl);
    const best = bestHlsVariant(master.variants);
    if (!best) {
        if (!/#EXTINF:/.test(body)) throw new Error("HLS-манифест не содержит видео.");
        let pendingSegment = false;
        let segments = 0;
        for (const line of body.split(/\r?\n/).map((line) => line.trim())) {
            if (line.startsWith("#EXTINF:")) {
                if (pendingSegment) throw new Error("HLS-манифест содержит пустой сегмент.");
                pendingSegment = true;
            }
            if (line && !line.startsWith("#")) {
                publicUrl(new URL(line, finalUrl).href);
                if (!pendingSegment) throw new Error("HLS-манифест не подтвердил длительность сегмента.");
                pendingSegment = false;
                segments++;
            }
            for (const uri of line.matchAll(/\bURI="([^"]+)"/g)) publicUrl(new URL(uri[1]!, finalUrl).href);
        }
        if (!segments || pendingSegment) throw new Error("HLS-манифест не содержит адресов видео.");
        return { url: finalUrl, family: [url, finalUrl], master: false };
    }
    const selected = await hlsPlaylist(probe, best.url, new Set(visited));
    const audio = best.audioGroup ? master.renditions.filter((item) => item.type === "audio" && item.groupId === best.audioGroup && item.url) : [];
    const selectedAudio = audio.length === 1 ? audio[0] : audio.find((item) => item.default);
    if (audio.length > 1 && !selectedAudio) throw new Error("Плеер не подтвердил выбранную аудиодорожку.");
    const audioPlaylist = selectedAudio?.url ? await hlsPlaylist(probe, selectedAudio.url, new Set(visited)) : undefined;
    const audioUrl = audioPlaylist?.url ?? selected.audioUrl;
    const renditions = master.renditions.flatMap((item) => item.url ? [publicUrl(item.url)] : []);
    return {
        ...selected, master: true,
        ...(audioUrl ? { audioUrl } : {}),
        family: [url, finalUrl, ...master.variants.map((variant) => variant.url), ...selected.family,
            ...renditions, ...(audioPlaylist?.family ?? [])],
        subtitles: master.renditions.filter((item) => item.type === "subtitles" && item.url)
            .map((item) => ({ title: item.name, url: publicUrl(item.url!), language: item.language })),
    };
}

async function mediaCandidate(probe: Probe, resource: Capture) {
    let url = publicUrl(resource.url);
    let body = resource.body ?? "";
    let kind = classifyMedia(url, resource.contentType, body);
    if (kind === "hls" || kind === "dash") {
        if (!body) {
            const response = await fetch(url, { credentials: "include", signal: probe.controller.signal });
            if (!response.ok) throw new SourceError([401, 403, 429].includes(response.status) ? "verification_required" : "selection_unavailable",
                "Плеер не отдал выбранный поток. Обновите источник и повторите действие.");
            url = publicUrl(response.url);
            body = await boundedText(response);
            kind = classifyMedia(response.url, response.headers.get("content-type") ?? "", body);
        }
        if (kind === "hls") {
            probe.resources.set(url, { ...resource, url, body });
            return { ...await hlsPlaylist(probe, url), resource };
        }
        if (kind === "dash") {
            const validation = await commandFrame(probe, { type: "manifest:validate", body, url });
            if (!validation?.playable) return undefined;
            return { url, family: [url], master: true, resource };
        }
    }
    if (kind === "video" && resource.mediaElement) return { url, family: [url], master: false, resource };
    return undefined;
}

async function resolve(command: BrowserCommand, controller: AbortController) {
    if (probes.size >= 3) throw new SourceError("browser_busy", "Браузер готовит другое видео. Повторите действие через несколько секунд.");
    const sourceUrl = publicUrl(command.url!);
    const referrer = command.referrer ? publicUrl(command.referrer) : sourceUrl;
    // Create blank, mute, then navigate: background player audio cannot escape.
    const tab = await backgroundTab("about:blank");
    if (tab.id === undefined) throw new Error("Не удалось открыть плеер в фоне.");
    const page: Frame = { id: 0, url: referrer };
    const probe: Probe = { tabId: tab.id, page, selected: page, resources: new Map(), headers: new Map(), controller, ready: false };
    probes.set(tab.id, probe);
    try {
        await chrome.tabs.update(tab.id, { muted: true, url: referrer });
        await loaded(tab.id, controller.signal);
        if (referrer !== sourceUrl) {
            page.url = publicUrl((await chrome.tabs.get(tab.id)).url!);
            await confirmSelection(probe, sourceUrl);
            await mountFrame(probe, sourceUrl, sourceUrl);
        }
        await confirmSelection(probe, sourceUrl);
        const prepared = await commandFrame(probe, { type: "prepare", selection: command.selection ?? {} });
        if (prepared?.error) throw new SourceError(prepared.code || "selection_unavailable", prepared.error, sourceUrl);
        const selectedUrl = publicUrl(prepared.url);
        if (selectedUrl !== probe.selected.url) {
            if (prepared.embedded || probe.selected !== page) await mountFrame(probe, selectedUrl, sourceUrl);
            else await navigateFromPage(probe, selectedUrl, sourceUrl);
            const confirmed = await commandFrame(probe, { type: "prepare", selection: command.selection ?? {} });
            if (confirmed?.error || confirmed.url !== selectedUrl) throw new SourceError(confirmed?.code || "selection_unavailable", confirmed?.error ?? "Плеер не подтвердил выбор серии и озвучки.", sourceUrl);
        }
        probe.ready = true;
        await commandFrame(probe, { type: "capture:start" });
        await delay(7_000, controller.signal);
        await confirmSelection(probe, sourceUrl);
        const candidates = (await Promise.all([...probe.resources.values()].slice(0, 24).map((resource) => mediaCandidate(probe, resource)))).filter((value) => value !== undefined);
        const byUrl = new Map<string, NonNullable<typeof candidates[number]>>();
        for (const candidate of candidates) {
            if (!byUrl.has(candidate.url) || candidate.master) byUrl.set(candidate.url, candidate);
        }
        const unique = [...byUrl.values()];
        const manifests = unique.filter((candidate) => candidate.master);
        const family = new Set(manifests.flatMap((candidate) => candidate.family));
        const unrelated = unique.filter((candidate) => !candidate.master && !family.has(candidate.url));
        const pool = manifests.length ? [...manifests, ...unrelated] : unique;
        if (!pool.length) {
            const problem = await commandFrame(probe, { type: "problem" });
            if (problem?.error) throw new SourceError(problem.code, problem.error, sourceUrl);
        }
        if (pool.length !== 1) {
            throw new SourceError(pool.length ? "selection_unavailable" : "media_unavailable", pool.length ? "Плеер вернул несколько потоков. Откройте нужную серию в источнике и повторите действие."
                : "Плеер не отдал доступный поток. Откройте источник, запустите нужную серию и повторите действие.", sourceUrl);
        }
        const selected = pool[0]!;
        const headers: Record<string, string> = { Referer: selected.resource.initiator ?? selectedUrl, ...probe.headers.get(selected.resource.url), "User-Agent": navigator.userAgent };
        const sessionCookies = await cookies(selected.url);
        if (sessionCookies.length) headers.Cookie = sessionCookies.map((cookie) => `${cookie.name}=${cookie.value}`).join("; ");
        await confirmSelection(probe, sourceUrl);
        return {
            kind: "video", resolver: "direct", url: selected.url, headers,
            selection: command.selection ?? {},
            ...("audioUrl" in selected && selected.audioUrl ? { audioUrl: selected.audioUrl } : {}),
            ...("subtitles" in selected && selected.subtitles?.length ? { subtitles: selected.subtitles } : {}),
        };
    } finally {
        probes.delete(tab.id);
        await chrome.tabs.remove(tab.id).catch(() => undefined);
    }
}

async function confirmSelection(probe: Probe, sourceUrl: string): Promise<void> {
    if ((await chrome.tabs.get(probe.tabId)).url !== probe.page.url) {
        throw new SourceError("selected_content_changed", "Источник перенаправил выбранный плеер на другую страницу. Выберите доступный вариант в источнике.", sourceUrl);
    }
    if (probe.selected !== probe.page) {
        const state = await commandFrame(probe, { type: "ready" });
        if (state?.url !== probe.selected.url) {
            throw new SourceError("selected_content_changed", "Источник перенаправил выбранный плеер на другую страницу. Выберите доступный вариант в источнике.", sourceUrl);
        }
    }
    for (const frame of [probe.page, probe.selected]) {
        if (frame.status && frame.status >= 400) {
            throw new SourceError([401, 403, 429].includes(frame.status) ? "verification_required" : "source_unavailable",
                `Выбранный плеер вернул HTTP ${frame.status}. Откройте источник и выберите доступный вариант.`, sourceUrl);
        }
    }
}

chrome.webRequest.onBeforeRequest.addListener((details) => {
    const probe = probes.get(details.tabId);
    if (!probe || probe.selected === probe.page || details.type !== "sub_frame" || details.parentFrameId !== 0) return;
    const expected = new URL(probe.selected.url);
    expected.hash = "";
    if (details.url !== expected.href) return;
    if (probe.selected.id !== undefined && probe.selected.id !== details.frameId) probe.selected.ambiguous = true;
    else probe.selected.id = details.frameId;
}, { urls: ["http://*/*", "https://*/*"] });

chrome.webRequest.onCompleted.addListener((details) => {
    const probe = probes.get(details.tabId);
    if (!probe || !["main_frame", "sub_frame"].includes(details.type)) return;
    if (details.frameId === 0) probe.page.status = details.statusCode;
    if (details.frameId === probe.selected.id) probe.selected.status = details.statusCode;
}, { urls: ["http://*/*", "https://*/*"] });

chrome.runtime.onMessage.addListener((message, sender, respond) => {
    const probe = sender.tab?.id === undefined ? undefined : probes.get(sender.tab.id);
    if (!probe?.ready || probe.selected.ambiguous || sender.frameId !== probe.selected.id || sender.url !== probe.selected.url) return;
    if (message?.type === "frame:ready") { respond({ capture: true }); return; }
    if (message?.type !== "capture" || !message.payload || typeof message.payload.url !== "string") return;
    try {
        const payload = message.payload as Capture;
        publicUrl(payload.url);
        if (payload.body && payload.body.length > 512 * 1024 || probe.resources.size >= 128) return;
        const previous = probe.resources.get(payload.url);
        probe.resources.set(payload.url, { ...previous, ...payload, mediaElement: !!(payload.mediaElement || previous?.mediaElement), body: payload.body || previous?.body });
    } catch { /* Captured page data is untrusted. */ }
});

const collectHeaders = (details: chrome.webRequest.OnBeforeSendHeadersDetails): undefined => {
    const probe = probes.get(details.tabId);
    if (!probe || details.frameId !== probe.selected.id) return;
    if (probe.headers.size >= 128 && !probe.headers.has(details.url)) return;
    const headers: Record<string, string> = {};
    for (const header of details.requestHeaders ?? []) {
        if (header.value && /^(?:authorization|origin|referer|user-agent|x-[\w-]+)$/i.test(header.name)) headers[header.name] = header.value;
    }
    probe.headers.set(details.url, headers);
    return undefined;
};
try {
    chrome.webRequest.onBeforeSendHeaders.addListener(collectHeaders, { urls: ["http://*/*", "https://*/*"] }, ["requestHeaders", "extraHeaders"]);
} catch {
    chrome.webRequest.onBeforeSendHeaders.addListener(collectHeaders, { urls: ["http://*/*", "https://*/*"] }, ["requestHeaders"]);
}

async function dispatch(value: unknown, connection: chrome.runtime.Port): Promise<void> {
    if (connection !== port || !value || typeof value !== "object") return;
    const command = value as BrowserCommand;
    if (!Number.isSafeInteger(command.id) || !["cookies", "fetch", "resolve", "cancel"].includes(command.method)) return;
    if (command.method === "cancel") { operations.get(command.id)?.abort(); return; }
    const controller = new AbortController();
    operations.set(command.id, controller);
    const response: BrowserResponse = { id: command.id };
    try {
        if (operations.size > 16) throw new SourceError("browser_busy", "Браузер занят. Повторите действие.");
        publicUrl(command.url!);
        if (command.method === "cookies") response.result = await cookies(command.url!);
        if (command.method === "fetch") response.result = await browserFetch(command, controller.signal);
        if (command.method === "resolve") response.result = command.kind && command.kind !== "video"
            ? await resolveFile(command, controller) : await resolve(command, controller);
        if (new TextEncoder().encode(JSON.stringify(response)).byteLength > MAX_MESSAGE_BYTES) throw new Error("Ответ источника слишком большой.");
    } catch (error) {
        response.result = undefined;
        const url = command.method === "resolve" ? command.url : error instanceof SourceError ? error.url : undefined;
        response.error = {
            code: error instanceof SourceError ? error.code : controller.signal.aborted ? "canceled" : "browser_error",
            message: error instanceof Error ? error.message : "Не удалось прочитать источник.",
            ...(!controller.signal.aborted && url ? { url } : {}),
        };
    } finally {
        if (operations.get(command.id) === controller) operations.delete(command.id);
    }
    if (port === connection) connection.postMessage(response);
}

chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
chrome.alarms.onAlarm.addListener((alarm) => { if (alarm.name === "qbutt:connect") connect(); });
void chrome.alarms.create("qbutt:connect", { periodInMinutes: 0.5 });
connect();
