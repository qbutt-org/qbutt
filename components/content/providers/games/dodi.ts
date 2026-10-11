import type { CheerioAPI } from "cheerio";

// DODI owns the destination behind each "Click Here" link. Only links in its
// download section are releases; gameplay, adverts and install guides are not.
export function expandDodiLinks($: CheerioAPI): void {
    const body = $(".entry-content");
    let downloads = false;
    let group = "";
    for (const element of body.children().toArray()) {
        const block = $(element);
        const text = block.text().trim();
        if (/^Download Links$/i.test(text)) {
            downloads = true;
            continue;
        }
        if (!downloads)
            continue;
        if (!block.find("a").length) {
            if (/Updates|Soundtrack/i.test(text))
                group = text;
            continue;
        }
        const label = text.split(/Click Here/i)[0]!.replace(/[-–—\s]+$/g, "");
        if (!label)
            continue;
        const publicationKind = /Update/i.test(label) ? "patch"
            : /Soundtrack/i.test(label) ? "optional" : "full";
        block.find("a[href]").each((_, anchor) => {
            const link = $(anchor);
            link.attr("data-publisher-mirror", "true");
            link.attr("data-publication-kind", publicationKind);
            link.attr("data-release-transport", /^Torrent$/i.test(label) ? "torrent" : "file");
            link.text([publicationKind === "patch" ? group : "", label,
                new URL(link.attr("href")!).hostname].filter(Boolean).join(", "));
        });
    }
}
