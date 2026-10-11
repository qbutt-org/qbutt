import { execFile, spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readFile, unlink, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import type { ResolvedContent } from "../contracts.ts";
import { discoveryPath, installedBrowsers, preferredBrowser, type InstalledBrowser } from "./discovery.ts";
import { MAX_MESSAGE_BYTES, publicUrl, type BrowserChoice, type BrowserCommand, type BrowserHello, type BrowserId, type BrowserResponse, type Selection } from "./protocol.ts";

export { runNativeMessaging } from "./native.ts";
export { installedBrowsers } from "./discovery.ts";
export type { BrowserChoice, BrowserId, Selection } from "./protocol.ts";

export class BrowserBridgeError extends Error {
    constructor(readonly code: string, message: string, readonly url?: string) {
        super(message);
        this.name = "BrowserBridgeError";
    }
}

interface Pending {
    connection: number;
    resolve(value: unknown): void;
    reject(error: Error): void;
}

export interface BrowserBridgeOptions {
    browser?: BrowserChoice;
    brokerPath?: string;
    discoveryPath?: string;
}

export class BrowserBridge {
    private broker?: ChildProcessWithoutNullStreams;
    private readonly connections = new Map<number, { browser: BrowserId; userAgent: string }>();
    private readonly pending = new Map<number, Pending>();
    private readonly browsers: InstalledBrowser[];
    private nextId = 0;
    private buffered = "";
    private unavailable?: string;
    private discoveryFile: string;

    constructor(private readonly options: BrowserBridgeOptions) {
        this.browsers = installedBrowsers();
        this.discoveryFile = options.discoveryPath ?? discoveryPath();
    }

    async start(): Promise<void> {
        if (this.broker || this.unavailable) return;
        if (process.platform !== "win32") {
            this.unavailable = "Подключение браузера доступно в Windows.";
            return;
        }
        const bundled = join(dirname(process.execPath), "browser", "private-pipe.ps1");
        const brokerPath = this.options.brokerPath ?? (existsSync(bundled) ? bundled : join(import.meta.dir, "private-pipe.ps1"));
        const pipeName = `qbutt-content-browser-${randomUUID()}`;
        const powershell = join(process.env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
        try {
            const broker = spawn(powershell, ["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", brokerPath, "-PipeName", pipeName], {
                windowsHide: true,
                stdio: ["pipe", "pipe", "pipe"],
            });
            this.broker = broker;
            broker.stdout.setEncoding("utf8");
            broker.stderr.resume();
            await new Promise<void>((resolve, reject) => {
                const timeout = setTimeout(() => reject(new Error("Browser bridge startup timed out")), 15_000);
                broker.once("error", reject);
                broker.once("exit", () => {
                    clearTimeout(timeout);
                    this.unavailable = "Не удалось подключить браузер. Перезапустите qbutt.";
                    this.disconnectAll();
                    reject(new Error("Browser bridge stopped"));
                });
                broker.stdout.on("data", (chunk: string) => {
                    try {
                        this.buffered += chunk;
                        if (Buffer.byteLength(this.buffered) > MAX_MESSAGE_BYTES * 2) throw new Error("Browser broker output exceeds the limit");
                        let end: number;
                        while ((end = this.buffered.indexOf("\n")) !== -1) {
                            const line = this.buffered.slice(0, end).trim();
                            this.buffered = this.buffered.slice(end + 1);
                            if (!line) continue;
                            const message = JSON.parse(line) as { ready?: boolean; connection?: number; closed?: boolean; frame?: string };
                            if (message.ready) {
                                clearTimeout(timeout);
                                resolve();
                            } else if (Number.isSafeInteger(message.connection)) {
                                if (message.closed) this.disconnect(message.connection!);
                                else if (typeof message.frame === "string") {
                                    try { this.receive(message.connection!, JSON.parse(message.frame)); }
                                    catch { this.broker?.stdin.write(`${JSON.stringify({ connection: message.connection, close: true })}\n`); }
                                }
                            }
                        }
                    } catch {
                        broker.kill();
                    }
                });
            });
            await mkdir(dirname(this.discoveryFile), { recursive: true });
            await writeFile(this.discoveryFile, JSON.stringify({ pipe: `\\\\.\\pipe\\${pipeName}`, pid: process.pid }), { mode: 0o600 });
        } catch {
            this.unavailable = "Не удалось подключить браузер. Перезапустите qbutt.";
            this.broker?.kill();
        }
    }

    status() {
        const selected = preferredBrowser(this.options.browser ?? "auto", this.browsers);
        const connection = this.connection();
        return {
            selected: this.options.browser ?? "auto",
            ready: !!connection,
            ...(connection ? {
                userAgent: this.connections.get(connection)!.userAgent,
                connectedBrowser: this.connections.get(connection)!.browser,
            } : {}),
            browsers: this.browsers.map((browser) => ({
                id: browser.id,
                name: browser.name,
                isDefault: browser.isDefault,
                connected: [...this.connections.values()].some((connection) => connection.browser === browser.id),
            })),
            ...(selected ? { detected: selected } : {}),
            ...(this.unavailable ? { error: this.unavailable } : {}),
        };
    }

    setBrowser(browser: BrowserChoice): void {
        this.options.browser = browser;
    }

    async connectBrowser(browser: BrowserChoice = this.options.browser ?? "auto") {
        const selected = preferredBrowser(browser, this.browsers)
            ?? (this.browsers.length === 1 ? this.browsers[0]?.id : undefined);
        if (!selected || !this.browsers.some((installed) => installed.id === selected)) {
            throw new BrowserBridgeError("browser_choice_required", "Choose an installed browser in Sources.");
        }
        const bundle = dirname(process.execPath);
        const script = join(bundle, "scripts", "register-content-browser.ps1");
        const extension = join(bundle, "browser-extension", selected === "firefox" ? "firefox" : "chromium");
        if (!existsSync(script) || !existsSync(join(extension, "manifest.json"))) {
            throw new BrowserBridgeError("browser_setup_missing", "Browser connection files are missing. Reinstall qbutt.");
        }
        await promisify(execFile)("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script,
            "-ContentExecutable", process.execPath, "-Browser", selected, "-ExtensionDirectory", extension, "-OpenSetup"], {
            windowsHide: true, timeout: 20_000, maxBuffer: 64 * 1024,
        });
        this.options.browser = browser;
        return {
            ...this.status(),
            browser: selected,
            extensionDirectory: extension,
        };
    }

    async cookies(url: string, options: { signal?: AbortSignal } = {}): Promise<{ name: string; value: string; domain: string; path: string; secure: boolean; hostOnly: boolean; expirationDate?: number }[]> {
        publicUrl(url);
        if (!this.connection()) return [];
        return this.request({ method: "cookies", url }, options.signal) as Promise<Awaited<ReturnType<BrowserBridge["cookies"]>>>;
    }

    async fetch(url: string, init: RequestInit = {}, options: { signal?: AbortSignal } = {}): Promise<Response> {
        publicUrl(url);
        if (init.body !== undefined && init.body !== null && typeof init.body !== "string") throw new Error("Browser fetch accepts a text body only");
        const response = await this.request({
            method: "fetch",
            url,
            init: {
                method: init.method ?? "GET",
                headers: Object.fromEntries(new Headers(init.headers).entries()),
                ...(typeof init.body === "string" ? { body: init.body } : {}),
            },
        }, options.signal ?? init.signal ?? undefined) as { status: number; headers: Record<string, string>; body: string; url: string };
        publicUrl(response.url);
        if (response.body.length > 512 * 1024) throw new Error("Browser response exceeds the limit");
        const result = new Response([204, 205, 304].includes(response.status) ? null : response.body, { status: response.status, headers: response.headers });
        Object.defineProperty(result, "url", { value: response.url });
        return result;
    }

    async resolve(url: string, selection: Selection = {}, options: { signal?: AbortSignal; kind?: "video" | "file" | "torrent"; referrer?: string } = {}): Promise<ResolvedContent> {
        publicUrl(url);
        const kind = options.kind ?? "video";
        const referrer = options.referrer ? publicUrl(options.referrer) : undefined;
        const result = await this.request({ method: "resolve", url, selection, kind, referrer }, options.signal) as ResolvedContent;
        const allowed = kind === "file" ? ["file", "torrent"] : [kind];
        if (!allowed.includes(result.kind) || result.resolver !== "direct" || typeof result.url !== "string") throw new Error("Invalid browser content response");
        publicUrl(result.url);
        const keys = new Set([...Object.keys(selection), ...Object.keys(result.selection ?? {})]);
        if ([...keys].some((key) => result.selection?.[key as keyof Selection] !== selection[key as keyof Selection])) throw new Error("Browser returned another media selection");
        return result;
    }

    async close(): Promise<void> {
        this.disconnectAll();
        this.broker?.stdin.end();
        this.broker?.kill();
        this.broker = undefined;
        try {
            const saved = JSON.parse(await readFile(this.discoveryFile, "utf8"));
            if (saved.pid === process.pid) await unlink(this.discoveryFile);
        } catch { /* Another profile may own discovery now. */ }
    }

    private connection(): number | undefined {
        const preferred = preferredBrowser(this.options.browser ?? "auto", this.browsers);
        if (preferred) return [...this.connections].find(([, connection]) => connection.browser === preferred)?.[0];
        return this.connections.size === 1 ? this.connections.keys().next().value : undefined;
    }

    private request(command: Omit<BrowserCommand, "id">, signal?: AbortSignal): Promise<unknown> {
        const connection = this.connection();
        if (!connection) throw new BrowserBridgeError("browser_required", this.unavailable ?? "Установите расширение qbutt в выбранном браузере.", command.url);
        if (this.pending.size >= 16) throw new BrowserBridgeError("browser_busy", "Браузер занят. Повторите действие через несколько секунд.", command.url);
        signal?.throwIfAborted();
        const id = ++this.nextId;
        return new Promise((resolve, reject) => {
            const cleanup = () => {
                clearTimeout(timeout);
                signal?.removeEventListener("abort", abort);
                this.pending.delete(id);
            };
            const abort = () => {
                this.send(connection, { id, method: "cancel" });
                cleanup();
                reject(signal?.reason ?? new Error("Browser request canceled"));
            };
            const timeout = setTimeout(() => {
                this.send(connection, { id, method: "cancel" });
                cleanup();
                reject(new BrowserBridgeError("browser_timeout", "Страница не ответила. Откройте источник и повторите действие.", command.url));
            }, command.method === "resolve" ? 45_000 : 20_000);
            signal?.addEventListener("abort", abort, { once: true });
            this.pending.set(id, {
                connection,
                resolve: (value) => { cleanup(); resolve(value); },
                reject: (error) => { cleanup(); reject(error); },
            });
            try { this.send(connection, { ...command, id }); }
            catch (error) { cleanup(); reject(error); }
        });
    }

    private send(connection: number, frame: unknown): void {
        const encoded = JSON.stringify(frame);
        if (Buffer.byteLength(encoded) > MAX_MESSAGE_BYTES) throw new Error("Browser message exceeds the limit");
        this.broker?.stdin.write(`${JSON.stringify({ connection, frame: encoded })}\n`);
    }

    private receive(connection: number, value: unknown): void {
        if (!value || typeof value !== "object") {
            this.broker?.stdin.write(`${JSON.stringify({ connection, close: true })}\n`);
            return;
        }
        if (!this.connections.has(connection)) {
            const hello = value as BrowserHello;
            if (hello.type !== "hello" || hello.version !== 1 || !["edge", "chrome", "firefox"].includes(hello.browser)
                || typeof hello.userAgent !== "string" || !hello.userAgent || hello.userAgent.length > 1024 || /[\r\n]/.test(hello.userAgent)) {
                this.broker?.stdin.write(`${JSON.stringify({ connection, close: true })}\n`);
                return;
            }
            for (const [old, connection] of this.connections) {
                if (connection.browser === hello.browser) {
                    this.broker?.stdin.write(`${JSON.stringify({ connection: old, close: true })}\n`);
                    this.disconnect(old);
                }
            }
            this.connections.set(connection, { browser: hello.browser, userAgent: hello.userAgent });
            return;
        }
        const response = value as BrowserResponse;
        const pending = this.pending.get(response.id);
        if (!pending || pending.connection !== connection) return;
        if (response.error) pending.reject(new BrowserBridgeError(response.error.code, response.error.message, response.error.url));
        else pending.resolve(response.result);
    }

    private disconnect(connection: number): void {
        this.connections.delete(connection);
        for (const pending of this.pending.values()) {
            if (pending.connection === connection) pending.reject(new BrowserBridgeError("browser_disconnected", "Браузер отключился. Откройте его и повторите действие."));
        }
    }

    private disconnectAll(): void {
        for (const connection of this.connections.keys()) this.disconnect(connection);
    }
}
