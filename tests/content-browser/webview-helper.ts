import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join } from "node:path";

// Run in an isolated Windows VM. The actual helper uses its silent mode and
// this closed proxy serves generated HTML only; it never forwards requests.
const executable = process.argv[2];
assert(executable && isAbsolute(executable), "Pass the absolute qbutt-webview.exe path");
const directory = await mkdtemp(join(tmpdir(), "qbutt-webview-html-"));
const origin = "http://fixture.example";
const digest = (value: string) => createHash("sha256").update(value).digest("hex");
const password = randomBytes(32).toString("hex");
const jsdPath = "/cdn-cgi/challenge-platform/scripts/jsd/main.js";
const scenarios = [
    { id: "normal-jsd", status: 200, body: `<script src="${jsdPath}"></script><main>Generated legal catalog</main>` },
    { id: "challenge-form", status: 200, body: '<form id="challenge-form"></form>', error: "browser_check_required" },
    { id: "challenge-stage", status: 200, body: '<div id="challenge-stage"></div>', error: "browser_check_required" },
    { id: "turnstile", status: 200, body: '<div class="cf-turnstile"></div>', error: "browser_check_required" },
    { id: "captcha", status: 200, body: '<textarea name="g-recaptcha-response"></textarea>', error: "browser_check_required" },
    { id: "forbidden", status: 403, body: "Access denied", error: "browser_check_required" },
    { id: "unavailable", status: 503, body: "Temporarily unavailable", error: "webview_failed" },
];
let scripts = 0;
const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch(request) {
    const url = new URL(request.url);
    if (url.origin !== origin)
        return new Response(null, { status: 403 });
    if (url.pathname === jsdPath) {
        scripts++;
        return new Response("", { headers: { "Content-Type": "application/javascript" } });
    }
    const scenario = scenarios.find(scenario => url.pathname === `/${scenario.id}`);
    if (!scenario)
        return new Response(null, { status: 404 });
    return new Response(`<!doctype html><html><head><title>Generated fixture</title></head><body>${scenario.body}</body></html>`, {
        status: scenario.status,
        headers: { "Content-Type": "text/html; charset=utf-8" },
    });
} });

let failure: unknown;
try {
    for (const scenario of scenarios) {
        const url = `${origin}/${scenario.id}`;
        const request = {
            id: scenario.id,
            url,
            profileDirectory: join(directory, "web-session", digest(origin)),
            mode: "silent",
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
            child.stdin.write(`${JSON.stringify(request)}\n`);
            await child.stdin.flush();
            const output = await new Response(child.stdout).text();
            assert(Buffer.byteLength(output) <= 256 * 1024, "Bounded helper response");
            assert.equal(await child.exited, 0, `Helper exit for ${scenario.id}`);
            const response = JSON.parse(output);
            assert.equal(response.id, scenario.id);
            if (scenario.error) {
                assert.equal(response.error?.code, scenario.error, scenario.id);
                assert.equal(response.session, undefined, `No session for ${scenario.id}`);
            }
            else {
                assert.equal(response.error, undefined, scenario.id);
                assert.equal(response.session?.url, url);
                assert(response.session.userAgent.length > 0);
                assert(Array.isArray(response.session.cookies));
            }
            console.log(`PASS hidden_helper ${scenario.id}=${scenario.error ?? "session"}`);
        }
        finally {
            clearTimeout(deadline);
            if (child.exitCode === null)
                child.kill();
            await child.exited;
        }
    }
    assert(scripts > 0, "The normal page really loaded its JSD script");
}
catch (error) {
    failure = error;
    throw error;
}
finally {
    server.stop(true);
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
