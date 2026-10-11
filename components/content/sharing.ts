import type { Category, Variant, Work } from "./contracts";
import { ContentError } from "./errors";
import { captureSelectionId, linkId, persistentVariant, persistentWork, publicReference } from "./references";

export interface SharedItem {
    version: 1;
    mode: "exact" | "work";
    work: Work;
    variant?: Variant;
    action?: "watch" | "download";
}

const categories = new Set<Category>(["all", "games", "movies", "tv", "anime", "software", "books", "manga"]);
const transports = new Set(["torrent", "video", "book", "manga", "file"]);

function text(value: unknown, limit: number): string {
    return typeof value === "string" ? value.slice(0, limit) : "";
}

function sharedVariant(input: Variant): Variant {
    if (captureSelectionId(input))
        throw new ContentError("capture_session", "This qcatch selection belongs to the current browser session. Share the source page instead.");
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

    for (const key of ["version", "edition", "translation", "quality", "episode", "season", "player", "author"] as const) {
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
        || !input.work?.id || !input.work.title || !categories.has(input.work.category)
        || !Array.isArray(input.work.variants))
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
    else {
        const publications = new Map<string, Variant>();
        for (const inputVariant of input.work.variants) {
            const variant = sharedVariant(inputVariant);
            const key = JSON.stringify([variant.sourceId, variant.url]);
            if (publications.has(key)) continue;
            publications.set(key, variant.sourceId === "link" ? variant : {
                id: variant.id, sourceId: variant.sourceId, sourceName: variant.sourceName,
                title: item.work.title, url: variant.url, transport: variant.transport,
                magnet: variant.magnet, infoHash: variant.infoHash, needsInspection: true,
            });
        }
        item.work.variants = [...publications.values()];
        if (input.variant) {
            item.variant = sharedVariant(input.variant);
            if (!item.work.variants.some(variant => variant.sourceId === item.variant!.sourceId
                && variant.url === item.variant!.url && variant.transport === item.variant!.transport))
                throw new Error("The preferred version does not belong to this shared title.");
        }
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
    if (url.protocol !== "qbutt:" || !["open", "capture"].includes(url.hostname))
        throw new Error("This qbutt link is not supported.");

    const payload = url.searchParams.get("d") || "";
    if (!/^[a-z\d_-]+$/i.test(payload))
        throw new Error("This qbutt link is damaged.");

    const data = JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
    if (url.hostname === "capture") {
        if (data?.v !== 1 || !["watch", "download"].includes(data.action)
            || typeof data.pageUrl !== "string" || data.pageUrl.length > 4096
            || typeof data.title !== "string" || data.title.length > 512
            || typeof data.captureId !== "string" || !/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i.test(data.captureId))
            throw new Error("This qcatch capture is incomplete or unsupported. Choose the media again in qcatch.");
        const page = new URL(data.pageUrl);
        page.hash = "";
        if (publicReference(page.href) !== page.href)
            throw new Error("qcatch must share a public source page without session credentials.");
        const id = linkId(page.href);
        const title = data.title.replace(/[\x00-\x1f\x7f]/g, " ").trim() || page.hostname;
        const variant: Variant = { id: `${id}:capture:${data.captureId}`, sourceId: "link", sourceName: "qcatch", title,
            url: page.href, transport: "video" };
        return { version: 1, mode: "exact", action: data.action,
            work: { id, title, category: "movies", url: page.href, variants: [variant] }, variant };
    }
    return normalize(data);
}
