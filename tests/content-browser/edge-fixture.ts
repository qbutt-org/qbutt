import { BrowserBridge } from "../../components/content/browser/index.ts";
import { ContentError } from "../../components/content/errors.ts";
import { dirname, join } from "node:path";

// The VM browser and independently installed qcatch inherit this same isolated
// discovery override. The fixture owns only the parent qbutt pipe broker.
process.env.QBUTT_BROWSER_BRIDGE_DISCOVERY ??= join(dirname(process.execPath), "fixture-browser-bridge.json");
const bridge = new BrowserBridge({ browser: "edge" });
await bridge.start();
const pageOrigin = "http://fixture.qbutt.test:43897";
const selection = { episode: "2", translationId: "17", player: "fixture" };
let unexpectedHosterScript = 0;
let payloadReads = 0;
let referrerPlayerVisits = 0;
const server = Bun.serve({
    port: 43897,
    hostname: "0.0.0.0",
    idleTimeout: 60,
    async fetch(request) {
        const url = new URL(request.url);
        if (url.pathname === "/status") return Response.json(bridge.status());
        if (["/check", "/check-hls", "/check-files", "/check-empty", "/check-selection", "/check-referrer", "/check-embedded"].includes(url.pathname)) {
            try {
                if (!bridge.status().userAgent?.includes("Edg/")) throw new Error("Actual Edge user agent missing");
                if (url.pathname === "/check-embedded") {
                    const source = "http://alloha.qbutt.test:43897/embedded-player";
                    const options = { referrer: `${pageOrigin}/embedded-source` };
                    const resolved = await bridge.resolve(source, {}, options);
                    if (resolved.url !== "http://alloha.qbutt.test:43897/episode-1-voice-9/embedded/high.m3u8") throw new Error("Embedded-only initial manifest lost");
                    const selected = await bridge.resolve(source, { episode: "2", translationId: "17", player: "alloha" }, options);
                    if (selected.url !== "http://alloha.qbutt.test:43897/episode-2-voice-17/embedded/high.m3u8") throw new Error("Embedded selection query update lost");
                    for (const [path, code] of [["/embedded-redirect", "selected_content_changed"], ["/embedded-missing", "source_unavailable"], ["/embedded-challenge", "verification_required"]]) {
                        const player = `http://alloha.qbutt.test:43897${path}`;
                        let rejected = false;
                        try { await bridge.resolve(player, {}, options); }
                        catch (error) { rejected = error instanceof ContentError && error.code === code && error.url === player; }
                        if (!rejected) throw new Error(`Embedded error lost: ${path}`);
                    }
                    return Response.json({ ok: true, cases: ["player-retains-iframe-context", "initial-extensionless-manifest", "ordinary-iframe-referrer", "unrelated-sibling-ignored", "selected-query-remounted-in-frame", "child-redirect-rejected", "child-http-status-classified", "child-challenge-not-bypassed"] });
                }
                if (url.pathname === "/check-referrer") {
                    const source = "http://player.qbutt.test:43897/referrer-player";
                    let rejected = false;
                    try { await bridge.resolve(source); }
                    catch (error) { rejected = error instanceof ContentError && error.code === "verification_required" && error.url === source; }
                    if (!rejected) throw new Error("Missing ordinary browser referrer accepted");
                    const resolved = await bridge.resolve(source, {}, { referrer: `${pageOrigin}/referrer-page` });
                    if (resolved.url !== "http://player.qbutt.test:43897/referrer/high.m3u8") throw new Error("Natural source-page referrer not preserved");
                    const canonical = await bridge.resolve(source, {}, { referrer: `${pageOrigin}/referrer-redirect` });
                    if (canonical.url !== resolved.url) throw new Error("Canonical source page with loading child did not navigate");
                    const deferred = await bridge.resolve(`${pageOrigin}/catalog-deferred`, { player: "cvh" });
                    if (deferred.url !== `${pageOrigin}/deferred/high.m3u8`) throw new Error("Deferred catalog player was selected before DOMContentLoaded");
                    const framed = await bridge.resolve(`${pageOrigin}/catalog-referrer`, { player: "alloha" });
                    if (framed.url !== "http://alloha.qbutt.test:43897/referrer/high.m3u8") throw new Error("Selected iframe lost natural referrer");
                    const visits = referrerPlayerVisits;
                    rejected = false;
                    try { await bridge.resolve(source, {}, { referrer: `${pageOrigin}/referrer-challenge` }); }
                    catch (error) { rejected = error instanceof ContentError && error.code === "verification_required" && error.url === source; }
                    if (!rejected || referrerPlayerVisits !== visits) throw new Error("Referrer challenge bypassed");
                    const catalog = `${pageOrigin}/catalog-missing`;
                    rejected = false;
                    try { await bridge.resolve(catalog, { player: "cvh" }); }
                    catch (error) { rejected = error instanceof ContentError && error.code === "source_unavailable" && error.url === catalog; }
                    if (!rejected) throw new Error("Selected iframe lost unavailable classification");
                    return Response.json({ ok: true, cases: ["missing-referrer-rejected", "natural-cross-origin-referrer", "canonical-source-with-loading-child", "deferred-catalog-selection", "selected-iframe-natural-referrer", "source-challenge-not-bypassed", "selected-iframe-unavailable-classified"] });
                }
                if (url.pathname === "/check-selection") {
                    for (const [path, code] of [["/player-redirect", "selected_content_changed"], ["/player-late-redirect", "selected_content_changed"],
                        ["/player-missing", "source_unavailable"], ["/player-missing-text", "source_unavailable"], ["/player-late-missing", "source_unavailable"], ["/empty-player", "media_unavailable"]]) {
                        const source = `${pageOrigin}${path}`;
                        let rejected = false;
                        try { await bridge.resolve(source); }
                        catch (error) { rejected = error instanceof ContentError && error.code === code && error.url === source; }
                        if (!rejected) throw new Error(`Changed or unavailable selection accepted: ${path}`);
                    }
                    const direct = await bridge.resolve(`${pageOrigin}/player?scenario=storage-frame`);
                    if (direct.url !== `${pageOrigin}/episode-1-voice-9/storage-frame/high.m3u8`) throw new Error("Storage iframe replaced direct player");
                    return Response.json({ ok: true, cases: ["initial-redirect-rejected", "late-redirect-rejected", "missing-page-classified", "missing-content-classified", "late-region-unavailable-classified", "empty-player-not-mislabeled-challenge", "direct-player-storage-frame-ignored"] });
                }
                if (url.pathname === "/check-hls") {
                    const redirected = await bridge.resolve(`${pageOrigin}/player?scenario=redirect`, selection);
                    if (redirected.url !== `${pageOrigin}/relocated/high.m3u8`) throw new Error("XHR redirect lost final manifest base");
                    const fetched = await bridge.resolve(`${pageOrigin}/player?scenario=fetch-redirect`, selection);
                    if (fetched.url !== `${pageOrigin}/relocated/high.m3u8`) throw new Error("Opaque fetch redirect lost observed final manifest");
                    const based = await bridge.resolve(`${pageOrigin}/player?scenario=fetch-redirect-base`, selection);
                    if (based.url !== `${pageOrigin}/relocated/high.m3u8`) throw new Error("Fetch observation ignored document base URI");
                    const audio = await bridge.resolve(`${pageOrigin}/player?scenario=audio`, selection);
                    if (audio.audioUrl !== `${pageOrigin}/episode-2-voice-17/audio/voice.m3u8`) throw new Error("External audio lost");
                    for (const scenario of ["encrypted", "missing", "advertisement"]) {
                        const source = `${scenario === "encrypted" ? pageOrigin.toUpperCase() : pageOrigin}/player?scenario=${scenario}`;
                        let rejected = false;
                        try { await bridge.resolve(source, selection); }
                        catch (error) {
                            rejected = error instanceof ContentError && error.url === source;
                        }
                        if (!rejected) throw new Error(`Unsafe HLS scenario accepted or lost source URL: ${scenario}`);
                    }
                    return Response.json({ ok: true, cases: ["xhr-redirect-final-base", "opaque-fetch-redirect-final-base", "fetch-document-base-uri", "external-audio-and-subtitle-family", "encrypted-highest-rejected", "missing-highest-no-downgrade", "advertisement-not-selected", "failure-preserves-source"] });
                }
                if (url.pathname === "/check-files") {
                    const file = await bridge.resolve(`${pageOrigin}/hoster-base`, {}, { kind: "file" });
                    if (file.url !== `${pageOrigin}/assets/fixture.zip`) throw new Error("Hoster HTML base ignored");
                    for (const name of ["challenge", "ambiguous"]) {
                        const source = `${pageOrigin}/hoster-${name}`;
                        let rejected = false;
                        try { await bridge.resolve(source, {}, { kind: "file" }); }
                        catch (error) { rejected = error instanceof ContentError && error.url === source; }
                        if (!rejected) throw new Error(`Hoster ${name} accepted`);
                    }
                    for (const [name, code] of [["missing", "source_unavailable"], ["empty", "file_unavailable"]]) {
                        const source = `${pageOrigin}/hoster-${name}`;
                        let rejected = false;
                        try { await bridge.resolve(source, {}, { kind: "file" }); }
                        catch (error) { rejected = error instanceof ContentError && error.code === code && error.url === source; }
                        if (!rejected) throw new Error(`Hoster ${name} mislabeled as verification`);
                    }
                    if (unexpectedHosterScript || payloadReads) throw new Error("Hoster script executed or browser downloaded payload");
                    return Response.json({ ok: true, cases: ["html-base-resolved", "challenge-ad-rejected", "multiple-files-rejected", "http-missing-file-classified", "empty-hoster-not-mislabeled-challenge", "remote-hoster-script-inert", "metadata-only-no-browser-writer"] });
                }
                if (url.pathname === "/check-empty") {
                    for (const scenario of ["empty-hls", "empty-dash", "private-base", "private-segment", "private-child-base"]) {
                        const format = scenario === "empty-hls" ? "hls" : "dash";
                        const source = `${pageOrigin}/player?scenario=${scenario}&format=${format}`;
                        let rejected = false;
                        try { await bridge.resolve(source, selection); }
                        catch (error) { rejected = error instanceof ContentError && error.url === source; }
                        if (!rejected) throw new Error(`Invalid manifest accepted: ${scenario}`);
                    }
                    return Response.json({ ok: true, cases: ["hls-empty-segment-rejected", "dash-empty-address-rejected", "dash-private-inherited-base-rejected", "dash-mixed-private-segment-rejected", "dash-private-child-xmlbase-rejected"] });
                }
                const session = await bridge.cookies(`${pageOrigin}/`);
                if (!session.some((cookie) => cookie.name === "qbutt_fixture" && cookie.value === "legal-fixture" && cookie.hostOnly)) throw new Error("Official browser host-only cookie missing");
                const fetched = await bridge.fetch(`${pageOrigin}/text`);
                if (await fetched.text() !== "fixture-response") throw new Error("Browser fetch response differs");
                const resolved = await bridge.resolve(`${pageOrigin}/player`, selection);
                if (resolved.url !== `${pageOrigin}/episode-2-voice-17/high.m3u8`) throw new Error("Wrong selected episode, translation, or quality");
                if (resolved.headers?.["User-Agent"] !== bridge.status().userAgent) throw new Error("Playback user agent differs from browser session");
                const dash = await bridge.resolve(`${pageOrigin}/player?format=dash`, selection);
                if (dash.url !== `${pageOrigin}/episode-2-voice-17/manifest.mpd`) throw new Error("Wrong DASH episode or translation");
                const file = await bridge.resolve(`${pageOrigin}/hoster`, {}, { kind: "file" });
                if (file.kind !== "file" || file.url !== `${pageOrigin}/fixture.zip`) throw new Error("Wrong selected file");
                const torrent = await bridge.resolve(`${pageOrigin}/fixture.torrent`, {}, { kind: "torrent" });
                if (torrent.kind !== "torrent") throw new Error("Torrent metadata misclassified");
                const abort = new AbortController();
                const pending = bridge.resolve(`${pageOrigin}/player`, selection, { signal: abort.signal });
                setTimeout(() => abort.abort(), 700);
                let canceled = false;
                try { await pending; } catch { canceled = true; }
                if (!canceled) throw new Error("Cancellation failed");
                return Response.json({ ok: true, cases: ["official-cookie-api", "browser-fetch", "exact-episode", "exact-translation", "selected-player", "maximum-hls-quality", "playable-dash", "hoster-exact-file", "torrent-metadata", "cancel-closes-owned-tab"] });
            } catch (error) {
                return Response.json({ ok: false, error: error instanceof Error ? error.message : String(error) }, { status: 500 });
            }
        }
        if (url.pathname === "/") return new Response("<h1>qbutt legal browser fixture</h1><p>Keep this tab active during background capture.</p>", { headers: { "content-type": "text/html", "set-cookie": "qbutt_fixture=legal-fixture; Path=/; SameSite=Lax" } });
        if (url.pathname === "/text") return new Response("fixture-response");
        if (url.pathname === "/referrer-page") return new Response("<!doctype html><title>Legal source page</title>", { headers: { "content-type": "text/html", "referrer-policy": "strict-origin-when-cross-origin" } });
        if (url.pathname === "/referrer-redirect") return Response.redirect(`${pageOrigin}/referrer-canonical`, 302);
        if (url.pathname === "/referrer-canonical") return new Response('<title>Canonical legal source</title><iframe src="/hanging-frame"></iframe>', { headers: { "content-type": "text/html", "referrer-policy": "strict-origin-when-cross-origin" } });
        if (url.pathname === "/hanging-frame") return new Response(new ReadableStream({ start(controller) { controller.enqueue(new TextEncoder().encode("<!doctype html><title>Unfinished child</title>")); } }), { headers: { "content-type": "text/html" } });
        if (url.pathname === "/referrer-challenge") return new Response("<!doctype html><title>Just a moment</title>", { headers: { "content-type": "text/html" } });
        if (url.pathname === "/catalog-missing") return new Response('<iframe src="/iframeCVH.html"></iframe>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/catalog-referrer") return new Response('<iframe src="http://alloha.qbutt.test:43897/referrer-player"></iframe>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/catalog-deferred") return new Response('<!doctype html><script defer src="/deferred.js"></script><body>Deferred catalog</body>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/deferred.js") {
            await Bun.sleep(1000);
            return new Response('const frame=document.createElement("iframe");frame.src="/iframeCVH.html?ready=true";document.body.append(frame);', { headers: { "content-type": "application/javascript" } });
        }
        if (url.pathname === "/iframeCVH.html") return new Response(url.searchParams.has("ready")
            ? '<video src="/deferred/master.m3u8"></video><button data-action="play" onclick="fetch(\'/deferred/master.m3u8\').then(r=>r.text())">Play</button>'
            : "Извините, но данной страницы не существует", { headers: { "content-type": "text/html; charset=utf-8" } });
        if (url.pathname === "/referrer-player") {
            referrerPlayerVisits++;
            if (request.headers.get("referer") !== `${pageOrigin}/`) return new Response("Source referrer required", { status: 403 });
            return new Response('<video src="/referrer/master.m3u8"></video><button data-action="play" onclick="fetch(\'/referrer/master.m3u8\').then(r=>r.text())">Play</button>', { headers: { "content-type": "text/html" } });
        }
        if (url.pathname === "/embedded-source") return new Response('<!doctype html><title>Legal source</title><iframe src="/embedded-ad"></iframe>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/embedded-ad") return new Response('<video src="/advertisement.mp4"></video><script>fetch("/advertisement/master.m3u8")</script>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/embedded-player") {
            const episode = url.searchParams.get("episode") ?? "1";
            const translation = url.searchParams.get("translation") ?? "9";
            return new Response(`<!doctype html><html><head><script>
                if(window===window.top)document.write("Контент не найден или недоступен в вашем регионе!");
                else fetch("/embedded-manifest?episode=${episode}&translation=${translation}").then(r=>r.text());
                </script><script defer src="/embedded-defer.js"></script></head><body>
                <select name="episode"><option value="1">1</option><option value="2">2</option></select>
                <select name="translation"><option value="9">9</option><option value="17">17</option></select>
                </body></html>`, { headers: { "content-type": "text/html; charset=utf-8" } });
        }
        if (url.pathname === "/embedded-manifest") {
            if (!request.headers.get("referer")?.startsWith("http://alloha.qbutt.test:43897/embedded-player")) return new Response("Player context required", { status: 403 });
            return new Response(`#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\n/episode-${url.searchParams.get("episode")}-voice-${url.searchParams.get("translation")}/embedded/high.m3u8\n`, { headers: { "content-type": "application/vnd.apple.mpegurl" } });
        }
        if (url.pathname === "/embedded-defer.js") {
            await Bun.sleep(500);
            return new Response("// Legal deferred source fixture", { headers: { "content-type": "application/javascript" } });
        }
        if (url.pathname === "/embedded-redirect") return Response.redirect("http://alloha.qbutt.test:43897/unselected-player", 302);
        if (url.pathname === "/embedded-missing") return new Response("Selected video unavailable", { status: 404 });
        if (url.pathname === "/embedded-challenge") return new Response("<!doctype html><title>CAPTCHA</title><p>Verify this source</p>", { headers: { "content-type": "text/html" } });
        if (url.pathname === "/player-redirect") return Response.redirect(`${pageOrigin}/unselected-player`, 302);
        if (url.pathname === "/player-late-redirect") return new Response('<button data-action="play" onclick="setTimeout(()=>location=\'/unselected-player\',1000)">Play</button>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/player-late-missing") return new Response('<button data-action="play" onclick="setTimeout(()=>document.body.textContent=\'Контент не найден или недоступен в вашем регионе!\',1000)">Play</button>', { headers: { "content-type": "text/html; charset=utf-8" } });
        if (url.pathname === "/empty-player") return new Response("<!doctype html><title>Legal empty player</title>", { headers: { "content-type": "text/html" } });
        if (url.pathname === "/player-missing") return new Response("Selected video unavailable", { status: 404 });
        if (url.pathname === "/player-missing-text") return new Response("Извините, но данной страницы не существует", { headers: { "content-type": "text/html; charset=utf-8" } });
        if (url.pathname === "/unselected-player") return new Response('<video src="/unrelated.mp4"></video>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/hoster") return new Response('<a download href="/fixture.zip">Download fixture</a>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/hoster-base") return new Response('<base href="/assets/"><a download href="fixture.zip">Download fixture</a><script>fetch("/script-executed");location="/fixture.zip"</script>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/hoster-challenge") return new Response('<title>Sign in</title><input type="password"><a download href="/fixture.zip">Advertisement</a>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/hoster-ambiguous") return new Response('<a download href="/fixture.zip">File one</a><a download href="/other.zip">File two</a>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/hoster-missing") return new Response("File not found", { status: 404 });
        if (url.pathname === "/hoster-empty") return new Response('<title>File mirror</title><p>No download is currently available.</p>', { headers: { "content-type": "text/html" } });
        if (url.pathname === "/script-executed") { unexpectedHosterScript++; return new Response("Unexpected script execution", { status: 500 }); }
        if (/\/(?:fixture|other)\.zip$/.test(url.pathname)) {
            if (request.method !== "HEAD") payloadReads++;
            return new Response(request.method === "HEAD" ? null : "fixture", { headers: { "content-type": "application/zip", "content-disposition": 'attachment; filename="fixture.zip"' } });
        }
        if (url.pathname === "/fixture.torrent") return new Response(request.method === "HEAD" ? null : "d4:infod4:name7:fixtureee", { headers: { "content-type": "application/x-bittorrent" } });
        if (url.pathname === "/player") {
            const episode = url.searchParams.get("episode") ?? "1";
            const translation = url.searchParams.get("translation") ?? "9";
            const scenario = url.searchParams.get("scenario");
            const format = url.searchParams.get("format") === "dash" ? "manifest.mpd" : "master.m3u8";
            const media = scenario === "fetch-redirect-base" ? "opaque-manifest"
                : scenario === "fetch-redirect" ? `${pageOrigin}/opaque-manifest`
                : scenario === "redirect" ? `${pageOrigin}/redirect/master.m3u8`
                : `${pageOrigin}/episode-${episode}-voice-${translation}/${scenario ? `${scenario}/` : ""}${format}`;
            const start = scenario === "redirect" ? `const xhr=new XMLHttpRequest();xhr.open("GET",${JSON.stringify(media)});xhr.send();`
                : `fetch(${JSON.stringify(media)}).then(r=>r.text());${scenario === "audio" ? `fetch("/episode-${episode}-voice-${translation}/audio/voice.m3u8");fetch("/episode-${episode}-voice-${translation}/audio/captions.m3u8");` : ""}`;
            return new Response(`<!doctype html><title>Legal selected-player fixture</title>
                ${scenario === "fetch-redirect-base" ? '<base href="/base/">' : ""}
                <select name="episode"><option value="1">Episode 1</option><option value="2">Episode 2</option></select>
                <select name="translation"><option value="9">Voice 9</option><option value="17">Voice 17</option></select>
                <select name="player"><option value="fixture">Fixture player</option></select>
                <video controls muted ${scenario === "redirect" ? "" : `src="${media}"`}></video>
                ${scenario === "advertisement" ? '<video muted src="/advertisement.mp4"></video>' : ""}
                ${scenario === "storage-frame" ? '<iframe src="/storage?kodik=advertisement"></iframe>' : ""}
                <button data-action="play" onclick='${start}'>Play fixture</button>`, { headers: { "content-type": "text/html" } });
        }
        if (url.pathname === "/redirect/master.m3u8") return Response.redirect(`${pageOrigin}/relocated/master.m3u8`, 302);
        if (["/opaque-manifest", "/base/opaque-manifest"].includes(url.pathname)) return Response.redirect(`${pageOrigin}/relocated/master.m3u8`, 302);
        if (url.pathname === "/redirect/high.m3u8") return new Response("Incorrect redirect base", { status: 404 });
        if (url.pathname.endsWith("/missing/high.m3u8")) return new Response("Selected maximum unavailable", { status: 403 });
        if (url.pathname.endsWith("/empty-hls/high.m3u8")) return new Response("#EXTM3U\n#EXTINF:2,\n", { headers: { "content-type": "application/vnd.apple.mpegurl" } });
        if (url.pathname.endsWith("/encrypted/high.m3u8")) return new Response('#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI="secret.key"\n#EXTINF:2,\nsegment.ts\n', { headers: { "content-type": "application/vnd.apple.mpegurl" } });
        if (url.pathname.endsWith("/audio/master.m3u8")) return new Response('#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="voice",NAME="Selected voice",DEFAULT=YES,URI="voice.m3u8"\n#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="sub",NAME="Captions",URI="captions.m3u8"\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720,AUDIO="voice",SUBTITLES="sub"\nhigh.m3u8\n', { headers: { "content-type": "application/vnd.apple.mpegurl" } });
        if (url.pathname.endsWith("master.m3u8")) return new Response("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\nhigh.m3u8\n", { headers: { "content-type": "application/vnd.apple.mpegurl" } });
        if (/\/(?:high|low|voice|captions)\.m3u8$/.test(url.pathname)) return new Response("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nsegment.ts\n#EXT-X-ENDLIST\n", { headers: { "content-type": "application/vnd.apple.mpegurl" } });
        if (url.pathname.endsWith("/empty-dash/manifest.mpd")) return new Response("<MPD><Representation/><BaseURL/></MPD>", { headers: { "content-type": "application/dash+xml" } });
        if (url.pathname.endsWith("/private-base/manifest.mpd")) return new Response('<MPD><BaseURL>http://127.0.0.1/private/</BaseURL><Period><Representation><SegmentTemplate media="segment-$Number$.m4s"/></Representation></Period></MPD>', { headers: { "content-type": "application/dash+xml" } });
        if (url.pathname.endsWith("/private-segment/manifest.mpd")) return new Response(`<MPD><BaseURL>${pageOrigin}/public/</BaseURL><Period><Representation><SegmentList><SegmentURL media="http://127.0.0.1/private"/></SegmentList></Representation></Period></MPD>`, { headers: { "content-type": "application/dash+xml" } });
        if (url.pathname.endsWith("/private-child-base/manifest.mpd")) return new Response('<MPD><Period><Representation><BaseURL xml:base="http://127.0.0.1/">private.mp4</BaseURL></Representation></Period></MPD>', { headers: { "content-type": "application/dash+xml" } });
        if (url.pathname.endsWith("manifest.mpd")) return new Response('<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" mediaPresentationDuration="PT2S"><Period><AdaptationSet mimeType="video/mp4"><Representation id="v" bandwidth="1000000" width="1280" height="720"><SegmentTemplate initialization="init.mp4" media="segment-$Number$.m4s" duration="2" timescale="1"/></Representation></AdaptationSet></Period></MPD>', { headers: { "content-type": "application/dash+xml" } });
        return new Response("Not found", { status: 404 });
    },
});
process.on("SIGTERM", () => { server.stop(true); void bridge.close().finally(() => process.exit(0)); });
console.log("qbutt browser fixture ready");
