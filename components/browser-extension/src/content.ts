// Exact YummyAnime selection uses the established Stream Catcher DOM flow.
import { publicUrl, type Selection } from "../../content/browser/protocol.ts";
import { playableDash } from "./dash.ts";
import { CAPTURE_FRAME_NAME } from "./media.ts";

const wait = (milliseconds: number) => new Promise((resolve) => setTimeout(resolve, milliseconds));
let capturing = false;
let playerFrame: HTMLIFrameElement | undefined;
let documentReady = document.readyState === "complete";
document.addEventListener("DOMContentLoaded", () => { documentReady = true; }, { once: true });

function pageProblem(): { code: string; error: string } | undefined {
    if (/just a moment|attention required|captcha/i.test(document.title) || document.querySelector("#challenge-form")) {
        return { code: "verification_required", error: "Откройте источник в браузере, завершите проверку и повторите действие." };
    }
    const text = document.body?.innerText.trim() ?? "";
    if (text.length < 2000 && /данной страницы не существует|запрашиваемый контент не найден|контент не найден или недоступен в вашем регионе/i.test(text)
        && !document.querySelector("video,audio")) {
        return { code: "source_unavailable", error: "Выбранный контент отсутствует в плеере. Выберите доступный вариант в источнике." };
    }
    return undefined;
}

function label(value: string): string {
    return value.replace(/\u00a0/g, " ").replace(/^(?:плеер|озвучка|player|voice(?: acting)?)\s*/i, "")
        .replace(/\s*\d+\s*\/\s*\d+(?:[.,]\d+)?\s*[KMBКМ]?\s*$/i, "").trim().toLowerCase();
}

async function selectPlayer(requested: string): Promise<void> {
    const inputs = [...document.querySelectorAll<HTMLInputElement>('input[role="combobox"], input[id^="react-select-"]')];
    const input = inputs.find((candidate) => /alloha|kodik|cvh|cdnvideo/i.test(candidate.parentElement?.parentElement?.textContent ?? ""));
    if (!input) return;
    const selected = input.parentElement?.parentElement?.querySelector('[class*="-singleValue"], [class*="__single-value"]')?.textContent ?? input.value;
    if (label(selected) === label(requested)) return;
    input.focus();
    input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, code: "ArrowDown", key: "ArrowDown" }));
    try {
        for (let attempt = 0; attempt < 12; attempt++) {
            await wait(50);
            const list = document.getElementById(input.getAttribute("aria-controls") ?? "");
            const option = [...list?.querySelectorAll<HTMLElement>('[role="option"]') ?? []].find((candidate) => label(candidate.textContent ?? "") === label(requested));
            if (option) { option.click(); await wait(650); return; }
        }
        throw new Error(`Плеер «${requested}» не найден на странице.`);
    } finally {
        input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
        input.blur();
    }
}

function targetUrl(selection: Selection): { url: string; embedded: boolean } {
    const pageIsPlayer = /alloha|vkvideo\.cloud|kodik|mimin\.cloud|solodcdn|cdnvideohub|plapi\.|aksor/i.test(location.hostname)
        || /\/iframeCVH\.html$/i.test(location.pathname);
    // A direct player URL already identifies the chosen content. Its storage
    // and advertising frames must never replace that selection.
    const frames = selection.player && !pageIsPlayer ? [...document.querySelectorAll<HTMLIFrameElement>("iframe[src]")] : [];
    const player = selection.player?.toLowerCase();
    const knownFrames = frames.filter((frame) => /alloha|kodik|iframeCVH|cdnvideohub/i.test(frame.src));
    const matching = knownFrames.filter((frame) => !player || frame.src.toLowerCase().includes(player)
        || player === "cvh" && /iframeCVH|cdnvideohub/i.test(frame.src));
    if (knownFrames.length && matching.length !== 1) throw new Error("Выбранный плеер не найден однозначно. Откройте источник и повторите выбор.");
    const url = new URL(matching[0]?.src ?? location.href);
    const alloha = /alloha|vkvideo\.cloud/i.test(url.hostname);
    const knownPlayer = alloha || /kodik|mimin\.cloud|solodcdn|cdnvideohub|plapi\.|aksor/i.test(url.hostname)
        || /\/iframeCVH\.html$/i.test(url.pathname);
    for (const [key, value] of Object.entries(selection)) {
        if (!value || key === "player" && knownPlayer) continue;
        const parameter = key === "translationId" ? "translation" : key;
        if (url.searchParams.get(parameter) === value) continue;
        const select = document.querySelector<HTMLSelectElement>(`select[name="${parameter}"],select[data-qbutt-selection="${parameter}"]`);
        if (select && [...select.options].some((option) => option.value === value)) {
            select.value = value;
            select.dispatchEvent(new Event("change", { bubbles: true }));
        } else if (!knownPlayer || key === "translationId" && !alloha) {
            throw new Error("Источник не подтвердил выбранную серию или озвучку. Откройте точную страницу плеера.");
        }
        url.searchParams.set(parameter, value);
    }
    if (player && knownPlayer && !url.href.toLowerCase().includes(player) && !(player === "cvh" && /plapi\.|cdnvideo|iframeCVH/i.test(url.href))) {
        throw new Error("Источник вернул другой плеер.");
    }
    return { url: url.href, embedded: matching.length === 1 };
}

function startCapture(): void {
    if (capturing) return;
    capturing = true;
    document.dispatchEvent(new Event("qbutt:start-capture"));
    let clickedPlay = false;
    const play = () => {
        const media = [...document.querySelectorAll<HTMLMediaElement>("video,audio")];
        for (const element of media) {
            element.muted = true;
            void element.play().catch(() => undefined);
        }
        const button = document.querySelector<HTMLElement>(".play_button,.play-button,.vjs-big-play-button,.plyr__control--overlaid,[data-action=play],button[data-plyr=play]");
        if (button && !clickedPlay) { clickedPlay = true; button.click(); }
    };
    document.addEventListener("DOMContentLoaded", play, { once: true });
    play();
}

window.addEventListener("message", (event: MessageEvent) => {
    if (!capturing || event.source !== window || event.data?.channel !== "qbutt:media") return;
    const payload = event.data.payload;
    if (!payload || typeof payload.url !== "string" || payload.url.length > 16_384
        || typeof payload.body === "string" && payload.body.length > 512 * 1024) return;
    const mediaElement = [...document.querySelectorAll<HTMLMediaElement>("video,audio")]
        .some((media) => media.currentSrc === payload.url || media.src === payload.url);
    const requestUrl = typeof payload.requestUrl === "string" && payload.requestUrl.length <= 16_384 ? payload.requestUrl : payload.url;
    if (!mediaElement && !performance.getEntriesByName(requestUrl).some((entry) => entry.entryType === "resource")) return;
    const body = typeof payload.body === "string" ? payload.body : undefined;
    const contentType = typeof payload.contentType === "string" ? payload.contentType.slice(0, 256) : "";
    void chrome.runtime.sendMessage({ type: "capture", payload: {
        url: payload.url, body, contentType, mediaElement, initiator: location.href,
    } }).catch(() => undefined);
});

chrome.runtime.onMessage.addListener((message, _sender, respond) => {
    if (message?.type === "ready") respond({ ready: documentReady, url: location.href });
    if (message?.type === "problem") respond(pageProblem() ?? {});
    if (message?.type === "frame:mount" && typeof message.url === "string") {
        const problem = pageProblem();
        if (problem) respond(problem);
        else {
            playerFrame?.remove();
            const iframe = document.createElement("iframe");
            iframe.name = CAPTURE_FRAME_NAME;
            iframe.src = publicUrl(message.url);
            iframe.allow = "autoplay; fullscreen";
            iframe.style.cssText = "width:100%;height:720px;border:0";
            document.body.append(iframe);
            playerFrame = iframe;
            respond({ mounted: true });
        }
    }
    if (message?.type === "navigate" && typeof message.url === "string") {
        const problem = pageProblem();
        if (problem) respond(problem);
        else {
            const url = publicUrl(message.url);
            respond({ navigating: true });
            // Let the response leave this document before normal page navigation.
            setTimeout(() => location.assign(url), 0);
        }
    }
    if (message?.type === "manifest:validate" && typeof message.body === "string" && message.body.length <= 512 * 1024 && typeof message.url === "string") {
        respond({ playable: playableDash(message.body, message.url) });
    }
    if (message?.type === "prepare") {
        void (async () => {
            if (!documentReady) await new Promise((resolve) => document.addEventListener("DOMContentLoaded", resolve, { once: true }));
            const problem = pageProblem();
            if (problem) return problem;
            if (message.selection?.player) await selectPlayer(message.selection.player);
            return targetUrl(message.selection ?? {});
        })().then(respond).catch((error) => respond({ error: error instanceof Error ? error.message : String(error) }));
        return true;
    }
    if (message?.type === "capture:start") {
        startCapture();
        respond({ started: true });
    }
});

void chrome.runtime.sendMessage({ type: "frame:ready" }).then((response) => {
    if (response?.capture) startCapture();
}).catch(() => undefined);
