import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ProviderContext } from "../../components/content/contracts";
import { readingProviders } from "../../components/content/providers/reading";
import { handleReaderRequest } from "../../components/content/reader/service";
import type { ReaderDocument, ReaderPage } from "../../components/content/reader/types";
import { openLink, share } from "../../components/content/sharing";

function require(value: unknown, message: string): asserts value {
    if (!value) throw new Error(message);
}

const restarting = process.argv[2] === "--reopen";
const directory = restarting ? process.argv[4]! : await mkdtemp(join(tmpdir(), "qbutt-reader-branches-"));
let sourceDefault: number | undefined;
const chapterRequests: number[] = [];
const context: ProviderContext = {
    dataDir: directory,
    settings: {},
    signal: AbortSignal.timeout(120_000),
    async fetch(url, init) {
        const response = await fetch(url, init);
        const address = new URL(url);
        if (address.pathname.endsWith("/chapter")) {
            const branch = Number(address.searchParams.get("branch_id"));
            chapterRequests.push(branch);
            if (branch === 0 && response.ok) {
                sourceDefault = (await response.clone().json()).data.branch_id ?? 0;
            }
        }
        return response;
    },
};

try {
    if (restarting) {
        const imported = openLink(process.argv[3]!);
        const document = await handleReaderRequest("reader.open", { work: imported.work, variant: imported.variant }, context) as ReaderDocument;
        const selectedBranch = Number(imported.variant!.id.split(":branch:")[1]);
        require(document.position?.chapterId === document.chapters[1]?.id && document.position.offset === 0.4,
            "Process restart lost the selected translation position");
        require(document.chapters.every(chapter => Number(new URL(chapter.url).searchParams.get("branch_id")) === selectedBranch),
            "Fresh provider inspection changed the exact shared translation");
        const offline = { ...context, fetch: async () => { throw new Error("Restarted reading must use the saved chapter"); } };
        const page = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: document.position.chapterId }, offline) as ReaderPage;
        require(page.cached && page.html?.trim(), "Process restart could not read the exact translation offline");
        console.info("PASS fresh process exact-share resolution and independent cached position");
    }
    else {
        const provider = readingProviders.find(provider => provider.source.id === "ranobelib")!;
        const works = await provider.search({ query: "Re:Zero", category: "books" }, context);
        const work = works.find(work => work.id === "ranobelib:94231--rezero-kara-hajimeru-isekai-seikatsu-outo-no-ichinichi-hen");
        require(work, "Live catalog did not return the multi-translation publication");
        const details = await provider.inspect!(work.variants[0]!, context);
        const translations = details.variants.filter(variant => variant.sourceId === "ranobelib");
        require(translations.length >= 3 && translations.every(variant => variant.translation), "Actual translation teams were not exposed as variants");
        require(details.variants.some(variant => variant.sourceId === "syosetu"), "Branch discovery lost the original-site publication");
        require(sourceDefault !== undefined && details.chapters!.every(chapter => Number(new URL(chapter.url).searchParams.get("branch_id")) === sourceDefault),
            "Default reading interleaves translations or guesses the first listed branch");
        const defaultDocument = await handleReaderRequest("reader.open", { work: details, variant: translations[0] }, context) as ReaderDocument;
        require(translations.every(variant => variant.id.includes(":branch:")), "A multi-branch exact variant still uses a floating default ID");
        const alternative = translations.find(variant => variant.id !== translations[0]!.id)!;
        const selectedBranch = Number(alternative.id.split(":branch:")[1]);
        const exactLink = share({ mode: "exact", work: details, variant: alternative });
        const imported = openLink(exactLink);
        require(imported.variant?.id === alternative.id && imported.variant.translation === alternative.translation,
            "Exact share discarded the selected translation");
        const document = await handleReaderRequest("reader.open", { work: imported.work, variant: imported.variant }, context) as ReaderDocument;
        require(document.id !== defaultDocument.id && document.chapters.length > 1, "Translations share a reader cache/position or have no next chapter");
        require(document.chapters.every(chapter => Number(new URL(chapter.url).searchParams.get("branch_id")) === selectedBranch),
            "Selected translation interleaves another branch");
        require(new Set(document.chapters.map(chapter => chapter.id)).size === document.chapters.length, "The chapter list contains duplicate alternatives");
        chapterRequests.length = 0;
        for (const chapter of document.chapters.slice(0, 2)) {
            const page = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: chapter.id }, context) as ReaderPage;
            require(page.kind === "text" && page.html?.trim(), "Selected translation returned an empty chapter");
        }
        require(chapterRequests.length === 2 && chapterRequests.every(branch => branch === selectedBranch),
            "Reading Next requested a different translation branch");
        await handleReaderRequest("reader.position", { documentId: document.id, chapterId: document.chapters[1]!.id, offset: 0.4 }, context);
        const reopened = await handleReaderRequest("reader.open", { work: imported.work, variant: imported.variant }, context) as ReaderDocument;
        require(reopened.id === document.id && reopened.position?.chapterId === document.chapters[1]!.id && reopened.position.offset === 0.4,
            "Exact translation reopening lost its independent position");
        const offline = { ...context, fetch: async () => { throw new Error("Offline branch verification must not use the network"); } };
        const cached = await handleReaderRequest("reader.chapter", { documentId: document.id, chapterId: document.chapters[1]!.id }, offline) as ReaderPage;
        require(cached.cached && cached.html?.trim(), "The selected translation was not available offline");
        const restart = Bun.spawn([process.execPath, "run", import.meta.path, "--reopen", exactLink, directory], {
            stdout: "pipe", stderr: "pipe", windowsHide: true,
        });
        const [exitCode, output, errors] = await Promise.all([
            restart.exited, new Response(restart.stdout).text(), new Response(restart.stderr).text(),
        ]);
        require(exitCode === 0 && output.includes("PASS fresh process"), `Restart verification failed: ${errors || output}`);
        console.info(output.trim());
        console.info(`PASS live ${translations.length} translation branches, source default ${sourceDefault}, exact share/reopen, next chapter ${selectedBranch}, independent position/offline cache`);
    }
}
finally {
    if (!restarting) await rm(directory, { recursive: true, force: true });
}
