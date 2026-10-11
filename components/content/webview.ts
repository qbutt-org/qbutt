import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { join, resolve } from "node:path";

import { ContentError } from "./errors";
import { publicUrl } from "./net";
import { NetworkProxy } from "./networkproxy";

export interface WebViewCookie {
    name: string;
    value: string;
    domain: string;
    path: string;
    expires?: number;
    secure: boolean;
    httpOnly: boolean;
    hostOnly: boolean;
}

export interface WebViewSession {
    url: string;
    userAgent: string;
    cookies: WebViewCookie[];
}

export type WebViewCookieIdentity = Pick<WebViewCookie, "name" | "domain" | "path" | "hostOnly">;

export interface WebViewCheck {
    configuredSession?: string;
    authoritativeCookies?: boolean;
    retainedCookies?: WebViewCookieIdentity[];
}

interface Clearance {
    controller: AbortController;
    promise: Promise<WebViewSession>;
    users: number;
    interactive: boolean;
    state: string;
}

export class WebViewSessions {
    private readonly pending = new Map<string, Clearance>();
    private readonly lifetime = new AbortController();

    constructor(private readonly dataDir: string, appDir: string,
        private readonly command = [join(appDir, "qbutt-webview.exe"), "--stdio"]) {}

    async check(url: string, signal: AbortSignal, interactive = false, initialCookies: WebViewCookie[] = [],
        options: WebViewCheck = {}): Promise<WebViewSession> {
        signal = AbortSignal.any([signal, this.lifetime.signal]);
        signal.throwIfAborted();
        const origin = (await publicUrl(url)).origin;
        signal.throwIfAborted();
        const configuredSession = options.configuredSession || createHash("sha256").update(JSON.stringify(initialCookies)).digest("hex");
        const state = createHash("sha256").update(JSON.stringify([configuredSession, !!options.authoritativeCookies,
            [...initialCookies].sort((left, right) => left.name.localeCompare(right.name)),
            [...(options.retainedCookies || [])].sort((left, right) => left.name.localeCompare(right.name))])).digest("hex");
        let entry = this.pending.get(origin);
        if (entry && (entry.controller.signal.aborted || interactive && !entry.interactive
            || entry.state !== state)) {
            try {
                await this.subscribe(entry, signal);
            }
            catch {
                signal.throwIfAborted();
            }
            return this.check(url, signal, interactive, initialCookies, options);
        }
        if (!entry) {
            const controller = new AbortController();
            entry = { controller, users: 0, interactive, state,
                promise: this.run(url, origin, controller.signal, interactive, initialCookies, { ...options, configuredSession }) };
            const current = entry;
            current.promise = current.promise.finally(() => {
                if (this.pending.get(origin) === current) this.pending.delete(origin);
            });
            this.pending.set(origin, current);
        }
        return this.subscribe(entry, signal);
    }

    private subscribe(entry: Clearance, signal: AbortSignal): Promise<WebViewSession> {
        signal.throwIfAborted();
        ++entry.users;
        return new Promise((resolve, reject) => {
            let finished = false;
            const finish = (action: () => void) => {
                if (finished) return;
                finished = true;
                signal.removeEventListener("abort", abort);
                if (--entry.users === 0) entry.controller.abort();
                action();
            };
            const abort = () => finish(() => reject(signal.reason));
            signal.addEventListener("abort", abort, { once: true });
            entry.promise.then(value => finish(() => resolve(value)), error => finish(() => reject(error)));
        });
    }

    private async run(url: string, origin: string, signal: AbortSignal, interactive: boolean,
        initialCookies: WebViewCookie[], options: WebViewCheck): Promise<WebViewSession> {
        const proxy = await NetworkProxy.start();
        try {
            signal.throwIfAborted();
            const endpoint = new URL(proxy.url);
            const request = {
                id: randomUUID(), url,
                profileDirectory: resolve(this.dataDir, "web-session", createHash("sha256").update(origin).digest("hex")),
                mode: interactive ? "interactive" : "silent", timeoutMs: interactive ? 120_000 : 15_000,
                initialCookies, configuredSession: options.configuredSession,
                ...(options.authoritativeCookies ? { authoritativeCookies: true, retainedCookies: options.retainedCookies || [] } : {}),
                proxy: { url: endpoint.origin, username: decodeURIComponent(endpoint.username), password: decodeURIComponent(endpoint.password) },
            };
            if (Buffer.byteLength(JSON.stringify(request)) > 16 * 1024)
                throw new ContentError("source_session_invalid", "The configured source session is too large.", url);
            const text = await new Promise<string>((resolve, reject) => {
                const child = spawn(this.command[0], this.command.slice(1), { windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
                let output = "";
                let failure: Error | undefined;
                let killTimer: ReturnType<typeof setTimeout> | undefined;
                const stop = (error: Error) => {
                    if (failure) return;
                    failure = error;
                    child.stdin.write(JSON.stringify({ id: request.id, method: "cancel" }) + "\n");
                    killTimer = setTimeout(() => child.kill(), 1000);
                };
                const abort = () => stop(signal.reason);
                const timeout = setTimeout(() => stop(new ContentError("webview_timeout", "The source check did not finish in time.", url)), request.timeoutMs + 3000);
                child.stdout.setEncoding("utf8");
                child.stderr.resume();
                child.stdin.on("error", () => {});
                child.stdout.on("data", (chunk: string) => {
                    if (failure) return;
                    output += chunk;
                    if (Buffer.byteLength(output) > 256 * 1024) {
                        stop(new ContentError("webview_failed", "The source returned too much session data.", url));
                        child.kill();
                    }
                });
                child.once("error", () => {
                    failure = new ContentError("webview_unavailable", "The source check component is unavailable. Reinstall qbutt.", url);
                });
                child.once("close", code => {
                    clearTimeout(timeout);
                    clearTimeout(killTimer);
                    signal.removeEventListener("abort", abort);
                    if (failure) reject(failure);
                    else if (code !== 0 && !output.trim()) reject(new ContentError("webview_failed", "Could not check the source.", url));
                    else resolve(output);
                });
                signal.addEventListener("abort", abort, { once: true });
                child.stdin.write(JSON.stringify(request) + "\n");
                if (signal.aborted) abort();
            });
            let response: { id?: string; session?: WebViewSession; error?: { code?: string; message?: string } };
            try { response = JSON.parse(text); }
            catch { throw new ContentError("webview_failed", "The source check returned an invalid response.", url); }
            if (!response || response.id !== request.id) throw new ContentError("webview_failed", "The source check response belongs to another request.", url);
            if (response.error)
                throw new ContentError(response.error.code || "webview_failed", response.error.message || "Could not complete the source check.", url);
            const session = response.session;
            if (!session || typeof session.url !== "string" || typeof session.userAgent !== "string"
                || !session.userAgent || session.userAgent.length > 1024 || /[\r\n]/.test(session.userAgent)
                || !Array.isArray(session.cookies) || session.cookies.length > 1024
                || session.cookies.some(cookie => !cookie || [cookie.name, cookie.value, cookie.domain, cookie.path]
                    .some(value => typeof value !== "string" || /[\r\n]/.test(value))
                    || typeof cookie.secure !== "boolean" || typeof cookie.httpOnly !== "boolean" || typeof cookie.hostOnly !== "boolean"
                    || cookie.expires !== undefined && !Number.isFinite(cookie.expires)))
                throw new ContentError("webview_failed", "The source check returned an invalid session.", url);
            const host = new URL(url).hostname;
            if (session.cookies.some(cookie => {
                const domain = cookie.domain.replace(/^\./, "").toLowerCase();
                return cookie.hostOnly ? domain !== host : domain !== host && !host.endsWith("." + domain);
            })) throw new ContentError("webview_failed", "The source check returned another source's session.", url);
            await publicUrl(session.url);
            return session;
        }
        finally {
            await proxy.close();
        }
    }

    close(): void {
        this.lifetime.abort();
    }
}
