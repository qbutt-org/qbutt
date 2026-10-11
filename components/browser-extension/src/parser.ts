// Parse fetched hoster HTML in an inert document. Never navigate to a file
// hoster or click its download button: only the central queue writes payloads.
chrome.runtime.onMessage.addListener((message, sender, respond) => {
    if (sender.id !== chrome.runtime.id || sender.tab || message?.type !== "file:parse"
        || typeof message.body !== "string" || message.body.length > 512 * 1024 || typeof message.url !== "string") return;
    const document = new DOMParser().parseFromString(message.body, "text/html");
    if (/just a moment|attention required|captcha|sign in|log in|вход|проверка/i.test(document.title)
        || document.querySelector('input[type="password"],iframe[src*="captcha"],[id*="captcha"],script[src*="challenge-platform"]')) {
        respond({ verification: true });
        return;
    }
    let base = message.url;
    const declaredBase = document.querySelector("base[href]")?.getAttribute("href");
    if (declaredBase) {
        try { base = new URL(declaredBase, base).href; }
        catch { /* Invalid bases are ignored by the browser as well. */ }
    }
    const mediafire = document.querySelector<HTMLAnchorElement>("a#downloadButton[href]");
    const anchors = mediafire ? [mediafire] : [...document.querySelectorAll<HTMLAnchorElement>("a[href]")]
        .filter((anchor) => anchor.hasAttribute("download") || /\.(?:torrent|zip|rar|7z|exe|msi|iso|tar|gz)(?:$|[?#])/i.test(anchor.getAttribute("href") ?? ""));
    const urls = anchors.flatMap((anchor) => {
        try { return [new URL(anchor.getAttribute("href")!, base).href]; }
        catch { return []; }
    });
    respond({ urls: [...new Set(urls)].slice(0, 32) });
});
