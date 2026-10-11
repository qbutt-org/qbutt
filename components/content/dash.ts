import { load } from "cheerio";
import type { AnyNode, Element } from "domhandler";

export async function downloadDash(body: string, url: string,
    resource: (url: string, representationIds: string[]) => Promise<string>): Promise<string> {
    if (/<!DOCTYPE|<!ENTITY/i.test(body))
        throw new Error("The DASH manifest contains unsupported XML declarations.");
    const $ = load(body, { xml: true });
    const roots = $.root().children().toArray();
    const name = (element: AnyNode) => "name" in element ? element.name.split(":").at(-1) : undefined;
    if (roots.length !== 1 || name(roots[0]) !== "MPD")
        throw new Error("The source returned an invalid DASH manifest.");
    if ($(roots[0]).attr("type")?.toLowerCase() !== "static")
        throw new Error("A live stream cannot be saved as a complete video.");

    const representations = $("*").toArray().filter(element => name(element) === "Representation");
    if (!representations.length)
        throw new Error("The DASH manifest contains no media representations.");
    const ids = representations.flatMap(element => $(element).attr("id") ? [$(element).attr("id")!] : []);
    const attributes: Record<string, string[]> = {
        SegmentTemplate: ["media", "initialization", "index"],
        SegmentURL: ["media", "index"],
        Initialization: ["sourceURL"],
        RepresentationIndex: ["sourceURL"],
    };

    const visit = async (element: Element, inherited: string): Promise<void> => {
        const node = $(element);
        const localName = name(element)!;
        if (["ContentProtection", "Location", "PatchLocation"].includes(localName)
            || Object.keys(element.attribs).some(attribute => attribute.endsWith(":href")))
            throw new Error("This DASH manifest requires unsupported protection or external metadata.");

        const xmlBase = node.attr("xml:base");
        let base = xmlBase ? new URL(xmlBase, inherited).href : inherited;
        node.removeAttr("xml:base");
        const declared = node.children().toArray().filter(child => name(child) === "BaseURL");
        const addresses: string[] = [];
        for (const child of declared) {
            const childBase = $(child).attr("xml:base");
            const address = new URL($(child).text().trim(), childBase ? new URL(childBase, base).href : base).href;
            addresses.push(address);
            $(child).removeAttr("xml:base").text(await resource(address, ids));
        }
        if (addresses.length)
            base = addresses[0];

        for (const attribute of attributes[localName] || []) {
            const value = node.attr(attribute);
            if (value)
                node.attr(attribute, await resource(new URL(value, base).href, ids));
        }
        for (const child of node.children().toArray()) {
            if (name(child) !== "BaseURL")
                await visit(child, base);
        }
    };
    await visit(roots[0], url);
    return $.xml();
}
