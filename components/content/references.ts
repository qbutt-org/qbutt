import type { Variant, Work } from "./contracts";

export function assertExactVariant(expected: Variant, current: Variant): void {
    if (expected.sourceId !== current.sourceId || expected.id !== current.id
        || expected.url !== current.url || expected.transport !== current.transport)
        throw new Error("Выбранный вариант изменился. Обновите варианты источника.");

    for (const field of ["infoHash", "version", "edition", "cooperative", "publicationKind",
        "episode", "season", "translation", "player"] as const) {
        if (expected[field] !== undefined && expected[field] !== current[field])
            throw new Error("Выбранная версия изменилась. Обновите варианты источника.");
    }
}

const sensitiveKey = /(?:token|passkey|password|secret|auth|cookie|signature|api.?key)|^(?:key|sig|expires|policy|x-amz-.+|x-goog-.+)$/i;

export function publicReference(value: unknown): string | undefined {
    if (typeof value !== "string" || value.length > 4096)
        return undefined;
    try {
        const url = new URL(value);
        if (url.protocol === "magnet:") {
            const magnet = new URL("magnet:?");
            for (const hash of url.searchParams.getAll("xt")) {
                if (/^urn:bt(?:ih|mh):[a-z0-9]+$/i.test(hash))
                    magnet.searchParams.append("xt", hash);
            }
            return magnet.searchParams.has("xt") ? magnet.href : undefined;
        }
        if (!["https:", "http:"].includes(url.protocol) || url.username || url.password)
            return undefined;
        for (const key of [...url.searchParams.keys()]) {
            if (sensitiveKey.test(key))
                url.searchParams.delete(key);
        }
        url.hash = "";
        return url.href;
    }
    catch {
        return undefined;
    }
}

export function linkId(url: string): string {
    return `link:${new Bun.CryptoHasher("sha256").update(publicReference(url) || url).digest("hex")}`;
}

export function captureSelectionId(variant: Variant): string | undefined {
    if (variant.sourceId !== "link") return;
    const prefix = `${linkId(variant.url)}:capture:`;
    if (!variant.id.startsWith(prefix)) return;
    const id = variant.id.slice(prefix.length);
    if (/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i.test(id)) return id;
}

function persistentNotes(notes: Work["notes"]): Work["notes"] {
    return notes?.flatMap(note => {
        const url = publicReference(note.url);
        return url ? [{ ...note, url }] : [];
    });
}

export function persistentVariant(variant: Variant): Variant {
    const { downloadUrl, ...stable } = variant;
    return {
        ...stable,
        id: variant.sourceId === "link" && !captureSelectionId(variant) ? linkId(variant.url) : variant.id,
        url: publicReference(variant.url) || "",
        magnet: publicReference(variant.magnet),
        notes: persistentNotes(variant.notes),
    };
}

export function persistentWork(work: Work): Work {
    const variants = work.variants.map(persistentVariant);
    return {
        ...work,
        id: work.variants.length && work.variants.every(variant => variant.sourceId === "link") ? linkId(variants[0].url)
            : /^(?:https?:|magnet:)/.test(work.id) ? linkId(work.id) : work.id,
        url: publicReference(work.url),
        cover: publicReference(work.cover),
        notes: persistentNotes(work.notes),
        variants,
    };
}
