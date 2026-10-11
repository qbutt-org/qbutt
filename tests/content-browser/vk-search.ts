import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";

import { BrowserBridge } from "../../components/content/browser/index.ts";
import { Catalog } from "../../components/content/catalog.ts";
import { LibraryStore } from "../../components/content/store.ts";
import { SourceSettings } from "../../components/content/settings.ts";
import { vkProvider } from "../../components/content/providers/video/social.ts";
import { FrameDecoder, encodeFrame } from "./framing.ts";
import type { BrowserCommand } from "../../components/content/browser/protocol.ts";

function argument(name: string): string {
    const index = process.argv.indexOf(name);
    const value = process.argv[index + 1];
    if (index < 0 || !value || value.startsWith("--")) throw new Error(`Missing ${name}.`);
    return resolve(value);
}

const qcatch = argument("--qcatch");
const directory = argument("--data-dir");
const brokerPath = argument("--broker");
const receiptPath = argument("--receipt");
const discovery = join(directory, "bridge.json");
const bridge = new BrowserBridge({ browser: "edge", discoveryPath: discovery, brokerPath });
const children: ReturnType<typeof spawn>[] = [];
const scenarios: string[] = [];
const query = "студсовет косяки";
const videos = [
    { id: "-900001_1001", url: "https://vkvideo.ru/video-900001_1001", title: "Даже у студсовета есть косяки! 1 серия [Озвучка A]", duration: 1455, author: "Fixture author A" },
    { id: "-900001_1002", url: "https://vkvideo.ru/video-900001_1002", title: "Даже у студсовета есть косяки! 2 серия [Озвучка B]", duration: 1460 },
    { id: "-900002_1003", url: "https://vkvideo.ru/video-900002_1003", title: "Студсовет косяки! Трейлер", duration: 90 },
    { id: "-900003_1004", url: "https://vkvideo.ru/video-900003_1004", title: "Нерелевантная комедия", duration: 1200 },
];

async function waitFor(predicate: () => boolean): Promise<void> {
    const deadline = Date.now() + 5_000;
    while (!predicate()) {
        if (Date.now() >= deadline) throw new Error("Integration deadline elapsed.");
        await Bun.sleep(25);
    }
}

function extension(capabilities?: string[]) {
    const child = spawn(qcatch, ["--native-messaging", "chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/"], {
        env: { ...process.env, QBUTT_BROWSER_BRIDGE_DISCOVERY: discovery }, windowsHide: true,
    });
    children.push(child);
    const decoder = new FrameDecoder();
    const messages: BrowserCommand[] = [];
    child.stdout.on("data", chunk => decoder.push(chunk, value => messages.push(value as BrowserCommand)));
    child.stderr.resume();
    const send = (value: unknown) => child.stdin.write(encodeFrame(value));
    send({ type: "hello", browser: "edge", version: 1, userAgent: "qcatch-vk-generated-integration/1", capabilities });
    return { child, messages, send };
}

const report: Record<string, unknown> = { recordedAt: new Date().toISOString(), scenarios, realBrowserDomClaimed: false,
    realVkResultClaimed: false, generatedPublicationsOnly: true, cookiesSaved: false };
let store: LibraryStore | undefined;
let catalog: Catalog | undefined;
try {
    await mkdir(directory);
    await bridge.start();
    await assert.rejects(bridge.searchVkVideos(query), (error: any) => error.code === "browser_required");
    scenarios.push("missing-extension-actionable");
    const legacy = extension();
    await waitFor(() => bridge.status().ready);
    await assert.rejects(bridge.searchVkVideos(query), (error: any) => error.code === "browser_update_required");
    assert.equal(legacy.messages.length, 0);
    scenarios.push("legacy-extension-capability-refused-before-request");
    legacy.child.stdin.end();
    await waitFor(() => !bridge.status().ready);
    const peer = extension(["vk-search"]);
    await waitFor(() => bridge.status().ready);
    store = await LibraryStore.open(join(directory, "library"));
    catalog = new Catalog(join(directory, "library"), dirname(brokerPath), new SourceSettings(store), bridge);
    const contextPromise = catalog.context("vkvideo", AbortSignal.timeout(15_000));
    await waitFor(() => peer.messages.length > 0);
    const cookies = peer.messages.shift()!;
    assert.equal(cookies.method, "cookies");
    peer.send({ id: cookies.id, result: [] });
    const context = await contextPromise;
    assert.equal(context.settings.accessToken, undefined);
    assert.equal(vkProvider.source.settings, undefined);
    const search = vkProvider.search({ query, category: "anime" }, context);
    await waitFor(() => peer.messages.length > 0);
    const command = peer.messages.shift()!;
    assert.equal(command.method, "vk-search");
    assert.equal(command.query, query);
    assert.equal(new URL(command.url!).searchParams.get("q"), query);
    peer.send({ id: command.id, result: { query, videos } });
    const works = await search;
    assert.equal(works.length, 2);
    assert.deepEqual(works.map(work => work.variants[0]!.id), ["vkvideo:-900001_1001", "vkvideo:-900001_1002"]);
    assert(works.every(work => work.category === "anime" && work.variants[0]!.sourceId === "vkvideo"));
    assert.equal(works[0]!.variants[0]!.author, "Fixture author A");
    scenarios.push("catalog-session-without-api-token", "unicode-native-search", "relevance-and-trailer-filter", "exact-publication-identity");
    const resolved = await vkProvider.resolve!(works[1]!.variants[0]!, context);
    assert.equal(resolved.url, videos[1]!.url);
    assert.equal(resolved.resolver, "yt-dlp");
    scenarios.push("exact-selected-direct-resolver");

    async function rejected(name: string, result: unknown) {
        const pending = bridge.searchVkVideos(query);
        const check = assert.rejects(pending);
        await waitFor(() => peer.messages.length > 0);
        peer.send({ id: peer.messages.shift()!.id, result });
        await check;
        scenarios.push(name);
    }
    await rejected("mismatched-query-rejected", { query: "other", videos });
    await rejected("mismatched-id-url-rejected", { query, videos: [{ ...videos[0], url: videos[1]!.url }] });
    await rejected("oversized-result-count-rejected", { query, videos: Array.from({ length: 101 }, () => videos[0]) });
    await rejected("private-thumbnail-rejected", { query, videos: [{ ...videos[0], cover: "http://127.0.0.1/secret" }] });

    const controller = new AbortController();
    const canceled = bridge.searchVkVideos(query, controller.signal);
    const cancelCheck = assert.rejects(canceled);
    await waitFor(() => peer.messages.length > 0);
    const canceledId = peer.messages.shift()!.id;
    controller.abort();
    await cancelCheck;
    await waitFor(() => peer.messages.length > 0);
    assert.deepEqual(peer.messages.shift(), { id: canceledId, method: "cancel" });
    scenarios.push("search-cancellation-relayed");
    report.ok = true;
}
catch (error) {
    report.ok = false;
    report.error = error instanceof Error ? error.message : "Integration failed.";
    process.exitCode = 1;
}
finally {
    catalog?.close();
    store?.close();
    for (const child of children) child.kill();
    await bridge.close();
    await mkdir(dirname(receiptPath), { recursive: true });
    await writeFile(receiptPath, JSON.stringify(report, null, 2) + "\n");
    console.log(JSON.stringify({ ok: report.ok, scenarios: scenarios.length, receiptPath }));
}
