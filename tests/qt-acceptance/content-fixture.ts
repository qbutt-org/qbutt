import { appendFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import type { Provider, ProviderContext, Request, Variant, Work } from "../../components/content/contracts";
import { createGameProvider } from "../../components/content/providers/games/catalog";
import { openLink } from "../../components/content/sharing";
import { mergeWorkMetadata } from "../../components/content/results";

const logPath = process.env.QBUTT_CONTENT_UI_FIXTURE_LOG!;
const tasks = new Set<Promise<void>>();
const inspections = new Map<string, number>();
const library = new Map<string, Record<string, unknown>>();
let runtimeRequests = 0;
let heldRuntime: { id: number; release: () => void } | undefined;
let continueRanking: (() => void) | undefined;
let continueEpisodes: (() => void) | undefined;
let browserSetupOpened = false;
let browserSetupChecks = 0;

function log(event: string, sourceId = "", data: Record<string, unknown> = {}): void {
    appendFileSync(logPath, `${JSON.stringify({ event, sourceId, time: Date.now(), ...data })}\n`);
}

function emit(id: number, event: string, data?: unknown): void {
    process.stdout.write(`${JSON.stringify({ id, event, data })}\n`);
}

function gameProvider(letter: string): Provider {
    return createGameProvider({ id: `fixture-${letter}`, name: `Source ${letter.toUpperCase()}`,
        url: `https://fixture-${letter}.example`, searchTitle: "a.search", content: ".full-story" });
}

function context(letter: string): ProviderContext {
    return {
        dataDir: "", signal: new AbortController().signal, settings: {},
        async fetch(url) {
            if (new URL(url).pathname === "/index.php") {
                return new Response(`<a class="search" href="/game">Original Cooperative Game v1.2</a>`);
            }
            return new Response(`
                <article class="full-story"><h1>Original Cooperative Game</h1>
                    <p>Install the game, then apply the cooperative patch.</p>
                    <p><a href="https://files.example/${letter}-full.torrent">Full game v1.2</a></p>
                    <p><a href="https://gofile.io/d/${letter}-patch">Patch v1.3</a></p>
                </article>
                <div id="comment-id-1"><b class="comm-author">Author ${letter.toUpperCase()}</b>
                    <div id="comm-id-1">Source comment &lt;literal&gt; &amp; provenance</div>
                </div>`);
        },
    };
}

const providers = new Map(["a", "b", "c"].map(letter => [
    `fixture-${letter}`, { provider: gameProvider(letter), context: context(letter) },
]));

const downloads = {
    provider: createGameProvider({ id: "onlinefix", name: "Online-Fix", url: "https://online-fix.me",
        searchTitle: "a.search", content: ".full-story" }),
    context: {
        dataDir: "", signal: new AbortController().signal, settings: {},
        async fetch(url: string) {
            if (new URL(url).pathname === "/index.php")
                return new Response(`<a class="search" href="/games/original">Original Game v0.4.4.3 Co-op</a>`);
            return new Response(`<article class="full-story"><h1 class="entry-title">Original Game v0.4.4.3 Co-op</h1>
                <div itemprop="articleBody">Original generated game description.</div>
                <p><a href="https://uploads.online-fix.me:2053/game/torrents/Original.v0.4.4.3-OFME.torrent">Original.v0.4.4.3-OFME.torrent</a></p>
                <p><a href="https://uploads.online-fix.me:2053/game/uploads/Original.v0.4.4.3-OFME.rar">Original.v0.4.4.3-OFME.rar</a></p>
                <p><a href="https://hosters.online-fix.me/game">Скачать с Online-Fix Hosters</a></p>
                <p><a href="https://drive.online-fix.me/game">Скачать с Online-Fix Drive</a></p>
                <p><a href="https://uploads.online-fix.me:2053/game/Fix%20Repair/Original_Fix.rar">Фикс: Original_Fix.rar</a></p>
            </article>`);
        },
    } satisfies ProviderContext,
};

function video(id: string, adStatus: Variant["adStatus"], sourceId = "ads"): Variant {
    return { id, sourceId, sourceName: "Fixture video", title: id,
        url: `https://video.example/${id}`, transport: "video", adStatus };
}

function videoWork(query: string): Work {
    const variants: Variant[] = query === "table-metadata"
        ? [{ ...video("metadata", "unknown"), transport: "torrent", size: 1024, seeders: 0 }]
        : query === "media-start" ? [video("accepted", "clean", "media")]
        : query === "media-reject" ? [video("rejected", "clean", "media")]
        : query === "ads-only" ? [video("reported", "reported")]
        : query === "ads-discovered" ? [video("discovered", "unknown", "ads-discovered")]
        : [video("reported", "reported"), video("clean", "clean"), video("unknown", "unknown")];
    return { id: query, title: query, category: "movies", variants };
}

function episodeWork(expanded: boolean, late = false): Work {
    const variants: Variant[] = [1, 2].map(number => ({
        ...video(number === 2 ? "collision" : "episode-first", "clean", "fixture-series"),
        sourceName: "Original episodes", episode: String(number), translation: "Studio B",
    }));
    for (let index = 0; index < 74; index++) {
        variants.push({ ...video(`torrent-${index}`, "unknown", "fixture-torrents"),
            sourceName: "Original Torrents", transport: "torrent", size: 1024 + index, seeders: index,
            ...(index === 0 ? { sourceId: "fixture-hybrid", sourceName: "Original hybrid", player: "Shared player" } : {}),
            ...(index === 1 ? { season: "2", episode: "2" } : index < 20 ? { episode: "1–12" } : {}),
        });
    }
    for (let index = 0; index < 5; index++) {
        variants.push({ ...video(`upload-${index}`, "unknown", "fixture-social"),
            sourceName: "Original uploads", ...(index < 3 ? { episode: String(index + 1) } : index === 3 ? { episode: "10" } : {}),
            ...(index === 1 ? { sourceId: "fixture-hybrid", sourceName: "Original hybrid",
                player: "Shared player", translation: "Studio B" } : {}),
        });
    }
    if (expanded) {
        for (const episode of ["1", "2"]) {
            for (const translation of ["Studio A", "Studio B", "Studio C", "Studio D", "Studio E", ""]) {
                for (const player of ["CVH", "Kodik", "Alloha", "Original player"]) {
                    const id = episode === "2" && translation === "Studio B" && player === "CVH"
                        ? "collision" : `yani-${episode}-${translation}-${player}`;
                    variants.push({ ...video(id, "unknown", "fixture-yani"), sourceName: "YummyAnime",
                        url: "https://yani.example/original", episode, translation, player });
                }
            }
        }
    } else {
        variants.push({ ...video("yani-publication", "unknown", "fixture-yani"), sourceName: "YummyAnime",
            url: "https://yani.example/original", needsInspection: true });
    }
    if (late) variants.push({ ...video("late-episode", "unknown", "fixture-late"),
        sourceName: "Later publication", episode: "2", translation: "Studio B" });
    return { id: "original-series", title: "Original generated series with five named studios", category: "anime", variants };
}

async function handle(request: Request): Promise<void> {
    const { id, method, params = {} } = request;
    if (method === "sources") {
        emit(id, "sources", [
            { id: "browser", name: "Browser", categories: [], settings: [
                { name: "browser", label: "Browser", type: "select", value: "edge",
                    options: [{ label: "Microsoft Edge", value: "edge" }] },
            ] },
            { id: "ai", name: "Translation", categories: [], settings: [] },
        ]);
    } else if (method === "connectBrowser" || method === "browserStatus") {
        if (method === "connectBrowser") browserSetupOpened = true;
        else if (browserSetupOpened) browserSetupChecks++;
        const ready = browserSetupChecks >= 2;
        emit(id, "browser", {
            ready,
            ...(ready ? { connectedBrowser: "edge" } : {}),
            browsers: [{ id: "edge", name: "Microsoft Edge", connected: ready }],
            ...(method === "connectBrowser" ? {
                browser: "edge", extensionDirectory: "C:\\Original qbutt\\browser-extension\\chromium",
            } : {}),
        });
        log(method, "edge", { params, ready });
    } else if (method === "search") {
        const query = String(params.query);
        if (query === "ranking") {
            const selected: Work = { id: "rank-selected", title: "Original film behind the scenes", category: "movies",
                variants: [video("rank-selected", "clean", "rank")] };
            await new Promise<void>(release => {
                continueRanking = release;
                emit(id, "result", { ...selected, relevance: 0 });
            });
            for (const [name, relevance] of [["exact", 2], ["equal", 2], ["alias", 1]] as const)
                emit(id, "result", { id: `rank-${name}`, title: name, category: "movies",
                    variants: [video(`rank-${name}`, "clean", "rank")], relevance });
            emit(id, "result", { ...selected, relevance: 2 });
            emit(id, "result", { id: "rank-unknown", title: "Other film result", category: "movies",
                variants: [video("rank-unknown", "clean", "rank")] });
            emit(id, "result", { ...selected, relevance: 2,
                notes: [{ text: "Latest ranked result", url: selected.variants[0]!.url }] });
        } else if (query === "episodes") {
            const work = episodeWork(false);
            await new Promise<void>(release => {
                continueEpisodes = release;
                emit(id, "result", work);
            });
            await Bun.sleep(100);
            emit(id, "result", episodeWork(false, true));
            log("streamed", "fixture-late");
            await Bun.sleep(700);
            emit(id, "result", episodeWork(false, true));
        } else if (query === "promoted") {
            const original = videoWork(query);
            emit(id, "result", original);
            await Bun.sleep(350);
            emit(id, "removeResult", { id: original.id });
            emit(id, "result", { ...original, id: "canonical-promoted" });
            await Bun.sleep(500);
        } else if (query === "reader-images" || query === "reader-text") {
            const image = query === "reader-images";
            emit(id, "result", { id: query, title: image ? "Original online comic" : "Original online book",
                category: image ? "manga" : "books", variants: [{ id: `${query}:variant`, sourceId: "fixture-reader",
                    sourceName: "Original Reader source", title: query, url: `https://reader.example/${query}`,
                    transport: image ? "manga" : "book" }] } satisfies Work);
        } else if (query === "game-downloads") {
            emit(id, "result", (await downloads.provider.search({ query, category: "games" }, downloads.context))[0]);
        } else if (query === "games-recovery") {
            const works = await Promise.all([...providers.values()].slice(0, 2).map(async entry =>
                (await entry.provider.search({ query, category: "games" }, entry.context))[0]!));
            emit(id, "result", { ...works[0]!, id: "fixture-game-recovery", variants: works.flatMap(work => work.variants) });
        } else if (query !== "games") {
            emit(id, "result", videoWork(query));
        } else {
            const works = await Promise.all([...providers.values()].map(async entry =>
                (await entry.provider.search({ query, category: "games" }, entry.context))[0]!));
            const work: Work = { ...works[0]!, id: "fixture-game", variants: works.slice(0, 2).flatMap(item => item.variants) };
            emit(id, "result", work);
            await Bun.sleep(150);
            emit(id, "result", { ...work, variants: works.flatMap(item => item.variants) });
            log("streamed", "fixture-c");
            await Bun.sleep(750);
        }
    } else if (method === "inspect") {
        const variant = params.variant as Variant;
        const work = params.work as Work;
        log("inspect", variant.sourceId, { id, variant });
        if ((variant.id === "rank-selected") && continueRanking) {
            continueRanking();
            continueRanking = undefined;
        }
        if ((work.id === "fixture-game-recovery") && (variant.sourceId === "fixture-a")) {
            emit(id, "error", { message: "Source A requires browser verification (HTTP 403)." });
            log("inspectionError", variant.sourceId);
            return;
        }
        if (variant.sourceId === "fixture-yani" && variant.needsInspection) {
            continueEpisodes?.();
            continueEpisodes = undefined;
            await Bun.sleep(300);
            emit(id, "details", episodeWork(true));
            log("details", variant.sourceId);
            emit(id, "complete");
            return;
        }
        const entry = providers.get(variant.sourceId) ?? (variant.sourceId === "onlinefix" ? downloads : undefined);
        if (entry) {
            const count = (inspections.get(variant.sourceId) ?? 0) + 1;
            inspections.set(variant.sourceId, count);
            const details = await entry.provider.inspect!(variant, entry.context);
            // Delayed replies model an already accepted provider response whose
            // publication is no longer selected. No product network is bypassed.
            await Bun.sleep(variant.sourceId === "fixture-b" ? 600 : count === 1 ? 350 : 50);
            emit(id, "details", { ...mergeWorkMetadata(work, details), id: work.id });
        } else {
            await Bun.sleep(50);
            emit(id, "details", variant.sourceId === "ads-discovered"
                ? { ...work, variants: [video("discovered", "reported", "ads-discovered")] } : work);
        }
        log("details", variant.sourceId);
    } else if (method === "openLink") {
        const item = openLink(String(params.url));
        const work = item.work.id === "original-series" ? episodeWork(true, true)
            : item.variant?.id === "reported" ? videoWork("ads-only")
            : { ...item.work, variants: [video("replacement", "clean")] };
        emit(id, "details", { ...work, preferredVariant: { id: item.variant?.id, sourceId: item.variant?.sourceId } });
        log("openLink", item.variant?.id);
    } else if (method === "reader.open") {
        const work = params.work as Work;
        emit(id, "complete", { id: `document:${work.id}`, workId: work.id, sourceId: "fixture-reader", title: work.title,
            chapters: [{ id: `${work.id}:chapter`, title: "Original legal chapter" }] });
        return;
    } else if (method === "reader.chapter") {
        const documentId = String(params.documentId);
        const chapterId = (params.chapter as { id: string }).id;
        const images = documentId.endsWith("reader-images");
        emit(id, "complete", { id: chapterId, documentId, title: "Original legal chapter", cached: true,
            kind: images ? "images" : "text",
            ...(images ? { images: [0, 1, 2].map(index => ({ id: `original-${index}`,
                url: pathToFileURL(join(dirname(logPath), "reader-original.png")).href })) }
                : { html: "<p>Original legal text for reading progress.</p>".repeat(100) }) });
        return;
    } else if (method === "reader.position") {
        log("reader.position", String(params.workId), { params });
    } else if (method === "progress") {
        const { id: workId, ...progress } = params;
        log("progress", String(workId), { params });
        const entry = library.get(String(workId));
        if (entry) library.set(String(workId), { ...entry, progress });
    } else if (method === "mediaRuntime") {
        const runtime = process.env.QBUTT_CONTENT_UI_RUNTIME!;
        log("mediaRuntime", "", { id });
        if (++runtimeRequests === 1) {
            await new Promise<void>(release => {
                heldRuntime = { id, release };
            });
        }
        emit(id, "complete", { mpv: join(runtime, "mpv.exe"),
            ffprobe: join(runtime, "profile", "tools", "bin", "ffprobe.exe"), tools: join(runtime, "tools") });
        log("runtimeReady", "", { id });
        return;
    } else if (method === "library") {
        emit(id, "library", [...library.values()]);
    } else if (method === "remember") {
        const work = params.work as Work;
        log("remember", work?.id ?? "", { params });
        if (!work?.id) throw new Error("Select a title.");
        const entry = { work, variant: params.variant, favorite: false, lastOpened: new Date().toISOString() };
        library.set(work.id, entry);
        emit(id, "complete", entry);
        return;
    } else if (method === "resolve" && (params.variant as Variant).sourceId === "fixture-reader") {
        emit(id, "resolved", { kind: (params.variant as Variant).transport });
    } else if (method === "resolve" && (params.variant as Variant).sourceId === "media") {
        log("resolve", "media");
        emit(id, "resolved", { kind: "video", url: pathToFileURL(process.env.QBUTT_CONTENT_UI_MEDIA!).href,
            ...((params.variant as Variant).id === "rejected"
                ? { audioTrack: { title: "Unavailable Studio", type: "Declared voice", codec: "pcm_s16le", channels: 1 } } : {}) });
    } else if (method === "resolve" || method === "download") {
        log("resolve", String((params.variant as Variant)?.sourceId), { params });
        emit(id, "error", { message: "This selection fixture does not download payloads." });
        return;
    } else if (method === "cancel") {
        log("cancel", "", { id: params.id });
        if (heldRuntime && (heldRuntime.id === params.id)) {
            heldRuntime.release();
            heldRuntime = undefined;
        }
    } else if (method === "jobs") {
        emit(id, "jobs", []);
    } else {
        throw new Error(`Unexpected native fixture method: ${method}`);
    }
    emit(id, "complete");
}

let buffer = "";
const decoder = new TextDecoder();
for await (const chunk of Bun.stdin.stream()) {
    buffer += decoder.decode(chunk, { stream: true });
    let end: number;
    while ((end = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 1);
        const request = JSON.parse(line) as Request;
        const task = handle(request).catch(error => emit(request.id, "error", { message: String(error) }));
        tasks.add(task);
        void task.finally(() => tasks.delete(task));
    }
}
await Promise.all(tasks);
