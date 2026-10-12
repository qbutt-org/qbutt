import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { BrowserBridge } from "../../components/content/browser";
import { Catalog } from "../../components/content/catalog";
import type { Variant, Work, WorkDetails } from "../../components/content/contracts";
import { assertExactVariant } from "../../components/content/references";
import { rankVariants } from "../../components/content/results";
import { SourceSettings } from "../../components/content/settings";
import { openLink, share } from "../../components/content/sharing";
import { LibraryStore } from "../../components/content/store";

// Uses the production HTTP/cookie and search aggregation path, with a fresh
// profile and no browser connection. Only publication metadata and HEAD requests
// are allowed; neither torrents nor commercial game files are downloaded.
const directory = await mkdtemp(join(tmpdir(), "qbutt-games-live-"));
const store = await LibraryStore.open(directory);
const browser = {
    async status() { return { selected: "auto", ready: false, browsers: [] }; },
    async cookies() { return []; },
    async close() {},
} as unknown as BrowserBridge;
const catalog = new Catalog(directory, directory, new SourceSettings(store), browser);
const evidence: unknown[] = [];
const blocked: { query: string; source: string; publication?: string; message: string }[] = [];
const providers = catalog.providers.filter(provider => provider.source.categories.includes("games"));
const queries = process.argv.slice(2).length ? process.argv.slice(2) : [
    "PEAK", "R.E.P.O.", "Split Fiction", "Hollow Knight Silksong", "Clair Obscur",
    "Forever Ago", "Silver Pines", "The Bell Echoes",
];

try {
    for (const query of queries) {
        const current = new Map<string, Work>();
        const statuses: { sourceId: string; status: string; message?: string }[] = [];
        await catalog.search({ query, category: "games", sources: providers.map(provider => provider.source.id) },
            AbortSignal.timeout(120_000), (event, value) => {
                if (event === "result") current.set((value as Work).id, value as Work);
                if (event === "removeResult") current.delete((value as { id: string }).id);
                if (event === "source") statuses.push(value as typeof statuses[number]);
            });
        const works = [...current.values()];
        assert(works.length, `No source found ${query}`);
        const publications = works.flatMap(work => work.variants);
        const counts = Object.fromEntries(providers.map(provider => [provider.source.id,
            publications.filter(variant => variant.sourceId === provider.source.id).length]));
        console.log(JSON.stringify({ query, counts, works: works.length,
            errors: statuses.filter(status => status.status === "error") }));
        for (const status of statuses.filter(status => status.status === "error")) {
            assert.match(status.message || "", /HTTP (?:403|429)/, "Unexpected source failure must fail acceptance");
            blocked.push({ query, source: status.sourceId, message: status.message! });
        }
        if (["PEAK", "R.E.P.O.", "Split Fiction", "Hollow Knight Silksong"].includes(query)) {
            assert(counts.byrut >= 2 && counts.freetp >= 1 && counts.onlinefix >= 1,
                `${query} must include ordinary and coop ByRut publications plus the coop sources`);
            const grouped = works.find(work => work.variants.some(variant => variant.sourceId === "byrut"));
            assert(grouped?.variants[0]?.cooperative, `${query} must rank its coop publication first`);
        }
        const record = { query, counts, statuses, works: works.map(work => ({ id: work.id, title: work.title,
            variants: work.variants.map(variant => ({ id: variant.id, source: variant.sourceId, url: variant.url,
                version: variant.version, cooperative: variant.cooperative })) })) };
        evidence.push(record);

        const primary = works.reduce((left, right) => left.variants.length >= right.variants.length ? left : right);
        for (const publication of primary.variants) {
            const provider = providers.find(provider => provider.source.id === publication.sourceId)!;
            const signal = AbortSignal.timeout(120_000);
            let details: WorkDetails;
            try {
                details = await provider.inspect!(publication, await catalog.context(provider.source.id, signal));
            }
            catch (error) {
                evidence.push({ query, source: provider.source.id, url: publication.url, inspectionError: String(error) });
                if (!/HTTP (?:403|429)/.test(String(error)))
                    throw error;
                blocked.push({ query, source: provider.source.id, publication: publication.url, message: String(error) });
                console.log(JSON.stringify(blocked.at(-1)));
                continue;
            }
            const concrete = rankVariants(details.variants.filter(variant => !variant.needsInspection));
            assert(concrete.length, `${provider.source.id} ${query} has no concrete release links`);
            assert(!concrete.some(variant => /^IDM$/i.test(variant.title)), "A download manager is not a game release");
            const selected = concrete.find(variant => variant.publicationKind === "full" && variant.transport === "torrent")
                || concrete.find(variant => variant.publicationKind === "full") || concrete[0]!;
            const exact = openLink(share({ mode: "exact", work: details, variant: selected })).variant!;
            assertExactVariant(selected, exact);
            const resolution = await provider.resolve!(exact, await catalog.context(provider.source.id, signal));
            assert(resolution.url, "The selected release must resolve to its exact source destination");
            const requested = selected.magnet || selected.downloadUrl;
            if (!requested?.includes("/getfile-"))
                assert.equal(resolution.url, requested);
            evidence.push({ query, source: provider.source.id, publication: publication.url, title: details.title,
                notes: details.notes?.length || 0, variants: concrete.map((variant: Variant) => ({ id: variant.id,
                    title: variant.title, version: variant.version, kind: variant.publicationKind,
                    cooperative: variant.cooperative, destination: variant.magnet || variant.downloadUrl })),
                selected: exact, resolution });
            console.log(JSON.stringify({ query, source: provider.source.id, variants: concrete.length,
                notes: details.notes?.length || 0, selectedVersion: selected.version, kind: resolution.kind, resolver: resolution.resolver }));
        }
    }
    console.log(JSON.stringify({ status: blocked.length ? "partial" : "passed", availableSourceChecksPassed: true,
        queries: queries.length, sources: providers.length, blocked,
        checks: ["fresh-profile", "production-fetch", "public-live-search", "coop-priority", "all-source-publications",
            "release-expansion", "source-notes", "exact-share-resolution", "no-game-payload-downloads"] }));
}
finally {
    if (process.env.QBUTT_GAME_EVIDENCE)
        await writeFile(process.env.QBUTT_GAME_EVIDENCE, JSON.stringify(evidence, null, 2));
    catalog.close();
    await browser.close();
    store.close();
    await rm(directory, { recursive: true, force: true });
}
