import { load } from "cheerio";

const forbidden = "script,style,iframe,object,embed,form,input,button,link,meta,base,svg,canvas,video,audio";

export function sanitizeChapter(html: string, baseUrl?: string, allowLocalFiles = false): string {
    const $ = load(html, { xml: false });
    $(forbidden).remove();
    $("[hidden]").remove();
    $("[style]").each((_, element) => {
        if (/(?:^|;)\s*(?:display\s*:\s*none|visibility\s*:\s*hidden)\s*(?:!important)?\s*(?:;|$)/i.test($(element).attr("style")!)) {
            $(element).remove();
        }
    });
    $("*").each((_, element) => {
        for (const attribute of Object.keys($(element).attr() ?? {})) {
            if (attribute.startsWith("on") || ["srcset", "style", "contenteditable", "formaction"].includes(attribute)) {
                $(element).removeAttr(attribute);
            }
        }
    });
    $("a[href],img[src]").each((_, element) => {
        const attribute = element.tagName === "img" ? "src" : "href";
        const value = $(element).attr(attribute);
        if (!value || value.startsWith("#")) {
            return;
        }
        try {
            const url = new URL(value, baseUrl);
            if (!["http:", "https:", "file:"].includes(url.protocol) || (url.protocol === "file:" && !allowLocalFiles)) {
                $(element).removeAttr(attribute);
            }
            else {
                $(element).attr(attribute, url.href);
            }
        }
        catch {
            $(element).removeAttr(attribute);
        }
    });
    return $("body").html() ?? "";
}

export function translationText(html: string): {
    blocks: { id: string; text: string }[];
    apply(translations: { id: string; text: string }[]): string;
} {
    const $ = load(html);
    $("rt,rp").remove();
    const nodes: { type: string; data?: string; children?: unknown[] }[] = [];

    function visit(node: { type: string; data?: string; children?: unknown[] }): void {
        if (node.type === "text" && node.data?.trim()) {
            nodes.push(node);
        }
        for (const child of node.children ?? []) {
            visit(child as typeof node);
        }
    }
    for (const node of $("body").contents().toArray()) {
        visit(node);
    }
    const blocks = nodes.map((node, index) => ({ id: String(index), text: node.data!.trim() }));
    return {
        blocks,
        apply(translations) {
            const translated = new Map(translations.map(block => [block.id, block.text]));
            for (let index = 0; index < nodes.length; ++index) {
                const node = nodes[index]!;
                const text = translated.get(String(index));
                if (text === undefined) {
                    throw new Error("Перевод пропустил фрагмент главы.");
                }
                const leading = node.data!.match(/^\s*/)?.[0] ?? "";
                const trailing = node.data!.match(/\s*$/)?.[0] ?? "";
                node.data = leading + text + trailing;
            }
            return $("body").html() ?? "";
        },
    };
}
