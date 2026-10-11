import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { CookieJar } from "../../components/content/node_modules/tough-cookie";
import type { BrowserBridge } from "../../components/content/browser";
import { Catalog } from "../../components/content/catalog";
import { SourceSettings } from "../../components/content/settings";
import { LibraryStore } from "../../components/content/store";

type BrowserCookie = Awaited<ReturnType<BrowserBridge["cookies"]>>[number];

// The browser adapter supplies generated official cookies API snapshots. Catalog,
// tough-cookie, SQLite and Windows DPAPI are the real integration boundaries.
class CookieBrowser {
    snapshot: BrowserCookie[] = [];
    queued: BrowserCookie[][] = [];
    calls = 0;

    async cookies(_url: string, options: { signal?: AbortSignal } = {}): Promise<BrowserCookie[]> {
        options.signal?.throwIfAborted();
        ++this.calls;
        return this.queued.shift() || this.snapshot;
    }

    status() {
        return { userAgent: "qbutt-generated-cookie-fixture" };
    }
}

const root = await mkdtemp(join(tmpdir(), "qbutt-catalog-cookies-"));
const store = await LibraryStore.open(root);
const settings = new SourceSettings(store);
const browser = new CookieBrowser();
const catalog = new Catalog(root, root, settings, browser as unknown as BrowserBridge);
const signal = new AbortController().signal;
const secret = `generated-cookie-${crypto.randomUUID()}`;

async function cookies(sourceId: string, url: string): Promise<Record<string, string>> {
    const headers = await catalog.acquisitionHeaders(sourceId, url, {});
    return Object.fromEntries((headers.Cookie || "").split(/;\s*/).filter(Boolean).map(cookie => {
        const separator = cookie.indexOf("=");
        return [cookie.slice(0, separator), cookie.slice(separator + 1)];
    }));
}

try {
    const hostCookie: BrowserCookie = {
        name: "__Host-qbutt_session", value: `${secret}-first`, domain: "freetp.org",
        path: "/", secure: true, hostOnly: true, httpOnly: true, sameSite: "strict",
    };
    const domainCookie: BrowserCookie = {
        name: "qbutt_domain", value: `${secret}-domain`, domain: ".freetp.org",
        path: "/", secure: true, hostOnly: false, httpOnly: false, sameSite: "no_restriction",
    };
    browser.snapshot = [hostCookie, domainCookie];
    await catalog.context("freetp", signal);
    assert.deepEqual(await cookies("freetp", "https://freetp.org/download"), {
        [hostCookie.name]: hostCookie.value,
        [domainCookie.name]: domainCookie.value,
    });
    assert.deepEqual(await cookies("freetp", "https://cdn.freetp.org/download"), {
        [domainCookie.name]: domainCookie.value,
    }, "A __Host cookie must not become a Domain cookie");
    assert.deepEqual(await cookies("freetp", "http://freetp.org/download"), {}, "Secure cookies require HTTPS");
    assert.deepEqual(await cookies("freetp", "https://unrelated.qbutt.test/download"), {});

    // Apply a generated source Set-Cookie to Catalog's actual jar.
    const jar = (catalog as unknown as { sessions: Map<string, { jar: CookieJar }> })
        .sessions.get("freetp")!.jar;
    const imported = await jar.getCookies("https://freetp.org/");
    assert.equal(imported.find(cookie => cookie.key === hostCookie.name)?.httpOnly, true);
    assert.equal(imported.find(cookie => cookie.key === hostCookie.name)?.sameSite, "strict");
    assert.equal(imported.find(cookie => cookie.key === domainCookie.name)?.httpOnly, false);
    assert.equal(imported.find(cookie => cookie.key === domainCookie.name)?.sameSite, "none");
    await jar.setCookie("qbutt_server=generated-source-cookie; Domain=freetp.org; Path=/; Secure", "https://freetp.org/");
    browser.snapshot = [{ ...hostCookie, value: `${secret}-replacement`, sameSite: "lax" }];
    await catalog.context("freetp", signal);
    assert.equal((await jar.getCookies("https://freetp.org/")).find(cookie => cookie.key === hostCookie.name)?.sameSite, "lax");
    assert.deepEqual(await cookies("freetp", "https://freetp.org/download"), {
        [hostCookie.name]: `${secret}-replacement`,
        qbutt_server: "generated-source-cookie",
    }, "Refresh must replace browser cookies and preserve unrelated source Set-Cookie");

    browser.snapshot = [];
    await catalog.context("freetp", signal);
    assert.deepEqual(await cookies("freetp", "https://freetp.org/download"), {
        qbutt_server: "generated-source-cookie",
    }, "Browser logout must remove the imported session");

    browser.snapshot = [{ ...hostCookie, sameSite: "unspecified" }];
    await catalog.context("freetp", signal);
    assert.equal((await jar.getCookies("https://freetp.org/")).find(cookie => cookie.key === hostCookie.name)?.sameSite, undefined);

    browser.snapshot = [hostCookie, domainCookie];
    await catalog.context("freetp", signal);
    const callsBeforeManual = browser.calls;
    const manualCookie = `manual_session=${secret}-manual`;
    await settings.configure("freetp", undefined, { cookie: manualCookie });
    const context = await catalog.context("freetp", signal);
    assert.equal(browser.calls, callsBeforeManual, "A manual session must skip browser import");
    assert.equal(context.settings.cookie, manualCookie);
    assert.deepEqual(await cookies("freetp", "https://freetp.org/download"), {
        manual_session: `${secret}-manual`,
    }, "A manual session must win over the browser and source jar");
    assert.deepEqual(await cookies("freetp", "https://cdn.freetp.org/download"), {
        qbutt_server: "generated-source-cookie",
    }, "Manual-session refresh must also revoke old imported Domain cookies");
    assert.equal((await new SourceSettings(store).values("freetp")).cookie, manualCookie,
        "The generated manual session must round-trip through Windows DPAPI");
    const database = await readFile(join(root, "library.sqlite"));
    const wal = await readFile(join(root, "library.sqlite-wal")).catch(() => Buffer.alloc(0));
    assert(!Buffer.concat([database, wal]).includes(Buffer.from(secret)), "Cookies must not be stored in plaintext");

    browser.queued = [
        [{
            name: "qbutt_previous", value: "generated-previous", domain: "byrutgame.org",
            path: "/", secure: true, hostOnly: true,
        }],
        [{
            name: "qbutt_current", value: "generated-current", domain: "byrutgame.org",
            path: "/", secure: true, hostOnly: true,
        }],
    ];
    await Promise.all([catalog.context("byrut", signal), catalog.context("byrut", signal)]);
    browser.snapshot = [];
    await catalog.context("byrut", signal);
    assert.deepEqual(await cookies("byrut", "https://byrutgame.org/download"), {},
        "Logout must revoke every imported cookie after concurrent provider contexts");

    console.log(JSON.stringify({ ok: true, checks: ["host-only-prefix", "domain-scope", "secure-scope",
        "refresh-replacement", "logout", "source-cookie-preservation", "manual-session-priority", "DPAPI",
        "concurrent-refresh-logout", "httpOnly", "sameSite"] }));
}
finally {
    catalog.close();
    store.close();
    await rm(root, { recursive: true, force: true });
}
