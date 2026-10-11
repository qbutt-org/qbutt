// Adapted from Stream Catcher; see ../NOTICE and ../LICENSE.
export const CAPTURE_FRAME_NAME = "qbutt:player";

export type MediaKind = "hls" | "dash" | "video" | "unknown";

export function classifyMedia(url: string, contentType = "", body = ""): MediaKind {
    if (/^#EXTM3U/m.test(body) || /\.m3u8(?:$|[?#])/i.test(url) || /mpegurl/i.test(contentType)) return "hls";
    if (/<MPD\b/i.test(body) || /\.mpd(?:$|[?#])/i.test(url) || /dash\+xml/i.test(contentType)) return "dash";
    if (/\.(?:mp4|mkv|webm|mov)(?:$|[?#])/i.test(url) || /^video\//i.test(contentType)) return "video";
    return "unknown";
}

export interface Capture {
    url: string;
    body?: string;
    contentType?: string;
    mediaElement?: boolean;
    initiator?: string;
    requestUrl?: string;
}
