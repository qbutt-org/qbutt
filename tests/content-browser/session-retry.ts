import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createConnection, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const [executable, certificate, key, evidence] = process.argv.slice(2);
assert(executable && certificate && key && evidence, "Expected acceptance executable, fixture cert/key and receipt path");
const root = await mkdtemp(join(tmpdir(), "qbutt-session-retry-"));
const discovery = join(root, "browser.json");
const phaseFile = join(root, "phase");
const readyFile = join(root, "ready");
const checkLog = join(root, "checks");
const hash = "0123456789abcdef0123456789abcdef01234567";
const login = "<title>Session expired poison title</title><form><input name='login_username'><input name='login_password'></form>";
const requests: { phase: string; path: string; cookie: string | null; ua: string | null }[] = [];
const cookies: { phase: string; ua: string }[] = [];
let edge: Socket | undefined;
let chrome: Socket | undefined;
let failure: unknown;
let version = 1;

function phase() {
    return readFileSync(phaseFile, "utf8");
}

function expired(value: string) {
    return value.startsWith("expired") || value === "manual" || value === "fault";
}

function topic(id: string) {
    return `<div class='nav'><a href='viewforum.php?f=33'>Anime</a></div>
        <a id='topic-title' href='viewtopic.php?t=${id}'>Generated legal ${id}</a>
        <div class='post_body'>Authored public domain fixture</div>
        <table class='attach'><tr><td><a class='dl-stub' href='dl.php?t=${id}'>Torrent</a>
        <span id='tor-size-humn' title='4'>4 B</span>
        <a class='magnet-link' href='magnet:?xt=urn:btih:${hash}'>Magnet</a></td></tr></table>`;
}

const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 443,
    tls: { cert: await readFile(certificate), key: await readFile(key) },
    fetch(request) {
        try {
            const url = new URL(request.url);
            assert.equal(url.hostname, "rutracker.org");
            const current = phase();
            const cookie = request.headers.get("cookie");
            const ua = request.headers.get("user-agent");
            requests.push({ phase: current, path: url.pathname + url.search, cookie, ua });
            let body: string;
            if (url.pathname === "/forum/index.php") {
                body = "<div id='f-map'><ul class='tree-root'><li><span><a href='33'>Anime</a></span></li></ul></div>";
            } else if (url.pathname === "/forum/tracker.php") {
                body = "<table id='tor-tbl'><tbody>" + [42, 43].map(id =>
                    `<tr id='trs-tr-${id}'><td><a href='tracker.php?f=33'>Anime</a></td>
                    <td class='t-title-col'><a class='tLink' data-topic_id='${id}' href='viewtopic.php?t=${id}'>Generated legal ${id}</a></td></tr>`
                ).join("") + "</tbody></table>";
            } else {
                assert.equal(url.pathname, "/forum/viewtopic.php");
                const manual = cookie === "bbsession=manual" && ua === "generated manual session";
                if (expired(current) && !manual) {
                    body = login;
                } else {
                    if (!manual) {
                        assert.equal(cookie, `bbsession=v${version}`);
                        assert.equal(ua, `generated Edge v${version}`);
                    }
                    body = topic(url.searchParams.get("t")!);
                }
            }
            return new Response(body, { headers: { "content-type": "text/html; charset=utf-8" } });
        } catch (error) {
            failure = error;
            return new Response("Generated fixture failed", { status: 500 });
        }
    },
});
const child = spawn(executable, ["--session-retry"], {
    windowsHide: true,
    env: {
        ...process.env,
        QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery,
        QBUTT_SESSION_RETRY_PHASE: phaseFile,
        QBUTT_SESSION_RETRY_READY: readyFile,
        QBUTT_SESSION_CHECK_LOG: checkLog,
        QBUTT_SESSION_RETRY_RENDER_DIR: dirname(evidence),
        QT_QPA_PLATFORM: "offscreen",
        QT_QPA_FONTDIR: "C:/Windows/Fonts",
        QT_QUICK_BACKEND: "software",
        QT_FORCE_STDERR_LOGGING: "1",
    },
});
let output = "";
let running = true;
child.stdout.on("data", chunk => { output += chunk; });
child.stderr.on("data", chunk => { output += chunk; });
const exited = new Promise<number | null>((resolve, reject) => {
    child.once("exit", code => {
        running = false;
        resolve(code);
    });
    child.once("error", reject);
});
const timer = setTimeout(() => child.kill(), 150_000);

function write(peer: Socket, message: unknown) {
    const body = Buffer.from(JSON.stringify(message));
    const header = Buffer.alloc(4);
    header.writeUInt32LE(body.length);
    peer.write(Buffer.concat([header, body]));
}

async function connect(pipe: string, browser: "edge" | "chrome", ua: string) {
    const peer = createConnection(pipe);
    await new Promise<void>((resolve, reject) => {
        peer.once("connect", resolve);
        peer.once("error", reject);
    });
    peer.on("error", error => {
        failure = error;
        child.kill();
    });
    let buffered = Buffer.alloc(0);
    peer.on("data", chunk => {
        try {
            buffered = Buffer.concat([buffered, chunk]);
            while (buffered.length >= 4) {
                const size = buffered.readUInt32LE();
                assert(size > 0 && size <= 768 * 1024);
                if (buffered.length < size + 4) break;
                const request = JSON.parse(buffered.subarray(4, size + 4).toString());
                buffered = buffered.subarray(size + 4);
                if (request.method === "cancel") continue;
                assert.equal(browser, "edge", "Explicit browser choice leaked into Chrome");
                const url = new URL(request.url);
                assert.equal(url.hostname, "rutracker.org");
                const current = phase();
                if (request.method === "cookies") {
                    assert.equal(url.href, "https://rutracker.org/forum/tracker.php");
                    cookies.push({ phase: current, ua });
                    if (current === "fault") {
                        write(peer, { id: request.id, error: { code: "browser_busy", message: "Generated unknown outcome" } });
                        continue;
                    }
                    write(peer, { id: request.id, result: [{
                        name: "bbsession", value: `v${version}`, domain: url.hostname,
                        path: "/forum/", secure: true, httpOnly: true, hostOnly: true,
                        expirationDate: expired(current) ? 1 : Date.now() / 1000 + 600,
                    }] });
                } else {
                    assert.equal(request.method, "fetch");
                    assert(expired(current));
                    assert.equal(url.href, "https://rutracker.org/forum/viewtopic.php?t=42");
                    write(peer, { id: request.id, result: {
                        status: 200, url: url.href, headers: { "content-type": "text/html" }, body: login,
                    } });
                }
            }
        } catch (error) {
            failure = error;
            child.kill();
        }
    });
    write(peer, { type: "hello", version: 1, browser, userAgent: ua });
    return peer;
}

const peers = (async () => {
    let pipe: string | undefined;
    for (let attempt = 0; attempt < 300 && running; ++attempt) {
        pipe = await readFile(discovery, "utf8").then(body => JSON.parse(body).pipe).catch(() => undefined);
        if (pipe) break;
        await Bun.sleep(30);
    }
    assert(pipe, `Native broker did not publish discovery:\n${output}`);
    edge = await connect(pipe, "edge", "generated Edge v1");
    chrome = await connect(pipe, "chrome", "generated unrelated Chrome");
    let previous = "";
    while (running) {
        const current = await readFile(phaseFile, "utf8").catch(() => "");
        if (current && current !== previous) {
            if (current.startsWith("renewed")) {
                edge.destroy();
                await Bun.sleep(80);
                ++version;
                edge = await connect(pipe, "edge", `generated Edge v${version}`);
                await Bun.sleep(80);
            }
            previous = current;
            await writeFile(readyFile, current);
        }
        await Bun.sleep(20);
    }
})().catch(error => {
    failure = error;
    child.kill();
});

try {
    const code = await exited;
    await peers;
    if (failure) throw failure;
    assert.equal(code, 0, output);
    assert(output.includes("PASS browser_session_retry"), output);
    for (const current of ["renewed-again", "renewed-stale"]) {
        const resolved = requests.filter(request => request.phase === current && request.path === "/forum/viewtopic.php?t=42");
        assert.equal(resolved.length, 1, `${current} repeated the exact request`);
    }
    assert.equal(cookies.filter(request => request.phase === "renewed-again").length, 1);
    const checks = await readFile(checkLog, "utf8");
    assert.equal(checks.split("\n").filter(mode => mode === "interactive").length, 1);
    for (const secret of ["bbsession", "generated Edge", "manual session"])
        assert(!output.includes(secret), "Session data entered native diagnostics");
    await writeFile(evidence, JSON.stringify({ ok: true, code, requests, cookies, checks, output }, null, 2));
    console.log(JSON.stringify({ ok: true, exactActionHandoffs: 2, staleActionHandoffs: 0, manualChecks: 1, requests: requests.length }));
} finally {
    clearTimeout(timer);
    edge?.destroy();
    chrome?.destroy();
    if (running) {
        child.kill();
        await exited;
    }
    server.stop(true);
    await rm(root, { recursive: true, force: true });
}
