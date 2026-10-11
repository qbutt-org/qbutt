import { createHash, randomUUID } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, mkdir, open, rename, unlink } from "node:fs/promises";
import { dirname } from "node:path";
import { ContentError } from "./errors";

export interface ArtifactPin {
    url: string;
    size: number;
    sha256: string;
}

export async function verifiedArtifact(pin: ArtifactPin, destination: string, signal: AbortSignal): Promise<void> {
    const origin = new URL(pin.url);
    if (origin.protocol !== "https:" || origin.hostname !== "github.com" || origin.username || origin.password
        || !/^\/qbutt-org\/(?:qplay|qcatch)\/releases\/download\/v[\w.-]+\/[^/]+$/.test(origin.pathname)
        || !Number.isSafeInteger(pin.size) || pin.size <= 0 || pin.size > 2 * 1024 ** 3
        || !/^[0-9a-f]{64}$/.test(pin.sha256)) {
        throw new ContentError("component_integrity_failed", "The component release pin is incomplete or invalid.");
    }
    const info = await lstat(destination).catch(error => {
        if (error.code === "ENOENT") return null;
        throw error;
    });
    if (info) {
        if (!info.isFile() || info.isSymbolicLink())
            throw new ContentError("component_integrity_failed", "The component cache path is not a regular file.");
        if (info.size === pin.size) {
            const hash = createHash("sha256");
            for await (const chunk of createReadStream(destination)) {
                signal.throwIfAborted();
                hash.update(chunk);
            }
            if (hash.digest("hex") === pin.sha256) return;
        }
    }
    await mkdir(dirname(destination), { recursive: true });
    const temporary = `${destination}.${randomUUID()}.partial`;
    try {
        let url = origin;
        let response: Response | undefined;
        for (let redirects = 0; redirects <= 5; ++redirects) {
            response = await fetch(url, { signal, redirect: "manual" });
            if (![301, 302, 303, 307, 308].includes(response.status)) break;
            const location = response.headers.get("Location");
            await response.body?.cancel();
            if (!location) throw new ContentError("component_download_failed", "The component download returned an incomplete redirect.");
            url = new URL(location, url);
            if (url.protocol !== "https:" || url.username || url.password
                || !["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"].includes(url.hostname))
                throw new ContentError("component_integrity_failed", "The component download redirected outside GitHub releases.");
        }
        if (!response?.ok || !response.body)
            throw new ContentError("component_download_failed", `The component download failed (HTTP ${response?.status ?? 0}). Try again.`);
        const file = await open(temporary, "wx", 0o600);
        const hash = createHash("sha256");
        let received = 0;
        try {
            for await (const chunk of response.body) {
                received += chunk.byteLength;
                if (received > pin.size) throw new ContentError("component_integrity_failed", "The component download exceeds the pinned size.");
                hash.update(chunk);
                await file.writeFile(chunk);
            }
            await file.sync();
        }
        finally { await file.close(); }
        if (received !== pin.size || hash.digest("hex") !== pin.sha256)
            throw new ContentError("component_integrity_failed", "The component download failed its integrity check. Try again.");
        signal.throwIfAborted();
        await rename(temporary, destination);
    }
    catch (error) {
        signal.throwIfAborted();
        if (error instanceof ContentError) throw error;
        throw new ContentError("component_download_failed", "The component download failed. Check your connection and try again.");
    }
    finally {
        await unlink(temporary).catch(error => {
            if (error.code !== "ENOENT") throw error;
        });
    }
}
