import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { zipSync, strToU8 } from "../components/content/node_modules/fflate";
import type { Event, Work } from "../components/content/contracts";

function require(value: unknown, message: string): asserts value {
    if (!value)
        throw new Error(message);
}

const root = await mkdtemp(join(tmpdir(), "qbutt-content-integration-"));
const appDir = resolve(process.env.QBUTT_CONTENT_APP_DIR || "C:/Temp/qbutt-content-build");
const executable = process.env.QBUTT_CONTENT_BIN;

class Client {
    private readonly process: ChildProcessWithoutNullStreams;
    private id = 0;
    private closed = false;
    private readonly exited: Promise<number | null>;
    private readonly requests = new Map<number, { events: Event[]; resolve(events: Event[]): void; reject(error: Error): void }>();

    constructor(profile = "profile") {
        const command = executable ? [executable] : [process.execPath, "components/content/main.ts"];
        this.process = spawn(command[0], [...command.slice(1), "--app-dir", appDir,
            "--data-dir", join(root, profile), "--download-dir", join(root, "downloads")], {
            windowsHide: true,
            stdio: ["pipe", "pipe", "pipe"],
            env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: join(root, "browser.json") },
        });
        this.process.stderr.resume();
        this.exited = new Promise(resolve => this.process.once("exit", resolve));
        this.process.on("error", error => this.fail(error));
        this.process.on("exit", code => this.fail(new Error(`Content helper exited: ${code}`)));
        let buffer = "";
        this.process.stdout.setEncoding("utf8");
        this.process.stdout.on("data", chunk => {
            buffer += chunk;
            let end: number;
            while ((end = buffer.indexOf("\n")) !== -1) {
                const event = JSON.parse(buffer.slice(0, end)) as Event;
                buffer = buffer.slice(end + 1);
                const request = this.requests.get(event.id);
                if (!request)
                    continue;
                request.events.push(event);
                if (event.event === "complete" || event.event === "error") {
                    this.requests.delete(event.id);
                    request.resolve(request.events);
                }
            }
        });
    }

    request(method: string, params = {}): Promise<Event[]> {
        const id = ++this.id;
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                this.requests.delete(id);
                reject(new Error(`Timed out: ${method}`));
            }, 180_000);
            this.requests.set(id, { events: [], resolve: events => { clearTimeout(timer); resolve(events); },
                reject: error => { clearTimeout(timer); reject(error); } });
            this.process.stdin.write(JSON.stringify({ id, method, params }) + "\n");
        });
    }

    async close(final?: { method: string; params: Record<string, unknown> }): Promise<void> {
        if (this.closed)
            return;
        this.closed = true;
        this.process.stdin.end(final ? JSON.stringify({ id: ++this.id, ...final }) + "\n" : undefined);
        const timeout = setTimeout(() => this.process.kill(), 10_000);
        const code = await this.exited;
        clearTimeout(timeout);
        require(code === 0, "Helper did not shut down cleanly after EOF");
    }

    private fail(error: Error): void {
        for (const request of this.requests.values())
            request.reject(error);
        this.requests.clear();
    }
}

function data<T>(events: Event[], event: string): T {
    const failure = events.find(item => item.event === "error");
    require(!failure, `${event}: ${JSON.stringify(failure?.data)}`);
    require(events.filter(item => item.event === "complete").length === 1, "Exactly one completion is required");
    const result = events.find(item => item.event === event);
    require(result, `Missing event: ${event}`);
    return result.data as T;
}

let client = new Client();
try {
    const sources = data<{ id: string; enabled: boolean; settings?: unknown[] }[]>(await client.request("sources"), "sources");
    for (const id of ["rutracker", "rutor", "nyaasi", "nekobt", "byrut", "freetp", "onlinefix", "yani", "mangalib", "ai", "browser"])
        require(sources.some(source => source.id === id), `Missing built-in source: ${id}`);
    require(sources.filter(source => source.enabled).length < 60, "Fresh install must bound the active source set");

    const configured = data<{ id: string; settings?: { name: string; value?: unknown }[] }[]>(
        await client.request("configureSource", { id: "rutor", settings: { stripcyrillic: true } }), "sources");
    require(sources.every(source => configured.some(updated => updated.id === source.id)),
        "Saving an indexer dropped built-in sources from the catalog");
    require(configured.find(source => source.id === "rutor")?.settings
        ?.find(setting => setting.name === "stripcyrillic")?.value === true, "Indexer changes were not reflected in the catalog");
    const protectedSources = data<unknown[]>(await client.request("configureSource", {
        id: "ai", settings: { provider: "auto", apiKey: "generated-fixture-secret_日本語" },
    }), "sources");
    require(!JSON.stringify(protectedSources).includes("generated-fixture-secret"), "Source list exposed a secret");
    const database = await readFile(join(root, "profile", "library.sqlite"));
    const wal = await readFile(join(root, "profile", "library.sqlite-wal")).catch(() => Buffer.alloc(0));
    require(!Buffer.concat([database, wal]).includes(Buffer.from("generated-fixture-secret")), "Settings were persisted in plaintext");

    const search = await client.request("search", { query: "Frieren", category: "anime", sources: ["nyaasi"] });
    const found = search.filter(event => event.event === "result").map(event => event.data as Work);
    require(found.length > 0 && search.at(-1)?.event === "complete", "Live search produced no usable results");
    const item = found[0];
    const selected = item.variants[0];
    await client.request("remember", { work: item, variant: selected, favorite: true, opened: true });
    const shared = data<{ url: string }>(await client.request("share", { work: item, variant: selected, mode: "exact" }), "share");
    require(shared.url.startsWith("qbutt://open?d="), "Exact share URL is missing");
    require(!shared.url.includes("Cookie"), "Share URL includes private request state");
    const opened = data<Work>(await client.request("openLink", { url: shared.url }), "details");
    require(opened.variants[0].id === selected.id, "Exact link selected another release");

    const magnet = "magnet:?xt=urn:btih:1111111111111111111111111111111111111111&dn=Generated%20legal%20fixture";
    const direct = data<Work>(await client.request("search", { query: magnet, category: "all" }), "result");
    require(direct.id.startsWith("link:") && direct.variants[0].id.startsWith("link:"), "Direct URLs must use opaque identities");
    const exactDirect = data<{ url: string }>(await client.request("share", {
        work: direct, variant: direct.variants[0], mode: "exact",
    }), "share");
    const workLink = data<{ url: string }>(await client.request("share", { work: direct, mode: "work" }), "share");
    const pasted = data<Work>(await client.request("search", { query: workLink.url, category: "all" }), "details");
    require(pasted.title === direct.title && pasted.variants[0].id === direct.variants[0].id,
        "A pasted work link must preserve its known publications without requiring a title search");

    const directSecret = "generated-direct-url-token_日本語";
    const directUrl = `https://media.w3.org/2010/05/sintel/trailer.mp4?token=${encodeURIComponent(directSecret)}`;
    const signed = data<Work>(await client.request("search", { query: directUrl, category: "movies" }), "result");
    const expectedNotes = [{ text: "Generated source comment", url: "https://example.org/publication?view=comments", author: "Fixture author" }];
    const privateNotes = [
        { ...expectedNotes[0], url: `${expectedNotes[0].url}&token=${encodeURIComponent(directSecret)}#comment-2` },
        { text: "Private credentials", url: `https://fixture:${encodeURIComponent(directSecret)}@example.org/publication` },
        { text: "Private local file", url: "file:///C:/fixture/private.txt" },
    ];
    signed.notes = privateNotes;
    signed.variants[0].notes = privateNotes;
    await client.request("remember", { work: signed, variant: signed.variants[0] });
    const temporaryShare = await client.request("share", { work: signed, variant: signed.variants[0], mode: "exact" });
    require(temporaryShare.at(-1)?.event === "error", "A temporary media URL must not become a dead recipient link");
    const stableLibrary = data<{ work: Work; variant?: Work["variants"][number] }[]>(await client.request("library"), "library");
    require(!JSON.stringify(stableLibrary).includes("generated-direct-url-token"), "Library metadata exposed a source URL token");
    const stableSigned = stableLibrary.find(entry => entry.work.id === signed.id);
    for (const notes of [stableSigned?.work.notes, stableSigned?.variant?.notes])
        require(JSON.stringify(notes) === JSON.stringify(expectedNotes), "Saved notes lost content or retained a private URL");
    const protectedDatabase = await readFile(join(root, "profile", "library.sqlite"));
    const protectedWal = await readFile(join(root, "profile", "library.sqlite-wal")).catch(() => Buffer.alloc(0));
    require(!Buffer.concat([protectedDatabase, protectedWal]).includes(Buffer.from("generated-direct-url-token")), "Acquisition URL was persisted in plaintext");

    const epub = join(root, "original.epub");
    await writeFile(epub, zipSync({
        "mimetype": strToU8("application/epub+zip"),
        "META-INF/container.xml": strToU8('<container><rootfiles><rootfile full-path="book.opf"/></rootfiles></container>'),
        "book.opf": strToU8('<package><metadata><title>Generated original</title></metadata><manifest><item id="one" href="one.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="one"/></spine></package>'),
        "one.xhtml": strToU8('<html><body><h1>Original</h1><p>これは独自に作成した検証用の文章です。</p></body></html>'),
    }));
    const document = data<{ id: string; chapters: { id: string }[] }>(await client.request("reader.local", { path: epub }), "complete");
    require(document.chapters.length === 1, "Local reader did not honor EPUB spine");
    const page = data<{ html: string }>(await client.request("reader.chapter", {
        documentId: document.id, chapterId: document.chapters[0].id,
    }), "complete");
    require(page.html.includes("検証用"), "Original chapter content was lost");
    await client.request("reader.position", { documentId: document.id, chapterId: document.chapters[0].id, offset: 0.42 });

    await client.close({ method: "reader.position", params: {
        documentId: document.id, chapterId: document.chapters[0].id, offset: 0.76,
    } });
    const recipient = new Client("recipient");
    try {
        const exact = data<Work>(await recipient.request("openLink", { url: exactDirect.url }), "details");
        require(exact.variants[0].id === direct.variants[0].id, "A fresh recipient lost the exact direct variant");
        require(exact.variants[0].magnet?.includes("1111111111111111111111111111111111111111"), "The recipient lost the torrent identity");
    }
    finally {
        await recipient.close();
    }
    client = new Client();
    const saved = data<{ work: Work; favorite: boolean }[]>(await client.request("library", { mode: "favorites" }), "library");
    require(saved.length === 1 && saved[0].work.id === item.id && saved[0].favorite, "Library failed restart persistence");
    const reopened = data<{ position?: { offset: number } }>(await client.request("reader.local", { path: epub }), "complete");
    require(reopened.position?.offset === 0.76, "The final queued reading position was lost on EOF");
    const restartedLibrary = data<{ work: Work; variant?: Work["variants"][number] }[]>(await client.request("library"), "library");
    const restoredSigned = restartedLibrary.find(entry => entry.work.id === signed.id);
    require(restoredSigned?.variant, "The saved direct variant did not survive restart");
    for (const notes of [restoredSigned.work.notes, restoredSigned.variant.notes])
        require(JSON.stringify(notes) === JSON.stringify(expectedNotes), "Restart lost sanitized work or variant notes");
    const resolvedSigned = data<{ url: string }>(await client.request("resolve", { variant: restoredSigned.variant }), "resolved");
    require(resolvedSigned.url === new URL(directUrl).href, "Restart lost the protected original acquisition address");
    const unsafe = await client.request("search", { query: "http://127.0.0.1/private", category: "all" });
    require(unsafe.at(-1)?.event === "error", "Local URL reached source discovery");
    await client.close();
    console.log(JSON.stringify({ ok: true, sources: sources.length, searchResults: found.length,
        exactShare: true, freshDirectRecipient: true, pastedWorkLink: true, protectedSettings: true,
        protectedAcquisitionRestart: true, sanitizedNotesRestart: true, libraryRestart: true, readerRestart: true,
        finalPositionOnEof: true, cleanEof: true }));
}
finally {
    await client.close().catch(() => {});
    await rm(root, { recursive: true, force: true });
}
