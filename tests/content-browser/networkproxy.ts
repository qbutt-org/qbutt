import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, writeFile } from "node:fs/promises";
import { createServer, request, type Server } from "node:http";
import { connect, createServer as createTcpServer, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { NetworkProxy } from "../../components/content/networkproxy";
import { extractMedia } from "../../components/content/media";
import { mediaRuntime } from "../../components/content/media-runtime";

const root = await mkdtemp(join(tmpdir(), "qbutt-networkproxy-"));
process.env.QBUTT_LAB_PATHS = "1";
const runtimeInfo = await mediaRuntime(root, AbortSignal.timeout(30_000));
const { ffmpeg, "yt-dlp": ytDlp } = runtimeInfo.tools;
const payload = Buffer.alloc(1024 * 1024 + 31, 0x52);
payload.write("Generated legal forward-proxy fixture\n");
const sha256 = createHash("sha256").update(payload).digest("hex");
const generator = Bun.spawn([ffmpeg, "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=blue:s=320x180:r=24:d=1",
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an", "-movflags", "frag_keyframe+empty_moov", "-f", "mp4", "pipe:1"], {
    cwd: root, windowsHide: true, stdout: "pipe", stderr: "pipe",
});
const [generatedVideo, generationError, generatedExit] = await Promise.all([
    new Response(generator.stdout).arrayBuffer(), new Response(generator.stderr).text(), generator.exited,
]);
assert.equal(generatedExit, 0, generationError);
const video = Buffer.from(generatedVideo);
let fixtureRequests = 0;
let forwardedProxyCredentials = false;
const sourceRequests: { host?: string; cookie?: string; authorization?: string }[] = [];
const fixture = createServer((incoming, response) => {
    ++fixtureRequests;
    forwardedProxyCredentials ||= incoming.headers["proxy-authorization"] !== undefined;
    sourceRequests.push({ host: incoming.headers.host, cookie: incoming.headers.cookie, authorization: incoming.headers.authorization });
    if (incoming.url === "/redirect.mp4") {
        response.writeHead(302, { Location: `http://example.org:${fixturePort}/legal.mp4` });
        response.end();
        return;
    }
    if (incoming.url?.endsWith(".mp4")) {
        response.writeHead(200, { "Content-Type": "video/mp4", "Content-Length": video.length });
        response.end(video);
        return;
    }
    if (incoming.url === "/incomplete") {
        response.writeHead(200, { "Content-Length": payload.length });
        response.end("short");
        response.socket?.end();
        return;
    }
    response.writeHead(200, { "Content-Type": "application/octet-stream", "Content-Length": payload.length });
    response.end(payload);
});
await new Promise<void>(resolve => fixture.listen(0, "127.0.0.1", resolve));
const fixtureAddress = fixture.address();
assert(fixtureAddress && typeof fixtureAddress !== "string");
const fixturePort = fixtureAddress.port;

const peers = new Set<Socket>();
const tunnelPayload = Buffer.alloc(32 * 1024 * 1024 + 17, 0x71);
const tunnelHash = createHash("sha256").update(tunnelPayload).digest("hex");
const echo = createTcpServer(socket => {
    peers.add(socket);
    socket.on("error", () => socket.destroy());
    socket.on("close", () => peers.delete(socket));
    socket.on("data", bytes => {
        if (bytes.toString() === "payload")
            socket.end(tunnelPayload);
        else
            socket.write(Buffer.concat([Buffer.from("echo:"), bytes]));
    });
});
await new Promise<void>(resolve => echo.listen(0, "127.0.0.1", resolve));
const echoAddress = echo.address();
assert(echoAddress && typeof echoAddress !== "string");
const echoPort = echoAddress.port;
const proxy = await NetworkProxy.start({ network: { async addresses(url: URL) {
    assert(["fixture.invalid", "example.com", "example.org"].includes(url.hostname));
    assert(["http:", "https:"].includes(url.protocol));
    assert([fixturePort, echoPort].includes(Number(url.port)));
    return [{ address: "127.0.0.1", family: 4 }];
} } });
const guarded = await NetworkProxy.start();

function authorization(url: URL): string {
    return `Basic ${Buffer.from(`${url.username}:${url.password}`).toString("base64")}`;
}

async function get(proxyUrl: string, destination: string, authenticated = true): Promise<{ status: number; body: Buffer }> {
    const url = new URL(proxyUrl);
    return new Promise((resolve, reject) => {
        const outgoing = request({ hostname: url.hostname, port: url.port, path: destination,
            agent: false, headers: authenticated ? { "Proxy-Authorization": authorization(url) } : {} }, incoming => {
            const chunks: Buffer[] = [];
            incoming.on("data", chunk => chunks.push(chunk));
            incoming.on("error", reject);
            incoming.on("end", () => resolve({ status: incoming.statusCode || 0, body: Buffer.concat(chunks) }));
        });
        outgoing.setTimeout(10000, () => outgoing.destroy(new Error("Fixture request timed out")));
        outgoing.on("error", reject);
        outgoing.end();
    });
}

async function tunnel(proxyUrl: string, destination: string, authenticated = true): Promise<{ status: number; socket: Socket }> {
    const url = new URL(proxyUrl);
    return new Promise((resolve, reject) => {
        const socket = connect({ host: url.hostname, port: Number(url.port) });
        socket.setTimeout(10000, () => socket.destroy(new Error("Fixture tunnel timed out")));
        socket.on("error", reject);
        let buffer = Buffer.alloc(0);
        const headers = (chunk: Buffer) => {
            buffer = Buffer.concat([buffer, chunk]);
            const end = buffer.indexOf("\r\n\r\n");
            if (end === -1)
                return;
            socket.removeListener("data", headers);
            const status = Number(/^HTTP\/1\.1 (\d+)/.exec(buffer.toString("ascii"))?.[1]);
            resolve({ status, socket });
        };
        socket.on("data", headers);
        socket.on("connect", () => socket.write(`CONNECT ${destination} HTTP/1.1\r\nHost: ${destination}\r\n`
            + (authenticated ? `Proxy-Authorization: ${authorization(url)}\r\n` : "") + "\r\n"));
    });
}

async function extract(proxyUrl: string, source: string): Promise<{ exit: number; output: string; error: string }> {
    const child = Bun.spawn([ytDlp, "--ignore-config", "--no-warnings", "--skip-download", "--dump-single-json",
        "--no-playlist", "--socket-timeout", "10", "--retries", "0", "--proxy", proxyUrl, "--", source], {
        windowsHide: true, stdout: "pipe", stderr: "pipe",
    });
    const timeout = setTimeout(() => child.kill(), 30000);
    try {
        const [output, error, exit] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
        return { exit, output, error: error.replace(/https?:\/\/\S+/g, "[source]") };
    }
    finally {
        clearTimeout(timeout);
    }
}

async function closeServer(server: Server): Promise<void> {
    server.closeAllConnections();
    await new Promise<void>(resolve => server.close(() => resolve()));
}

try {
    const proxyCheck = "http://qbutt-webview.invalid/";
    assert.equal((await get(proxy.url, proxyCheck, false)).status, 407);

    const checked = await get(proxy.url, proxyCheck);
    assert.equal(checked.status, 200);
    assert.equal(checked.body.length, 0);

    for (const destination of [`${proxyCheck}?query`, `${proxyCheck}other`, "http://qbutt-webview.invalid:80/"])
        assert.equal((await get(proxy.url, destination)).status, 403);

    const checkTunnel = await tunnel(proxy.url, "qbutt-webview.invalid:443");
    assert.equal(checkTunnel.status, 403);
    checkTunnel.socket.destroy();

    const destination = `http://fixture.invalid:${fixturePort}/legal.bin`;
    const unauthorized = await get(proxy.url, destination, false);
    assert.equal(unauthorized.status, 407);
    assert.equal(fixtureRequests, 0);
    const received = await get(proxy.url, destination);
    assert.equal(received.status, 200);
    assert.equal(createHash("sha256").update(received.body).digest("hex"), sha256);
    assert.equal(forwardedProxyCredentials, false, "The proxy credential must never reach the source");
    await assert.rejects(() => get(proxy.url, `http://fixture.invalid:${fixturePort}/incomplete`));

    const unauthorizedTunnel = await tunnel(proxy.url, `fixture.invalid:${echoPort}`, false);
    assert.equal(unauthorizedTunnel.status, 407);
    unauthorizedTunnel.socket.destroy();
    const connected = await tunnel(proxy.url, `fixture.invalid:${echoPort}`);
    assert.equal(connected.status, 200);
    const echoed = new Promise<Buffer>(resolve => connected.socket.once("data", resolve));
    connected.socket.write("generated legal TCP fixture");
    assert.equal((await echoed).toString(), "echo:generated legal TCP fixture");

    // A normal source EOF must drain buffered downstream bytes, including when
    // the recipient applies backpressure instead of reading at socket speed.
    for (let attempt = 0; attempt < 3; ++attempt) {
        const transfer = await tunnel(proxy.url, `fixture.invalid:${echoPort}`);
        assert.equal(transfer.status, 200);
        const completed = new Promise<Buffer>((resolve, reject) => {
            const chunks: Buffer[] = [];
            transfer.socket.on("data", chunk => {
                chunks.push(chunk);
                transfer.socket.pause();
                setTimeout(() => transfer.socket.resume(), 2);
            });
            transfer.socket.on("error", reject);
            transfer.socket.on("end", () => resolve(Buffer.concat(chunks)));
        });
        transfer.socket.write("payload");
        const bytes = await completed;
        assert.equal(bytes.length, tunnelPayload.length);
        assert.equal(createHash("sha256").update(bytes).digest("hex"), tunnelHash);
        transfer.socket.destroy();
    }

    const localHttp = await get(guarded.url, `http://127.0.0.1:${fixturePort}/must-not-run`);
    assert.equal(localHttp.status, 403);
    const localTunnel = await tunnel(guarded.url, `127.0.0.1:${fixturePort}`);
    assert.equal(localTunnel.status, 403);
    localTunnel.socket.destroy();
    const beforeRedirect = fixtureRequests;
    const redirect = `https://httpbin.org/redirect-to?url=${encodeURIComponent(`http://127.0.0.1:${fixturePort}/must-not-run`)}`;
    const blockedExtraction = await extract(guarded.url, redirect);
    assert.notEqual(blockedExtraction.exit, 0);
    assert.equal(fixtureRequests, beforeRedirect, "An extractor redirect must not reach the local fixture");

    // Real source DNS still passes the product's public-address guard. Only the
    // code-injected proxy connector routes these public fixture names locally.
    const cookie = "session=generated-fixture-cookie";
    const sameOrigin = await extractMedia(`http://example.com:${fixturePort}/legal.mp4`, { Cookie: cookie },
        root, AbortSignal.timeout(30000), proxy.url);
    assert.equal(sameOrigin.headers?.Cookie, cookie, "The original media origin must retain its session");
    assert(sourceRequests.some(request => request.host === `example.com:${fixturePort}` && request.cookie === cookie));
    const otherOrigin = await extractMedia(`http://example.com:${fixturePort}/redirect.mp4`, { Cookie: cookie },
        root, AbortSignal.timeout(30000), proxy.url);
    assert.equal(new URL(otherOrigin.url!).hostname, "example.org");
    assert.equal(otherOrigin.headers?.Cookie, undefined);
    assert.equal(otherOrigin.headers?.cookie, undefined);
    assert(sourceRequests.some(request => request.host === `example.org:${fixturePort}`));
    assert(sourceRequests.filter(request => request.host === `example.org:${fixturePort}`)
        .every(request => request.cookie === undefined && request.authorization === undefined), "Session headers crossed the source origin");
    const beforeAuthorization = fixtureRequests;
    await assert.rejects(() => extractMedia(`http://example.com:${fixturePort}/legal.mp4`, { Authorization: "Bearer generated-fixture-token" },
        root, AbortSignal.timeout(30000), proxy.url), /browser player/);
    assert.equal(fixtureRequests, beforeAuthorization, "Unsupported extractor authorization must fail before sending credentials");

    // Blender's public Sintel trailer is used only for extractor metadata. The
    // byte-integrity and streaming scenarios above use generated local content.
    const publicExtraction = await extract(guarded.url, "https://download.blender.org/durian/trailer/sintel_trailer-480p.mp4");
    assert.equal(publicExtraction.exit, 0, `A guarded extractor must retain public HTTPS media support: ${publicExtraction.error}`);
    const metadata = JSON.parse(publicExtraction.output);
    assert.equal(new URL(metadata.url).hostname, "download.blender.org");
    assert(!publicExtraction.output.includes(new URL(guarded.url).password), "Extractor metadata must not expose proxy credentials");

    const closed = new Promise<void>(resolve => connected.socket.once("close", () => resolve()));
    await proxy.close();
    await closed;
    await proxy.close();
    await writeFile(join(root, "result.json"), JSON.stringify({ status: "passed", forwardedBytes: payload.length, sha256,
        verifiedTunnelBytes: tunnelPayload.length, tunnelSha256: tunnelHash, backpressureRuns: 3,
        authentication: true, credentialStripping: true, pinnedHttp: true, pinnedConnect: true,
        incompleteResponseRejected: true, localHttpRejected: true, localConnectRejected: true,
        extractorRedirectBlocked: true, exactOriginCookieJar: true, crossOriginSessionHeadersRemoved: true,
        unsupportedAuthorizationRejected: true, publicHttpsExtractor: true, closeDrainsOwnedSockets: true }, null, 2));
    console.log(JSON.stringify({ status: "passed", root, forwardedBytes: payload.length }));
}
finally {
    await Promise.all([proxy.close(), guarded.close()]);
    for (const socket of peers)
        socket.destroy();
    await Promise.all([closeServer(fixture), new Promise<void>(resolve => echo.close(() => resolve()))]);
}
