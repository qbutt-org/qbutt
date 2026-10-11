// MAIN-world observation adapted from Stream Catcher; see ../NOTICE.
import { CAPTURE_FRAME_NAME, classifyMedia, type Capture } from "./media.ts";

const CHANNEL = "qbutt:media";
const LIMIT = 512 * 1024;
let started = false;
let forwarding = false;
const pending = new Map<string, Capture>();

function capture(url: string, body = "", contentType = "", mediaElement = false, requestUrl = url): void {
    if (!/^https?:/i.test(url) || url.length > 16_384 || body.length > LIMIT) return;
    const kind = classifyMedia(url, contentType, body);
    if (kind === "unknown") return;
    const payload: Capture = { url, mediaElement, initiator: location.href };
    if (requestUrl !== url) payload.requestUrl = requestUrl;
    if (body) payload.body = body;
    if (contentType) payload.contentType = contentType;
    if (!forwarding) {
        const previous = pending.get(url);
        if (pending.size >= 24 && !previous) return;
        pending.set(url, { ...previous, ...payload, ...(previous?.body && !body ? { body: previous.body } : {}) });
        return;
    }
    window.postMessage({ channel: CHANNEL, payload }, "*");
}

async function responseBody(response: Response): Promise<string> {
    if (!response.body || Number(response.headers.get("content-length") ?? 0) > LIMIT) return "";
    const reader = response.body.getReader();
    const parts: Uint8Array[] = [];
    let size = 0;
    try {
        while (true) {
            const { value, done } = await reader.read();
            if (done) break;
            size += value.byteLength;
            if (size > LIMIT) return "";
            parts.push(value);
        }
    } finally {
        await reader.cancel().catch(() => undefined);
    }
    const body = new Uint8Array(size);
    let offset = 0;
    for (const part of parts) { body.set(part, offset); offset += part.length; }
    return new TextDecoder().decode(body);
}

function start(): void {
    if (started) return;
    started = true;
    const originalFetch = window.fetch;
    const observedFetch = async function (input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
        const requestUrl = new URL(input instanceof Request ? input.url : String(input), document.baseURI).href;
        const response = await Reflect.apply(originalFetch, window, [input, init]);
        const contentType = response.headers.get("content-type") ?? "";
        capture(response.url, "", contentType, false, requestUrl);
        if (["hls", "dash"].includes(classifyMedia(response.url, contentType))) {
            void responseBody(response.clone()).then((body) => capture(response.url, body, contentType, false, requestUrl)).catch(() => undefined);
        }
        return response;
    };
    Object.assign(observedFetch, originalFetch);
    window.fetch = observedFetch as typeof window.fetch;

    const urls = new WeakMap<XMLHttpRequest, string>();
    const originalOpen = XMLHttpRequest.prototype.open;
    const originalSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method: string, url: string | URL, async = true, username?: string | null, password?: string | null) {
        urls.set(this, new URL(String(url), document.baseURI).href);
        Reflect.apply(originalOpen, this, [method, url, async, username, password]);
    };
    XMLHttpRequest.prototype.send = function (body) {
        this.addEventListener("loadend", () => {
            const url = urls.get(this);
            if (!url) return;
            let text = "";
            try {
                if (this.responseType === "" || this.responseType === "text") text = this.responseText;
            } catch { /* A binary XHR still exposes its URL. */ }
            // Performance entries identify the original XHR URL. Refetch a
            // redirected manifest so relative children use its final URL.
            capture(url, this.responseURL === url && text.length <= LIMIT ? text : "", this.getResponseHeader("content-type") ?? "");
        }, { once: true });
        Reflect.apply(originalSend, this, [body]);
    };

    new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) capture(entry.name);
    }).observe({ entryTypes: ["resource"] });
    for (const entry of performance.getEntriesByType("resource")) capture(entry.name);

    const scan = () => {
        for (const media of document.querySelectorAll<HTMLMediaElement>("video,audio")) {
            capture(media.currentSrc || media.src, "", "", true);
        }
    };
    scan();
    new MutationObserver(scan).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ["src"] });
    document.addEventListener("loadedmetadata", scan, true);
}

document.addEventListener("qbutt:start-capture", () => {
    forwarding = true;
    start();
    for (const payload of pending.values()) window.postMessage({ channel: CHANNEL, payload }, "*");
    pending.clear();
});
// The source document names only the one child it creates for this operation.
// Observe before inline player scripts; forwarding still needs authorization.
if (window.name === CAPTURE_FRAME_NAME) start();
