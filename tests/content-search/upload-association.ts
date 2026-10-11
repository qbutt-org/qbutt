import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { CookieJar } from "../../components/content/node_modules/tough-cookie";
import type { ResolvedContent, Work } from "../../components/content/contracts";
import { SearchEngine } from "../../components/content/engine";
import { sourceFetch } from "../../components/content/net";
import { videoProviders } from "../../components/content/providers/video";
import { SearchResults } from "../../components/content/results";
import { ContentClient, completed, searchWorks } from "./client";

const directory = await mkdtemp(join(tmpdir(), "qbutt-upload-association-"));
const client = new ContentClient(directory);
const query = "студсовет косяки";
const targetId = "yani:20398";
const previewId = "rutube:afe8b6265d9e6572523a2ac594b13ff7";
const episodeIds = ["ebcdfa03f868f3d36da34f860b60cae0", "fa276b0c16b85b3ae174b91f82ada8c0",
    "65f76209c1aee4a71c7d60e5be7d2999", "0b3a6a67283f86640aef15fe70bf0aa6", "4c7b4357d95cbff51af8b941b2f2fd8b"]
    .map(id => `rutube:${id}`).sort();

async function sourceWorks(title: string): Promise<Work[]> {
    const works: Work[] = [];
    for (const provider of videoProviders.filter(provider => ["yani", "rutube"].includes(provider.source.id))) {
        const signal = AbortSignal.timeout(90_000);
        works.push(...await provider.search({ query: title, category: "all" }, {
            signal, settings: {}, dataDir: directory, fetch: sourceFetch(signal, new CookieJar()),
        }));
    }
    return works;
}

async function torrentWorks(title: string): Promise<Work[]> {
    const engine = new SearchEngine(join(process.env.QBUTT_CONTENT_APP_DIR || "C:/Temp/qbutt-content-build", "qbutt-search.exe"),
        join(directory, "raw-indexers"));
    const works: Work[] = [];
    try {
        await engine.request("search", { query: title, category: "anime", sources: ["nyaasi"] }, event => {
            if (event.event === "result") works.push(event.data as Work);
        }, AbortSignal.timeout(90_000));
    }
    finally { engine.close(); }
    assert(works.length, "The actual public anime tracker fixture must return publications");
    return works;
}

function collect(title: string, publications: Work[], aliases: string[] = []): Work[] {
    const current = new Map<string, Work>();
    const results = new SearchResults(title, (event, data) => {
        const work = data as Work;
        if (event === "result") current.set(work.id, work);
        else if (event === "removeResult") current.delete(work.id);
    });
    for (const alias of aliases) results.addQuery(alias);
    for (const work of structuredClone(publications)) results.add(work);
    return [...current.values()];
}

function associated(works: Work[]): Work {
    const target = works.find(work => work.id === targetId);
    assert(target?.category === "anime" && target.identifiers?.mal === "61578");
    assert.deepEqual(target.variants.filter(variant => variant.sourceId === "rutube").map(variant => variant.id).sort(), episodeIds,
        "The exact episode publications must become variants of one authoritative catalog work");
    assert(target.variants.some(variant => variant.episode === "2" && variant.translation === "OnWave"));
    assert(target.variants.some(variant => variant.episode === "1" && variant.translation === "Kazoku Project"));
    assert(target.variants.some(variant => variant.episode === "1" && variant.translation === "Субтитры | Kazoku Project"));
    assert.equal(new Set(target.variants.map(variant => `${variant.sourceId}:${variant.id}`)).size, target.variants.length);
    assert(!works.some(work => work.id === previewId), "A short announced-series preview must not appear as a full anime result");
    assert.equal(works.length, 1, "The real fixture contains one catalog work with its exact episode publications");
    return target;
}

try {
    const live = await sourceWorks(query);
    associated(collect(query, live));
    associated(collect(query, [...live].reverse()));

    const all = await client.request("search", { query, category: "all" }, 180_000);
    const errors = all.filter(event => event.event === "source" && (event.data as { status: string }).status === "error");
    assert(!errors.some(event => ["yani", "rutube"].includes((event.data as { sourceId: string }).sourceId)),
        "The two live publication sources must complete");
    const target = associated(searchWorks(all));
    const torrent = target.variants.find(variant => variant.sourceId === "nyaasi"
        && /^\[ToonsHub\]/.test(variant.title) && variant.episode);
    assert(torrent && torrent.episode && torrent.infoHash, "The live fixture must expose an exact promoted episode torrent");
    const torrentInspection = await client.request("inspect", { work: target, variant: torrent });
    completed(torrentInspection);
    const torrentDetails = torrentInspection.find(event => event.event === "details")!.data as Work;
    assert.equal(torrentDetails.id, target.id);
    assert.equal(torrentDetails.title, target.title, "Torrent inspection overwrote the authoritative work title");
    assert.equal(torrentDetails.description, target.description, "Torrent inspection overwrote the catalog synopsis");
    assert.deepEqual(torrentDetails.identifiers, target.identifiers);
    const refreshedTorrent = torrentDetails.variants.find(variant => variant.id === torrent.id && variant.sourceId === torrent.sourceId);
    assert(refreshedTorrent);
    assert.equal(refreshedTorrent.infoHash, torrent.infoHash);
    assert.equal(refreshedTorrent.episode, torrent.episode, "Indexer inspection lost the selected episode");
    assert.equal(refreshedTorrent.season, torrent.season, "Indexer inspection lost the selected season");
    assert.deepEqual(refreshedTorrent.notes, torrent.notes, "Indexer inspection lost publication provenance");
    for (const previous of target.variants) {
        assert(torrentDetails.variants.some(variant => variant.id === previous.id && variant.sourceId === previous.sourceId
            || previous.infoHash && variant.infoHash === previous.infoHash), "Torrent inspection lost another source publication");
    }
    const placeholder = torrentDetails.variants.find(variant => variant.sourceId === "yani" && !variant.episode);
    assert(placeholder && placeholder.needsInspection, "A catalog placeholder must request episode inspection before playback");
    const yaniInspection = await client.request("inspect", { work: torrentDetails, variant: placeholder });
    completed(yaniInspection);
    const expanded = yaniInspection.find(event => event.event === "details")!.data as Work;
    const voices = expanded.variants.filter(variant => variant.sourceId === "yani");
    assert(voices.length > 1 && voices.every(variant => variant.episode && variant.translation && variant.player && !variant.needsInspection),
        "Inspecting the Yani placeholder must replace it with ready episode/voice/player variants");
    assert(!voices.some(variant => variant.id === placeholder.id));
    assert(expanded.variants.some(variant => variant.id === torrent.id && variant.sourceId === torrent.sourceId
        && variant.infoHash === torrent.infoHash && variant.episode === torrent.episode),
    "Catalog episode expansion lost the selected torrent identity");
    const torrents = await torrentWorks("Seitokai ni mo Ana wa Aru");
    const native = live.find(work => work.id === targetId)!;
    for (const order of [[...live, ...torrents], [...torrents, ...live]]) {
        const grouped = associated(collect(query, order, native.aliases?.slice(0, 3)));
        for (const torrent of torrents) {
            for (const variant of torrent.variants) {
                assert(grouped.variants.some(current => current.id === variant.id && current.sourceId === variant.sourceId
                    || variant.infoHash && current.infoHash === variant.infoHash), "Catalog promotion lost a tracker publication or its exact hash");
            }
        }
        assert(grouped.variants.filter(variant => variant.transport === "torrent").every(variant => variant.episode),
            "Torrent association lost the explicit episode metadata");
    }
    const colliding = await torrentWorks("Seitokai Yakuindomo");
    const collision = collect("Seitokai", [native, ...colliding]);
    assert(collision.find(work => work.id === targetId)!.variants.every(variant => !/Yakuindomo/i.test(variant.title)),
        "A shared word or release group merged an unrelated catalog title");
    const berserk = await sourceWorks("Berserk");
    const earlier = berserk.find(work => work.id === "yani:70")!;
    const later = berserk.find(work => work.id === "yani:74")!;
    assert(earlier?.aliases?.includes("Berserk") && later?.aliases?.includes("Berserk"));
    const berserkTorrents = await torrentWorks('"Berserk - 01"');
    const ambiguous = berserkTorrents.find(work => /^\[HorribleSubs\] Berserk - 01 \[1080p\]/.test(work.title))!;
    assert(ambiguous, "The live fixture must contain an episode with no distinguishing year or season");
    for (const catalog of [earlier, later]) {
        assert(collect("Berserk", [catalog, ambiguous]).find(work => work.id === catalog.id)!.variants
            .some(variant => variant.infoHash === ambiguous.variants[0]!.infoHash),
        "The real publication must exercise an otherwise eligible exact-alias association");
    }
    for (const order of [[earlier, ambiguous, later], [earlier, later, ambiguous], [ambiguous, later, earlier]]) {
        const retained = collect("Berserk", order);
        assert.equal(retained.length, 3, "A later matching catalog must leave an ambiguous publication independent");
        assert(retained.some(work => work.id === ambiguous.id && work.variants.length === 1));
    }
    const gluttony = berserkTorrents.filter(work => /Boushoku no Berserk.* - 01\b/.test(work.title));
    assert(gluttony.length);
    const otherSeries = collect("Berserk", [earlier, ...gluttony]);
    assert(otherSeries.find(work => work.id === earlier.id)!.variants.length === 1,
        "An exact shorter alias inside a different series title must not associate that series");
    const batches = berserkTorrents.filter(work => /Kenpuu Denki Berserk - 01 ~ 25/.test(work.title));
    assert(batches.length);
    assert.equal(collect("Berserk", [earlier, ...batches]).find(work => work.id === earlier.id)!.variants.length, 1,
        "An episode range must not be relabeled as its first episode");
    const special = berserkTorrents.find(work => /Berserk - 12\.5\b/.test(work.title));
    assert(special);
    assert.equal(collect("Berserk", [earlier, special]).find(work => work.id === earlier.id)!.variants.length, 1,
        "A fractional special episode must not be relabeled as an ordinary episode");
    const alias = searchWorks(await client.request("search", {
        query: "Seitokai ni mo Ana wa Aru!", category: "all", sources: ["yani", "rutube"],
    }));
    const aliasTarget = alias.find(work => work.id === targetId);
    assert(aliasTarget && aliasTarget.variants.some(variant => variant.sourceId === "rutube"),
        "Automatic title alias expansion must associate actual Russian episode publications");

    const selected = target.variants.find(variant => variant.id === "rutube:ebcdfa03f868f3d36da34f860b60cae0")!;
    const inspection = await client.request("inspect", { work: target, variant: selected });
    completed(inspection);
    const details = inspection.find(event => event.event === "details")!.data as Work;
    assert.equal(details.id, target.id);
    assert.equal(details.title, target.title, "Episode inspection overwrote the authoritative work title");
    assert.equal(details.description, target.description, "Episode inspection overwrote the catalog synopsis");
    assert.deepEqual(details.identifiers, target.identifiers);
    assert.deepEqual(details.variants.map(variant => variant.id).sort(), target.variants.map(variant => variant.id).sort());
    const resolved = await client.request("resolve", { variant: selected });
    completed(resolved);
    const media = resolved.find(event => event.event === "resolved")!.data as ResolvedContent;
    assert(media.url && media.kind === "video");
    assert.equal(media.selection?.episode, "2", "Resolution lost the selected episode");
    assert.equal(media.translation, "OnWave", "Resolution lost the selected voice");
    assert(/2\s*серия/i.test(media.title || ""), "The live media extractor opened a different episode");

    const frieren = await sourceWorks("Провожающая в последний путь Фрирен");
    const seasons = collect("Провожающая в последний путь Фрирен", frieren);
    const base = seasons.find(work => work.id === "yani:10661")!;
    const sequel = seasons.find(work => work.id === "yani:15084")!;
    assert(base && sequel && base.id !== sequel.id);
    assert(base.variants.every(variant => !variant.season || variant.season === "1"),
        "An explicit different season was associated through a shared series synopsis");
    assert(base.variants.every(variant => !/2\s*сезон/i.test(variant.title)));
    assert(sequel.variants.every(variant => !/1\s*сезон/i.test(variant.title)));
    const negatives = frieren.filter(work => /за 20 минут|Опенинг|Магия ●● — 22 серия/i.test(work.title));
    assert(negatives.length >= 3);
    for (const negative of negatives) {
        assert(seasons.some(work => work.id === negative.id), "A recap, opening or short anime was absorbed or excluded");
    }
    const secondSeasonTorrents = await torrentWorks("Sousou no Frieren S02E01");
    const seasonTorrents = collect("Sousou no Frieren", [base, sequel, ...secondSeasonTorrents]);
    const retainedBase = seasonTorrents.find(work => work.id === base.id)!;
    const retainedSequel = seasonTorrents.find(work => work.id === sequel.id)!;
    assert(retainedBase.variants.every(variant => variant.season !== "2"));
    assert(retainedSequel.variants.some(variant => variant.transport === "torrent" && variant.season === "2" && variant.episode === "1"),
        "An explicit S02E01 filename did not associate with the exact second-season catalog identity");

    console.info(JSON.stringify({ ok: true, allResults: searchWorks(all).length,
        catalogId: target.id, episodePublications: episodeIds, previewId,
        arrivalOrders: ["catalog-first", "uploads-first", "tracker-first"], aliasVariants: aliasTarget.variants.length,
        torrentPublications: target.variants.filter(variant => variant.transport === "torrent").length,
        preservedHashes: target.variants.flatMap(variant => variant.infoHash ? [variant.infoHash] : []),
        collisionPublications: colliding.length,
        inspectedTorrent: { id: torrent.id, infoHash: torrent.infoHash, episode: refreshedTorrent.episode,
            season: refreshedTorrent.season, canonicalTitle: torrentDetails.title },
        expandedCatalog: { placeholderId: placeholder.id, voices: voices.length,
            episodes: [...new Set(voices.map(variant => variant.episode))], ready: voices.every(variant => !variant.needsInspection) },
        ambiguousPublication: ambiguous.id, ambiguousCatalogs: [earlier.id, later.id],
        unrelatedSeriesPublications: gluttony.length, rangePublications: batches.length,
        resolved: { episode: media.selection?.episode, translation: media.translation, title: media.title },
        preservedSeasons: [base.id, sequel.id], negativePublications: negatives.map(work => work.id),
        unavailableSources: errors.map(event => (event.data as { sourceId: string }).sourceId) }));
}
finally {
    await client.close();
    await rm(directory, { recursive: true, force: true });
}
