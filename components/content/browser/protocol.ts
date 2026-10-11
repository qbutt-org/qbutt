export const MAX_MESSAGE_BYTES = 768 * 1024;

export type BrowserId = "edge" | "chrome" | "firefox";
export type BrowserChoice = "auto" | BrowserId;
export type Selection = {
    episode?: string;
    season?: string;
    translationId?: string;
    player?: string;
    captureId?: string;
};

export interface VkSearchVideo {
    id: string;
    title: string;
    url: string;
    author?: string;
    duration?: number;
    cover?: string;
}

export interface BrowserCommand {
    id: number;
    method: "cookies" | "fetch" | "resolve" | "vk-search" | "cancel";
    url?: string;
    query?: string;
    init?: { method?: string; headers?: Record<string, string>; body?: string };
    selection?: Selection;
    kind?: "video" | "file" | "torrent";
    referrer?: string;
}

export interface BrowserResponse {
    id: number;
    result?: unknown;
    error?: { code: string; message: string; url?: string };
}

export interface BrowserHello {
    type: "hello";
    browser: BrowserId;
    version: 1;
    userAgent: string;
    capabilities?: string[];
}

export function publicUrl(value: string): string {
    const url = new URL(value);
    const host = url.hostname.replace(/^\[|\]$/g, "").toLowerCase();
    const ipv4 = host.split(".").map(Number);
    const privateIpv4 = ipv4.length === 4 && ipv4.every(Number.isInteger)
        && (ipv4[0] === 0 || ipv4[0] === 10 || ipv4[0] === 127 || ipv4[0]! >= 224
            || ipv4[0] === 169 && ipv4[1] === 254
            || ipv4[0] === 172 && ipv4[1]! >= 16 && ipv4[1]! <= 31
            || ipv4[0] === 192 && ipv4[1] === 168
            || ipv4[0] === 100 && ipv4[1]! >= 64 && ipv4[1]! <= 127);
    if (!/^https?:$/.test(url.protocol) || url.username || url.password
        || host === "localhost" || host.endsWith(".localhost") || host.endsWith(".local")
        || !host.includes(".") || host.includes(":") || privateIpv4) {
        throw new Error("Источник должен использовать публичный HTTP или HTTPS адрес.");
    }
    return url.href;
}
