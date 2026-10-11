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
    payload?: {
        sourceUrl: string;
        headers: Record<string, string>;
        signal: AbortSignal;
    };
}

interface PayloadResource {
    parts: string[];
    variables: { name: string; token: string }[];
    representationIds: Set<string>;
    suffix: string;
}

interface PayloadManifest {
    body: Buffer;
    suffix: string;
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
    private readonly resources = new Map<string, PayloadResource | PayloadManifest>();
    private manifestBytes = 0;
    private readonly payloadOrigin: string;
    private readonly payloadPrefix: string;
    private payloadRequests = 0;
    private closing?: Promise<void>;

    private constructor(private readonly server: Server, password: string,
        private readonly options: ProxyOptions) {
        const address = server.address();
        if (!address || typeof address === "string")
            throw new Error("The media connection could not be started.");
        this.authorization = Buffer.from(`Basic ${Buffer.from(`qbutt:${password}`).toString("base64")}`);
        this.url = `http://qbutt:${password}@127.0.0.1:${address.port}`;
        this.payloadOrigin = `http://127.0.0.1:${address.port}`;
        this.payloadPrefix = `/_qbutt/${password}/`;

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
        options.payload?.signal.throwIfAborted();
        const server = createServer({ maxHeaderSize: 16 * 1024 });
        await new Promise<void>((resolve, reject) => {
            server.once("error", reject);
            server.listen(0, "127.0.0.1", () => {
                server.removeListener("error", reject);
                resolve();
            });
        });
        const proxy = new NetworkProxy(server, randomBytes(24).toString("hex"), options);
        if (options.payload?.signal.aborted) {
            await proxy.close();
            options.payload.signal.throwIfAborted();
        }
        return proxy;
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

    async resource(input: string, representationIds: string[] = []): Promise<string> {
        if (!this.options.payload)
            throw new Error("This connection does not expose media resources.");
        this.options.payload.signal.throwIfAborted();
        if (this.resources.size >= 10000)
            throw new Error("The video manifest contains too many resources.");

        const target = new URL(input);
        await this.addresses(target);
        if (target.origin.includes("$"))
            throw new Error("DASH cannot substitute a server address.");

        const parts: string[] = [];
        const variables: PayloadResource["variables"] = [];
        let offset = 0;
        for (const match of target.href.matchAll(/\$\$|\$(RepresentationID|Number|Bandwidth|Time)(?:%0[1-9]\d?d)?\$/g)) {
            if (match[0] === "$$")
                continue;
            parts.push(target.href.slice(offset, match.index).replaceAll("$$", "$"));
            variables.push({ name: match[1], token: match[0] });
            offset = match.index! + match[0].length;
        }
        parts.push(target.href.slice(offset).replaceAll("$$", "$"));
        if (variables.some(variable => variable.name === "RepresentationID") && !representationIds.length)
            throw new Error("The video manifest omitted its representation IDs.");

        const id = String(this.resources.size);
        const suffix = `resource${target.pathname.match(/\.[a-z\d]{1,8}$/i)?.[0] || ".bin"}`;
        this.resources.set(id, { parts, variables, representationIds: new Set(representationIds), suffix });
        return this.payloadOrigin + this.payloadPrefix + [id, ...variables.map(variable => variable.token), suffix].join("/");
    }

    manifest(content: string): string {
        if (!this.options.payload || this.closing)
            throw new Error("This connection does not expose media resources.");
        this.options.payload.signal.throwIfAborted();
        const body = Buffer.from(content);
        if (this.resources.size >= 10000 || body.length > 4 * 1024 * 1024
            || this.manifestBytes + body.length > 16 * 1024 * 1024)
            throw new Error("The video manifest is too large.");

        const id = String(this.resources.size);
        const suffix = "playlist.m3u8";
        this.resources.set(id, { body, suffix });
        this.manifestBytes += body.length;
        return this.payloadOrigin + this.payloadPrefix + [id, suffix].join("/");
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
        const addresses = await (this.options.network?.addresses || publicAddresses)(url);
        if (this.closing)
            throw new Error("The media connection is closing.");
        return addresses;
    }

    private async forward(incoming: IncomingMessage, response: ServerResponse): Promise<void> {
        if (this.options.payload) {
            await this.payload(incoming, response);
            return;
        }
        if (!this.authenticated(incoming)) {
            response.writeHead(407, { "Proxy-Authenticate": 'Basic realm="qbutt"', Connection: "close" });
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

    private async payload(incoming: IncomingMessage, response: ServerResponse): Promise<void> {
        const scope = this.options.payload!;
        let target: URL;
        try {
            scope.signal.throwIfAborted();
            const local = new URL(incoming.url || "", this.payloadOrigin);
            if (local.origin !== this.payloadOrigin || !local.pathname.startsWith(this.payloadPrefix)
                || local.search || !["GET", "HEAD"].includes(incoming.method || "") || ++this.payloadRequests > 10000)
                throw new Error("Invalid media resource request.");
            const [id, ...path] = local.pathname.slice(this.payloadPrefix.length).split("/");
            const resource = this.resources.get(id);
            if (!resource || path.pop() !== resource.suffix)
                throw new Error("Unknown media resource.");
            if ("body" in resource) {
                if (path.length)
                    throw new Error("Unknown media resource.");
                response.writeHead(200, {
                    "Content-Type": "application/vnd.apple.mpegurl",
                    "Content-Length": resource.body.length,
                    "Cache-Control": "no-store",
                    Connection: "close",
                });
                response.end(incoming.method === "HEAD" ? undefined : resource.body);
                return;
            }
            if (path.length !== resource.variables.length)
                throw new Error("Unknown media resource.");
            let input = resource.parts[0];
            for (let index = 0; index < resource.variables.length; ++index) {
                const value = decodeURIComponent(path[index]);
                const variable = resource.variables[index];
                if (variable.name === "RepresentationID" ? !resource.representationIds.has(value) : !/^\d{1,20}$/.test(value))
                    throw new Error("Invalid DASH template value.");
                input += value + resource.parts[index + 1];
            }
            target = new URL(input);
        }
        catch {
            response.writeHead(403, { Connection: "close" });
            response.end();
            return;
        }

        try {
            const original = new URL(scope.sourceUrl).origin;
            for (let redirect = 0; redirect <= 5; ++redirect) {
                const addresses = await this.addresses(target);
                scope.signal.throwIfAborted();
                if (incoming.destroyed || response.destroyed)
                    return;
                const outgoingHeaders: Record<string, string> = { "Accept-Encoding": "identity" };
                for (const [name, value] of Object.entries(scope.headers)) {
                    if (!/^[\w-]+$/.test(name) || /[\r\n]/.test(value))
                        throw new Error("The source supplied invalid media headers.");
                    if (hopHeaders.has(name.toLowerCase()) || /^(?:host|accept-encoding|content-length|range)$/i.test(name))
                        continue;
                    if (target.origin !== original && !/^(?:user-agent|referer|origin|accept|accept-language)$/i.test(name))
                        continue;
                    outgoingHeaders[name] = value;
                }
                if (incoming.headers.range) {
                    if (!/^bytes=\d*-\d*$/.test(incoming.headers.range))
                        throw new Error("Invalid media byte range.");
                    outgoingHeaders.Range = incoming.headers.range;
                }

                const upstream = await new Promise<IncomingMessage>((resolve, reject) => {
                    const send = target.protocol === "https:" ? httpsRequest : httpRequest;
                    const request = send(target, {
                        method: incoming.method,
                        headers: outgoingHeaders,
                        signal: scope.signal,
                        agent: false,
                        lookup(_host, options, callback) {
                            if (options.all)
                                callback(null, addresses);
                            else
                                callback(null, addresses[0].address, addresses[0].family);
                        },
                    }, resolve);
                    request.on("socket", socket => this.track(socket));
                    request.setTimeout(30_000, () => request.destroy(new Error("The media source stopped responding.")));
                    request.on("error", reject);
                    response.once("close", () => request.destroy());
                    request.end();
                });
                if ([301, 302, 303, 307, 308].includes(upstream.statusCode || 0)) {
                    const location = upstream.headers.location;
                    upstream.destroy();
                    if (!location)
                        throw new Error("The media source returned an incomplete redirect.");
                    target = new URL(location, target);
                    continue;
                }
                if ((upstream.statusCode || 0) < 200 || (upstream.statusCode || 0) >= 300) {
                    response.writeHead(upstream.statusCode || 502, { Connection: "close" });
                    upstream.destroy();
                    response.end();
                    return;
                }
                response.writeHead(upstream.statusCode!, { ...headers(upstream), Connection: "close" });
                upstream.on("error", () => response.destroy());
                upstream.pipe(response);
                return;
            }
            throw new Error("The media source redirected too many times.");
        }
        catch {
            if (!response.headersSent) {
                response.writeHead(502, { Connection: "close" });
                response.end();
            }
            else
                response.destroy();
        }
    }

    private async tunnel(request: IncomingMessage, client: Duplex, head: Buffer): Promise<void> {
        if (this.options.payload) {
            client.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n");
            return;
        }
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
