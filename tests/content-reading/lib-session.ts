import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createConnection, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

// The acceptance build supplies only fixture TLS trust and DNS overrides.
// Catalog, providers, the broker, Qt models and reader lifecycle are unchanged.
const [executable, certificate, key] = process.argv.slice(2);
assert(executable && certificate && key, "Expected fixture acceptance executable, certificate and key");
const root = await mkdtemp(join(tmpdir(), "qbutt-native-lib-session-"));
const discovery = join(root, "browser.json");
const token = "generated.native.account";
const imports = { mangalib: 0, ranobelib: 0 };
let images = 0;
let cookieRequests = 0;
let anonymousSearches = 0;
let privateRequests = 0;
let socket: Socket | undefined;
let failure: unknown;

function record(site: number, id: number) {
    return {
        id,
        slug_url: `${id}--generated`,
        name: `Generated original ${id}`,
        rus_name: "",
        eng_name: "",
        model: "manga",
        site,
        cover: null,
        summary: null,
    };
}

const image = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQUAAAAJcEhZcwAADsMAAA7DAcdvqGQAAAANSURBVBhXY/j///9/AAn7A/0FQ0XKAAAAAElFTkSuQmCC", "base64");
const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 443,
    tls: { cert: await readFile(certificate), key: await readFile(key) },
    fetch(request) {
        try {
            const url = new URL(request.url);
            if (url.hostname === "img3.cdnlibs.org") {
                assert.equal(request.headers.get("authorization"), null);
                assert.equal(request.headers.get("cookie"), "image=generated");
                assert(url.pathname.startsWith("/manga/generated/chapters/7/"));
                ++images;
                return new Response(image, { headers: { "content-type": "image/png" } });
            }
            assert.equal(url.hostname, "api.cdnlibs.org");
            const site = Number(request.headers.get("site-id"));
            assert([1, 3].includes(site));
            const authorized = request.headers.get("authorization") === `Bearer ${token}`;
            if (!authorized) {
                assert.equal(url.pathname, "/api/manga");
                ++anonymousSearches;
                return Response.json({ data: [record(site, 41)] });
            }
            assert.equal(request.headers.get("cookie"), "api=generated");
            assert.equal(request.headers.get("user-agent"), "generated native Lib connection");
            ++privateRequests;
            if (url.pathname === "/api/manga")
                return Response.json({ data: [record(site, 41), record(site, 42)] });
            if (url.pathname.endsWith("/chapters")) {
                return Response.json({ data: [{
                    index: 1,
                    volume: "1",
                    number: "1",
                    name: "Generated chapter",
                    bundle_id: null,
                    branches: [{
                        id: 7,
                        branch_id: null,
                        teams: [{ name: "Generated translation" }],
                        expired_type: 0,
                    }],
                }] });
            }
            if (url.pathname.endsWith("/chapter")) {
                const chapter = { id: 7, manga_id: 42, branch_id: null, volume: "1", number: "1" };
                if (site === 3) {
                    return Response.json({ data: {
                        ...chapter,
                        model: "chapter",
                        expired_type: 0,
                        bundle_id: null,
                        content: "<h2>Generated heading</h2><p>Original generated text.</p>",
                    } });
                }
                return Response.json({ data: {
                    ...chapter,
                    pages: [1, 2].map(page => ({
                        id: page,
                        chapter_id: 7,
                        slug: page,
                        external: 0,
                        chunks: 0,
                        url: `//manga/generated/chapters/7/${page}.png`,
                    })),
                } });
            }
            if (url.pathname === "/api/constants") return Response.json({ data: {
                imageServers: [{ id: "download", site_ids: [1], url: "https://img3.cdnlibs.org/" }],
            } });
            assert.equal(url.pathname, "/api/manga/42--generated");
            return Response.json({ data: record(site, 42) });
        } catch (error) {
            failure = error;
            return new Response("Generated fixture failed", { status: 500 });
        }
    },
});
const child = spawn(executable, ["--lib-session"], {
    windowsHide: true,
    env: {
        ...process.env,
        QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery,
        QT_QPA_PLATFORM: "offscreen",
        QT_QUICK_BACKEND: "software",
        QT_FORCE_STDERR_LOGGING: "1",
    },
});
const exited = new Promise<number | null>((resolve, reject) => {
    child.once("exit", resolve);
    child.once("error", reject);
});
let output = "";
child.stdout.on("data", chunk => { output += chunk; });
child.stderr.on("data", chunk => { output += chunk; });
const timer = setTimeout(() => child.kill(), 100_000);

function write(value: unknown) {
    const body = Buffer.from(JSON.stringify(value));
    const header = Buffer.alloc(4);
    header.writeUInt32LE(body.length);
    socket!.write(Buffer.concat([header, body]));
}

try {
    let pipe: string | undefined;
    for (let attempt = 0; attempt < 300 && child.exitCode === null; ++attempt) {
        pipe = await readFile(discovery, "utf8").then(body => JSON.parse(body).pipe).catch(() => undefined);
        if (pipe) break;
        await Bun.sleep(100);
    }
    if (failure) throw failure;
    assert(pipe, `Native broker did not publish isolated discovery (exit ${child.exitCode}, anonymous searches ${anonymousSearches}):\n${output}`);
    socket = createConnection(pipe);
    await new Promise<void>((resolve, reject) => {
        socket!.once("connect", resolve);
        socket!.once("error", reject);
    });
    socket.on("error", error => {
        failure = error;
        child.kill();
    });
    let buffered = Buffer.alloc(0);
    socket.on("data", chunk => {
        try {
            buffered = Buffer.concat([buffered, chunk]);
            while (buffered.length >= 4) {
                const size = buffered.readUInt32LE();
                assert(size > 0 && size <= 768 * 1024);
                if (buffered.length < size + 4) break;
                const request = JSON.parse(buffered.subarray(4, size + 4).toString());
                buffered = buffered.subarray(size + 4);
                if (request.method === "cancel") continue;
                const url = new URL(request.url);
                if (request.method === "lib-session") {
                    const source = url.hostname === "mangalib.me" ? "mangalib" : "ranobelib";
                    assert.equal(url.href, `https://${source}.me/`);
                    ++imports[source];
                    write({ id: request.id, result: {
                        accessToken: token,
                        cookies: [{
                            name: "api",
                            value: "generated",
                            domain: "api.cdnlibs.org",
                            path: "/api",
                            secure: true,
                            hostOnly: true,
                        }],
                    } });
                } else {
                    assert.equal(request.method, "cookies");
                    assert.equal(url.hostname, "img3.cdnlibs.org");
                    ++cookieRequests;
                    write({ id: request.id, result: [{
                        name: "image",
                        value: "generated",
                        domain: url.hostname,
                        path: "/",
                        secure: true,
                        hostOnly: true,
                    }] });
                }
            }
        } catch (error) {
            failure = error;
            child.kill();
        }
    });
    write({
        type: "hello",
        version: 1,
        browser: "edge",
        userAgent: "generated native Lib connection",
        capabilities: ["lib-session"],
    });
    assert.equal(await exited, 0, output);
    if (failure) throw failure;
    assert(output.includes("PASS Lib account"), output);
    assert.equal(anonymousSearches, 1);
    assert.deepEqual(imports, { mangalib: 4, ranobelib: 4 }, "Each operation imports once; image pages share their chapter session");
    assert(images >= 2 && cookieRequests >= 2 && privateRequests >= 12);
    assert(!output.includes(token), "Generated credential entered native diagnostics");
    console.log(JSON.stringify({ ok: true, imports, images, cookieRequests, anonymousSearches, privateRequests }));
} finally {
    clearTimeout(timer);
    socket?.destroy();
    if (child.exitCode === null) {
        child.kill();
        await exited;
    }
    server.stop(true);
    await rm(root, { recursive: true, force: true });
}
