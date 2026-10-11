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
}

export interface Source {
    id: string;
    name: string;
    url: string;
    categories: Category[];
    enabled: boolean;
    requiresLogin?: boolean;
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
    languages?: string[];
    quality?: string;
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
    selection?: { episode?: string; season?: string; translationId?: string; player?: string };
    title?: string;
    translation?: string;
}

export interface ProviderContext {
    signal: AbortSignal;
    dataDir: string;
    appDir?: string;
    settings: Record<string, string>;
    fetch(url: string, init?: RequestInit): Promise<Response>;
    saveSettings?(changes: Record<string, string>): Promise<void>;
}

export interface Provider {
    source: Source;
    search(request: SearchRequest, context: ProviderContext): Promise<Work[]>;
    inspect?(variant: Variant, context: ProviderContext): Promise<WorkDetails>;
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
