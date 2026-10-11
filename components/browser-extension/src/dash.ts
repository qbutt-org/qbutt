import { publicUrl } from "../../content/browser/protocol.ts";

export function playableDash(body: string, url: string): boolean {
    const xml = new DOMParser().parseFromString(body, "application/xml");
    if (xml.documentElement.localName !== "MPD" || xml.getElementsByTagNameNS("*", "parsererror").length
        || xml.getElementsByTagNameNS("*", "ContentProtection").length
        || !xml.getElementsByTagNameNS("*", "Representation").length) return false;

    let hasMedia = false;
    const resolve = (value: string, bases: string[]) => bases.map((base) => publicUrl(new URL(value, base).href));
    const mediaAddress = (value: string) => {
        const address = new URL(value);
        return !address.pathname.endsWith("/") || !!address.search;
    };
    const visit = (element: Element, inherited: string[], hasBase: boolean): void => {
        const xmlBase = element.getAttributeNS("http://www.w3.org/XML/1998/namespace", "base")?.trim();
        let bases = xmlBase ? resolve(xmlBase, inherited) : inherited;
        const declared = [...element.children].filter((child) => child.localName === "BaseURL").flatMap((child) => {
            const childXmlBase = child.getAttributeNS("http://www.w3.org/XML/1998/namespace", "base")?.trim();
            const inherited = childXmlBase ? resolve(childXmlBase, bases) : bases;
            const value = child.textContent?.trim();
            return value ? resolve(value, inherited) : [];
        });
        if (declared.length) {
            bases = declared;
            hasBase = true;
        }
        if (bases.length > 32) throw new Error("Too many DASH bases");

        for (const attribute of ["media", "initialization", "sourceURL"]) {
            const value = element.getAttribute(attribute)?.trim();
            if (!value) continue;
            const addresses = resolve(value, bases);
            if (attribute === "media" && ["SegmentTemplate", "SegmentURL"].includes(element.localName)) {
                hasMedia ||= addresses.some(mediaAddress);
            }
        }
        if (element.localName === "Representation" && hasBase) hasMedia ||= bases.some(mediaAddress);
        for (const child of element.children) {
            if (child.localName !== "BaseURL") visit(child, bases, hasBase);
        }
    };

    try {
        visit(xml.documentElement, [publicUrl(url)], false);
        return hasMedia;
    } catch { return false; }
}
