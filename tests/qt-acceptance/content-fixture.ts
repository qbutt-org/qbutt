import { appendFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import type { Provider, ProviderContext, Request, Variant, Work } from "../../components/content/contracts";
import { createGameProvider } from "../../components/content/providers/games/catalog";
import { openLink, share } from "../../components/content/sharing";
import { mergeWorkMetadata } from "../../components/content/results";
import { LibraryStore, type VariantFilters } from "../../components/content/store";

const logPath = process.env.QBUTT_CONTENT_UI_FIXTURE_LOG!;
const tasks = new Set<Promise<void>>();
const inspections = new Map<string, number>();
const library = await LibraryStore.open(join(dirname(logPath), "library"));
const searchWorks = new Map<string, Work>();
const fixtureStages = new Map<string, () => void>();
let continueEpisodes: (() => void) | undefined;
let browserInstalled = false;
let browserReady = false;
let failNextConnect = false;
let interactiveChecks = 0;
let sourceFailed = true;
let sourceFailureCode = "browser_check_required";
let recoveryWork: Work | undefined;
let holdNextLibraryWork = "";

function log(event: string, sourceId = "", data: Record<string, unknown> = {}): void {
    appendFileSync(logPath, `${JSON.stringify({ event, sourceId, time: Date.now(), ...data })}\n`);
}

function emit(id: number, event: string, data?: unknown): void {
    if (event === "result") {
        const { relevance: _relevance, ...work } = data as Work & { relevance?: number };
        searchWorks.set(work.id, work);
    } else if (event === "removeResult") {
        searchWorks.delete((data as { id: string }).id);
    }
    process.stdout.write(`${JSON.stringify({ id, event, data })}\n`);
}

function stagedResult(id: number, query: string, data: unknown): Promise<void> {
    return new Promise<void>(resume => {
        fixtureStages.set(query, resume);
        emit(id, "result", data);
    });
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
        : query === "media-start" ? [
            { ...video("accepted", "clean", "media"), title: "Original series Episode 2 [Studio B]",
                episode: "2", translation: "Studio B", languages: ["ru"] },
            { ...video("accepted", "clean", "history-other"), episode: "2", translation: "Studio B", languages: ["ru"] },
            { ...video("alternate", "clean", "media"), episode: "1", translation: "Studio A", languages: ["en"] },
        ]
        : query === "media-reject" ? [video("rejected", "clean", "media")]
        : query === "ads-only" ? [video("reported", "reported")]
        : query === "ads-discovered" ? [{ ...video("discovered", "unknown", "ads-discovered"), needsInspection: true }]
        : [video("reported", "reported"), video("clean", "clean"), video("unknown", "unknown")];
    return { id: query, title: query === "media-start" ? "Original complete series" : query,
        category: query === "media-start" ? "anime" : "movies", variants };
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

function canonicalFacetsWork(): Work {
    const variants: (Variant & { facets: { episode: string; season: string; translation: string } })[] = [
        { ...video("onwave-direct", "clean", "fixture-yani"), sourceName: "YummyAnime",
            episode: "2", translation: "OnWave", languages: ["ru"], quality: "1080p",
            facets: { episode: "2", season: "1", translation: "OnWave" } },
        { ...video("onwave-torrent", "clean", "fixture-nyaa"), sourceName: "Nyaa.si", transport: "torrent",
            season: "1", episode: "02", translation: "Дубляж OnWave", languages: ["ru"], quality: "1080p", seeders: 5,
            facets: { episode: "2", season: "1", translation: "OnWave" } },
        { ...video("kazoku-sub", "clean", "fixture-yani"), sourceName: "YummyAnime",
            episode: "2", translation: "Субтитры Kazoku Project", languages: ["ja"], quality: "720p",
            facets: { episode: "2", season: "1", translation: "Kazoku Project" } },
        { ...video("kazoku-dub", "clean", "fixture-rutube"), sourceName: "Rutube",
            season: "1", episode: "2", translation: "Дубляж Kazoku Project", languages: ["ru"], quality: "1080p",
            facets: { episode: "2", season: "1", translation: "Kazoku Project" } },
        { ...video("english", "clean", "fixture-other"), sourceName: "Original long source name for minimum window width",
            episode: "10", translation: "Original long studio name for minimum window width", languages: ["en"], quality: "2160p",
            facets: { episode: "10", season: "1", translation: "Original long studio name for minimum window width" } },
    ];
    return { id: "canonical-facets", title: "Original normalized selection fixture", category: "anime", variants };
}

async function handle(request: Request): Promise<void> {
    const { id, method, params = {} } = request;
    if (method === "sources" || method === "checkSource") {
        if (method === "checkSource") {
            log(method, String(params.id), { params });
            sourceFailed = params.interactive === true && ++interactiveChecks === 1;
            if (!sourceFailed && recoveryWork && params.id === "fixture-error") {
                const publication = recoveryWork.variants.find(variant => variant.needsInspection)!;
                const recovered = { ...publication, needsInspection: false };
                emit(id, "details", { ...recoveryWork, variants: [recovered], inspectedPublication: publication });
                const unrelated = { ...recovered, id: "unrelated-clean", url: "https://recovery.example/other" };
                emit(id, "details", { ...recoveryWork, id: "unrelated-work", title: "Unrelated publication",
                    variants: [unrelated], inspectedPublication: unrelated });
            }
        }
        emit(id, "sources", [
            { id: "browser", name: "Browser", categories: [], settings: [
                { name: "browser", label: "Browser", type: "select", value: "edge",
                    options: [{ label: "Microsoft Edge", value: "edge" }] },
            ] },
            { id: "ai", name: "Translation", categories: [], settings: [] },
            { id: "fixture-error", name: "Original failed source", categories: ["games"], enabled: true,
                settings: [{ name: "address", label: "Original address", type: "text", value: "Original saved address" }],
                ...(sourceFailed ? { health: { status: "error", message: "Не удалось завершить проверку исходного источника.",
                    code: sourceFailureCode, url: "https://recovery.example/original",
                    checkedAt: "2026-10-11T00:00:00.000Z" } } : {}) },
        ]);
    } else if (method === "localizationFailure") {
        sourceFailed = true;
        sourceFailureCode = String(params.code);
        const failure = { status: "error", message: "Не удалось завершить проверку исходного источника.",
            code: sourceFailureCode, url: "https://recovery.example/original", checkedAt: "2026-10-11T00:00:00.000Z" };
        emit(id, "job", failure);
        emit(id, "jobs", [failure]);
        emit(id, "error", failure);
        return;
    } else if (method === "connectBrowser" || method === "browserStatus") {
        log(method, "edge", { params, ready: browserReady });
        if (method === "connectBrowser") {
            if (failNextConnect) {
                failNextConnect = false;
                emit(id, "error", { message: "Original setup failure. Try again." });
                return;
            }
            browserInstalled = true;
        }
        emit(id, "browser", {
            selected: "edge", detected: "edge", qcatchInstalled: browserInstalled, ready: browserReady,
            ...(browserReady ? { connectedBrowser: "edge" } : {}),
            browsers: [{ id: "edge", name: "Microsoft Edge", connected: browserReady }],
            ...(method === "connectBrowser" ? {
                browser: "edge", extensionDirectory: "C:\\Original qcatch\\extension",
            } : {}),
        });
    } else if (method === "setBrowserFixture") {
        if (typeof params.installed === "boolean") browserInstalled = params.installed;
        if (typeof params.ready === "boolean") browserReady = params.ready;
        if (typeof params.failNextConnect === "boolean") failNextConnect = params.failNextConnect;
        log(method, "edge", { params });
    } else if (method === "advanceFixture") {
        const query = String(params.query);
        const resume = fixtureStages.get(query);
        if (!resume) throw new Error(`No pending fixture stage: ${query}`);
        fixtureStages.delete(query);
        resume();
    } else if (method === "search") {
        const query = String(params.query);
        searchWorks.clear();
        log("search", query, { params });
        if (query === "categories") {
            for (const category of ["movies", "anime", "games"] as const) {
                emit(id, "result", { id: `category-${category}`, title: category, category,
                    variants: [video(`category-${category}`, "clean")] });
                await Bun.sleep(350);
            }
            emit(id, "source", { sourceId: "fixture-error", status: "error", message: "Hidden background failure" });
        } else if (query.startsWith("heading-")) {
            const titles: Record<string, string> = {
                "heading-frieren": "Фрирен, провожающая в последний путь",
                "heading-ranobe": "Реинкарнация Безработного (ЛН)",
                "heading-long": "Очень длинное название произведения ".repeat(8),
            };
            emit(id, "result", { id: query, title: titles[query]!, category: "anime",
                variants: [video(query, "clean")] });
        } else if (query === "ranking") {
            const selected: Work = { id: "rank-selected", title: "Original film behind the scenes", category: "movies",
                variants: [{ ...video("rank-selected", "clean", "rank"), needsInspection: true }] };
            await stagedResult(id, query, { ...selected, relevance: 0 });
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
        } else if (query === "canonical-facets") {
            emit(id, "result", canonicalFacetsWork());
        } else if (query === "facet-refinement") {
            const work = canonicalFacetsWork();
            const variant = work.variants[0]!;
            await stagedResult(id, query, { ...work, variants: [{ ...variant,
                facets: { episode: "2", season: "", translation: "OnWave" } }] });
            emit(id, "result", { ...work, variants: [{ ...variant,
                facets: { episode: "2", season: "2", translation: "OnWave" } }] });
        } else if (query === "archive-parts") {
            const publication: Variant = { ...video("full-one", "clean", "fixture-parts"),
                url: "https://parts.example/release", transport: "torrent", publicationKind: "full", version: "1.2.3" };
            emit(id, "result", { id: query, title: "Original complete releases and archive parts", category: "games", variants: [
                { ...publication, magnet: `magnet:?xt=urn:btih:${"0".repeat(40)}` },
                { ...publication, id: "part-one", transport: "file", publicationKind: "archivePart" },
                { ...publication, id: "part-two", transport: "file", publicationKind: "archivePart", version: "2.0" },
                { ...publication, id: "part-unknown", transport: "file", publicationKind: "archivePart", version: undefined },
                { ...publication, id: "part-other", sourceId: "fixture-other", transport: "file", publicationKind: "archivePart" },
                { ...publication, id: "full-three", version: "3.0" },
                { ...publication, id: "part-three", transport: "file", publicationKind: "archivePart", version: "3.0" },
            ] } satisfies Work);
        } else if (query === "promoted") {
            const original = videoWork(query);
            await stagedResult(id, query, original);
            emit(id, "result", { ...original, id: "canonical-promoted", variants: [...original.variants,
                { ...video("late-clean", "clean", "promotion-pending"), needsInspection: true }] });
            emit(id, "removeResult", { id: original.id, replacementId: "canonical-promoted" });
            await Bun.sleep(500);
            log("promotionSearchComplete", "promotion-pending");
        } else if (query === "reader-images" || query === "reader-text") {
            const image = query === "reader-images";
            emit(id, "result", { id: query, title: image ? "Original online comic" : "Original online book",
                category: image ? "manga" : "books", variants: [{ id: `${query}:variant`, sourceId: "fixture-reader",
                    sourceName: "Original Reader source", title: query, url: `https://reader.example/${query}`,
                    transport: image ? "manga" : "book" }] } satisfies Work);
        } else if (query === "game-downloads") {
            emit(id, "result", (await downloads.provider.search({ query, category: "games" }, downloads.context))[0]);
        } else if (query === "source-recovery") {
            sourceFailed = true;
            recoveryWork = { id: query, title: "Original source recovery", category: "movies", variants: [
                video("ready-clean", "clean", "recovery-ready"),
                { ...video("recover-clean", "clean", "fixture-error"),
                    url: "https://recovery.example/original", needsInspection: true },
            ] };
            emit(id, "result", recoveryWork);
        } else if (query === "games-recovery") {
            const works = await Promise.all([...providers.values()].slice(0, 2).map(async entry =>
                (await entry.provider.search({ query, category: "games" }, entry.context))[0]!));
            emit(id, "result", { ...works[0]!, id: "fixture-game-recovery", variants: works.flatMap(work => work.variants) });
        } else if (query === "browser-prerequisite") {
            emit(id, "result", { id: query, title: "Original browser selection", category: "anime",
                variants: [1, 2].map(episode => ({ ...video(`episode-${episode}`, "clean", "fixture-browser"),
                    sourceName: "VK Video", episode: String(episode), translation: episode === 1 ? "Studio A" : "Studio B",
                    languages: ["ru"], player: "Original player" })) } satisfies Work);
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
    } else if (method === "inspectWork") {
        const work = params.work as Work;
        let latest = work;
        await Promise.all(work.variants.filter(variant => variant.needsInspection).map(async variant => {
            log("inspect", variant.sourceId, { id, variant });
            if (work.id === "source-recovery" && variant.sourceId === "fixture-error") {
                emit(id, "source", { sourceId: variant.sourceId, status: "error", message: "Original HTTP 403." });
                log("inspectionError", variant.sourceId);
                return;
            }
            if (work.id === "fixture-game-recovery" && variant.sourceId === "fixture-a") {
                emit(id, "source", { sourceId: variant.sourceId, status: "error",
                    message: "Source A requires browser verification (HTTP 403)." });
                log("inspectionError", variant.sourceId);
                return;
            }
            let details: Work;
            if (variant.sourceId === "fixture-yani") {
                continueEpisodes?.();
                continueEpisodes = undefined;
                await Bun.sleep(300);
                details = episodeWork(true);
            } else {
                const entry = providers.get(variant.sourceId) ?? (variant.sourceId === "onlinefix" ? downloads : undefined);
                if (entry) {
                    const count = (inspections.get(variant.sourceId) ?? 0) + 1;
                    inspections.set(variant.sourceId, count);
                    details = await entry.provider.inspect!(variant, entry.context);
                    await Bun.sleep(variant.sourceId === "fixture-b" ? 600 : count === 1 ? 350 : 50);
                } else {
                    await Bun.sleep(variant.sourceId === "promotion-pending" ? 650 : 50);
                    const inspected = variant.sourceId === "ads-discovered"
                        ? video("discovered", "reported", "ads-discovered") : { ...variant, needsInspection: false };
                    details = { ...work, variants: work.variants.map(candidate => candidate.sourceId === variant.sourceId
                        && candidate.id === variant.id ? inspected : candidate) };
                    if (work.id === "rank-selected")
                        details.year = 1984;
                }
            }
            latest = mergeWorkMetadata(latest, details);
            const searched = searchWorks.get(work.id);
            if (searched)
                latest = { ...mergeWorkMetadata(latest, searched), variants: latest.variants };
            emit(id, "details", { ...latest, id: work.id, inspectedPublication: variant });
            log("details", variant.sourceId);
        }));
    } else if (method === "share") {
        const work = params.work as Work;
        log("share", work.id, { params });
        emit(id, "share", { url: share({ mode: "work", work, ...(params.variant ? { variant: params.variant as Variant } : {}) }) });
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
        library.update(String(workId), { progress: progress as Record<string, string | number> });
    } else if (method === "holdLibrarySnapshot") {
        holdNextLibraryWork = String(params.workId);
    } else if (method === "library") {
        const entries = library.entries(String(params.mode || "all"));
        const heldWork = entries.some(entry => entry.work.id === holdNextLibraryWork) ? holdNextLibraryWork : "";
        if (heldWork) {
            holdNextLibraryWork = "";
            await new Promise<void>(release => {
                fixtureStages.set("library-snapshot", release);
                log("librarySnapshotHeld", heldWork, { id });
            });
        }
        for (const entry of entries)
            emit(id, "library", [entry]);
        emit(id, "library", []);
        if (heldWork)
            log("librarySnapshotReleased", heldWork, { id });
    } else if (method === "remember") {
        const work = params.work as Work;
        log("remember", work?.id ?? "", { params });
        if (!work?.id) throw new Error("Select a title.");
        const entry = library.remember(work, {
            ...(params.variant ? { variant: params.variant as Variant } : {}),
            ...(params.variantFilters ? { variantFilters: params.variantFilters as VariantFilters } : {}),
            ...(typeof params.favorite === "boolean" ? { favorite: params.favorite } : {}),
            ...(params.opened === true ? { lastOpened: new Date().toISOString() } : {}),
        });
        emit(id, "complete", entry);
        return;
    } else if (method === "resolve" && (params.variant as Variant).sourceId === "fixture-reader") {
        emit(id, "resolved", { kind: (params.variant as Variant).transport });
    } else if (method === "resolve" && (params.variant as Variant).sourceId === "media") {
        log("resolve", "media");
        emit(id, "resolved", { kind: "video", url: pathToFileURL(process.env.QBUTT_CONTENT_UI_MEDIA!).href,
            ...((params.variant as Variant).id === "rejected"
                ? { audioTrack: { title: "Unavailable Studio", type: "Declared voice", codec: "pcm_s16le", channels: 1 } } : {}) });
    } else if (method === "resolve" && (params.variant as Variant).sourceId === "fixture-browser") {
        log("resolve", "fixture-browser", { params, ready: browserReady });
        emit(id, "error", browserReady
            ? { message: "Original resumed selection." }
            : { code: "browser_required", message: "Connect the browser extension.", url: (params.variant as Variant).url });
        return;
    } else if (method === "resolve" || method === "download") {
        log("resolve", String((params.variant as Variant)?.sourceId), { params });
        emit(id, "error", { message: "This selection fixture does not download payloads." });
        return;
    } else if (method === "cancel") {
        log("cancel", "", { id: params.id });
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
library.close();
