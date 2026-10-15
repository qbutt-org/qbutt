import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { lstat, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join } from "node:path";

// Run in an isolated Windows VM. This closed proxy serves generated HTML only;
// it never forwards requests. Interactive cases may show a fixture window.
const executable = process.argv[2];
assert(executable && isAbsolute(executable), "Pass the absolute qbutt-webview.exe path");
const directory = await mkdtemp(join(tmpdir(), "qbutt-webview-html-"));
const origin = "http://fixture.example";
const digest = (value: string) => createHash("sha256").update(value).digest("hex");
const jsdPath = "/cdn-cgi/challenge-platform/scripts/jsd/main.js";
const scenarios = [
    { id: "normal-jsd", status: 200, body: `<script src="${jsdPath}"></script><main>Generated legal catalog</main>` },
    { id: "normal-jsd-restart", status: 200, body: `<script src="${jsdPath}"></script><main>Generated legal catalog after restart</main>` },
    { id: "challenge-form", status: 200, body: '<form id="challenge-form"></form>', error: "browser_check_required" },
    { id: "challenge-stage", status: 200, body: '<div id="challenge-stage"></div>', error: "browser_check_required" },
    { id: "turnstile", status: 200, body: '<div class="cf-turnstile"></div>', error: "browser_check_required" },
    { id: "captcha", status: 200, body: '<textarea name="g-recaptcha-response"></textarea>', error: "browser_check_required" },
    { id: "forbidden", status: 403, body: "Access denied", error: "browser_check_required" },
    {
        id: "silent-recovery", status: 403,
        body: '<p>Generated delayed access check</p><script>setTimeout(() => location.replace("/accepted"), 1800)</script>',
        arrived: `${origin}/accepted`,
    },
    { id: "interactive-forbidden", status: 403, body: "Access denied", mode: "interactive", error: "browser_check_required" },
    {
        id: "interactive-recovery", status: 403, mode: "interactive",
        body: '<p>Generated delayed access check</p><script>setTimeout(() => location.replace("/accepted"), 1800)</script>',
        arrived: `${origin}/accepted`,
    },
    { id: "unavailable", status: 503, body: "Temporarily unavailable", error: "webview_failed" },
];
let scripts = 0;
let failure: unknown;
try {
    for (const scenario of scenarios) {
        const password = randomBytes(32).toString("hex");
        const authorization = `Basic ${Buffer.from(`qbutt:${password}`).toString("base64")}`;
        let challenges = 0;
        let authenticated = 0;
        let bootstraps = 0;
        const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch(incoming) {
            if (incoming.headers.get("proxy-authorization") !== authorization) {
                challenges++;
                return new Response(null, { status: 407, headers: {
                    "Proxy-Authenticate": 'Basic realm="qbutt"',
                    "Connection": "close",
                    "Content-Length": "0",
                } });
            }
            authenticated++;
            const address = new URL(incoming.url);
            if (incoming.method === "GET" && incoming.url === "http://qbutt-webview.invalid/") {
                bootstraps++;
                return new Response("", { headers: { "Content-Type": "text/html", "Cache-Control": "no-store" } });
            }
            if (address.origin !== origin)
                return new Response(null, { status: 403 });
            if (address.pathname === jsdPath) {
                scripts++;
                return new Response("", { headers: { "Content-Type": "application/javascript" } });
            }
            if (scenario.arrived === `${origin}${address.pathname}`) {
                return new Response("<!doctype html><title>Generated accepted catalog</title><main>Ready</main>", {
                    headers: { "Content-Type": "text/html", "Cache-Control": "no-store" },
                });
            }
            if (address.pathname !== `/${scenario.id}`)
                return new Response(null, { status: 404 });
            return new Response(`<!doctype html><html><head><title>Generated fixture</title></head><body>${scenario.body}</body></html>`, {
                status: scenario.status,
                headers: { "Content-Type": "text/html; charset=utf-8" },
            });
        } });
        const url = `${origin}/${scenario.id}`;
        const request = {
            id: scenario.id,
            url,
            profileDirectory: join(directory, "web-session", digest(origin)),
            mode: scenario.mode ?? "silent",
            timeoutMs: 5_000,
            configuredSession: digest("[]"),
            initialCookies: [],
            authoritativeCookies: false,
            proxy: { url: `http://127.0.0.1:${server.port}`, username: "qbutt", password },
        };
        const child = Bun.spawn([executable, "--stdio"], {
            stdin: "pipe", stdout: "pipe", stderr: "ignore", windowsHide: true,
        });
        const deadline = setTimeout(() => child.kill(), 8_000);
        try {
            const started = performance.now();
            child.stdin.write(`${JSON.stringify(request)}\n`);
            await child.stdin.flush();
            const output = await new Response(child.stdout).text();
            assert(Buffer.byteLength(output) <= 256 * 1024, "Bounded helper response");
            assert.equal(await child.exited, 0, `Helper exit for ${scenario.id}`);
            const response = JSON.parse(output);
            assert.equal(response.id, scenario.id);
            assert(challenges > 0, `Proxy challenge for ${scenario.id}`);
            assert(authenticated > 0, `Proxy authenticated for ${scenario.id}`);
            assert.equal(bootstraps, 1, `Single proxy bootstrap for ${scenario.id}`);
            if (scenario.id === "forbidden" || scenario.id === "interactive-forbidden")
                assert(performance.now() - started >= 4_500, "Access check remains active until its deadline");
            if (scenario.error) {
                assert.equal(response.error?.code, scenario.error, scenario.id);
                assert.equal(response.session, undefined, `No session for ${scenario.id}`);
            }
            else {
                assert.equal(response.error, undefined, scenario.id);
                assert.equal(response.session?.url, scenario.arrived ?? url);
                assert(response.session.userAgent.length > 0);
                assert(Array.isArray(response.session.cookies));
            }
            console.log(`PASS webview_helper ${scenario.id}=${scenario.error ?? "session"}`);
        }
        finally {
            clearTimeout(deadline);
            if (child.exitCode === null)
                child.kill();
            await child.exited;
            server.stop(true);
        }
    }
    assert(scripts > 0, "The normal page really loaded its JSD script");
}
catch (error) {
    failure = error;
    throw error;
}
finally {
    assert(!(await lstat(directory)).isSymbolicLink(), "Fixture cleanup rejects reparse links");
    const resolved = await realpath(directory);
    const temporaryRoot = await realpath(tmpdir());
    assert.equal(dirname(resolved).toLowerCase(), temporaryRoot.toLowerCase(), "Fixture cleanup stays within Temp");
    let cleanup: unknown;
    for (let attempt = 0; attempt < 50; attempt++) {
        try {
            await rm(directory, { recursive: true, force: true });
            cleanup = undefined;
            break;
        }
        catch (error) {
            cleanup = error;
            await Bun.sleep(100);
        }
    }
    if (cleanup) {
        if (failure)
            console.error("Fixture profile cleanup failed after helper exit");
        else
            throw cleanup;
    }
}
