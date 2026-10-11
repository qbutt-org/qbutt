import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import type { BrowserChoice, BrowserId } from "./protocol.ts";

export interface InstalledBrowser {
    id: BrowserId;
    name: string;
    executable: string;
    isDefault: boolean;
}

export function installedBrowsers(): InstalledBrowser[] {
    if (process.platform !== "win32") return [];
    const defaultKey = "HKCU\\Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\UserChoice";
    const defaultValue = spawnSync("reg.exe", ["query", defaultKey, "/v", "ProgId"], { encoding: "utf8", windowsHide: true }).stdout ?? "";
    const defaultId: BrowserId | undefined = /MSEdgeHTM/i.test(defaultValue) ? "edge"
        : /ChromeHTML/i.test(defaultValue) ? "chrome"
        : /FirefoxURL/i.test(defaultValue) ? "firefox" : undefined;
    const candidates: [BrowserId, string, string[]][] = [
        ["edge", "Microsoft Edge", ["Microsoft/Edge/Application/msedge.exe"]],
        ["chrome", "Google Chrome", ["Google/Chrome/Application/chrome.exe"]],
        ["firefox", "Mozilla Firefox", ["Mozilla Firefox/firefox.exe"]],
    ];
    const roots = [process.env.LOCALAPPDATA, process.env.ProgramFiles, process.env["ProgramFiles(x86)"]].filter(Boolean) as string[];
    return candidates.flatMap(([id, name, relative]) => {
        const executable = roots.flatMap((root) => relative.map((path) => join(root, path))).find(existsSync);
        return executable ? [{ id, name, executable, isDefault: id === defaultId }] : [];
    });
}

export function preferredBrowser(choice: BrowserChoice, installed: InstalledBrowser[]): BrowserId | undefined {
    if (choice !== "auto") return choice;
    return installed.find((browser) => browser.isDefault)?.id;
}

export function discoveryPath(): string {
    return process.env.QBUTT_BROWSER_BRIDGE_DISCOVERY
        ?? join(process.env.LOCALAPPDATA ?? process.env.HOME ?? ".", "qbutt", "content", "browser-bridge.json");
}
