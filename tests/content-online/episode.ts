import assert from "node:assert/strict";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Event, ResolvedContent, Variant, WorkDetails } from "../../components/content/contracts.ts";
import { share } from "../../components/content/sharing.ts";

const [executable, appDir, mediaRuntime, player = "all"] = Bun.argv.slice(2);
if (!executable || !appDir || !mediaRuntime || !["all", "CVH", "Kodik"].includes(player)) {
    throw new Error("Usage: bun tests/content-online/episode.ts <helper.exe> <portable-directory> <lab-media-directory> [all|CVH|Kodik]");
}

const lab = await mkdtemp(join(tmpdir(), "qbutt-anime-episode-"));
const helper = Bun.spawn([executable, "--data-dir", join(lab, "profile"), "--app-dir", appDir], {
    windowsHide: true,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "ignore",
    env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: join(lab, "browser.json") },
});
const pending = new Map<number, { events: Event[]; finish(events: Event[]): void; fail(error: Error): void }>();
let nextId = 0;
const read = (async () => {
    let buffered = "";
    const decoder = new TextDecoder();
    for await (const chunk of helper.stdout) {
        buffered += decoder.decode(chunk, { stream: true });
        let end: number;
        while ((end = buffered.indexOf("\n")) !== -1) {
            const event = JSON.parse(buffered.slice(0, end)) as Event;
            buffered = buffered.slice(end + 1);
            const current = pending.get(event.id);
            if (!current) continue;
            current.events.push(event);
            if (event.event === "complete" || event.event === "error") {
                pending.delete(event.id);
                current.finish(current.events);
            }
        }
    }
    for (const current of pending.values()) current.fail(new Error("Helper exited before replying."));
    pending.clear();
})();

function request(method: string, params: Record<string, unknown> = {}): Promise<Event[]> {
    const id = ++nextId;
    return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
            pending.delete(id);
            reject(new Error(`Helper request timed out: ${method}`));
        }, 75_000);
        pending.set(id, {
            events: [],
            finish(events) { clearTimeout(timeout); resolve(events); },
            fail(error) { clearTimeout(timeout); reject(error); },
        });
        helper.stdin.write(JSON.stringify({ id, method, params }) + "\n");
        helper.stdin.flush();
    });
}

function data<T>(events: Event[], event: string): T {
    const error = events.find(item => item.event === "error");
    assert(!error, `The actual helper returned a source error: ${JSON.stringify(error?.data)}`);
    const result = events.find(item => item.event === event);
    assert(result, `The actual helper omitted ${event}.`);
    return result.data as T;
}

async function decode(resolved: ResolvedContent, label: string) {
    assert(resolved.url && resolved.proxyUrl, "The helper omitted the guarded media connection.");
    const headers = Object.entries(resolved.headers ?? {}).map(([name, value]) => `${name}: ${value}\r\n`).join("");
    const child = Bun.spawn([join(mediaRuntime, "tools", "ffmpeg.exe"),
        "-hide_banner", "-loglevel", "error", "-nostats", "-threads", "2",
        "-http_proxy", resolved.proxyUrl, "-rw_timeout", "10000000", "-headers", headers,
        "-protocol_whitelist", "http,https,httpproxy,tcp,tls,crypto", "-i", resolved.url,
        "-t", "2", "-progress", "pipe:1", "-f", "null", "-"], {
        windowsHide: true, stdout: "pipe", stderr: "pipe",
    });
    const timeout = setTimeout(() => child.kill(), 30_000);
    const [progress, error, exitCode] = await Promise.all([
        new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
    ]);
    clearTimeout(timeout);
    const frames = Math.max(0, ...[...progress.matchAll(/^frame=(\d+)$/gm)].map(match => Number(match[1])));
    await Bun.write(join(lab, `decode-${label.replace(/[^\w-]/g, "-")}.json`), JSON.stringify({
        label, exitCode, frames, progress, error: error.replace(/https?:\/\/\S+/g, "[source]"),
    }, null, 2));
    assert.equal(exitCode, 0, `${label}: decode failed; diagnostics in ${lab}`);
    assert(frames > 0, `${label}: no decoded video frames; diagnostics in ${lab}`);
    assert.equal(error.trim(), "", `${label}: the decoder reported damaged or missing media; diagnostics in ${lab}`);
    return { exitCode, frames, secondsLimit: 2 };
}

try {
    const search = await request("search", {
        query: "Даже у студсовета есть косяки!", category: "anime", sources: ["yani"],
    });
    const work = search.filter(event => event.event === "result")
        .map(event => event.data as WorkDetails).find(work => work.id === "yani:20398");
    if (!work) {
        await Bun.write(join(lab, "search-error.json"), JSON.stringify(search, null, 2));
        assert.fail(`The title was not discoverable through the actual helper search; diagnostics in ${lab}`);
    }
    const details = data<WorkDetails>(await request("inspect", { work, variant: work.variants[0] }), "details");
    const pasted = data<WorkDetails>(await request("search", {
        query: "https://ru.yummyani.me/catalog/item/dazhe-u-studsoveta-est-kosyaki", category: "anime",
    }), "result");
    assert.equal(pasted.id, details.id, "The supplied YummyAnime page did not open its built-in title.");
    assert.deepEqual(new Set(pasted.variants.map(item => item.id)), new Set(details.variants.map(item => item.id)));
    const choices = [
        ["FumoDub", "Kodik"], ["OnWave", "Kodik"], ["Flowers Media", "Kodik"],
        ["AnimeVost", "CVH"], ["AniStar", "Kodik"],
        ["Flowers Media", "CVH"], ["Дубляж OnWave", "CVH"],
    ];
    const selected: Variant[] = choices.map(([translation, player]) => {
        const variant = details.variants.find(item => item.episode === "2"
            && item.translation === translation && item.player === player);
        assert(variant, `Episode 2 / ${translation} / ${player} was not discoverable.`);
        return variant;
    }).filter(variant => player === "all" || variant.player === player);
    const evidence = [];
    for (const variant of selected) {
        const shared = data<{ url: string }>(await request("share", { work: details, variant }), "share");
        const opened = data<WorkDetails & { preferredVariantId: string }>(
            await request("openLink", { url: shared.url }), "details");
        assert.equal(opened.preferredVariantId, variant.id);
        assert.equal(opened.variants[0].episode, "2");
        assert.equal(opened.variants[0].translation, variant.translation);
        assert.equal(opened.variants[0].player, variant.player);
        const resolved = data<ResolvedContent>(await request("resolve", { variant: opened.variants[0] }), "resolved");
        assert.equal(resolved.kind, "video");
        assert.equal(resolved.resolver, "direct");
        assert.equal(resolved.translation, variant.translation);
        assert.equal(resolved.selection?.episode, "2");
        assert.equal(resolved.selection?.player, variant.player);
        evidence.push({ id: variant.id, episode: variant.episode, translation: variant.translation,
            player: variant.player, exactShareRoundTrip: true, ...await decode(resolved, variant.id) });
    }

    for (const changes of [{ episode: "9999" }, { translation: "Another studio" }, { player: "Another player" }]) {
        const changed = { ...selected[0], ...changes };
        const stale = share({ mode: "exact", work: details, variant: changed });
        const events = await request("openLink", { url: stale });
        assert(events.some(event => event.event === "error"), "A stale exact shared choice was silently substituted.");
        assert(!events.some(event => event.event === "details"), "A stale choice produced substitute variants.");
    }

    let film;
    if (player !== "Kodik") {
        const details = data<WorkDetails>(await request("inspect", { variant: {
            id: "yani:1463", sourceId: "yani", sourceName: "YummyAnime", title: "Твоё имя",
            url: "https://yani.tv/catalog/item/tvoe-imya", transport: "video",
        } }), "details");
        const variant = details.variants.find(item => item.player === "CVH" && item.translation === "SHIZA Project");
        assert(variant, "The live CVH film publication is unavailable.");
        const resolved = data<ResolvedContent>(await request("resolve", { variant }), "resolved");
        assert.equal(resolved.translation, variant.translation);
        assert.equal(resolved.selection?.episode, "1");
        film = { title: details.title, translation: variant.translation, ...await decode(resolved, variant.id) };
    }
    const history = data<unknown[]>(await request("library", { mode: "history" }), "library");
    assert.equal(history.length, 0, "Resolution alone must not create playback history.");
    const result = { status: "passed", helperSha256: new Bun.CryptoHasher("sha256")
        .update(await readFile(executable)).digest("hex"), title: details.title,
        titleDiscovered: true, pastedCatalogUrlOpened: true, fiveRequestedVoicesDiscovered: true, variants: evidence,
        film,
        staleSharedEpisodeVoicePlayerRejected: true, resolveDoesNotCreateHistory: true };
    await Bun.write(join(lab, "evidence.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify({ ...result, evidence: join(lab, "evidence.json") }));
}
finally {
    helper.stdin.end();
    const timeout = setTimeout(() => helper.kill(), 5000);
    const exitCode = await helper.exited;
    clearTimeout(timeout);
    await read;
    assert.equal(exitCode, 0, "The helper failed to drain and exit after EOF.");
}
