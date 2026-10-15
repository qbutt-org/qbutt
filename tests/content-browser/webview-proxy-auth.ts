import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { lstat, mkdtemp, readFile, realpath, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { createConnection, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join } from "node:path";

// The wrapper owns trust for this generated certificate. No request leaves
// the loopback fixtures; the public origin is reached only through CONNECT.
const [executable, certificate, key] = process.argv.slice(2);
assert(executable && isAbsolute(executable) && certificate && key, "Expected helper and generated certificate/key");
const directory = await mkdtemp(join(tmpdir(), "qbutt-webview-auth-"));
const origin = "https://fixture.example";
const digest = (value: string) => createHash("sha256").update(value).digest("hex");
let phase = 0;
let retainedCookie = false;
const tls = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    tls: { cert: await readFile(certificate), key: await readFile(key) },
    fetch(request) {
        const url = new URL(request.url);
        assert.equal(url.origin, origin);
        if (url.pathname !== "/catalog")
            return new Response(null, { status: 404 });
        if (phase === 2)
            retainedCookie ||= request.headers.get("cookie")?.includes("fixture=retained") ?? false;
        return new Response("<!doctype html><html><head><title>Generated legal catalog</title></head><body>Fixture</body></html>", {
            headers: {
                "Content-Type": "text/html; charset=utf-8",
                "Cache-Control": "no-store",
                "Set-Cookie": "fixture=retained; Path=/; Secure; SameSite=Lax; Max-Age=3600",
            },
        });
    },
});

try {
    for (phase = 1; phase <= 2; phase++) {
        const password = randomBytes(32).toString("hex");
        const authorization = `Basic ${Buffer.from(`qbutt:${password}`).toString("base64")}`;
        const sockets = new Set<Socket>();
        let challenges = 0;
        let authenticated = 0;
        let bootstraps = 0;
        const proxy = createServer((request, response) => {
            if (request.headers["proxy-authorization"] !== authorization) {
                challenges++;
                response.writeHead(407, {
                    "Proxy-Authenticate": 'Basic realm="qbutt"',
                    "Connection": "close",
                    "Content-Length": "0",
                });
                response.end();
                return;
            }
            if (request.method === "GET" && request.url === "http://qbutt-webview.invalid/") {
                bootstraps++;
                response.writeHead(200, {
                    "Content-Type": "text/html",
                    "Cache-Control": "no-store",
                    "Content-Length": "0",
                });
                response.end();
                return;
            }
            response.writeHead(403, { "Content-Length": "0" });
            response.end();
        });
        proxy.on("connection", socket => {
            sockets.add(socket);
            socket.once("close", () => sockets.delete(socket));
        });
        proxy.on("connect", (request, socket, head) => {
            if (request.headers["proxy-authorization"] !== authorization) {
                challenges++;
                socket.end('HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm="qbutt"\r\nConnection: close\r\nContent-Length: 0\r\n\r\n');
                return;
            }
            if (request.url !== "fixture.example:443") {
                socket.end("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
                return;
            }
            assert.equal(bootstraps, 1, "Bootstrap completes before target CONNECT");
            authenticated++;
            const upstream = createConnection({ host: "127.0.0.1", port: tls.port });
            sockets.add(upstream);
            upstream.once("close", () => {
                sockets.delete(upstream);
                socket.destroy();
            });
            socket.once("close", () => upstream.destroy());
            upstream.once("error", () => socket.destroy());
            socket.once("error", () => upstream.destroy());
            upstream.once("connect", () => {
                socket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
                if (head.length)
                    upstream.write(head);
                socket.pipe(upstream).pipe(socket);
            });
        });
        await new Promise<void>(resolve => proxy.listen(0, "127.0.0.1", resolve));
        const address = proxy.address();
        assert(address && typeof address !== "string");
        const request = {
            id: `phase-${phase}`,
            url: `${origin}/catalog`,
            profileDirectory: join(directory, "web-session", digest(origin)),
            mode: "silent",
            timeoutMs: 8_000,
            configuredSession: digest("[]"),
            initialCookies: [],
            authoritativeCookies: false,
            proxy: { url: `http://127.0.0.1:${address.port}`, username: "qbutt", password },
        };
        const child = Bun.spawn([executable, "--stdio"], {
            stdin: "pipe",
            stdout: "pipe",
            stderr: "ignore",
            windowsHide: true,
        });
        const deadline = setTimeout(() => child.kill(), 11_000);
        try {
            child.stdin.write(`${JSON.stringify(request)}\n`);
            await child.stdin.flush();
            const output = await new Response(child.stdout).text();
            assert(Buffer.byteLength(output) <= 256 * 1024, "Bounded helper response");
            assert.equal(await child.exited, 0, `Helper exit for phase ${phase}`);
            const response = JSON.parse(output);
            assert.equal(response.error?.code, undefined, `HTTPS proxy phase ${phase}`);
            assert.equal(response.session?.url, request.url);
            assert(challenges > 0, `Fresh proxy challenge in phase ${phase}`);
            assert(authenticated > 0, `Proxy authenticated in phase ${phase}`);
            assert.equal(bootstraps, 1, `Single proxy bootstrap in phase ${phase}`);
            console.log(`PASS hidden_helper https_proxy_auth phase=${phase} challenges=${challenges} authenticated=${authenticated}`);
        }
        finally {
            clearTimeout(deadline);
            if (child.exitCode === null)
                child.kill();
            await child.exited;
            for (const socket of sockets)
                socket.destroy();
            await new Promise<void>(resolve => proxy.close(() => resolve()));
        }
    }
    assert(retainedCookie, "HTTPS cookie survives same-profile restart");
}
finally {
    tls.stop(true);
    assert(!(await lstat(directory)).isSymbolicLink(), "Fixture cleanup rejects reparse links");
    const resolved = await realpath(directory);
    const temporaryRoot = await realpath(tmpdir());
    assert.equal(dirname(resolved).toLowerCase(), temporaryRoot.toLowerCase(), "Fixture cleanup stays within Temp");
    await rm(directory, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
}
