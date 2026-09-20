import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { createConnection, createServer as createTcpServer } from "node:net";
import { networkInterfaces } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, waitFor } from "../lab";
import { encode } from "./bencode";
import { startProxy } from "./proxy";

const HOST = "127.0.0.12";
const NAME = "tls-rejection.bin";
const payload = Buffer.from("qbutt isolated HTTPS certificate rejection fixture\n".repeat(100));
const lab = await createLab("webseed-tls-rejection");
const loopback = Object.entries(networkInterfaces()).find(([, addresses]) =>
    addresses?.some(address => address.internal && address.family === "IPv4"))?.[0];
assert(loopback, "A loopback interface is required");

const listen = (server: ReturnType<typeof createTcpServer>) =>
    new Promise<number>((done, fail) => {
        server.once("error", fail);
        server.listen(0, HOST, () => {
            const address = server.address();
            assert(address && typeof address !== "string");
            done(address.port);
        });
    });
const close = (server: ReturnType<typeof createTcpServer>) =>
    new Promise<void>((done, fail) => server.close(error => error ? fail(error) : done()));

// The controlled OpenSSL server records the TLS alert received from qbutt.
const tlsServer = `import json, os, socket, ssl, sys
cert, key, payload_path, endpoint_path, events_path = sys.argv[1:]
payload = open(payload_path, "rb").read()
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(cert, key)
def record(event, **details):
    with open(events_path, "a", encoding="utf-8") as out:
        out.write(json.dumps({"event": event, **details}) + "\\n")
with socket.socket() as listener:
    listener.bind(("127.0.0.12", 0))
    listener.listen(16)
    with open(endpoint_path + ".tmp", "w", encoding="utf-8") as out:
        json.dump({"port": listener.getsockname()[1]}, out)
    os.replace(endpoint_path + ".tmp", endpoint_path)
    while True:
        client, _ = listener.accept()
        record("tcp")
        try:
            client.settimeout(5)
            with context.wrap_socket(client, server_side=True) as tls:
                record("tls")
                request = tls.recv(8192)
                if request:
                    record("http", requestLine=request.split(b"\\r\\n", 1)[0].decode("ascii", "replace"))
                    status = b"200 OK" if request.startswith(b"GET /tls-rejection.bin HTTP/1.") else b"400 Bad Request"
                    body = payload if status == b"200 OK" else b""
                    header = b"HTTP/1.1 " + status + b"\\r\\nContent-Length: " + str(len(body)).encode() + b"\\r\\nConnection: close\\r\\n\\r\\n"
                    tls.sendall(header + body)
        except ssl.SSLError as error:
            record("tlsError", reason=error.reason)
        except (OSError, TimeoutError) as error:
            record("socketError", reason=type(error).__name__)
`;
interface TlsEvent { event: "tcp" | "tls" | "http" | "tlsError" | "socketError"; reason?: string; requestLine?: string }
let nativeConnections = 0;
const canary = createTcpServer(socket => { nativeConnections++; socket.destroy(); });
let responder: ReturnType<typeof Bun.spawn> | undefined;
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let failure: unknown;
const eventsPath = join(lab.root, "tls-events.jsonl");
async function readEvents(): Promise<TlsEvent[]> {
    let content: string;
    try { content = await readFile(eventsPath, "utf8"); }
    catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        return [];
    }
    return content.split("\n").slice(0, -1).filter(Boolean).map(line => JSON.parse(line) as TlsEvent);
}
try {
    const canaryPort = await listen(canary);
    // Prove the destination is reachable directly, then exclude this controlled probe.
    await new Promise<void>((done, fail) => {
        const socket = createConnection({ host: HOST, port: canaryPort });
        socket.once("connect", () => { socket.destroy(); done(); });
        socket.once("error", fail);
    });
    await waitFor("Native canary reachability", async () => nativeConnections, count => count === 1, 1000);
    nativeConnections = 0;

    const certs = join(lab.root, "certificates");
    const go = process.env.QBUTT_LAB_GO ?? "go";
    const generator = Bun.spawn([go, "run", join(import.meta.dir, "gateway-certificates.go"), certs, HOST], {
        cwd: resolve(import.meta.dir, "../.."), env: { ...process.env, GOTOOLCHAIN: "local" }, stdout: "pipe", stderr: "pipe",
    });
    const [exitCode, stderr] = await Promise.all([generator.exited, new Response(generator.stderr).text()]);
    assert.equal(exitCode, 0, `Certificate generator failed: ${stderr}`);
    const payloadPath = join(lab.root, NAME);
    await writeFile(payloadPath, payload);
    const scriptPath = join(lab.root, "tls-server.py");
    const endpointPath = join(lab.root, "tls-endpoint.json");
    await writeFile(scriptPath, tlsServer);
    responder = Bun.spawn([lab.python, "-u", scriptPath, join(certs, "server.pem"),
        join(certs, "server-key.pem"), payloadPath, endpointPath, eventsPath], {
        stdout: Bun.file(join(lab.root, "tls-server.stdout.log")),
        stderr: Bun.file(join(lab.root, "tls-server.stderr.log")),
    });
    const responderPort = (await waitFor("TLS responder ready", async () => {
        assert(responder!.exitCode === null, "Python TLS responder exited before readiness");
        return JSON.parse(await readFile(endpointPath, "utf8").catch(() => "{}")) as { port?: number };
    }, endpoint => !!endpoint.port, 10000)).port!;
    const credentials = { username: randomBytes(8).toString("hex"), password: randomBytes(16).toString("hex") };
    proxy = await startProxy({ ...credentials, listenAddress: "127.0.0.20", targets: [
        { host: HOST, port: canaryPort, connectHost: HOST, connectPort: responderPort },
    ] });
    const configPath = join(lab.root, "nodes.json");
    await writeFile(configPath, JSON.stringify({ proxies: [{ name: "tls-path", type: "socks5",
        server: proxy.host, port: proxy.port, ...credentials }] }));
    const info = { name: Buffer.from(NAME), length: payload.length, "piece length": 16384,
        pieces: createHash("sha1").update(payload).digest() };
    const hash = createHash("sha1").update(encode(info)).digest("hex");
    const torrent = join(lab.root, "tls.torrent");
    const url = `https://${HOST}:${canaryPort}/${NAME}`;
    await writeFile(torrent, encode({ info, "url-list": Buffer.from(url) }));
    const destination = join(lab.root, "downloads");
    await mkdir(destination);

    await lab.start();
    const status = () => lab.json<{ busy: boolean; mode: string; paths: { pathId: string; generation: number; open: boolean }[];
        diagnostics: { routes: { payloadDownload: number; verifiedDownload: number }[] } }>("qbuttPaths/status");
    await lab.request("qbuttPaths/open", { configPath, proxyName: "tls-path", interfaceName: loopback });
    const opened = await waitFor("TLS path ready", status, current => !current.busy && current.paths.length === 1 && current.paths[0]!.open);
    await lab.request("qbuttPaths/policy", { mode: "tunnels" });
    const form = new FormData(); form.set("torrents", Bun.file(torrent)); form.set("savepath", destination);
    form.set("stopped", "true"); form.set("autoTMM", "false"); form.set("contentLayout", "Original");
    await lab.request("torrents/add", form);
    await waitFor("TLS torrent registered", () => lab.json<{ hash: string }[]>(`torrents/info?hashes=${hash}`), jobs => jobs.length === 1);
    await lab.request("torrents/start", { hashes: hash });
    await waitFor("managed TLS handshake attempted", async () => ({ events: await readEvents(), stats: proxy!.stats }),
        current => current.events.some(event => event.event === "tcp") && current.stats.authenticatedConnections > 0, 20000);
    assert(proxy.stats.uploadStreamBytes > 0 && proxy.stats.downloadStreamBytes > 0,
        "Managed relay did not carry a TLS handshake");
    const rejected = await waitFor("qbutt certificate alert", readEvents,
        events => events.some(event => event.event === "tlsError" && /UNKNOWN_CA|BAD_CERTIFICATE/.test(event.reason ?? "")), 10000);
    await Bun.sleep(3000);
    const torrentStatus = await lab.info(hash);
    const routes = await status();
    assert.equal(routes.mode, "tunnels");
    assert.equal(torrentStatus.completed, 0);
    assert.equal(torrentStatus.progress, 0);
    assert(routes.diagnostics.routes.every(route => route.payloadDownload === 0 && route.verifiedDownload === 0));
    assert.equal(nativeConnections, 0, "HTTPS webseed reached the available Native destination");
    assert.equal(rejected.filter(event => event.event === "http").length, 0,
        "Untrusted TLS peer received an HTTP request");
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("rejected torrent stopped", () => lab.info(hash), current => current.state.startsWith("stopped"));
    assert.equal(nativeConnections, 0);
    const appEvents = await readEvents();
    assert(appEvents.some(event => event.event === "tlsError" && event.reason === "TLSV1_ALERT_UNKNOWN_CA"));
    assert(appEvents.every(event => event.event !== "tls" && event.event !== "http"),
        "qbutt completed TLS or reached HTTP despite rejecting the certificate");

    // A separate stdlib client trusts only the fixture CA and verifies the
    // numeric SAN before accepting the exact body.
    const trusted = Bun.spawn([lab.python, "-c", `import hashlib,http.client,json,ssl,sys
ca,host,port,path=sys.argv[1:]
conn=http.client.HTTPSConnection(host,int(port),context=ssl.create_default_context(cafile=ca),timeout=5)
conn.request("GET",path)
response=conn.getresponse()
data=response.read()
print(json.dumps({"status":response.status,"size":len(data),"sha256":hashlib.sha256(data).hexdigest()}))
conn.close()`, join(certs, "ca.pem"), HOST, String(responderPort), `/${NAME}`], { stdout: "pipe", stderr: "pipe" });
    const [trustedExit, trustedOut, trustedError] = await Promise.all([trusted.exited,
        new Response(trusted.stdout).text(), new Response(trusted.stderr).text()]);
    assert.equal(trustedExit, 0, `Trusted HTTPS control failed: ${trustedError}`);
    const result = JSON.parse(trustedOut) as { status: number; size: number; sha256: string };
    assert.equal(result.status, 200);
    assert.equal(result.size, payload.length);
    assert.equal(result.sha256, createHash("sha256").update(payload).digest("hex"));
    const allEvents = await readEvents();
    assert.equal(allEvents.filter(event => event.event === "http").length, 1);
    await lab.checkpoint({ check: "untrusted-https-webseed-rejected", hash, url, path: opened.paths[0],
        appTlsConnections: appEvents.filter(event => event.event === "tcp").length,
        appCertificateAlerts: appEvents.filter(event => event.event === "tlsError").map(event => event.reason),
        managedProxy: proxy.stats, nativeConnections, verifiedBytes: torrentStatus.completed,
        usefulPayloadBytes: routes.diagnostics.routes.reduce((sum, route) => sum + route.payloadDownload, 0),
        untrustedHttpRequests: 0, trustedControlSha256: result.sha256 });
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); } catch (error) { failure ??= error; }
    try { if (proxy) await proxy.close(); } catch (error) { failure ??= error; }
    if (responder) { responder.kill(); await responder.exited; }
    try { await close(canary); } catch (error) { failure ??= error; }
    await lab.checkpoint({ check: "tls-final-observations", tlsEvents: await readEvents(),
        nativeConnections, proxy: proxy?.stats });
    for (const name of ["fixtures", "profile", "certificates", "nodes.json", "tls.torrent", "downloads",
        NAME, "tls-server.py", "tls-endpoint.json"]) {
        const path = resolve(lab.root, name); assert(dirname(path) === resolve(lab.root));
        try { await rm(path, { recursive: true, force: true }); } catch (error) { failure ??= error; }
    }
}
await lab.finish(failure);
if (failure) throw failure;
