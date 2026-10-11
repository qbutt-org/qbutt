import assert from "node:assert/strict";
import { deflateRawSync } from "node:zlib";
import type { ProviderContext } from "../../components/content/contracts";
import { gamingProviders } from "../../components/content/providers/games";
import { openLink, share } from "../../components/content/sharing";

const provider = gamingProviders.find(provider => provider.source.id === "fitgirl")!;
const origin = "https://games-fixture.example";
const publication = `${origin}/generated-game/`;
const mirror = "https://fuckingfast.co/v4212hw5mfaq#Generated_UNCENSORED_.part02.rar";
const secret = new Uint8Array(32);
const adata = [[Buffer.alloc(12).toString("base64"), Buffer.alloc(8).toString("base64"),
    1, 256, 128, "aes", "gcm", "zlib"], "plaintext", 0, 0];
const imported = await crypto.subtle.importKey("raw", secret, "PBKDF2", false, ["deriveKey"]);
const key = await crypto.subtle.deriveKey({ name: "PBKDF2", salt: new Uint8Array(8), iterations: 1, hash: "SHA-256" },
    imported, { name: "AES-GCM", length: 256 }, false, ["encrypt"]);
const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv: new Uint8Array(12), tagLength: 128,
    additionalData: new TextEncoder().encode(JSON.stringify(adata)) }, key,
deflateRawSync(JSON.stringify({ paste: mirror })));
const paste = { status: 0, v: 2, adata, ct: Buffer.from(ciphertext).toString("base64") };
let oldVersion = "66884";
const requests: { path: string; method: string }[] = [];

const server = Bun.serve({
    port: 0,
    fetch(request) {
        const url = new URL(request.url);
        requests.push({ path: url.pathname, method: request.method });
        if (request.method === "HEAD")
            return new Response(null, { headers: { "Content-Type": "text/html" } });
        if (url.searchParams.has("s"))
            return new Response(`<h2 class="entry-title"><a href="${publication}">Generated Game</a></h2>`);
        if (url.pathname === "/generated-game/") {
            return new Response(`<h1 class="entry-title">Generated Game: Digital Deluxe Edition, v112610 + v${oldVersion}</h1>
                <div class="entry-content"><h3>Download Mirrors (Torrent)</h3>
                <ul><li><a href="magnet:?xt=urn:btih:${"a".repeat(40)}">magnet</a></li></ul>
                <h1>Old, Uncensored Version v${oldVersion}</h1>
                <h3>Old, Uncensored Version v${oldVersion}, Download Mirrors (Direct Links)</h3>
                <style>.dlinks { margin: 0; }</style>
                <ul><li><a href="https://paste.fitgirl-repacks.site/?generated#${"1".repeat(32)}">Filehoster: Fixture</a>
                    <div class="su-spoiler"><div class="su-spoiler-content">
                        <a href="https://fuckingfast.co/inline#Generated_UNCENSORED_.part01.rar">Generated_UNCENSORED_.part01.rar</a>
                    </div></div></li></ul>
                <h3>Old, Uncensored Version v${oldVersion}, Download Mirrors (Torrent)</h3>
                <ul><li><a href="magnet:?xt=urn:btih:${"b".repeat(40)}">magnet</a></li></ul></div>`);
        }
        if (url.searchParams.has("generated"))
            return Response.json(paste);
        throw new Error(`Unexpected game payload request: ${url.pathname}`);
    },
});

const context: ProviderContext = {
    signal: AbortSignal.timeout(10_000), dataDir: "", settings: { baseUrl: origin },
    fetch(url, init) {
        const target = new URL(url);
        return fetch(new URL(target.pathname + target.search, server.url), init);
    },
};

try {
    const [work] = await provider.search({ query: "Generated Game", category: "games" }, context);
    assert(work);
    assert.equal(requests.length, 1, "Global search must not fetch release metadata");
    const details = await provider.inspect!(work.variants[0]!, context);
    assert.equal(details.variants.length, 4);
    const newer = details.variants.find(variant => variant.infoHash === "a".repeat(40))!;
    const older = details.variants.filter(variant => variant !== newer);
    assert.equal(newer.version, "112610");
    assert(older.every(variant => variant.version === oldVersion),
        "Every inline file, decoded public mirror and magnet must retain its own release heading's version");
    assert(!details.variants.some(variant => variant.version === "4212"), "An opaque file URL must not invent a version");
    assert.equal(older.filter(variant => variant.publicationKind === "archivePart").length, 2,
        "Inline and decoded mirrors must use the published archive filename, including a filename in the URL fragment");
    for (const variant of details.variants) {
        const selected = openLink(share({ mode: "exact", work: details, variant })).variant!;
        const resolved = await provider.resolve!(selected, context);
        assert.equal(resolved.url, variant.magnet ?? variant.downloadUrl);
    }
    oldVersion = "67473";
    await assert.rejects(provider.resolve!(older[0]!, context), /изменилась/,
        "Updating a release section at the same file URL must reject the previous exact selection");
    assert(requests.every(request => request.method === "HEAD" || request.path === "/" || request.path === "/generated-game/"),
        "Only publication metadata, public mirror metadata and content-type probes may be requested");
    console.log(JSON.stringify({ ok: true, variants: details.variants.length, primaryVersion: newer.version,
        oldVersion: "66884", encryptedPublicMirror: true, opaqueUrlVersionRejected: true,
        exactShareResolution: true, sectionVersionChangeRejected: true, noPayloadDownloads: true }));
}
finally {
    server.stop(true);
}
