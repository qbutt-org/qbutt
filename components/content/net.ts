import { lookup } from "node:dns/promises";
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import { isIP } from "node:net";
import { brotliDecompressSync, gunzipSync, inflateSync } from "node:zlib";
import { CookieJar } from "tough-cookie";

import type { ProviderContext } from "./contracts";

const maxResponseBytes = 24 * 1024 * 1024;
export const defaultUserAgent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/142.0.0.0 Safari/537.36";

function isPrivateAddress(address: string): boolean {
    let value = address.toLowerCase();
    const mapped = /^::ffff:([a-f\d]{1,4}):([a-f\d]{1,4})$/.exec(value);
    if (mapped) {
        const high = parseInt(mapped[1], 16);
        const low = parseInt(mapped[2], 16);
        value = `${high >>> 8}.${high & 255}.${low >>> 8}.${low & 255}`;
    }
    else
        value = value.replace(/^::ffff:/, "");
    if (isIP(value) === 4) {
        const [a, b] = value.split(".").map(Number);
        return a === 0 || a === 10 || a === 127 || a >= 224
            || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31)
            || (a === 192 && b === 168) || (a === 100 && b >= 64 && b <= 127);
    }

    return !/^[23][a-f\d]{3}:/.test(value);
}

export async function publicAddresses(url: URL) {
    if (!["https:", "http:"].includes(url.protocol) || url.username || url.password)
        throw new Error("This source URL is not supported.");

    const host = url.hostname.replace(/^\[|\]$/g, "");
    const addresses = isIP(host)
        ? [{ address: host, family: isIP(host) }]
        : await lookup(host, { all: true });

    if (!addresses.length || addresses.some(item => isPrivateAddress(item.address)))
        throw new Error("Sources cannot access local network addresses.");

    return addresses;
}

export async function publicUrl(input: string): Promise<URL> {
    const url = new URL(input);
    await publicAddresses(url);
    return url;
}

async function request(url: URL, init: RequestInit, signal: AbortSignal): Promise<Response> {
    const addresses = await publicAddresses(url);
    signal.throwIfAborted();

    const outgoing = new Request(url, init);
    const body = outgoing.body ? Buffer.from(await outgoing.arrayBuffer()) : undefined;
    if (body && body.length > 1024 * 1024)
        throw new Error("The source request is too large.");

    return new Promise((resolve, reject) => {
        const send = url.protocol === "https:" ? httpsRequest : httpRequest;
        const connection = send(url, {
            method: outgoing.method,
            headers: Object.fromEntries(outgoing.headers.entries()),
            signal: AbortSignal.any([signal, AbortSignal.timeout(25_000)]),
            // Pin the connection to checked public addresses: resolving again
            // inside the socket would allow a DNS change to bypass this guard.
            lookup(_host, options, callback) {
                if (options.all)
                    callback(null, addresses);
                else
                    callback(null, addresses[0].address, addresses[0].family);
            },
        }, incoming => {
            if (Number(incoming.headers["content-length"]) > maxResponseBytes) {
                incoming.destroy(new Error("The source response is too large."));
                return;
            }

            const chunks: Buffer[] = [];
            let length = 0;
            incoming.on("data", chunk => {
                length += chunk.length;
                if (length > maxResponseBytes)
                    incoming.destroy(new Error("The source response is too large."));
                else
                    chunks.push(chunk);
            });
            incoming.on("error", reject);
            incoming.on("end", () => {
                try {
                    let payload = Buffer.concat(chunks);
                    const encoding = incoming.headers["content-encoding"];
                    const limits = { maxOutputLength: maxResponseBytes };
                    if (encoding === "gzip")
                        payload = gunzipSync(payload, limits);
                    else if (encoding === "br")
                        payload = brotliDecompressSync(payload, limits);
                    else if (encoding === "deflate")
                        payload = inflateSync(payload, limits);

                    const headers = new Headers();
                    for (const [name, value] of Object.entries(incoming.headers)) {
                        for (const item of Array.isArray(value) ? value : value ? [value] : [])
                            headers.append(name, item);
                    }
                    headers.delete("Content-Encoding");
                    headers.delete("Content-Length");

                    const status = incoming.statusCode || 502;
                    const response = new Response([204, 205, 304].includes(status) ? null : payload, { status, headers });
                    Object.defineProperty(response, "url", { value: url.href });
                    resolve(response);
                }
                catch (error) {
                    reject(error);
                }
            });
        });

        connection.on("error", reject);
        connection.end(body);
    });
}

export function sourceFetch(signal: AbortSignal, jar: CookieJar,
    cookie?: { origin: string; value: string }, userAgent?: string): ProviderContext["fetch"] {
    return async (input, init = {}) => {
        const requestSignal = init.signal ? AbortSignal.any([signal, init.signal]) : signal;
        let url = new URL(input);
        let method = init.method || "GET";
        let body = init.body;
        const headers = new Headers(init.headers);
        if (!headers.has("User-Agent")) headers.set("User-Agent", userAgent || defaultUserAgent);
        headers.set("Accept-Encoding", "identity");

        for (let redirects = 0; redirects <= 5; ++redirects) {
            const currentHeaders = new Headers(headers);
            if (!currentHeaders.has("Cookie")) {
                const value = cookie?.origin === url.origin ? cookie.value : await jar.getCookieString(url.href);
                if (value)
                    currentHeaders.set("Cookie", value);
            }

            const response = await request(url, { ...init, method, body, headers: currentHeaders }, requestSignal);
            for (const value of response.headers.getSetCookie())
                await jar.setCookie(value, url.href, { ignoreError: true });

            if (![301, 302, 303, 307, 308].includes(response.status))
                return response;

            const target = response.headers.get("Location");
            if (!target)
                throw new Error("The source returned an incomplete redirect.");

            const next = new URL(target, url);
            if (next.origin !== url.origin) {
                headers.delete("Cookie");
                headers.delete("Authorization");
                headers.delete("Proxy-Authorization");
            }
            if (response.status === 303 || ([301, 302].includes(response.status) && method.toUpperCase() === "POST")) {
                method = "GET";
                body = undefined;
                headers.delete("Content-Type");
                headers.delete("Content-Length");
            }
            url = next;
        }

        throw new Error("The source redirected too many times.");
    };
}

export function safeError(error: unknown): string {
    if (error instanceof DOMException && error.name === "AbortError")
        return "Cancelled";

    const message = error instanceof Error ? error.message : "The source is unavailable.";
    return message.replace(/https?:\/\/[^\s"<>]+/g, "[source]")
        .replace(/(cookie|token|passkey|api[_-]?key|authorization)\s*[:=]\s*[^\s,;]+/gi, "$1=[hidden]")
        .slice(0, 240);
}
