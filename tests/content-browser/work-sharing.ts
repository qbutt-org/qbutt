import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import type { Event, Variant, Work } from "../../components/content/contracts";
import { openLink } from "../../components/content/sharing";

const directory = await mkdtemp(join(tmpdir(), "qbutt-work-sharing-"));
const sourceRoot = resolve(import.meta.dir, "../../components/content");
const title = "Generated whole series";
const secret = "generated-private-acquisition";
const inspected: string[] = [];
const variants: Variant[] = [];
for (const source of ["alpha", "beta"]) {
    for (const episode of ["1", "2"]) {
        for (const translation of ["Generated dub", "Generated subtitles"]) {
            variants.push({ id: `${source}:${episode}:${translation}`, sourceId: source, sourceName: source,
                title, url: `https://example.com/${source}/publication`, transport: "video",
                episode, season: "2", translation, player: "Generated player", quality: "1080p",
                downloadUrl: `https://example.com/media.mp4?token=${secret}`,
                ...{ headers: { Cookie: secret }, cookie: secret } });
        }
    }
}
const work: Work = { id: "generated:whole-work", title, category: "anime", variants };
const selected = variants.find(item => item.sourceId === "alpha" && item.episode === "2"
    && item.translation === "Generated subtitles")!;
const server = Bun.serve({ port: 0, fetch(request) {
    const source = new URL(request.url).pathname.slice(1);
    inspected.push(source);
    return Response.json({ id: `${source}:provider-work`, title: `Generated ${source} provider title`, category: "anime",
        variants: variants.filter(variant => variant.sourceId === source).map(({ downloadUrl, ...variant }) => variant) });
} });
const adapter = join(directory, "catalog-adapter.ts");
await writeFile(adapter, `
import { Catalog as ProductionCatalog } from ${JSON.stringify(join(sourceRoot, "catalog.ts"))};
export class Catalog extends ProductionCatalog {
    constructor(...args) {
        super(...args);
        this.providers.splice(0, this.providers.length, ...["alpha", "beta"].map(id => ({
            source: { id, name: id, url: "https://example.com", enabled: true, categories: ["anime"] },
            async search() { return []; },
            async inspect(variant, context) {
                const response = await fetch(${JSON.stringify(server.url.href)} + id, { signal: context.signal });
                return response.json();
            },
        })));
        this.engine.request = async () => {};
    }
}
`);
const build = await Bun.build({ entrypoints: [join(sourceRoot, "main.ts")], target: "bun", outdir: directory,
    plugins: [{ name: "generated-publication-services", setup(build) {
        build.onResolve({ filter: /^\.\/catalog$/ }, args => args.importer === join(sourceRoot, "main.ts")
            ? { path: adapter } : undefined);
    } }] });
assert(build.success, String(build.logs));
const child = spawn(process.execPath, [join(directory, "main.js"), "--app-dir", directory,
    "--data-dir", join(directory, "profile"), "--download-dir", join(directory, "downloads")],
    { windowsHide: true, stdio: ["pipe", "pipe", "pipe"],
        env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: join(directory, "browser.json") } });
child.stderr.resume();
const exited = new Promise<number | null>(resolve => child.once("exit", resolve));
let sequence = 0;
let buffered = "";
const pending = new Map<number, { events: Event[]; resolve(events: Event[]): void; reject(error: Error): void }>();
child.stdout.setEncoding("utf8");
child.stdout.on("data", chunk => {
    buffered += chunk;
    let end;
    while ((end = buffered.indexOf("\n")) >= 0) {
        const event = JSON.parse(buffered.slice(0, end)) as Event;
        buffered = buffered.slice(end + 1);
        const request = pending.get(event.id);
        if (!request) continue;
        request.events.push(event);
        if (event.event === "complete" || event.event === "error") {
            pending.delete(event.id);
            request.resolve(request.events);
        }
    }
});
child.once("exit", code => {
    for (const request of pending.values()) request.reject(new Error(`Content helper exited: ${code}`));
});
function request(method: string, params: Record<string, unknown>): Promise<Event[]> {
    const id = ++sequence;
    return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error(`Content request timed out: ${method}`)), 15_000);
        pending.set(id, { events: [], resolve: events => { clearTimeout(timer); resolve(events); },
            reject: error => { clearTimeout(timer); reject(error); } });
        child.stdin.write(JSON.stringify({ id, method, params }) + "\n");
    });
}
function data<T>(events: Event[], name: string): T {
    assert(!events.some(event => event.event === "error"), JSON.stringify(events));
    assert.equal(events.filter(event => event.event === "complete").length, 1);
    const result = events.findLast(event => event.event === name);
    assert(result, `Missing ${name} event`);
    return result.data as T;
}
async function share(params: Record<string, unknown>) {
    return data<{ url: string }>(await request("share", { mode: "work", ...params }), "share").url;
}

try {
    const link = await share({ work, variant: selected });
    const decoded = openLink(link);
    assert.equal(decoded.mode, "work");
    assert.equal(decoded.work.variants.length, 2, "Episode/voice variants must collapse to publication references");
    assert(decoded.work.variants.every(variant => variant.needsInspection));
    assert.equal(decoded.variant?.id, selected.id);
    assert.equal(decoded.variant?.season, "2");
    assert(!JSON.stringify(decoded).includes(secret));
    assert(!JSON.stringify(decoded).includes("headers") && !JSON.stringify(decoded).includes("downloadUrl"));
    const opened = data<Work & { preferredVariant: { id: string; sourceId: string } }>(
        await request("openLink", { url: link }), "details");
    assert.equal(opened.id, work.id);
    assert.equal(opened.title, title);
    assert.deepEqual(opened.preferredVariant, { id: selected.id, sourceId: selected.sourceId });
    assert.equal(opened.variants.filter(variant => variant.sourceId === "alpha").length, 4);
    assert(opened.variants.some(variant => variant.sourceId === "beta" && variant.needsInspection));
    assert.deepEqual(inspected, ["alpha"], "Opening a preferred edition must leave other sources lazy");
    const expanded = data<Work>(await request("inspectWork", { work: opened }), "details");
    assert.equal(expanded.variants.length, 8);
    assert(expanded.variants.some(variant => variant.id === selected.id));
    assert.deepEqual(inspected, ["alpha", "beta"]);

    const noSelection = await share({ work });
    const before = inspected.length;
    const unselected = data<Work & { preferredVariant?: unknown }>(await request("openLink", { url: noSelection }), "details");
    assert.equal(unselected.variants.length, 2);
    assert.equal(unselected.preferredVariant, undefined);
    assert.equal(inspected.length, before, "Opening without a preferred version must not fetch a source eagerly");
    assert.equal(data<Work>(await request("inspectWork", { work: unselected }), "details").variants.length, 8);

    const missing = await share({ work, variant: { ...selected, id: "alpha:removed" } });
    const rejected = await request("openLink", { url: missing });
    assert.equal(rejected.at(-1)?.event, "error");
    assert(!rejected.some(event => event.event === "details"), "Missing preferred editions must not select an alternative");
    const changed = await share({ work, variant: { ...selected, season: "1" } });
    assert.equal((await request("openLink", { url: changed })).at(-1)?.event, "error");
    const exact = data<{ url: string }>(await request("share", { mode: "exact", work, variant: selected }), "share");
    assert.equal(openLink(exact.url).mode, "exact");
    assert.equal(data<Work>(await request("openLink", { url: exact.url }), "details").variants[0].id, selected.id);
    const capturePayload = { v: 1, action: "watch", pageUrl: "https://example.com/capture", title,
        captureId: "12345678-1234-1234-1234-123456789abc" };
    const captureLink = `qbutt://capture?d=${Buffer.from(JSON.stringify(capturePayload)).toString("base64url")}`;
    const capture = data<Work & { captureAction: string }>(await request("openLink", { url: captureLink }), "details");
    assert.equal(capture.captureAction, "watch");
    assert.equal((await request("share", { mode: "work", work: capture, variant: capture.variants[0] })).at(-1)?.event, "error");

    const repeated = { ...work, variants: Array.from({ length: 3000 }, (_, index) =>
        ({ ...selected, id: `alpha:episode:${index}`, episode: String(index) })) };
    const compact = await share({ work: repeated, variant: repeated.variants[2999] });
    assert(compact.length < 4096, "Large episode lists must not produce giant URLs");
    const oversized = { ...work, variants: Array.from({ length: 200 }, (_, index) =>
        ({ ...selected, id: `alpha:${index}`, url: `https://example.com/publication/${index}` })) };
    assert.equal((await request("share", { mode: "work", work: oversized })).at(-1)?.event, "error");
    const unsafe = { ...selected, sourceId: "link", url: `https://example.com/media.mp4?token=${secret}` };
    assert.equal((await request("share", { mode: "work", work: { ...work, variants: [unsafe] } })).at(-1)?.event, "error");
    const wal = await readFile(join(directory, "profile/library.sqlite-wal")).catch(() => Buffer.alloc(0));
    assert(!wal.includes(Buffer.from(secret)));
    console.log(JSON.stringify({ ok: true, wholeWorkRoundtrip: true, preferredExact: true, seasonIdentity: true,
        allKnownSources: true, lazyOtherSources: true, noSelection: true, missingPreferredRejected: true, legacyExactAndCapture: true,
        compactEpisodes: true, oversizedRejected: true, noAcquisitionSecrets: true }));
}
finally {
    child.stdin.end();
    const timeout = setTimeout(() => child.kill(), 5000);
    assert.equal(await exited, 0);
    clearTimeout(timeout);
    server.stop(true);
    await rm(directory, { recursive: true, force: true });
}
