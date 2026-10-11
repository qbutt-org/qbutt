import type { Category, Variant, Work } from "./contracts";
import { persistentVariant, persistentWork, publicReference } from "./references";

export interface SharedItem {
    version: 1;
    mode: "exact" | "work";
    work: Work;
    variant?: Variant;
}

const categories = new Set<Category>(["all", "games", "movies", "tv", "anime", "software", "books", "manga"]);
const transports = new Set(["torrent", "video", "book", "manga", "file"]);

function text(value: unknown, limit: number): string {
    return typeof value === "string" ? value.slice(0, limit) : "";
}

function sharedVariant(input: Variant): Variant {
    input = persistentVariant(input);
    if (!transports.has(input.transport) || !input.id || !input.sourceId)
        throw new Error("This variant cannot be shared.");

    // A stable publication is sufficient to re-resolve media at the recipient.
    // Never copy transient CDN URLs, request headers, cookies or author notes.
    const url = publicReference(input.url);
    if (!url)
        throw new Error("This item has no public source link.");

    const variant: Variant = {
        id: text(input.id, 1024),
        sourceId: text(input.sourceId, 128),
        sourceName: text(input.sourceName, 128),
        title: text(input.title, 512),
        url,
        transport: input.transport,
    };

    for (const key of ["version", "edition", "translation", "quality", "episode", "player", "author"] as const) {
        if (input[key])
            variant[key] = text(input[key], 256);
    }

    if (typeof input.cooperative === "boolean")
        variant.cooperative = input.cooperative;

    if (input.needsInspection)
        variant.needsInspection = true;

    if (input.publicationKind)
        variant.publicationKind = input.publicationKind;

    if (input.infoHash && /^(?:[a-f\d]{40}|[a-f\d]{64})$/i.test(input.infoHash))
        variant.infoHash = input.infoHash.toLowerCase();

    const magnet = publicReference(input.magnet);
    if (magnet?.startsWith("magnet:"))
        variant.magnet = magnet;

    return variant;
}

function normalize(input: SharedItem): SharedItem {
    if (input.version !== 1 || !["exact", "work"].includes(input.mode)
        || !input.work?.id || !input.work.title || !categories.has(input.work.category))
        throw new Error("This qbutt link is incomplete or unsupported.");

    const stableWork = persistentWork(input.work);
    const item: SharedItem = {
        version: 1,
        mode: input.mode,
        work: {
            id: text(stableWork.id, 1024),
            title: text(input.work.title, 512),
            category: input.work.category,
            url: publicReference(input.work.url),
            variants: [],
        },
    };

    if (input.mode === "exact") {
        if (!input.variant)
            throw new Error("The exact variant is missing from this link.");

        item.variant = sharedVariant(input.variant);
        item.work.variants = [item.variant];
    }

    return item;
}

export function share(input: Omit<SharedItem, "version">): string {
    const payload = JSON.stringify(normalize({ ...input, version: 1 }));
    if (Buffer.byteLength(payload) > 16 * 1024)
        throw new Error("This item is too large to share as a link.");

    return `qbutt://open?d=${Buffer.from(payload).toString("base64url")}`;
}

export function openLink(input: string): SharedItem {
    if (input.length > 24 * 1024)
        throw new Error("This qbutt link is too long.");

    const url = new URL(input);
    if (url.protocol !== "qbutt:" || url.hostname !== "open")
        throw new Error("This qbutt link is not supported.");

    const payload = url.searchParams.get("d") || "";
    if (!/^[a-z\d_-]+$/i.test(payload))
        throw new Error("This qbutt link is damaged.");

    return normalize(JSON.parse(Buffer.from(payload, "base64url").toString("utf8")));
}
