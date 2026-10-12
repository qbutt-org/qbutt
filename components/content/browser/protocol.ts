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
