import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const executable = process.env.QBUTT_SEARCH_BIN;
assert(executable, "Set QBUTT_SEARCH_BIN to the built qbutt-search executable");
const directory = await mkdtemp(join(tmpdir(), "qbutt-search-integration-"));
type Event = { id: number; event: string; data?: any };

class Helper {
    process;
    sequence = 0;
    requests = new Map<number, { events: Event[]; resolve: (events: Event[]) => void }>();
    output = "";
    pump: Promise<void>;

    constructor(readonly dataDir: string) {
        this.process = Bun.spawn([executable!, "--data-dir", dataDir], {
            stdin: "pipe",
            stdout: "pipe",
            stderr: "pipe",
        });
        this.pump = this.read();
    }

    async read() {
        const decoder = new TextDecoder();
        let pending = "";
        for await (const chunk of this.process.stdout) {
            pending += decoder.decode(chunk, { stream: true });
            for (;;) {
                const newline = pending.indexOf("\n");
                if (newline < 0) break;
                const line = pending.slice(0, newline);
                pending = pending.slice(newline + 1);
                this.output += line + "\n";
                const event = JSON.parse(line) as Event;
                const request = this.requests.get(event.id);
                assert(request, `Unexpected event for request ${event.id}`);
                request.events.push(event);
                if (event.event === "complete" || event.event === "error") {
                    assert.equal(request.events.filter(item => ["complete", "error"].includes(item.event)).length, 1);
                    request.resolve(request.events);
                }
            }
        }
    }

    request(method: string, params: Record<string, unknown> = {}) {
        const id = ++this.sequence;
        const promise = new Promise<Event[]>((resolve, reject) => {
            const timer = setTimeout(() => reject(Error(`${method} timed out`)), 65_000);
            this.requests.set(id, { events: [], resolve: events => { clearTimeout(timer); resolve(events); } });
        });
        this.process.stdin.write(JSON.stringify({ id, method, params }) + "\n");
        return { id, promise };
    }

    async call(method: string, params: Record<string, unknown> = {}) {
        return await this.request(method, params).promise;
    }

    async close() {
        this.process.stdin.end();
        await this.process.exited;
        await this.pump;
        assert.equal(this.process.exitCode, 0);
        assert.equal(await new Response(this.process.stderr).text(), "");
    }
}

function data(events: Event[], name: string) {
    const event = events.find(event => event.event === name);
    assert(event, `Missing ${name}: ${JSON.stringify(events.at(-1))}`);
    return event.data;
}

let helper = new Helper(directory);
try {
    const sources = data(await helper.call("sources"), "sources");
    assert(sources.length >= 500);
    assert(sources.filter((source: any) => source.enabled).length < 30);
    assert.equal(sources.find((source: any) => source.id === "rutracker").url, "https://rutracker.org");
    assert(sources.find((source: any) => source.id === "rutor").categories.includes("all"));
    assert(["books", "games", "movies", "anime"].every(category => sources.find((source: any) => source.id === "rutor").categories.includes(category)));
    const rutorSettings = sources.find((source: any) => source.id === "rutor").settings;
    for (const name of ["sort", "stripcyrillic", "addrussiantotitle", "proxyUrl"])
        assert.equal(rutorSettings.find((setting: any) => setting.name === name).advanced, true);
    for (const name of ["username", "password", "cookie"])
        assert(!sources.find((source: any) => source.id === "rutracker").settings.find((setting: any) => setting.name === name).advanced);

    const secret = `integration-secret-${crypto.randomUUID()}`;
    await helper.call("configureSource", { id: "rutracker", enabled: false, settings: { username: "integration", password: secret } });
    assert(!helper.output.includes(secret));
    assert(!(await readFile(join(directory, "sources.dat"))).includes(Buffer.from(secret)));
    await helper.call("configureSource", { id: "rutracker", settings: { username: "", password: "" } });
    assert.equal(data(await helper.call("sources"), "sources").find((source: any) => source.id === "rutracker").settings.find((setting: any) => setting.name === "username").value, "");

    const refused = await helper.call("discover", { url: "http://127.0.0.1:80/" });
    assert.equal(refused.at(-1)?.event, "error");
    assert.match(refused.at(-1)?.data.message, /private network/);

    const cancelled = helper.request("search", { query: crypto.randomUUID(), category: "all", sources: ["rutor", "nyaasi"] });
    await helper.call("cancel", { requestId: cancelled.id });
    assert.equal((await cancelled.promise).at(-1)?.event, "error");

    const searches = await Promise.all([
        helper.call("search", { query: "Dune", category: "movies", sources: ["rutor"] }),
        helper.call("search", { query: "Frieren", category: "anime", sources: ["nyaasi"] }),
        helper.call("search", { query: "Frieren", category: "anime", sources: ["nekobt"] }),
    ]);
    for (const [index, events] of searches.entries()) {
        assert.equal(events.at(-1)?.event, "complete");
        assert(events.some(event => event.event === "result"), `Live source ${index} returned no releases`);
    }
    assert(searches[0]!.filter(event => event.event === "result").every(event => event.data.category === "movies"), "RuTor movies must use the site's actual movie sections");
    assert(searches.slice(1).flat().filter(event => event.event === "result").every(event => event.data.category === "anime"), "Anime must come from actual tracker metadata");
    const booksControl = await helper.call("search", { query: "Moby Dick", category: "books", sources: ["rutor", "nyaasi", "nekobt", "yts"] });
    assert.equal(booksControl.at(-1)?.event, "complete");
    assert(booksControl.filter(event => event.event === "result").every(event => event.data.category === "books"), "Movies and audio must not appear as Books");
    const categoryChecks = await Promise.all([
        helper.call("search", { query: "Моби Дик", category: "books", sources: ["rutor"] }),
        helper.call("search", { query: "Cyberpunk", category: "games", sources: ["rutor"] }),
        helper.call("search", { query: "Frieren", category: "anime", sources: ["rutor"] }),
        helper.call("search", { query: "Moby Dick", category: "all", sources: ["rutor"] }),
    ]);
    for (const [index, events] of categoryChecks.entries()) {
        const category = ["books", "games", "anime", "all"][index];
        assert.equal(events.at(-1)?.event, "complete");
        const works = events.filter(event => event.event === "result").map(event => event.data);
        assert(works.length > 0, `RuTor ${category} returned no releases`);
        assert(works.every(work => work.category === category), `RuTor ${category} must use source category metadata`);
    }
    assert(categoryChecks[0]!.some(event => event.event === "result" && /FB2|EPUB/i.test(event.data.title)), "Books must include the actual Moby Dick ebook release");
    const work = data(searches[0]!, "result");
    const variant = work.variants[0];
    const resolution = data(await helper.call("resolve", { variant: { ...variant, downloadUrl: "http://127.0.0.1/leak" } }), "resolved");
    assert.equal(resolution.kind, "torrent");
    assert.match(resolution.url, /^magnet:\?/);
    const changedIdentity = await helper.call("resolve", { variant: { ...variant, infoHash: "0".repeat(40) } });
    assert.equal(changedIdentity.at(-1)?.event, "error");

    const discovery = await helper.call("discover", { url: "https://webtorrent.io/free-torrents", query: "Sintel" });
    const discovered = data(discovery, "result");
    assert.equal(discovered.title, "Sintel");
    const metadata = data(await helper.call("resolve", { variant: discovered.variants[0] }), "resolved");
    assert.equal(metadata.kind, "torrent");
    assert(metadata.torrentData || metadata.url?.startsWith("magnet:?"));
    const urlSecret = `public-url-secret-${crypto.randomUUID()}`;
    const protectedDiscovery = await helper.call("discover", { url: `https://webtorrent.io/free-torrents?apikey=${urlSecret}`, query: "Sintel" });
    assert.equal(protectedDiscovery.at(-1)?.event, "complete");
    assert(!JSON.stringify(protectedDiscovery).includes(urlSecret));

    const browserCookie = `bb_session=browser-fixture-${crypto.randomUUID()}`;
    await helper.call("search", { query: "", cookies: { rutracker: browserCookie }, userAgents: { rutracker: "Mozilla/5.0 qbutt-integration" } });
    const disabledWithSession = data(await helper.call("sources"), "sources").find((source: any) => source.id === "rutracker");
    assert.equal(disabledWithSession.enabled, false, "An explicit disabled choice wins over browser authentication");
    assert(!(await readFile(join(directory, "sources.dat"))).includes(Buffer.from(browserCookie)));
    assert(!helper.output.includes(browserCookie));

    await helper.close();
    helper = new Helper(directory);
    const restored = data(await helper.call("sources"), "sources");
    assert.equal(restored.find((source: any) => source.id === "rutracker").enabled, false);
    assert.equal(data(await helper.call("resolve", { variant }), "resolved").url, resolution.url);
    assert(!(await readFile(join(directory, "releases.dat"))).includes(Buffer.from(variant.title)));

    const browserDir = await mkdtemp(join(tmpdir(), "qbutt-search-browser-"));
    let browser = new Helper(browserDir);
    try {
        await browser.call("search", { query: "", cookies: { rutracker: browserCookie }, userAgents: { rutracker: "Mozilla/5.0 qbutt-integration" } });
        assert.equal(data(await browser.call("sources"), "sources").find((source: any) => source.id === "rutracker").enabled, true);
        await browser.close();
        browser = new Helper(browserDir);
        assert.equal(data(await browser.call("sources"), "sources").find((source: any) => source.id === "rutracker").enabled, false);
    } finally {
        await browser.close();
        await rm(browserDir, { recursive: true, force: true });
    }

    const recipientDir = await mkdtemp(join(tmpdir(), "qbutt-search-recipient-"));
    const recipient = new Helper(recipientDir);
    try {
        const exact = data(await recipient.call("inspect", { variant }), "details");
        assert(exact.variants.some((candidate: any) => candidate.infoHash === variant.infoHash));
        const wrong = await recipient.call("inspect", { variant: { ...variant, id: "unknown", infoHash: "0".repeat(40) } });
        assert.equal(wrong.at(-1)?.event, "error");
    } finally {
        await recipient.close();
        await rm(recipientDir, { recursive: true, force: true });
    }

    const rollbackDir = await mkdtemp(join(tmpdir(), "qbutt-search-rollback-"));
    const originalDefinition = await readFile(new URL("../../components/search-engine/internal/indexer/definitions/vendor/rutor.yml", import.meta.url), "utf8");
    const currentDir = join(rollbackDir, "definitions", "current");
    const previousDir = join(rollbackDir, "definitions", "previous");
    await mkdir(currentDir, { recursive: true });
    await mkdir(previousDir, { recursive: true });
    await writeFile(join(currentDir, "rutor.yml"), originalDefinition.replace('selector: tr:has(td:has(a[href^="magnet:?xt="]))', "selector: .qbutt-integration-missing-selector"));
    await writeFile(join(previousDir, "rutor.yml"), originalDefinition);
    await writeFile(join(rollbackDir, "definition-update.json"), JSON.stringify({ checkedAt: new Date().toISOString() }));
    let rollback = new Helper(rollbackDir);
    try {
        const recoveredSearch = await rollback.call("search", { query: "Dune", category: "movies", sources: ["rutor"] });
        assert(recoveredSearch.some(event => event.event === "result"), "Previous definition must recover actual rows");
        assert.equal(await readFile(join(currentDir, "rutor.yml"), "utf8"), originalDefinition);
        await rollback.close();
        await rm(currentDir, { recursive: true, force: true });
        rollback = new Helper(rollbackDir);
        data(await rollback.call("sources"), "sources");
        assert.equal(await readFile(join(currentDir, "rutor.yml"), "utf8"), originalDefinition);
    } finally {
        await rollback.close();
        await rm(rollbackDir, { recursive: true, force: true });
    }

    console.log(JSON.stringify({ status: "passed", sources: sources.length, liveResults: searches.map(events => events.filter(event => event.event === "result").length), rutorCategories: categoryChecks.map(events => events.filter(event => event.event === "result").length), checks: ["catalogue", "DPAPI", "SSRF", "cancel", "live-search", "actual-categories", "resolve-binding", "legal-site-discovery", "restart", "fresh-recipient-exact-hash", "selector-rollback", "update-crash-recovery"] }));
} finally {
    await helper.close();
    await rm(directory, { recursive: true, force: true });
}
