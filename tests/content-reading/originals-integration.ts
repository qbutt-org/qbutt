import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ProviderContext } from "../../components/content/contracts";
import { readingProviders } from "../../components/content/providers/reading";
import { handleReaderRequest } from "../../components/content/reader/service";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";

function require(value: unknown, message: string): asserts value {
    if (!value) throw new Error(message);
}

const directory = await mkdtemp(join(tmpdir(), "qbutt-reader-original-"));
const context: ProviderContext = {
    dataDir: directory, settings: {}, signal: AbortSignal.timeout(120_000), fetch,
};
try {
    const provider = readingProviders.find(provider => provider.source.id === "ranobelib")!;
    const results = await provider.search({ query: "Re:Zero", category: "books" }, context);
    const work = results.find(work => work.id === "ranobelib:94231--rezero-kara-hajimeru-isekai-seikatsu-outo-no-ichinichi-hen");
    require(work, "Live catalog did not return the published Re:Zero web-novel metadata");
    const details = await provider.inspect!(work.variants[0]!, context);
    const original = details.variants.find(variant => variant.sourceId === "syosetu");
    require(original?.id === "syosetu:n2267be" && original.url === "https://ncode.syosetu.com/n2267be/",
        "Actual externalLinks did not become the existing Syosetu reading variant");
    require(details.identifiers?.syosetu === "n2267be" && details.aliases?.includes("Re：ゼロから始める異世界生活")
        && original.title === "Re：ゼロから始める異世界生活", "Japanese title/identifier was lost or invented");
    const document = await handleReaderRequest("reader.open", { work: details, variant: original }, context) as ReaderDocument;
    require(document.sourceId === "syosetu" && document.chapters.length > 100 && document.title.includes("ゼロ"),
        "Original metadata opened another work or a translated publication");
    const page = await handleReaderRequest("reader.chapter", {
        documentId: document.id, chapterId: document.chapters[0]!.id,
    }, context) as ReaderPage;
    require(page.html && /[\p{Script=Hiragana}\p{Script=Katakana}]/u.test(page.html), "Original chapter has no Japanese text");
    const offline = { ...context, fetch: async () => { throw new Error("Offline verification cannot use the network"); } };
    const cached = await handleReaderRequest("reader.chapter", {
        documentId: document.id, chapterId: page.id,
    }, offline) as ReaderPage;
    require(cached.cached && cached.html === page.html, "Original-site reading did not preserve the offline chapter");
    console.info(`PASS live RU metadata → actual JP original variant → ${document.chapters.length} Syosetu chapters → original text/offline cache`);
}
finally {
    await rm(directory, { recursive: true, force: true });
}
