import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { handleReaderRequest } from "../../components/content/reader/service";
import type { ProviderContext } from "../../components/content/contracts";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";

const root = await mkdtemp(join(tmpdir(), "qbutt-reading-loading-"));
const image = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a0u8AAAAASUVORK5CYII="), value => value.charCodeAt(0));
let activeImages = 0;
let imageRequests = 0;
let abortedImages = 0;
let metadataGate: { started(): void; released: Promise<void>; status: number } | undefined;

function holdMetadata(status: number): { started: Promise<void>; release(): void } {
    const started = Promise.withResolvers<void>();
    const released = Promise.withResolvers<void>();
    metadataGate = { started: started.resolve, released: released.promise, status };
    return { started: started.promise, release: released.resolve };
}

const server = Bun.serve({
    hostname: "127.0.0.1", port: 0,
    async fetch(request): Promise<Response> {
        const path = new URL(request.url).pathname;
        if (path === "/works/fixture") {
            const gate = metadataGate;
            metadataGate = undefined;
            if (gate) {
                gate.started();
                await gate.released;
                if (gate.status !== 200) return new Response("Legal metadata failure", { status: gate.status });
            }
            return new Response(`<script id="__NEXT_DATA__">${JSON.stringify({ props: { pageProps: { __APOLLO_STATE__: {
                work: { __typename: "Work", id: "fixture", title: "Legal loading fixture" },
                episode: { __typename: "Episode", id: "chapter", title: "Original chapter" },
            } } } })}</script>`);
        }
        if (path.includes("/episodes/")) {
            return new Response(`<div class="widget-episodeBody"><p>凛は庭で猫に挨拶しました。</p>${["good", "bad", "slow"].map(name => `<img src="http://127.0.0.1:${server.port}/${name}.png"/>`).join("")}</div>`);
        }
        ++imageRequests;
        ++activeImages;
        try {
            await new Promise<void>(resolve => {
                const timer = setTimeout(resolve, path === "/slow.png" ? 5000 : path === "/bad.png" ? 80 : 20);
                request.signal.addEventListener("abort", () => {
                    ++abortedImages;
                    clearTimeout(timer);
                    resolve();
                }, { once: true });
            });
            return path === "/bad.png" ? new Response("Legal injected source failure", { status: 503 })
                : new Response(image, { headers: { "Content-Type": "image/png" } });
        }
        finally {
            --activeImages;
        }
    },
});
const context: ProviderContext = {
    signal: new AbortController().signal,
    dataDir: join(root, "profile"),
    settings: {},
    fetch(url, init) {
        const address = new URL(url);
        if (address.hostname === "kakuyomu.jp") {
            return fetch(`http://127.0.0.1:${server.port}${address.pathname}`, init);
        }
        return fetch(url, init);
    },
};

try {
    const variant = { id: "kakuyomu:fixture", sourceId: "kakuyomu", sourceName: "Legal fixture", title: "Original", transport: "book", url: "https://kakuyomu.jp/works/fixture" };
    const document = await handleReaderRequest("reader.open", { variant }, context) as ReaderDocument;
    for (const [status, offset] of [[200, 0.25], [503, 0.75]] as const) {
        const gate = holdMetadata(status);
        const opening = handleReaderRequest("reader.open", { variant, work: { id: "canonical-fixture" } }, context) as Promise<ReaderDocument>;
        await gate.started;
        await handleReaderRequest("reader.position", {
            documentId: document.id, chapterId: document.chapters[0]!.id, offset,
        }, context);
        gate.release();
        const reopened = await opening;
        if (reopened.position?.offset !== offset || reopened.workId !== "canonical-fixture") {
            throw new Error(`Metadata HTTP ${status} overwrote concurrent reading progress or the canonical work identity`);
        }
    }
    console.info("PASS live/cached metadata completion preserves concurrent reading position and canonical work identity");
    const received: ReaderPage[] = [];
    let failed = false;
    try {
        await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: document.chapters[0]!.id }, context,
            (event, data) => { if (event === "page") received.push(data as ReaderPage); });
    }
    catch {
        failed = true;
    }
    await new Promise(resolve => setTimeout(resolve, 100));
    if (!failed || !received[0]?.html?.includes("凛は庭") || imageRequests !== 3 || activeImages !== 0 || abortedImages !== 1) {
        throw new Error(`Loading ownership failed: ${JSON.stringify({ failed, earlyPages: received.length, imageRequests, activeImages, abortedImages })}`);
    }
    console.info("PASS original text is readable before illustrations; source error aborts and drains all image workers");
}
finally {
    server.stop(true);
    await rm(root, { recursive: true, force: true });
}
