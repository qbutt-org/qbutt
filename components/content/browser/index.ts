import { ContentError } from "../errors";
import type { ResolvedContent } from "../contracts.ts";
import {
    MAX_MESSAGE_BYTES,
    type BrowserChoice,
    type BrowserCommand,
    type BrowserResponse,
    type Selection,
    type VkSearchVideo,
} from "./protocol.ts";

export type { BrowserChoice, BrowserId, Selection } from "./protocol.ts";

export interface BrowserStatus {
    selected: BrowserChoice;
    ready: boolean;
    userAgent?: string;
    connectedBrowser?: "edge" | "chrome";
    detected?: "edge" | "chrome";
    browsers: { id: "edge" | "chrome"; name: string; isDefault: boolean; connected: boolean }[];
}

interface BrowserCookie {
    name: string;
    value: string;
    domain: string;
    path: string;
    secure: boolean;
    hostOnly: boolean;
    httpOnly?: boolean;
    sameSite?: "no_restriction" | "lax" | "strict" | "unspecified";
    expirationDate?: number;
}

type Command = Omit<BrowserCommand, "id"> | { method: "status" | "initialize" };
type Emit = (event: "browserRequest" | "browserCancel", data: unknown) => void;

interface Pending {
    resolve(value: unknown): void;
    reject(error: Error): void;
}

// The application owns qcatch's connection. This adapter only forwards the
// remaining helper consumers over their existing private parent connection.
export class BrowserBridge {
    private readonly pending = new Map<number, Pending>();
    private nextId = 0;
    private closed = false;

    constructor(private readonly options: { browser?: BrowserChoice; emit: Emit }) {}

    async status(signal?: AbortSignal, initialize = false): Promise<BrowserStatus> {
        return this.request({ method: initialize ? "initialize" : "status" }, signal) as Promise<BrowserStatus>;
    }

    setBrowser(browser: BrowserChoice): void {
        this.options.browser = browser;
    }

    async cookies(url: string, options: { signal?: AbortSignal } = {}): Promise<BrowserCookie[]> {
        if (!(await this.status(options.signal)).ready)
            return [];
        return this.request({ method: "cookies", url }, options.signal) as Promise<BrowserCookie[]>;
    }

    async searchVkVideos(query: string, signal?: AbortSignal): Promise<VkSearchVideo[]> {
        const address = new URL("https://vkvideo.ru/");
        address.searchParams.set("q", query);
        const result = await this.request({ method: "vk-search", url: address.href, query }, signal) as {
            query: string; videos: VkSearchVideo[];
        };
        return result.videos;
    }

    async fetch(url: string, init: RequestInit = {}, options: { signal?: AbortSignal } = {}): Promise<Response> {
        if (init.body !== undefined && init.body !== null && typeof init.body !== "string")
            throw new Error("Browser fetch accepts a text body only");
        const response = await this.request({
            method: "fetch", url,
            init: {
                method: init.method ?? "GET",
                headers: Object.fromEntries(new Headers(init.headers).entries()),
                ...(typeof init.body === "string" ? { body: init.body } : {}),
            },
        }, options.signal ?? init.signal ?? undefined) as {
            status: number; headers: Record<string, string>; body: string; url: string;
        };
        const result = new Response([204, 205, 304].includes(response.status) ? null : response.body,
            { status: response.status, headers: response.headers });
        Object.defineProperty(result, "url", { value: response.url });
        return result;
    }

    async resolve(url: string, selection: Selection = {}, options: {
        signal?: AbortSignal; kind?: "video" | "file" | "torrent"; referrer?: string;
    } = {}): Promise<ResolvedContent> {
        return this.request({ method: "resolve", url, selection, kind: options.kind ?? "video",
            referrer: options.referrer }, options.signal) as Promise<ResolvedContent>;
    }

    receive(response: BrowserResponse): void {
        const pending = this.pending.get(response.id);
        if (!pending)
            return;
        if (response.error)
            pending.reject(new ContentError(response.error.code, response.error.message, response.error.url));
        else
            pending.resolve(response.result);
    }

    async close(): Promise<void> {
        this.closed = true;
        for (const [id, pending] of this.pending) {
            this.options.emit("browserCancel", { id });
            pending.reject(new ContentError("browser_disconnected", "The content service stopped."));
        }
    }

    private request(command: Command, signal?: AbortSignal): Promise<unknown> {
        signal?.throwIfAborted();
        if (this.closed)
            throw new ContentError("browser_disconnected", "The content service stopped.");
        if (this.pending.size >= 16)
            throw new ContentError("browser_busy", "The browser is busy. Try again in a few seconds.");
        const id = ++this.nextId;
        const data = { id, browser: this.options.browser ?? "auto", command,
            optional: command.method === "cookies" };
        if (Buffer.byteLength(JSON.stringify(data)) > MAX_MESSAGE_BYTES)
            throw new Error("Browser message exceeds the limit");
        return new Promise((resolve, reject) => {
            const cleanup = () => {
                clearTimeout(timeout);
                signal?.removeEventListener("abort", abort);
                this.pending.delete(id);
            };
            const abort = () => {
                this.options.emit("browserCancel", { id });
                cleanup();
                reject(signal?.reason ?? new Error("Browser request canceled"));
            };
            // The native owner includes reconnection in its bounded request budget.
            // This longer watchdog only detects a lost parent IPC response.
            const timeout = setTimeout(() => {
                this.options.emit("browserCancel", { id });
                cleanup();
                reject(new ContentError("browser_timeout", "The browser took too long. Try again."));
            }, 100_000);
            this.pending.set(id, {
                resolve: value => { cleanup(); resolve(value); },
                reject: error => { cleanup(); reject(error); },
            });
            signal?.addEventListener("abort", abort, { once: true });
            try {
                this.options.emit("browserRequest", data);
            }
            catch (error) {
                cleanup();
                reject(error);
            }
        });
    }
}
