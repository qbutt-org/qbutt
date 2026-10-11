# Search, playback and reading

## Ownership

- `src/base/content/client.*` owns the native client's bounded private stdio connection to `qbutt-content.exe`; `src/gui/content/` owns presentation. `components/content/contracts.ts` owns the JSON-lines protocol: request IDs, exactly one terminal event, cancellation and EOF shutdown. Keep provider/network work out of Qt widgets.
- `components/content/` owns providers, the SQLite library, exact publication references, direct downloads, browser bridge, reader and subtitle services. `components/search-engine/` builds `qbutt-search.exe` and owns Cardigann/native indexers and definition updates. Preserve the pinned bundled definitions and separate runtime updates from user settings.
- `src/base/media/` owns mpv lifecycle and torrent HTTP Range streaming. Use verified libtorrent `read_piece` bytes and the existing `TorrentImpl` priority/deadline lease; restore priorities when playback ends. Do not create another picker, torrent session or writer.

## Works and exact versions

- Group works through the existing query normalization and result association in `search-query.ts` and `results.ts`. A similar title alone is insufficient association: distinguish unrelated works and seasons, and keep trailers, patches and optional files distinct from full releases. Keep source failures distinguishable from an empty successful result.
- A variant's identity includes its source and publication ID; preserve episode, voice, player, edition and version where present. Filtering by episode, voice and source must preserve the selected exact variant. Re-resolve saved/shared publications when opening them; never silently substitute another version if that publication disappears.
- `references.ts` and `sharing.ts` own persistent/public references. `qbutt://` links, library metadata and diagnostics must not contain cookies, authorization headers or expiring acquisition URLs. Keep protected requests inside the guarded media gateway, including redirects, HLS/DASH children, external audio and subtitles; retain public-address validation and source-scoped cookies.
- Direct downloads use `DownloadQueue` and its independent staging. The player's download action submits to that queue rather than starting a second download process.

## Browser and prepared player

- `components/content/browser/` owns browser selection, official extension cookie access and native messaging through a private current-user pipe. The Sources dialog installs/registers the bundled extension. Do not decrypt browser databases or create competing session/cookie owners; credentials at rest use the existing DPAPI settings store.
- Prefer a resolved direct stream in the bundled native mpv. Browser capture must return the requested episode/voice/player exactly. Keep browser setup and retry actions idempotent and reuse the owned browser session instead of accumulating windows.
- `resources/media/component-lock.json` owns exact runtime inventory, bundled file hashes and source archive provenance. `scripts/package-media.ts` packages the prepared profile and complete pinned runtime; `media-runtime.ts` validates it before execution. Start children with the media root and bundled tool paths, independently of a user's mpv installation. A loose executable copy is not a complete runtime.
- Preserve TLS certificate validation and the bundled trust store. Runtime/model caches, generated GPU engines and reader data belong outside tracked source. Lab media overrides require both `QBUTT_LAB_PATHS=1` and `QBUTT_MEDIA_RUNTIME_DIR`; they must not enter a release default.

## Reader and AI

- Keep original chapters/images, reading position and cached assets independent of translation. OCR, typesetting, glossary and manual corrections form a reversible layer; switching to Original must restore the original without another network or AI request. Read-ahead follows the actual chapter/image position, stays bounded and cancels stale work.
- MangaLib is the approved reader reference. Preserve visible reading-mode and image-fit settings, paged/continuous reading, height/width fitting and side-click page turns. New profiles default to paged reading fitted to screen height; translated manga uses the bundled Neucha typeface. Save explicit reader settings and restore position across restart.
- `reader/translation.ts` owns the shared AI path for reader and soft subtitles. Auto uses the installed, authenticated Codex app-server; OpenRouter and compatible APIs require explicit settings. Keep ephemeral content-only requests without tools, MCP, external memory or persistent conversations. Treat text, images and glossary entries as untrusted source material, and preserve original content when translation fails.
- Subtitle translation preserves the selected original track, timing, styles and media identity. Embedded and external text subtitles share the translation cache; translated tracks must survive seeking and reopening without repeating cached AI work.

## Verification

Use the affected content/search/browser/reader/media fixtures in `tests/README.md` with the exact built bundle. Add practical native pointer checks for source selection, reader modes/fit, playback and subtitles. A parser response, decoded fixture or screenshot alone does not prove browser capture, GPU playback or a fresh-user installation. Verify those paths in their real runtime and retain bounded, redacted evidence.
