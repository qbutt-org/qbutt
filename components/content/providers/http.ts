import { load } from "cheerio";
import type { ProviderContext } from "../contracts.ts";
import { ContentError } from "../errors.ts";

function htmlDocument(text: string, url: string, headers: Headers): ReturnType<typeof load> {
    const $ = load(text);
    const anubis = $("script#anubis_challenge[type='application/json']").length > 0
        && $("script#anubis-main[src*='/.within.website/x/cmd/anubis/']").length > 0;
    const cloudflare = headers.get("cf-mitigated") === "challenge"
        || $("#challenge-form, #cf-challenge-running").length > 0
        && $("script[src*='/cdn-cgi/challenge-platform/']").length > 0;
    const ddosGuard = $("script[src*='check.ddos-guard.net/check.js']").length > 0;
    const captcha = $("form#captcha-form, form[action*='/captcha']").length > 0;
    if (anubis || cloudflare || ddosGuard || captcha)
        throw new ContentError("browser_check_required", "Откройте источник в браузере, пройдите проверку и повторите действие.", url);
    return $;
}

async function sourceResponse(url: string, context: ProviderContext, init?: RequestInit): Promise<Response> {
    const response = await context.fetch(url, init);
    const address = response.url || url;
    if (!response.ok) {
        if ([401, 403, 420].includes(response.status)) {
            throw new ContentError("browser_check_required", `Источник требует входа или проверки в браузере (HTTP ${response.status}).`, address);
        }
        throw new Error(`Источник вернул HTTP ${response.status}.`);
    }
    return response;
}

export async function json<T>(url: string, context: ProviderContext, init?: RequestInit): Promise<T> {
    const response = await sourceResponse(url, context, init);
    const text = await response.text();
    try {
        return JSON.parse(text) as T;
    }
    catch {
        htmlDocument(text, response.url || url, response.headers);
        throw new Error("Источник не предоставил ожидаемые данные.");
    }
}

export async function html(url: string, context: ProviderContext, init?: RequestInit): Promise<ReturnType<typeof load>> {
    const response = await sourceResponse(url, context, init);
    const charset = response.headers.get("content-type")?.match(/charset=([^;\s]+)/i)?.[1] ?? "utf-8";
    const text = new TextDecoder(charset).decode(await response.arrayBuffer());
    return htmlDocument(text, response.url || url, response.headers);
}
