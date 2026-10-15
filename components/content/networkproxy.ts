import { randomBytes, timingSafeEqual } from "node:crypto";
import { createServer, request as httpRequest, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { request as httpsRequest } from "node:https";
import { connect, type Socket } from "node:net";
import type { Duplex } from "node:stream";

import { publicAddresses } from "./net";

interface ProxyOptions {
    // Only generated integration fixtures supply a connector. There is no
    // request, setting or environment override for the public-network policy.
    network?: { addresses: typeof publicAddresses };
}

const hopHeaders = new Set(["connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "proxy-connection", "te", "trailer", "transfer-encoding", "upgrade"]);

function headers(message: IncomingMessage): Record<string, string | string[]> {
    const excluded = new Set([...hopHeaders, ...(message.headers.connection || "").toLowerCase().split(/\s*,\s*/)]);
    return Object.fromEntries(Object.entries(message.headers)
        .filter(([name, value]) => value !== undefined && !excluded.has(name))) as Record<string, string | string[]>;
}

export class NetworkProxy {
    readonly url: string;
    private readonly authorization: Buffer;
    private readonly sockets = new Set<Socket>();
    private closing?: Promise<void>;

    private constructor(private readonly server: Server, password: string,
        private readonly options: ProxyOptions) {
        const address = server.address();
        if (!address || typeof address === "string")
            throw new Error("The media connection could not be started.");
        this.authorization = Buffer.from(`Basic ${Buffer.from(`qbutt:${password}`).toString("base64")}`);
        this.url = `http://qbutt:${password}@127.0.0.1:${address.port}`;

        server.on("connection", socket => {
            if (this.closing || this.sockets.size >= 64) {
                socket.destroy();
                return;
            }
            this.track(socket);
            socket.setTimeout(60_000, () => socket.destroy());
        });
        server.on("request", (request, response) => { void this.forward(request, response); });
        server.on("connect", (request, socket, head) => { void this.tunnel(request, socket, head); });
        server.on("upgrade", (_request, socket) => socket.destroy());
        server.on("clientError", (_error, socket) => socket.destroy());
        server.headersTimeout = 10_000;
        server.requestTimeout = 60_000;
    }

    static async start(options: ProxyOptions = {}): Promise<NetworkProxy> {
        const server = createServer({ maxHeaderSize: 16 * 1024 });
        await new Promise<void>((resolve, reject) => {
            server.once("error", reject);
            server.listen(0, "127.0.0.1", () => {
                server.removeListener("error", reject);
                resolve();
            });
        });
        return new NetworkProxy(server, randomBytes(24).toString("hex"), options);
    }

    close(): Promise<void> {
        if (this.closing)
            return this.closing;
        this.closing = new Promise(resolve => {
            this.server.close(() => resolve());
            for (const socket of this.sockets)
                socket.destroy();
        });
        return this.closing;
    }

    private track(socket: Socket): void {
        this.sockets.add(socket);
        socket.on("error", () => socket.destroy());
        socket.once("close", () => this.sockets.delete(socket));
    }

    private authenticated(request: IncomingMessage): boolean {
        const supplied = Buffer.from(request.headers["proxy-authorization"] || "");
        return supplied.length === this.authorization.length && timingSafeEqual(supplied, this.authorization);
    }

    private async addresses(url: URL) {
        if (url.hostname === "qbutt-webview.invalid")
            throw new Error("The proxy check URL cannot be forwarded.");

        const addresses = await (this.options.network?.addresses || publicAddresses)(url);
        if (this.closing)
            throw new Error("The media connection is closing.");
        return addresses;
    }

    private async forward(incoming: IncomingMessage, response: ServerResponse): Promise<void> {
        if (!this.authenticated(incoming)) {
            response.writeHead(407, { "Proxy-Authenticate": 'Basic realm="qbutt"', Connection: "close" });
            response.end();
            return;
        }

        if (incoming.method === "GET" && incoming.url === "http://qbutt-webview.invalid/"
            && incoming.headers["transfer-encoding"] === undefined
            && (incoming.headers["content-length"] === undefined || incoming.headers["content-length"] === "0")) {
            response.writeHead(200, {
                "Content-Type": "text/html",
                "Cache-Control": "no-store",
                "Content-Length": "0",
                "Connection": "close",
            });
            response.end();
            return;
        }

        let target: URL;
        let addresses: Awaited<ReturnType<typeof publicAddresses>>;
        try {
            target = new URL(incoming.url || "");
            addresses = await this.addresses(target);
        }
        catch {
            response.writeHead(403, { Connection: "close" });
            response.end();
            return;
        }
        if (incoming.destroyed || response.destroyed)
            return;

        const send = target.protocol === "https:" ? httpsRequest : httpRequest;
        const outgoing = send(target, {
            method: incoming.method,
            headers: { ...headers(incoming), Host: target.host },
            agent: false,
            lookup(_host, options, callback) {
                if (options.all)
                    callback(null, addresses);
                else
                    callback(null, addresses[0].address, addresses[0].family);
            },
        }, upstream => {
            response.writeHead(upstream.statusCode || 502, { ...headers(upstream), Connection: "close" });
            upstream.on("error", () => response.destroy());
            upstream.pipe(response);
        });
        outgoing.on("socket", socket => this.track(socket));
        outgoing.setTimeout(60_000, () => outgoing.destroy(new Error("The media source stopped responding.")));
        outgoing.on("error", () => {
            if (!response.headersSent) {
                response.writeHead(502, { Connection: "close" });
                response.end();
            }
            else
                response.destroy();
        });
        incoming.on("aborted", () => outgoing.destroy());
        incoming.on("error", () => outgoing.destroy());
        response.on("close", () => outgoing.destroy());
        let bodyBytes = 0;
        incoming.on("data", chunk => {
            bodyBytes += chunk.length;
            if (bodyBytes > 4 * 1024 * 1024) {
                outgoing.destroy();
                response.destroy();
                incoming.destroy();
            }
        });
        incoming.pipe(outgoing);
    }

    private async tunnel(request: IncomingMessage, client: Duplex, head: Buffer): Promise<void> {
        if (!this.authenticated(request)) {
            client.end('HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm="qbutt"\r\nConnection: close\r\n\r\n');
            return;
        }
        let target: URL;
        let addresses: Awaited<ReturnType<typeof publicAddresses>>;
        try {
            if (!/^(?:\[[a-f\d:]+\]|[a-z\d.-]+):\d{1,5}$/i.test(request.url || ""))
                throw new Error("Invalid target");
            target = new URL(`https://${request.url}`);
            if (!Number(target.port || 443))
                throw new Error("Invalid port");
            addresses = await this.addresses(target);
        }
        catch {
            client.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n");
            return;
        }
        if (client.destroyed)
            return;

        const upstream = connect({
            host: target.hostname.replace(/^\[|\]$/g, ""),
            port: Number(target.port || 443),
            lookup(_host, options, callback) {
                if (options.all)
                    callback(null, addresses);
                else
                    callback(null, addresses[0].address, addresses[0].family);
            },
        });
        this.track(upstream);
        upstream.setTimeout(60_000, () => upstream.destroy());
        let connected = false;
        upstream.once("connect", () => {
            connected = true;
            client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
            if (head.length)
                upstream.write(head);
            client.pipe(upstream);
            upstream.pipe(client);
        });
        upstream.on("error", () => {
            if (!connected)
                client.end("HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n");
            else
                client.destroy();
        });
        upstream.on("close", () => {
            if (!upstream.readableEnded)
                client.destroy();
        });
        client.on("error", () => upstream.destroy());
        client.on("close", () => upstream.destroy());
    }
}
