import type { Selection, VkSearchVideo } from "./browser/protocol";

export const maxMessageBytes = 16 * 1024 * 1024;

export type Category =
    | "all"
    | "games"
    | "movies"
    | "tv"
    | "anime"
    | "software"
    | "books"
    | "manga";

export interface SearchRequest {
    query: string;
    category: Category;
    expanded?: boolean;
    sources?: string[];
    language?: string;
}

export interface Source {
    id: string;
    name: string;
    url: string;
    categories: Category[];
    enabled: boolean;
    requiresLogin?: boolean;
    authorizationSetting?: string;
    health?: { status: "error"; message: string; checkedAt: string; code?: string; url?: string };
    settings?: SourceSetting[];
}

export interface SourceSetting {
    name: string;
    label: string;
    type: "text" | "password" | "boolean" | "select";
    value?: string | boolean;
    options?: { value: string; label: string }[];
    configured?: boolean;
    advanced?: boolean;
}

export interface Variant {
    id: string;
    sourceId: string;
    sourceName: string;
    title: string;
    url: string;
    transport: "torrent" | "video" | "book" | "manga" | "file";
    magnet?: string;
    downloadUrl?: string;
    infoHash?: string;
    size?: number;
    seeders?: number;
    publishedAt?: string;
    duration?: number;
    version?: string;
    edition?: string;
    cooperative?: boolean;
    translation?: string;
    translationType?: "dub" | "subtitles";
    languages?: string[];
    quality?: string;
    facets?: { episode?: string; season?: string; translation?: string };
    episode?: string;
    season?: string;
    player?: string;
    author?: string;
    publicationKind?: "full" | "patch" | "fix" | "optional" | "archivePart" | "upload";
    needsInspection?: boolean;
    adStatus?: "unknown" | "reported" | "clean";
    notes?: { text: string; url: string; author?: string }[];
}

export interface Work {
    id: string;
    title: string;
    category: Category;
    url?: string;
    cover?: string;
    description?: string;
    year?: number;
    aliases?: string[];
    identifiers?: Record<string, string>;
    notes?: { text: string; url: string; author?: string }[];
    variants: Variant[];
}

export interface Chapter {
    id: string;
    title: string;
    url: string;
    number?: number;
    volume?: number;
}

export interface WorkDetails extends Work {
    chapters?: Chapter[];
}

export interface ResolvedContent {
    kind: "torrent" | "video" | "book" | "manga" | "file";
    resolver?: "direct" | "yt-dlp" | "browser";
    url?: string;
    torrentData?: string;
    headers?: Record<string, string>;
    audioUrl?: string;
    audioTrack?: { title: string; type?: string; language?: string; codec?: string; channels?: number };
    proxyUrl?: string;
    subtitles?: { title: string; url: string; language?: string }[];
    selection?: Selection;
    title?: string;
    translation?: string;
    quality?: string;
}

export interface ProviderContext {
    signal: AbortSignal;
    dataDir: string;
    appDir?: string;
    settings: Record<string, string>;
    fetch(url: string, init?: RequestInit): Promise<Response>;
    searchVkVideos?(query: string): Promise<VkSearchVideo[]>;
    // Catalog-owned recovery lets optional requests fail without discarding the publication.
    withSourceSession?<T>(url: string, operation: (context: ProviderContext) => Promise<T>): Promise<T>;
    saveSettings?(changes: Record<string, string>): Promise<void>;
    reportError?(error: unknown): void;
}

export interface Provider {
    source: Source;
    search(request: SearchRequest, context: ProviderContext, emit?: (work: Work) => void): Promise<Work[]>;
    inspect?(variant: Variant, context: ProviderContext, emit?: (work: WorkDetails) => void): Promise<WorkDetails>;
    resolve?(variant: Variant, context: ProviderContext): Promise<ResolvedContent>;
}

// All helpers use the same bounded JSON-lines protocol over private stdio.
// Each request ends with exactly one complete or error event. Search may emit
// result and source events before completion; cancellation uses its own ID.
// EOF cancels remote work and drains accepted persistence before closing storage.
export interface Request {
    id: number;
    method: string;
    params?: Record<string, unknown>;
}

export interface Event {
    id: number;
    event: string;
    data?: unknown;
}
