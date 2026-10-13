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
