import type { Chapter } from "../contracts";

export interface ReaderPosition {
    chapterId: string;
    offset?: number;
    imageIndex?: number;
}

export interface ReaderDocument {
    id: string;
    title: string;
    sourceId: string;
    workId?: string;
    chapters: Chapter[];
    position?: ReaderPosition;
    local?: boolean;
}

export interface ReaderImage {
    id: string;
    url: string;
    cleanedUrl?: string;
    width?: number;
    height?: number;
}

export interface ReaderRegion {
    id: string;
    imageIndex: number;
    text: string;
    translation: string;
    x: number;
    y: number;
    width: number;
    height: number;
    layout: { x: number; y: number; width: number; height: number };
    orientation: "horizontal" | "vertical";
    ink: "dark" | "light";
}

export interface ReaderPage {
    id: string;
    documentId: string;
    title: string;
    kind: "text" | "images";
    html?: string;
    images?: ReaderImage[];
    direction?: "ltr" | "rtl" | "vertical";
    original?: { html?: string; images?: ReaderImage[] };
    regions?: ReaderRegion[];
    translatedImages?: number[];
    translated?: boolean;
    cached?: boolean;
    provider?: string;
    model?: string;
}

export type ReaderEvent = (event: string, data: unknown) => void;

export function contentId(value: string | Uint8Array): string {
    return new Bun.CryptoHasher("sha256").update(value).digest("hex");
}
