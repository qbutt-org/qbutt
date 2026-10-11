import { inflateRawSync } from "node:zlib";
import type { CheerioAPI } from "cheerio";
import type { ProviderContext } from "../../contracts.ts";

interface PublicPaste {
    status: number;
    v: number;
    adata: [[string, string, number, number, number, string, string, string], string, number, number];
    ct: string;
}

function base58(value: string): ArrayBuffer {
    if (!value || value.length > 44) throw new Error("Неверная длина ключа публичного зеркала FitGirl.");
    const alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let number = 0n;
    for (const character of value) {
        const digit = alphabet.indexOf(character);
        if (digit < 0) throw new Error("Неверный ключ публичного зеркала FitGirl.");
        number = number * 58n + BigInt(digit);
    }
    const hexadecimal = number.toString(16);
    const bytes = number === 0n ? Buffer.alloc(0)
        : Buffer.from(hexadecimal.padStart(Math.ceil(hexadecimal.length / 2) * 2, "0"), "hex");
    const decoded = new Uint8Array(Buffer.concat([Buffer.alloc(value.match(/^1*/)?.[0].length ?? 0), bytes]));
    if (decoded.byteLength !== 32) throw new Error("Неверная длина ключа публичного зеркала FitGirl.");
    return decoded.buffer;
}

async function mirrorText(url: URL, context: ProviderContext): Promise<string> {
    // PrivateBin encrypts public mirrors with the key published in the link fragment.
    // Decode the documented format locally; never run publisher-owned JavaScript.
    const response = await context.fetch(url.href.split("#")[0]!, {
        headers: { Accept: "application/json", "X-Requested-With": "JSONHttpRequest" },
    });
    if (!response.ok) throw new Error("Публичное зеркало FitGirl временно недоступно.");
    const paste = await response.json() as PublicPaste;
    if (paste.status !== 0 || paste.v !== 2 || !Array.isArray(paste.adata?.[0]) || typeof paste.ct !== "string") {
        throw new Error("Формат публичного зеркала FitGirl изменился.");
    }
    const [iv, salt, iterations, bits, tag, cipher, mode, compression] = paste.adata[0];
    if (cipher !== "aes" || mode !== "gcm" || compression !== "zlib" || bits !== 256 || tag !== 128
        || !Number.isInteger(iterations) || iterations < 1 || iterations > 2000000 || paste.ct.length > 2 * 1024 * 1024) {
        throw new Error("Параметры публичного зеркала FitGirl не поддерживаются.");
    }
    const secret = await crypto.subtle.importKey("raw", base58(url.hash.slice(1)), "PBKDF2", false, ["deriveKey"]);
    const key = await crypto.subtle.deriveKey({
        name: "PBKDF2", salt: Buffer.from(salt, "base64"), iterations, hash: "SHA-256",
    }, secret, { name: "AES-GCM", length: bits }, false, ["decrypt"]);
    const clear = await crypto.subtle.decrypt({
        name: "AES-GCM", iv: Buffer.from(iv, "base64"), tagLength: tag,
        additionalData: new TextEncoder().encode(JSON.stringify(paste.adata)),
    }, key, Buffer.from(paste.ct, "base64"));
    const decoded = JSON.parse(inflateRawSync(Buffer.from(clear), { maxOutputLength: 2 * 1024 * 1024 }).toString("utf8")) as { paste?: string };
    return typeof decoded.paste === "string" ? decoded.paste : "";
}

export async function expandFitgirlMirrors($: CheerioAPI, context: ProviderContext): Promise<void> {
    for (const element of $('.entry-content a[href*="paste.fitgirl-repacks.site/"]').toArray()) {
        const anchor = $(element);
        const url = new URL(anchor.attr("href")!);
        if (url.hostname !== "paste.fitgirl-repacks.site" || !url.hash) continue;
        const text = await mirrorText(url, context);
        const links = [...new Set(text.match(/https?:\/\/[^\s<>"\])]+/g) ?? [])];
        // An embedded .torrent attachment has no public HTTP URL: retain its
        // publisher link for browser acquisition instead of inventing one.
        if (!links.length) continue;
        const label = anchor.text();
        const list = $("<ul></ul>");
        for (const link of links) {
            const mirror = new URL(link);
            const filename = decodeURIComponent(mirror.pathname.split("/").pop() ?? "");
            const entry = $("<li></li>");
            entry.append($("<a></a>").attr("href", mirror.href).attr("data-publisher-mirror", "true")
                .text([label, mirror.hostname, filename].filter(Boolean).join(", ")));
            list.append(entry);
        }
        anchor.closest("li").append(list);
        anchor.remove();
    }
}
