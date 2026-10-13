import type { Chapter } from "../contracts";

interface ReaderPosition {
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

interface ReaderImage {
    id: string;
    url: string;
    width?: number;
    height?: number;
}

export interface ReaderPage {
    id: string;
    documentId: string;
    title: string;
    kind: "text" | "images";
    html?: string;
    images?: ReaderImage[];
    direction?: "ltr" | "rtl" | "vertical";
    cached?: boolean;
}

export type ReaderEvent = (event: string, data: unknown) => void;

export function contentId(value: string | Uint8Array): string {
    return new Bun.CryptoHasher("sha256").update(value).digest("hex");
}
