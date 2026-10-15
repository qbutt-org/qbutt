# Integration lab

`cargo run --manifest-path src/content/Cargo.toml --example lib-session` checks
the native Lib account transport with generated private-pipe and HTTPS peers on
Windows. It needs free loopback port 443, uses its own temporary discovery and
TLS trust, and never accesses the installed browser or a live profile. It covers
account import before a partial anonymous search, one 401 retry, removal/expiry,
API-origin isolation, CDN cookies without Bearer, bounded input, cancellation,
connection replacement and interruption. Lib credentials are also rejected by
the foreign reply adapter.
Real qcatch site storage and live accounts remain separate acceptance scenarios.

The live `mangalib` example accepts a publication ID, search query and zero-based
chapter index. It uses an anonymous session and chooses the first translation
branch returned by the native provider. This invocation passed after building the
release example in the existing MSVC/Cargo environment:

```powershell
& .\build\dev\cargo\build\x86_64-pc-windows-msvc\release\examples\mangalib.exe '242614--dungeon-crawler-carl' 'Dungeon Crawler Carl' 1
```

Require exit zero and inspect the JSON's exact branch/chapter, page count and
downloaded image SHA. The example also rejects absent branches/uploads. Without
arguments it retains the Yotsuba control case. It does not establish native image
decoding or account-required access; an age notice on the website alone does not
establish an API login requirement.

`tests/content-reading/lib-session.ts` runs the hidden Qt `--lib-session`
scenario against generated HTTPS content and a private qcatch pipe. Pass the
acceptance executable, PEM certificate and key. The fixture build resolves
`api.cdnlibs.org` and `img3.cdnlibs.org` to `127.0.0.1:443` and trusts that
certificate through a temporary `SourceHttp::new` builder overlay; restore the
source and rebuild production transport afterwards. The scenario checks complete
account search, native manga decoding and text reading, chapter-scoped imports,
rejection of the Qt credential relay, and absence of credentials in SQLite.

The lab generates its own legal payload and v1, v2 and hybrid torrents. It creates
new directories under the OS temporary directory and never reuses an existing
output tree. Manifests contain exact lengths, SHA-256 hashes, native torrent file
indices, padding and v1 file offsets. The v1 16 KiB fixture has a piece crossing
`alpha.bin` and `nested/beta.bin`; the 64 KiB fixture changes that layout.

Install test-only bindings using CPython 3.12 on Windows x64:

```powershell
python -m venv "$env:TEMP/qbutt-lab-python"
& "$env:TEMP/qbutt-lab-python/Scripts/python.exe" -m pip install --require-hashes -r tests/fixtures/requirements.txt
$env:QBUTT_LAB_PYTHON = "$env:TEMP/qbutt-lab-python/Scripts/python.exe"
bun run fixtures
```

The wheel revision and SHA-256 are separate from the production libtorrent lock.
Python is only the test adapter to libtorrent's metadata generation and seed
session; Bun owns the fixtures and integration orchestration. No custom v2 Merkle
implementation is used. The default torrents are private with no public tracker.
`v1-public` exists for Mixed routing tests; the isolated application and seed
sessions disable public discovery and use only explicitly added controlled peers.

## Rust and Qt Quick content

`bun tests/content-browser/session-retry.ts <acceptance-exe> <server-cert.pem>
<server-key.pem> <receipt.json>` exercises explicit browser-session retry through
the native controller. It checks session refresh before a manual source check,
exact continuation, stale results and failed retries with generated RuTracker
responses and a private qcatch peer. The fixture build redirects `rutracker.org`
to `127.0.0.1:443` and trusts its CA in `SourceHttp::new`; restore the overlay and
rebuild afterwards. No real browser session or account is used.

`bun tests/content-search/progressive-native.ts <acceptance-exe> <server-cert.pem>
<server-key.pem> success` checks a late publication joining an already selected
work: variants load automatically while the exact selected variant and row stay
stable. A held slow inspection must not disable the ready variant or block its
exact torrent handoff. Favorites must verify their own publication before acting.
Repeat with `failure` for an unavailable source, `initial` for the first ready
variant while another source is loading, and `missing` for a vanished saved
variant that must not be silently replaced. The isolated fixture build maps
`anilibria.top` and `api.yani.tv` to `127.0.0.1:443` and trusts the fixture CA through
a temporary `SourceHttp::new` overlay. Restore it and rebuild production transport
afterwards; this scenario never uses a live profile or source.
Set `QBUTT_PROGRESSIVE_RENDER_DIR` to an absolute directory for an offscreen
product render of the ready action while inspection is pending. This does not
establish native pointer interaction.
For an interactive VM pass, also set `QBUTT_PROGRESSIVE_POINTER=1`. At
`FIXTURE click_ready_action`, click the actual Download button within ten seconds.
The fixture requires the exact handoff while the other inspection remains held;
it does not substitute a controller call for the pointer action.

`bun tests/content-reading/library.ts <acceptance-exe> <server-cert.pem>
<server-key.pem> sharing` runs only the hidden native Share scenario. The isolated
fixture build redirects `anilibria.top` to `127.0.0.1:443` and trusts the local CA
through a temporary `SourceHttp::new` overlay, as in the Lib session fixture above.
Use a leaf server certificate signed by that CA; do not install either certificate
in the OS store. Restore the source and rebuild the product after the fixture.
This scenario checks missing exact variants/publications, unrelated references,
private or malformed links, and one action already running when another confirmed
publication arrives. It must retain the chosen variant, add the alternatives and
emit that exact torrent without a second resolve. Require exit zero and the
`sharing` receipt; the accepted run made eight inspections and zero searches.
It exercises the controller and SQLite, not native pointer interaction.

Native library storage has an isolated process integration scenario with generated
public metadata. It creates and removes its temporary profile, reopens the database
in another process, and checks exact selection, full variants and filters,
partial/full metadata, independent favorites/history, rejected writes, SQLite
abort and corrupt references/schema. It never reads an existing profile or starts
a download:

```powershell
cargo run --locked --offline -p qbutt-content --example library
cargo run --locked --offline -p qbutt-content --example reader_progress
cargo run --locked --offline -p qbutt-content --example sources
```

Require exit code zero and every reported check to pass. This checks storage;
it does not establish that Favorites or history are wired into the UI.
The reader example preserves the exact chapter, zero-based page and normalized
scroll fraction across a process restart. It also checks that variant choices
remain independent and rejected references, invalid positions and SQLite abort
leave the previous confirmed entry intact.

The sources scenario reopens persisted enablement and the exact failed operation
in a second process. It checks that unrelated, stale and repeated-operation
success cannot clear the current failure, that SQLite abort preserves the prior
record, and that corrupt source metadata does not prevent reading independent
library and reader settings. This does not establish Sources UI or browser
authorization.

Run `--sources --lang ru` in the isolated offscreen host to check the typed
SourceModel and production Sources overlay. The scenario covers delayed profile
initialization, persisted enablement, ordered writes, stale/reentrant outcomes,
exact operation retry and SQLite failures. Require exit zero and `PASS sources`.
Set `QBUTT_QCATCH_SETUP_FIXTURE_EXE` to the built `qcatch-fixture/qcatch.exe`.
The isolated registry redirects the selected browser to this CLI sink: the
generated Challenge checks its exact site URL, missing-browser feedback and
retained failure until the saved operation is retried.
This process scenario does not establish native pointer, blur or browser login.

The generated 1337x scenario exercises the production provider through private
HTTPS and qcatch pipe peers. It checks the `/srch` cookie jar, six categories,
exact magnets, changed hashes, isolated uploader exclusion, browser failures and
cancellation, malformed/foreign responses, empty results and SQLite reopen:

```powershell
cargo run --locked --offline --release --target x86_64-pc-windows-msvc -p qbutt-content --example x1337-fixtures
```

It never acquires torrent payload. Live qcatch access and native Add dialog
cancellation require separate acceptance; generated browser replies do not prove
that a live source's challenge will pass. The `x1337` live example requires an
explicit isolated discovery path for an already configured qcatch native host.

The protected playback transport has a generated HTTP/HTTPS integration scenario:

```powershell
cargo run --locked --offline -p qbutt-content --example media-gateway -- 'C:/Tools/qbutt/native-media-gateway-fixtures' 'C:/Program Files/mpv/mpv.com'
```

The scenario requires its local HLS/TLS fixture files;
it does not use a live application profile. Require exit zero and all checks to
pass. It exercises real mpv decoding, TLS trust, proxy authentication and framing,
socket/header limits, retirement, a 67-second single HTTPS GET and closure after
60 seconds of silence. It also checks the prepared MP4 capability, Range and
full GET, private headers, manifest rejection and retirement. This does not
establish controller, qplay or History UI.

Native downloads have generated MP4, HLS and DASH scenarios. Set
`QBUTT_LAB_QPLAY` to the verified bundle's absolute `qplay.exe` path. Generate
media below a fresh temporary directory; FFmpeg's DASH segments are relative to
its working directory:

```powershell
$runtime = & $env:QBUTT_LAB_QPLAY --runtime-info-json | ConvertFrom-Json
$ffmpeg = $runtime.tools.ffmpeg
$ffprobe = $runtime.tools.ffprobe
$root = Join-Path $env:TEMP ('qbutt-downloads-' + [guid]::NewGuid().ToString('N'))
$media = Join-Path $root 'media'
New-Item -ItemType Directory -Path $media | Out-Null
Push-Location $media
try {
    & $ffmpeg -y -hide_banner -loglevel error -f lavfi -i 'testsrc2=size=160x90:rate=24:duration=3' -an -c:v libx264 -g 24 -pix_fmt yuv420p video.mp4
    & $ffmpeg -y -hide_banner -loglevel error -f lavfi -i 'sine=frequency=880:duration=3' -c:a aac audio.m4a
    & $ffmpeg -y -hide_banner -loglevel error -i video.mp4 -map 0:v -c copy -hls_time 1 -hls_list_size 0 -hls_segment_filename 'video-%03d.ts' video.m3u8
    & $ffmpeg -y -hide_banner -loglevel error -i audio.m4a -map 0:a -c copy -hls_time 1 -hls_list_size 0 -hls_segment_filename 'audio-%03d.ts' audio.m3u8
    & $ffmpeg -y -hide_banner -loglevel error -i video.mp4 -i audio.m4a -map 0:v -map 1:a -c copy -seg_duration 1 -use_template 1 -use_timeline 1 -adaptation_sets 'id=0,streams=v id=1,streams=a' manifest.mpd
}
finally {
    Pop-Location
}
cargo run --locked --offline --release --target x86_64-pc-windows-msvc -p qbutt-content --features download-fixture --example downloads -- (Join-Path $root 'mp4') (Join-Path $media 'video.mp4')
cargo run --locked --offline --release --target x86_64-pc-windows-msvc -p qbutt-content --features download-fixture --example segmented_downloads -- (Join-Path $root 'vod') $media $ffmpeg $ffprobe
```

Require exit zero and `PASS native_downloads`, `PASS native_download_audio`
and `PASS native_segmented`. The MP4 fixture checks exact bytes, progress,
cancel/retry, restart, incomplete responses and same-name payload preservation.
Its `PASS native_download_publication_shutdown` line reports the actual shutdown
outcome during filename publication, unchanged collision files, removed staging
and confirmed SQLite reopen. Publishing remains outside ordinary cancellation.
The VOD fixture checks HLS/DASH, external audio, ASS styles, decoding,
cancel/restart/retry, unsupported live/DRM manifests and secret-free persistence.
Its generated multi-voice media verifies the exact selected audio packet hash,
default disposition, rejection of ambiguous/missing/type/codec/channel choices
and absence of requests for unselected HLS voices.

With the Qt acceptance executable built, run it offscreen with `--downloads
--profile <run/profile>` and require `PASS native Qt downloads model`.
These fixtures do not establish a visible Downloads page or live source media.

RuTor has generated HTTPS/fault and two-process library scenarios:

```powershell
cargo run --locked -p qbutt-content --example rutor-fixtures
cargo run --locked -p qbutt-content --example rutor-library
cargo run --locked -p qbutt-content --example rutor
```

The fixtures check bounded responses, confirmed category routes, guarded redirects,
cancellation and exact publication/hash selection. Library reopening preserves all
six supported categories, raw release/version titles and preferences; rejected
references leave confirmed entries unchanged. The last command uses live public
Software metadata and resolves the exact magnet without downloading a payload.

Igruha's generated HTTPS scenario serves Windows-1251 documents and checks
search, two versions on one article, the publication-bound download page,
the nonfirst exact torrent, replacement rejection, empty results, invalid
responses and cancellation. It serves only four-byte legal fixture content
inside torrent metadata:

```powershell
cargo run --locked --offline --release -p qbutt-content --example igruha-fixtures
```

The source is `itorrents-igruha.org`. Direct requests may receive a DDoS-Guard
challenge; live acquisition uses an existing browser session. The OpenTTD
metadata handoff runs in an isolated MainWindow profile without game payloads.

TheLastGame has generated HTTPS/fault and two-process SQLite checks:

```powershell
cargo run --release --target x86_64-pc-windows-msvc --locked --offline -p qbutt-content --example thelastgame-fixtures --jobs 2
cargo run --release --target x86_64-pc-windows-msvc --locked --offline -p qbutt-content --example thelastgame-fixtures --jobs 2 -- --live
```

The fixture checks WordPress search redirects, exact nonfirst torrent selection,
changed metadata, partially or wholly missing attachments, source identity,
MIME/bencode/size limits, guarded redirects and cancellation through a closed
connection. Favorites and source enablement survive a process restart; Share
links retain the exact publication and hash. Resolution leaves History empty.
The live command requests public
search, article and torrent metadata. Neither scenario downloads game payloads
or establishes the native Add Torrent dialog and pointer path.

YummyAnime has generated HTTPS/SQLite scenarios and a bounded live resolver check:

```powershell
cargo run --locked --offline -p qbutt-content --example yummyanime-fixtures
cargo run --locked --offline -p qbutt-content --example kodik-fixtures
cargo run --locked -p qbutt-content --example yummyanime
```

The generated scenario checks search and inspection, changed or missing exact
identity, duplicate videos, browser challenges, redirects, response limits,
cancellation and exact selection after reopening SQLite in another process.
The Kodik scenario checks exact season, episode, movie and voice selection,
subtitle type, ambiguous or changed IDs/hashes, the highest advertised quality,
URL encoding, bounded form/HLS responses, final URLs, redirects and cancellation.
The live check resolves Frieren episode 1 with AniLibria at 1080p on Aksor/CVH
and 720p on Kodik, and rejects changed episode, voice and player selectors. It
requests catalog/player metadata and HLS manifests; it does not establish media
playback, browser capture or the native Qt interface.

Rutube has generated HTTPS/fault and two-process library checks, plus a bounded
live metadata/manifest check using Big Buck Bunny:

```powershell
cargo run --locked --offline -p qbutt-content --example rutube-fixtures
cargo run --locked --offline -p qbutt-content --example sources
cargo run --locked --offline -p qbutt-content --example rutube
cargo run --locked --offline -p qbutt-content --example aniliberty-rutube -- 9467 2
```

`aniliberty-rutube` searches and inspects the specified AniLiberty publication,
then resolves only its selected episode's Rutube reference. Its defaults are
publication 9467, which reports licensed free viewing, and episode 2. The first
JSON line contains only public identity; require exit zero and the final `PASS`
line for a successful manifest check. Rutube may deny this publication by region
or rights; that is a failed resolve, not playback evidence. Neither live example
downloads media segments or proves native playback, seeking or History resume.

The fixtures reject paid, locked, hidden, deleted and substituted publications,
denied access, DRM, malformed quality data, unsafe redirects and oversized
responses. They check cancellation, exact public identity after SQLite reopening,
and unknown categories without relabeling them from the requested category.
The AniLiberty slice resolves its nonfirst Rutube episode through the same native
Rutube owner. It pins both episode and video IDs, rejects replacements before
requesting the other provider, and preserves the original reference through
sharing and a second Library process.
The sources scenario also initializes a newly registered source in an existing
database while preserving other sources' saved enablement and failures.
The live command verifies the exact free publication and selects the concrete
media playlist for its highest unambiguous HLS rendition. Identical CDN mirror
attributes keep the first declared URI; differing maximum attributes fail.
These checks fetch no media segments and do not establish native playback.

VK Video has a generated native browser/SQLite scenario:

```powershell
cargo run --locked --offline -p qbutt-content --example vkvideo-fixtures
```

It checks native search, exact selected-video resolution, stale identity rejection,
browser choice, challenges, cancellation, public Share references and reopened
Favorites/History. Browser thumbnail query URLs and playback credentials do not
enter stored works. `vkvideo-live` is an explicit acceptance executable using
`QBUTT_BROWSER_BRIDGE_DISCOVERY` and the already connected Edge session. Its query
is the first argument; optional `QBUTT_VKVIDEO_PUBLICATION` selects one returned
public ID for guarded MP4 range checks. It prints public metadata only. These
provider checks do not establish qplay playback or native pointer input.

RanobeHub has generated HTTPS/fault and two-process SQLite checks, plus a bounded
public search, work, chapter-list and nonfirst-chapter check:

```powershell
cargo run --locked --offline -p qbutt-content --example ranobehub-fixtures
cargo run --locked --offline -p qbutt-content --example ranobehub
```

The fixtures preserve the declared chapter order, fractional numbers, exact human
translation branches, paragraphs, emphasis, ruby and image-only chapters. They
reject substituted identities, duplicate or incomplete pagination, paid/draft
chapters, AI or unknown translator types, active markup, offsite or invalid images,
unsafe redirects and oversized responses; they also check timeouts, cancellation
and saved exact reading position after restart.
The live command reads only one public chapter and prints counts without chapter
text. These commands do not establish native reader rendering.

DODI has generated HTTPS, torrent-host form and isolated SQLite checks, plus a
bounded public FileMe acquisition check:

```powershell
cargo run --locked --offline -p qbutt-content --example dodi-fixtures
cargo run --locked --offline -p qbutt-content --example dodi
cargo run --locked --offline -p qbutt-content --example dodi -- --resolve
```

The fixtures check exact publication and mirror identity, FileMe's guarded CDN
redirect, cookie and authorization scopes, cancellation, metadata validation and
size limits, and saved selection after reopening. Changed mirrors and form IDs,
foreign redirects and cross-origin POST replay are rejected. `--resolve` obtains
only the known Devil May Cry 5 torrent metadata. Up4Ever is not offered.

Online-Fix has a generated HTTPS acquisition fixture and a read-only live
example. The live command fetches only the full-game `.torrent` metadata for
Stardew Valley; it does not request a fix archive or game payload.

```powershell
cargo run --manifest-path src/content/Cargo.toml --example online-fix-fixtures --locked
cargo run --manifest-path src/content/Cargo.toml --example online-fix --locked -- 'Stardew Valley'
```

E★エブリスタ has generated parsing, formatting and access faults, plus a bounded
public search, work, chapter-list and nonfirst-chapter check:

```powershell
cargo run --locked -p qbutt-content --example estar-fixtures
cargo run --locked -p qbutt-content --example estar
```

The live command reads three public viewer pages and prints counts without
chapter text. The provider checks each page for ticket access before reading it.
The quick acceptance target below accepts `--estar` to open that exact nonfirst
chapter in the production text reader, confirm its QML blocks, and reopen it
from History in an isolated profile.

nhentai uses its public API v2 and the shared native image reader. It is an adult
catalogue, disabled until explicitly enabled in Sources. The generated TLS
scenario uses only invented gallery metadata and authored solid-color PNGs.
Build `qbutt-reader-decode-fixture` in the quick-acceptance tree, then pass its
executable to the TLS scenario:

```powershell
cargo run --locked --offline -p qbutt-content --example nhentai-fixtures -- <qbutt-reader-decode-fixture.exe>
```

It checks exact gallery/variant/chapter references, page 2 acquisition and its
decoded pixels through the production Qt reader, image guards and corrupt-image
rejection, cancellation, source opt-in and Library position across process restart.
The decoder uses `QCoreApplication` and creates no window.
The explicit live probe takes a public gallery ID and a receipt path outside the
repository; it requests metadata and HEAD only, with zero public image bodies:

```powershell
cargo run --locked --offline -p qbutt-content --example nhentai -- 687445 "$env:TEMP/qbutt-nhentai-receipt.json"
```

Metadata and HEAD do not establish live reading or GUI acceptance. The shared
reader still needs a VM check of the second page, fit/mode, History and restart.

The production `ContentView` and Rust controller have a separate process-level
acceptance target. Prepare the local Windows preset as described in
[development setup](../docs/development.md), then run:

```powershell
cmake --preset windows -DQBUTT_QT_ACCEPTANCE=ON
cmake --build --preset windows --target qbutt-quick-acceptance --parallel 8
$env:PATH = 'C:/Tools/Qt/6.12.0/msvc2022_64/bin;' + $env:PATH
$env:QT_COMMAND_LINE_PARSER_NO_GUI_MESSAGE_BOXES = '1'
$env:QT_QPA_PLATFORM = 'offscreen'
$env:QT_QUICK_BACKEND = 'software'
$quick = Start-Process ./build/dev/qbutt-quick-acceptance.exe -ArgumentList '--exercise' -WindowStyle Hidden -Wait -PassThru -RedirectStandardError "$env:TEMP/qbutt-quick-acceptance.log"
Get-Content "$env:TEMP/qbutt-quick-acceptance.log"
if ($quick.ExitCode -ne 0) { throw "Quick acceptance failed: $($quick.ExitCode)" }
```

Require exit code zero and the `PASS` line. The scenario uses live AniLiberty
and MangaDex metadata, verifies typed model bindings and GUI-thread notifications, replaces
a query during result insertion, checks the selected infohash in the emitted
magnet, distinguishes idle/cancelled/successful empty searches, checks Home/End
on a broad live result list and End/Enter through the QML variant list, cancels
during resolution notification and closes with a pending request.
It does not download a payload or exercise `AddTorrentManager`.

For native inspection in the VM, deploy this executable using `windeployqt
--quickwidgets --qmldir src/gui/quick/qml`. Run it without `--exercise` or the offscreen
environment; use `--lang ru` for Russian and `--dark` for Dark. The default is
English and Light. This host uses an isolated application identity and does not
open an existing qbutt profile.

For native restart acceptance, use `--profile <absolute isolated directory>`
and keep the same fixture for both launches. This option was exercised in the VM
with RU/Dark and EN/Light; never point it at an existing user profile. Without the
option, the host owns a fresh temporary profile until QApplication has drained
its workers.

The same executable's `--library` scenario uses a temporary profile and checks
the native Favorites model, exact selection across reopening, fresh inspection,
missing preferred variants, delayed initialization, nested model notifications,
error ordering, SQLite lock failure/retry and worker entry before accepting writes.
It checks that accepted writes drain after QObject destruction while the process
remains alive; this does not establish application Exit behavior.
Selection, resolution and `torrentReady` must leave history empty.
Run it with the same offscreen environment;
use `--library --lang ru` to check localized library errors. Require exit zero
and the `PASS library` line with every reported check passing. This backend
scenario does not replace native bookmark/tab acceptance.

Run `--rutor --lang en` in that environment for live incremental Software search,
fresh inspection, corrected category notifications, exact favorite reopening and
the emitted `torrentReady` magnet. Run `--work-filter --lang en` for the production
Qt proxy's role mapping, exact visible IDs, insertion/reset, source changes and
destruction. These scenarios retain the original collection; neither adds a torrent.

Run `--category --lang en` and `--category --lang ru` for the production category
control. Generated stored publications exercise equal-count filters, hidden selection
restoration, Search/Favorites/History, exact keyboard selection and empty states.
A real search switches category during result insertion and verifies that no second
search starts and no original rows disappear. Fresh inspection changes a cached
category and must update the visible projection. Require exit zero and every `PASS`
check. Native popup, pointer, focus and viewport acceptance remains separate.

The `--variant-filters` scenario uses two generated SQLite works with twelve
episodes, two translations, two player paths and two languages. It exercises the
production QML controls, the source-only and exact-player projections, combined
facets, same-count translation swaps, text search, the hidden-selection notice,
empty results and reset. It verifies
that filtering retains all stored variants and starts no search, inspection or
resolution. Every visible delegate must have the proxy row's exact identity and
roles, with no duplicate or stale rows after filtering and reset.
Opening the stored fixture temporarily disables its source, then
enables it through the ordinary API without inspecting the public catalog.
Per-work choices and the exact selected variant survive reopening the isolated
profile in a new controller and QML view. The searchable picker preserves its
query and list position on reopen, and accepts Enter. A SQLite abort preserves
confirmed preferences and the displayed intent; submitting the same filter again
must persist it and clear the write error. Require exit zero and
`PASS variant_filters`.

Run this scenario with the same offscreen environment, in EN/Light and RU/Dark.
With `--dark`, the offscreen host explicitly applies a dark test palette; the
scenario checks the palette consumed by the production screen.
Set `QBUTT_VARIANT_FILTER_RENDER_DIR` to a temporary absolute directory to save
production-window PNGs at 1440×900 and 1280×720. These renders and programmatic
keyboard actions do not establish native pointer interaction, backdrop blur or
application switching.

Run `bun tests/content-search/video-actions-native.ts <acceptance-exe>
<server-cert.pem> <server-key.pem> --lang ru`, then repeat with `--dark --lang en`.
Use the generated TLS fixture overlay for `api.yani.tv` described above.
The fixture holds inspection to check cached-action guards, then returns seven
YummyAnime variants with raw prefixed and unprefixed player names. Watch and Save
must agree with supported acquisition; unsupported players
retain selection and show the localized next action. Direct rejected calls must
start no resolution and create no downloads or History. Require exit zero and
`PASS video_actions`. Add `--profile <fresh-absolute-directory>` to retain product
renders at 1440×900 and 1280×720. This scenario does not prove real playback or
download completion.

Run `--work-details` with the same offscreen environment in EN/Light and
RU/Dark. Generated SQLite publications contain different descriptions and an
exact saved variant. The production view must retain its three-line summary,
offer Details only for truncated text and show complete selectable plain text
with the selected source in the existing modal. The scenario checks scrolling,
fast wheel bounds, Escape/Close and reopen with the same scroll position, and
publication changes that close and reset the modal. Another variant of the same
publication preserves its position. It also checks refresh and restart at
1280×720 and 1440×900.
An unavailable selected publication must leave its description empty. Require
exit zero and `PASS work_details`. Native pointer, blur and focus acceptance
remains separate.

Run `--worklist --lang ru` with the same isolated offscreen environment to check
the production results delegates. It scrolls the live metadata list, saves an
exact translation, switches Search/Favorites/History, invokes the row action and
removes the favorite. Actual QML image acknowledgement records reading; reopening
from History retains the full variant list, exact translation, chapter, page and
rendered scroll position. Removing History leaves the bookmark and progress intact.
It then repeatedly swaps equal-count models with different row order and compares
every instantiated row's five model roles, rendered title, accessible name and
selection state with the active model. Require exit zero and `PASS worklist`
with every reported check passing. This process scenario does not establish
native pointer interaction.

Run `--reader --lang ru` with the same isolated offscreen environment for the
native MangaDex path. It searches both source classes, inspects a publication,
selects its exact Russian translation and invokes the production QML reading
action. Require exit zero and `PASS reader` with all checks passing. The scenario
decodes actual cached pages, filters chapters without discarding the full list,
changes the query during a native model reset, rejects invalid page indices,
switches fit, cancels stale chapter work and reopens the exact translation.
At 1280×720 and 1440×900 it checks the decoded image's aspect, viewport bounds
and the full-height scrollbar hit area at the right edge in both fit modes.
Reading content receives focus on entry and after a fit action. Tab reaches it;
Up/Down scroll through the native control and update the saved position.
An isolated SQLite settings abort exercises the QML Width action, its applied
selection and specific error. An unrelated favorite write must retain that
error; the QML retry must commit the same fit and clear the notice.
It also makes the temporary cache destination a file, restores the environment
and verifies that retry reloads the same chapter. It checks deletion of the
retired page file and chapter directories after Back, reopening and retry.
Live publication availability
is an external prerequisite; a timeout is a failure.

The same reader scenario also exercises the production continuous list, confirmed
page-relative anchors, mode/fit persistence, automatic relayout, stale decode
acknowledgements and settings failure/retry. It does not establish native pointer
interaction, popup blur, application deactivation or translation.
Check the reader and chapter picker in the deployed Windows host separately.

Run `--kakuyomu-reader --lang ru --profile <isolated-directory>` and
`--ranobehub-reader --lang ru --profile <another-isolated-directory>` in the
same offscreen environment. Each scenario searches a current public work,
selects its exact text variant and nonfirst chapter, scrolls the production QML
canvas with synthetic PageDown keys, and confirms the saved block anchor in
SQLite and History. Kakuyomu also renders a live ruby block from its first
chapter; RanobeHub selects its human translation branch. Repeat each command
with `--reader-source-reopen` and the same profile to verify the exact chapter
and visible anchor after a process restart. Require exit zero and the matching
`PASS kakuyomu_reader` or `PASS ranobehub_reader` line in both phases, and inspect
the saved QQuickWidget PNGs. These runs do not establish host pointer input.

Run `--reader-translation-choice --lang ru` in the same offscreen environment
for delayed translation-choice persistence. This scenario needs the live MangaDex
publication used by the reader fixture. It holds the SQLite writer while the
user switches to Original, reopens the reader or choice panel, or changes the
target language. The saved choice must be acknowledged without restoring a
translation in an obsolete reader session. Require exit zero and
`PASS reader_translation_choice`. This checks controller state and persistence,
not a completed AI translation or native pointer interaction.

Run `--reader-text-translation --lang ru --profile <isolated-directory>` in the
same offscreen environment with an installed, signed-in Codex. It creates a
small legal FB2 in the isolated profile and uses the production QML reader to
choose a live model and reasoning effort, translate the first chapter, return
to Original and reopen the exact cached translation with Codex unavailable to
that process. The second chapter contains the first chapter's gender clue;
the accepted context must cite it. Require exit zero and
`PASS reader_text_translation`. The profile receives three QQuickWidget renders.
Set `QT_QPA_FONTDIR=C:/Windows/Fonts` for legible offscreen renders. This is a
generated native UI path at 1280×720 and 1440×900, not a host pointer test.

For bounded text read-ahead, build `qbutt-quick-acceptance` and
`qbutt-codex-text-peer`, then run the following with the Qt `bin` directory on
`PATH`. The command was verified on main `414f34429`; the evidence directory
must not exist before running.

```powershell
bun tests/content-reading/text-ahead.ts `
    'C:/Users/cawa1/Documents/Projects/qbutt/build/dev/qbutt-quick-acceptance.exe' `
    'C:/Users/cawa1/Documents/Projects/qbutt/build/dev/tests/quick-acceptance/codex-text-peer/codex.exe' `
    'C:/Users/cawa1/Documents/Projects/qbutt/build/dev/text-ahead-main-qa'
```

The hidden Qt process imports an authored FB2 and uses a controlled Codex peer,
with no account or paid AI calls. Require exit zero and all four
`PASS reader_text_ahead` lines. Its 17 turns check one following chapter,
retained names/gender context, promotion without duplicate AI, cache reuse,
foreground priority, cancellation on Original/language/model/effort/book/close,
unchanged reading/history, stale context rejection and child-process cleanup.
Online original prefetch and native pointer interaction remain separate checks.

For manga read-ahead, build `qbutt-quick-acceptance` and the
`manga-imagegen-protocol` example. Generate the authored page fixture with
`tests/manga-render`, then run the following with Qt's `bin` directory on `PATH`:

```powershell
bun tests/content-reading/manga-ahead.ts `
    'C:/Tools/qbutt/agent-build/2/manga-cleanup-quick/qbutt-quick-acceptance.exe' `
    'C:/Tools/qbutt/agent-build/2/cargo/x86_64-pc-windows-msvc/release/examples/manga-imagegen-protocol.exe' `
    'C:/Tools/qbutt/agent-build/2/manga-cleanup-native-evidence' `
    'C:/Tools/qbutt/agent-build/2/manga-ahead-qa-final-identity'
```

This verified command requires a fresh evidence directory. Its authored CBZ and
owned Codex peer make no live inference calls. Require exit zero and all five
`PASS reader_manga_ahead` lines. The 23 turns check one next page, accepted names
and facts, promotion without duplicate inference, exact Original/offline PNG
cache, unchanged History, current-page priority, cancellation and drain across
Original/language/model/effort/close/book/chapter resets, stale context rejection
and foreground retry after a background failure. Each page is prefetched once
within the current chapter and translation-settings lifecycle; a later prefetch does not permit an earlier failed or canceled target
to repeat automatically. Visiting that target may retry foreground work.
Native pointer interaction and model recognition remain separate acceptance checks.

Run `--reader-manga-translation --lang ru --profile <isolated-directory>` in
the same offscreen environment with an installed, signed-in Codex. The native
Qt fixture draws an original page with three vertical Japanese balloons and
scene artwork, imports it through LocalBook, selects an image-capable model in
the QML reader and requests an automatic translation/style map, one built-in
Codex cleanup edit and native lettering. It checks the exact source digest, page,
normalized geometry and PNG
cache identity, Original, and pixel-identical reopening with Codex unavailable. Require exit zero
and `PASS reader_manga_translation`; inspect the saved original, translated
page and QQuickWidget PNGs in the isolated profile. This is a generated UI
scenario, not a host pointer or representative artwork benchmark. The generated
cleanup may alter artwork inside its source-lettering rectangles. Semantic
accuracy, source-style recognition and local background quality still require
reviewing the saved images; the PASS line does not establish them.

`tests/manga-render` is a standalone Qt 6.12 Gui CMake fixture. Configure it in
the assigned build slot with `CMAKE_BUILD_TYPE=RelWithDebInfo`, then build
`qbutt-manga-render-acceptance` with two jobs. Run that executable with one
isolated output-directory argument and `QT_QPA_PLATFORM=offscreen`.
Require exit zero and `PASS manga_lettering`. Inspect `original.png` and
`result.png`: the authored page exercises all 12 bundled physical font faces,
Russian letters including Ё/ё, mixed emphasis, rotated text and light captions.
`colored.png` checks explicit text/outline colors while retaining the same
cleaned layer. Typography preflight returns glyph geometry without destination
bitmaps. The fixture checks variable outward outline width, explicit unoutlined
text, unchanged pixels outside cleanupRects/layout, deterministic rerendering,
missing-glyph retention, malformed geometry and cancellation. It does not establish
AI style recognition or cleanup quality on scanned artwork.

The same fixture creates `longstrip/` with an authored 800×20000 color page,
two overlapping windows, a complete overlap balloon and a balloon crossing the
first window's lower edge. Require `PASS manga_long_native`. It checks full-page
geometry, colored text and outlines, exact pixels outside cleanup/layout,
ignored noise outside declared regions, invalid crop bounds and cancellation.
Inspect `seam-original.png` and `seam-result.png` at reading size.
It also authors JPEG rotation and mirror metadata for the reader rejection checks.

The same executable accepts `--normalize <original> <generated> <output.png>`
to replay a saved imagegen result without inference. Multiple original arguments
describe the geometry of consecutive equal-width vertical segments. This mode
checks full decoding, bounded pixel counts and aspect ratio before resizing;
it does not stitch segments or assess internal panel/artwork preservation.
Require exit zero and `PASS generated_page_normalized` for valid geometry;
an altered aspect ratio must return nonzero without writing the output.

A joint no-inference reader check uses the same page fixture and owned protocol
peer. Build `manga-imagegen-protocol`, copy it as the peer discovery tree's
`codex.exe`, set its adjacent `case.txt` to `native`, and set
`fixture-directory.txt` to the authored `tests/manga-render` output directory.
Set `QBUTT_MANGA_PEER_ROOT` to that isolated discovery root before running
`--reader-manga-translation --translation-input <authored original.png>` with
`gpt-6.1-sol` and `low`. The fixture selects this peer inside its own process,
then checks native composition, atomic PNG cache, exact Original and offline reuse.
This setup uses only authored fixture maps and PNGs; it does not establish live
model recognition.

Point the peer's `fixture-directory.txt` at the authored `longstrip/` directory
and pass its `original.png` as `--translation-input` to exercise the long-page
path through the same hidden native reader. The controlled peer serves each
window's map and cleanup without inference. Require the same reader PASS: it
compares every full-page result pixel with the native expected PNG, verifies one
context revision against the full original digest, pipeline 4, Original and
offline cache reuse. Ordinary pages retain pipeline 3. `--reader-longstrip`
separately checks original tiling, source-relative position and cleanup; it
does not request translation.

For controlled rejection, set `QBUTT_MANGA_EXPECT_FAILURE=window` with an omitted
second-window balloon, or `orientation` with either authored orientation JPEG.
Require `PASS reader_manga_rejection`, the concrete native/seam error in the log,
exact Original, zero PNG/context rows and `manga-reader-error.png` after leaving
the busy state. For small authored-page proposal scenarios, put `retry-geometry`,
`retry-context` or `retry-typography` in the peer's `submission-case.txt` to verify
one rejected submission followed by native success. Use `invalid-geometry`,
`invalid-context`, `invalid-typography`, `submission-limit`, `final-bypass` or
`exhaustion-bypass` with `QBUTT_MANGA_EXPECT_FAILURE=proposal` for zero cleanup,
zero PNG/context rows and an actual reader error-state capture. Remove
`submission-case.txt` for ordinary and long-page regressions. Orientation traces
must contain zero turns and zero cleanup edits. The fixture preserves supplied encoded bytes;
it does not strip metadata by re-encoding them.

The two QML translation scenarios accept `--translation-model <model>` and
`--translation-effort <effort>` through the QML controls. Their PASS lines report
milliseconds from requesting translation until the reader leaves its busy state. Use a fresh profile for
each configuration so an earlier cache cannot replace the live request.
Set `QT_FORCE_STDERR_LOGGING=1` on Windows to retain Qt PASS/failure diagnostics.
`--translation-target ru|en` selects the target through QML. With
`--translation-input <file>`, the manga scenario uses the supplied image; the
text scenario expects a two-chapter FB2 with Ren's Japanese name (`レン`) and
the later sister (`姉`) clue. It translates both chapters, preserves the model
choice and verifies both complete originals and both offline cached chapters.
Inspect the translated prose and images separately: passing persistence and
pixel checks does not establish semantic translation quality.

`cargo run --locked --offline --release --target x86_64-pc-windows-msvc -p
qbutt-content --example translation-cache-fixture` verifies SQLite reopening,
exact cache identity and offline reuse after another chapter expands the work
context. Require exit zero and `PASS translation_cache`; a job with an outdated
context must still be rejected when saving.

`cargo run --locked --offline -p qbutt-codex --example imagegen-protocol`
checks the isolated public Codex APIs through an owned Windows mock CLI. It
requires no account or inference. The generated fixture covers one PNG plus
final JSON, exact event identity, size limits, malformed output, cancellation,
setup timeout and process reaping; ordinary structured requests keep image
generation disabled. Require exit zero and every `PASS imagegen_protocol` check.
`imagegen-live` is a separate opt-in installed-account example, taking a prepared
comparison JSON and output directory. It spends image-generation quota and saves
the PNG and acceptance receipt, including rejected candidates.

`cargo run --locked --offline --release --target x86_64-pc-windows-msvc -p
qbutt-content --example manga-imagegen-protocol` checks the public two-phase page
consumer with authored PNGs and an owned mock CLI. Require exit zero and every
`PASS manga_imagegen` check. Invalid term/quote evidence must start no imagegen
request. Valid RU/EN proposals carry exact context additions and mixed regular,
italic-only and bold-italic text runs, colors, outward outline width and normalized
cleanup/layout rectangles. Cleanup receives source text and cleanup rectangles,
without target translations or typography. The original is restored after a
neighboring context image and custom tools stay disabled in the image phase.
The structured request exposes `submit_page`, which validates the complete map,
context, window seams and native typography before returning native contact sheets.
Each region compares its original crop, exact cyan cleanup coverage and native
target lettering on a neutral background. Tiles are at most 512 pixels per side;
shrinking below nine font pixels rejects the proposal instead of attaching an
unreadable preview. Sheets are at most 2048 pixels per side. Their aggregate
16M-pixel allocation uses the remaining shared 64M inspection budget, with the
transport's existing 24-image and 12 MiB encoded batch limits. This bounds preview
RGBA buffers to 64 MB; original decode and encoded output have separate limits.
The model can correct geometry, coverage and typography within three evaluated
submissions. A fully validated preview replaces the latest candidate; a rejected
replacement retains it. The final response is only `accepted` and `reason` and
selects that exact latest candidate, or declines it. It cannot bypass submission
or start cleanup after rejection. The fixture checks validation repair, visual
replacement, invalid replacement, missing/oversized previews and shared long-page
budget exhaustion, with one cleanup after acceptance. Repeated invalid data,
a fourth submission without a valid candidate and malicious final decisions must
start no generation. Existing accepted image caches retain their pipeline identity.
Each translation window emits one bounded `MANGA_SUBMISSION_SUMMARY` with actual
tool/submit/evaluation counts, returned preview counts/bytes, the model's decision
and fixed outcome/rejection categories. It includes the image digest, page and
window offset, never page text, translations, glossary entries or model reasons.
Owned subprocess cases verify fabricated exhaustion, the fourth-submit limit,
acceptance without a candidate, rejected replacements and cancellation, including
zero PNG/context rows after refusal or cancellation and atomic rows after success.
The fixture also checks native preparation and final-render rejection,
cooperative cancellation and owner abort during preparation and final rendering,
complete/partial usage and child reaping. Its recording renderer does not
establish Qt PNG geometry or translation quality; those require the native reader
scenario.

Require every `PASS manga_long` line and the two long owner-abort PASS lines as
well. The owned process peer checks sequential accepted context, full-original
evidence coordinates, complete seam dialogue, rejection of omitted/split/merged
or repeated balloons, explicit unsupported crossings, final output limits,
second-window cancellation, native crop/composition drain and child reaping.
No partial PNG/context leaves the page job. The small-page cases remain separate.
Context images pass the same native geometry gate: each tool batch fits 16M
pixels and 16000 per side; all primary windows and accepted tool images share
the 64M inspection budget. Oversized context and exhausted-budget process cases
must reject the image before attachment.

Run `--reader-settings --lang ru` in the same offscreen environment for typed
profile-wide fit persistence through the existing SQLite worker. Require exit
zero and `PASS reader_settings`. The fixture checks Height defaults, explicit
choice during Loading, late and ABA acknowledgements, reentrant intent, SQLite
abort and queue rejection followed by exact retry. Failed or corrupt startup
must reopen the same worker; unrelated History, progress and bookmark outcomes
must preserve the settings failure, and successful settings must preserve other
write failures. Separate process phases verify restart and an accepted fit write
at graceful Exit. This fixture does not establish QML selection or native pointer
interaction; those checks belong to the production reader and deployed VM path.

Run `cargo run --locked --offline -p qbutt-content --example text-reader` for
generated original-text parsing, escaping, stable anchors and cache bounds.
Run `cargo run --locked --offline -p qbutt-content --example text-reader-progress`
for separate-process SQLite restoration of an exact text chapter, block and
within-block offset, finite bounds, atomic failure and immutable retry.
Require exit zero and the respective `PASS` output. These fixtures do not
establish QML viewport confirmation or native pointer interaction.

With Pillow installed for the generated image formats, create isolated local-book fixtures with
`python tests/quick-acceptance/local-book-fixtures.py <fixture-directory>`, then
run `cargo run --manifest-path src/content/Cargo.toml --example local-books --locked -- <fixture-directory>`.
The fixture opens EPUB, FB2, TXT, HTML documents and fragments, CBZ/ZIP and individual image files.
It checks chapter and natural page order, formatting, exclusion of HTML head text, local image bounds,
changed-file identity, private path restoration, invalid page bytes, and unsafe,
duplicate, encrypted and oversized archives.
Run `--local-book-import --lang en` and `--local-book-import --lang ru` with the
same offscreen Qt Quick acceptance environment. Require exit zero and
`PASS local_book_import_ui`: opening an oversized file over a valid chapter must
show a dismissible error while preserving that chapter.
Open the same files through the native file dialog in a separate profile to
confirm rendering and position restoration.

Run `--reader-progress --lang ru` with the same offscreen environment to check
the native progress bridge without QML acknowledgement. The consumer confirms
each local page only after `QImageReader` has decoded it. Require exit zero and
`PASS reader_progress`. It checks exact reopen and controller restart, duplicate
confirmation, stale URLs and completions, finite scroll bounds, SQLite write
abort and queue rejection followed by exact write retry, including navigation to
an unconfirmed page and failed acquisition of another chapter. It checks failed
resume reads, locked startup and invalid initial metadata followed by explicit
reopen of the same SQLite owner, and missing saved chapter/page recovery.
An explicitly selected other branch starts independently. Acquisition alone
must leave history empty; confirmed reading retains the full known work and
current choices. This scenario does not establish QML `Image.Ready`, native
pointer interaction or application Exit with an accepted write.

Run `--history --lang ru` with the same isolated offscreen environment for the
native History bridge. Require exit zero and `PASS history`. It checks confirmed
reading, full Work and exact
choices, action-time ordering across restart, fresh inspection, delayed choices,
reentrant model publication, SQLite removal abort and retry, independent
bookmarks, retained progress and unrelated payload. A committed reading write
followed by a failed history snapshot must retain successful reader status;
`refreshHistory()` retries the read without another write. Rejected reads report
a History error, and an older snapshot failure preserves a newer reentrant write
error. This bridge scenario does not establish QML acknowledgement or native
pointer interaction; those use the production work list and deployed host.

The standalone acceptance target enables `QT_QML_DEBUG` for development preview;
the shipping application does not inherit it. The project-local `qt-preview`
MCP launches this host, but its tool response alone does not prove hot reload.
Use the VM for visual work without opening windows on the host desktop.

The combined application's `quick-handoff` scenario searches live AniLiberty,
selects a non-default variant, opens the normal Add Torrent dialog and cancels
it. `--source rutor` checks a live Software publication and its full-hash selector.
`--source dodi --main`, `--source byrut --main`, `--source freetp --main`
and `--source onlinefix --main` check exact torrent bytes. `--source fitgirl --main`
selects the older Cyberpunk variant. `--source igruha --main`
uses the connected browser session for OpenTTD metadata. These paths verify the
native dialog's hash, select through the production QML category projection,
freshly resolve the exact variant and check one handoff, the isolated
destination and empty payload before and after shutdown:

```powershell
cmake --build --preset windows --target qbutt-qt-acceptance --parallel 6
$env:QBUTT_QT_ACCEPTANCE_EXE = 'C:/VMShared/qbutt-p1-quick/qbutt-qt-acceptance.exe'
bun tests/qt-acceptance/quick-handoff.ts
bun tests/qt-acceptance/quick-handoff.ts --source rutor
bun tests/qt-acceptance/quick-handoff.ts --main
bun tests/qt-acceptance/quick-handoff.ts --source dodi --main
bun tests/qt-acceptance/quick-handoff.ts --source byrut --main
bun tests/qt-acceptance/quick-handoff.ts --source freetp --main
bun tests/qt-acceptance/quick-handoff.ts --source onlinefix --main
bun tests/qt-acceptance/quick-handoff.ts --source fitgirl --main
bun tests/qt-acceptance/quick-handoff.ts --source igruha --main
```

FitGirl has separate native metadata checks:

```powershell
cargo run --locked --offline -p qbutt-content --example fitgirl-fixtures
cargo run --locked --offline -p qbutt-content --example fitgirl
```

The live check fetches only catalog pages and
public `.torrent` attachments, verifies the selected publication and v1 hash,
and requires at least one successful attachment. It never starts a torrent.
The HTTPS fixture checks both published paste hosts, exact hash binding, SQLite
identity, cancelled requests, unavailable-attachment recovery to the same magnet,
and rejection of changed identity, authentication failures, excessive work factor
and encrypted/expanded data. Optional attachment acquisition has a 10-second
deadline before recovery to the exact magnet. Encrypted responses use the transport's 2 MiB bound;
PBKDF2 is capped at 2,000,000 iterations and acquired metadata at 10 MiB. Expanded
JSON is bounded to that metadata's base64 size plus 64 KiB framing. These
checks do not establish AddDialog interaction or game payload completion.

Deploy the current executable and its Qt Quick runtime together using
`windeployqt --quickwidgets --qmldir src/gui/quick/qml`, including `qoffscreen`
and `qt.conf`.
Use the executable inside that deployed directory: Qt resolves its QML imports
relative to the executable, not the working directory. The runner cleans only its
own profile, payload and temporary directories. This process-level scenario
synthesizes Qt input; it does not replace native pointer acceptance or download
a payload.

`--main` runs through the actual MainWindow ContentView. It checks one controller
and library owner across hide/reopen, accepted Favorites writes at application
Exit, the Downloads and AI-settings routes, and actual QML Light/Dark palette
roles with white primary-action text. The Qt acceptance executable and its
dependencies must be deployed together. This still synthesizes Qt input; native
pointer and System appearance acceptance are separate.

The MainWindow direct-video path has a real playback scenario. Run it in the
isolated VM; it opens native qbutt and qplay windows:

```powershell
$env:QBUTT_QT_ACCEPTANCE_NATIVE = '1'
$env:QBUTT_QT_ACCEPTANCE_EXE = 'C:/Tools/qbutt/main-combined-qa/runtime/qbutt-qt-acceptance.exe'
$env:QBUTT_QT_ACCEPTANCE_BUNDLE = 'C:/Tools/qbutt/main-combined-qa/runtime'
$env:QBUTT_LAB_QPLAY = 'C:/Tools/qbutt/main-combined-qa/runtime/qplay/qplay.exe'
bun tests/qt-acceptance/native-watch.ts
bun tests/qt-acceptance/native-watch.ts --source=anitokyo
bun tests/qt-acceptance/native-watch.ts --source=animespirit
bun tests/qt-acceptance/native-watch.ts --source=vkvideo
```

The bundle must include the verified qplay distribution. The runner uses fresh
qbutt/qplay profiles and Windows QPA. Require exit zero and passing evidence: exact
AniLiberty episode/quality, real mpv file-loaded and advancing time, History only
after ACK with the full Work/preferences, and retired gateway after stop.
The AniTokyo and AnimeSpirit modes isolate their source, select the exact live
publication, episode and voice, then check qplay's matching variant identity,
reported media tracks, advancing time and the same saved selection in History.
AniTokyo uses Psyren episode 1 with Дублированный; AnimeSpirit uses Magical
Explorer episode 2 with Субтитры AnimeJoy. Each run stops after a brief decode.
It does not establish native pointer or GPU playback.
The VK mode searches for the Blender open movie Sintel, preserves publication
`-76820671_456239057`, exercises Video category filtering and uses the existing
qcatch session to resolve the selected video. qplay receives only the guarded
local media URL. This mode requires the real connected browser; it checks the
same actual playback ACK, exact History and gateway retirement. Product-window
renders are saved at 1440×900 and 1280×720.

Native qplay Save uses `tests/media-save.ts` in two phases. Host `prepare` builds
only the isolated driver, then restores SourceHttp byte-for-byte and rebuilds the
production targets in `finally`; it never launches qplay. `run` opens native
Windows windows and must run in the isolated VM through its interactive runner.
Reuse a verified Qt 6.12 closure and a verified qplay portable on the VM's local
disk; keep transport and evidence in the shared directory. Do not run packaged
qplay with `QT_QPA_PLATFORM=offscreen`.

Command templates with a fresh shared directory (the prepare/run phases were
executed with driver `f8cf9c29e`; the subsequent SourceHttp-only overlay cleanup
has syntax and matcher validation, pending a fresh preparation build):

```powershell
# Host, in the prepared MSVC build environment
bun tests/media-save.ts prepare C:/Users/cawa1/Documents/Projects/qbutt/build/dev C:/VMShared/save-queue
# VM, after prepare reports restored=true and no cleanup errors
bun S:/save-queue/media-save.ts run C:/QA/qbutt-native-handoff-current/qbutt C:/QA/qplay-save-current/qplay.exe S:/qplay-native-save-qa/legal.mp4 S:/save-queue
```

The generated catalog injects one failed exact resolve, while playback and the
queue fetch the official public Sintel trailer. Require passing native evidence,
one persisted original episode despite a changed catalog selection, exact payload
bytes/SHA-256 and empty cleanup errors. Save requests use actual mpv IPC here;
real pointer acceptance of the qplay action is separate.

The `language-switch` scenario uses the deployed combined executable and Qt
catalogs beside its `qt.conf`:

```powershell
$env:QBUTT_QT_ACCEPTANCE_EXE = 'C:/Tools/qbutt/language-switch-qa/runtime/qbutt-qt-acceptance.exe'
$env:QBUTT_QT_ACCEPTANCE_BUNDLE = 'C:/Tools/qbutt/language-switch-qa/runtime'
bun tests/qt-acceptance/language-switch.ts
```

It creates an isolated profile and runs RU → EN → RU → EN, restarts in EN,
applies RU and restarts in RU. It checks the Main menus and tabs, Options,
Qt's Apply button, QML text, an actual AniLiberty video delegate and accessibility,
and unchanged exact selection. Locale must persist in the isolated configuration.
Another launch seeds a source timeout and SQLite write failures, then checks the
visible source, source settings and library errors through RU → EN → RU.
The source metadata must be available; no video is resolved or downloaded.
Require exit zero and `status: passed`. This offscreen scenario does not establish
native pointer interaction or visual acceptance.

The metadata cancellation scenario uses a generated private torrent and an
isolated loopback session. It cancels before the first native add callback,
then adds the same hash stopped and checks its destination and handle cleanup:

```powershell
cmake --build --preset windows --target qbutt-qt-acceptance --parallel 8
$env:QBUTT_QT_ACCEPTANCE_EXE = "$PWD/build/dev/qbutt-qt-acceptance.exe"
$env:QBUTT_QT_ACCEPTANCE_BUNDLE = 'C:/VMShared/qbutt-p1-package/portable'
bun tests/qt-acceptance/metadata-cancel.ts
bun tests/qt-acceptance/metadata-cancel.ts --torrent-data
```

Use your deployed portable directory for `QBUTT_QT_ACCEPTANCE_BUNDLE`. The runner
requires exit zero and a passing evidence file; it removes its temporary profile
and payload directory after the application exits.

The `--torrent-data` scenario passes generated metadata through the production
ContentController, MainWindow and asynchronous native add manager into the normal
add dialog. It checks malformed data, byte/decode limits captured before worker
dispatch, manager destruction, exact libtorrent hash/file paths and cancellation
without creating payload. It uses an offscreen application and does not establish
provider acquisition or native pointer interaction.

## Content, reading and playback

The Windows product build contains no Bun content worker. The old
`components/content/` process remains as an explicit search and reader
integration fixture for scenarios that have not moved to the Rust controller.
The helper fixture forwards browser actions over its parent stdio connection.
The retained reader path owns original acquisition, cached originals and positions.
Reader translation acceptance uses the Rust/Qt scenarios above.
`tests/content-reading/ipc-integration.ts`
checks local text/CBZ, confirmed position writes, clean EOF, exact page/scroll restoration
in a new worker and cached originals through the real native browser parent.
The independent qplay product owns the player, prepared
profile, FFmpeg, ffprobe and yt-dlp. Content tests create their own profiles and
bridge discovery files; they do not use a signed-in application profile. Tool
paths come from the verified qplay `--runtime-info-json` response; fixtures do
not copy or modify its immutable engine.
Browser scenarios use the independent qcatch executable and extension.

Build the legacy fixture executable separately with Bun pinned in
`upstream-lock.json`. The example setup below is unverified for this change and
is not part of the release build:

```powershell
$env:QBUTT_CONTENT_APP_DIR = 'C:/path/to/legacy-fixture'
New-Item -ItemType Directory -Force $env:QBUTT_CONTENT_APP_DIR | Out-Null
Push-Location components/content
bun install --frozen-lockfile --ignore-scripts
bun build --compile --target=bun-windows-x64-baseline ./main.ts --outfile "$env:QBUTT_CONTENT_APP_DIR/qbutt-content.exe"
Pop-Location
```

Point the legacy scenarios at that fixture and qplay in the verified product
bundle:

```powershell
$env:QBUTT_CONTENT_BIN = "$env:QBUTT_CONTENT_APP_DIR/qbutt-content.exe"
$env:QBUTT_LAB_QPLAY = 'C:/path/to/portable/qplay/qplay.exe'
$env:QBUTT_QCATCH_EXE = 'C:/path/to/qcatch/qcatch.exe'
$env:QBUTT_QCATCH_EXTENSION_DIR = 'C:/path/to/qcatch/extension'
$env:QBUTT_QUICK_ACCEPTANCE_EXE = 'C:/path/to/build/qbutt-quick-acceptance.exe'
$env:QBUTT_QCATCH_SETUP_FIXTURE_EXE = 'C:/path/to/build/qcatch-fixture/qcatch.exe'
$env:QBUTT_LAB_PATHS = '1'
bun tests/content-search/catalog.ts
bun tests/content-search/progressive-integration.ts
bun tests/content-search/authorization.ts
bun run smoke:content-online
bun run smoke:content-reading
bun run smoke:content-browser
bun tests/content-browser/settings-native.ts
bun tests/content-browser/startup.ts
bun tests/content-browser/optional-cookies.ts
bun tests/content-browser/catalog-cookies.ts
bun tests/content-browser/publication-recovery.ts
bun tests/content-browser/resolve-health.ts
bun tests/content-browser/game-recovery.ts
bun tests/content-browser/networkproxy.ts
bun tests/content-reading/ipc-integration.ts
bun tests/content-reading/originals-integration.ts
bun tests/content-reading/branches-integration.ts
bun tests/content-reading/loading-integration.ts
bun tests/content-reading/ranobehub-integration.ts
```

Browser fixtures run the production Qt/Rust relay in an offscreen acceptance
process, with explicit temporary discovery and profile paths. The IPC scenario
checks the real qcatch native host, the pipe's current-user DACL, exact media
selection, malformed peers, cancellation and owned discovery cleanup.
The settings scenario uses a generated qcatch CLI fixture and direct legal
pipe peers to check the native controller's browser choice, Auto ambiguity,
setup callback and failure state. It never opens the extension setup page.
The startup scenario uses disposable per-process registry overrides: a missing
native host fails immediately, while a registered host reconnecting after 31
seconds completes the first exact capture without manual retry.
These generated scenarios do not establish a signed-in browser or live playback.

`bun tests/content-browser/capture-native-receiver.ts` runs the native qcatch
host against the production Qt/Rust capture receiver. Set `QBUTT_QCATCH_EXE`,
`QBUTT_QUICK_ACCEPTANCE_EXE`, `QBUTT_LAB_QPLAY` and `QBUTT_LAB_PATHS=1` as above.
Set `QBUTT_LAB_FFMPEG` and `QBUTT_LAB_FFPROBE` from that qplay bundle's verified
`--runtime-info-json` response, with its Qt runtime available on `PATH`.
The isolated fixture build temporarily allows loopback in
`src/content/src/http.rs`'s `public_address`; restore that source byte-for-byte
after building, and rebuild before using the executable outside the lab.
The scenario uses generated MP4, clear HLS, static DASH, external AAC and ASS.
It checks exact capture and session data, guarded Watch bytes, durable Download
acknowledgements, completed decoded media and subtitle style, Qt Downloads
state, and expired and cancelled selections without fallback jobs. Native
pointer QA remains a separate check.

Search, online-source and original-language tests make bounded real upstream
requests. A blocked source is an error, not evidence of an unavailable account
working. Browser IPC tests use generated fixtures; the separate Edge fixture
requires an isolated VM browser profile and native-host registration.
The VM browser and qcatch native host must inherit the same isolated
`QBUTT_BROWSER_BRIDGE_DISCOVERY` as the fixture; the fixture does not provide
its own native host.

The browser network proxy fixture runs the pinned extractor: redirects to local addresses must make zero
requests, and session cookies must remain scoped to their source host.

For torrent playback, deploy `qbutt-qt-acceptance.exe` beside the bundled Qt,
set `QBUTT_QT_ACCEPTANCE_EXE` to it and run `bun run smoke:media`.
Content and media scenarios require the absolute `QBUTT_LAB_QPLAY` path above.
The override is accepted only with `QBUTT_LAB_PATHS=1`; missing paths fail before
the fixture can discover a different installed player.
They use an isolated qplay profile through `--profile-dir` and do not modify
its bundled runtime.
Use the Python fixture environment above. The fixture checks incomplete MP4/MKV
ranges, actual decoding and seeking, restored priorities, cancellation,
completion leases and replay after the seed stops for v1, v2 and hybrid torrents.
Native pointer, appearance and hardware playback checks remain separate VM
acceptance; a headless decoder does not establish those results.

`bun tests/media-runtime.ts` exercises C++ qplay discovery/verification in the
MSVC developer environment with the same Qt acceptance, bundle and qplay
variables. Generated console probes cover missing/incompatible installations,
failed processes, the 64 KiB bound, the 30-second timeout and cancellation.
Select one with `--case=<name>`; these fault-only cases may use offscreen QPA.

The default run, `--case=native-playback` and `--case=cancel-and-retry` launch
actual qplay. Run them in the VM with `QBUTT_QT_ACCEPTANCE_NATIVE=1`; the runner
uses Windows QPA. Reuse a verified native qplay runtime with its matching build
manifest. `native-playback` checks the qbutt `Media::Player` launch, named-pipe
IPC, decoding, track selection, stop and replacement. Inspect the resulting
process PIDs and require that each has exited. These two Windows-QPA commands
still need a run after the launch guard change; pointer/GPU acceptance is separate.

The obsolete `media-audio.ts` Lua-trace fixture has been removed: native qplay
does not load its mpv.conf or trace script. Provider voice selection, retaining
a user-selected track and first-unpause/bookmark resume through the qbutt
bridge still need a generated native bridge fixture. The standalone qplay audio
fixture verifies player controls; it does not establish that provider handoff.

On Windows, run the lab from an elevated terminal with PowerShell 7 available.
Before launching a fixture, the shared runner registers persistent inbound firewall
rules for its canonical executable paths, including Bun and the actual Python
interpreter. This prevents firewall prompts when using a fresh temporary bundle.
Standalone native drivers must first run `bun tests/windows-firewall.ts <exe-path>`.
Rules belong to the `qbutt integration lab` group; firewall profiles stay enabled.

Point the lab at a built, deployed Windows executable with Qt's `qoffscreen.dll`:

```powershell
$env:QBUTT_LAB_EXE = 'C:/path/to/portable/qbutt.exe'
$env:QBUTT_LAB_APP_NAME = 'qbutt' # use qBittorrent for the upstream control build
bun run smoke:native
bun run smoke:native-inbound
bun run smoke:proxy
bun run smoke:network
bun run smoke:server-identity
bun run smoke:gateway
bun run smoke:repair
bun run smoke:repair-mappings
bun run smoke:staging
bun run smoke:staging-mappings
bun run smoke:storage-faults
bun run smoke:completion
bun tests/auto-remove.ts
bun run smoke:mixed
bun run smoke:mixed-baseline
bun run smoke:tunnels
bun run smoke:native-route
bun run smoke:route-policy
bun run smoke:policy-transition
bun run smoke:policy-transition-utp
bun run smoke:direct-transition
bun run smoke:direct-transition-utp
bun run smoke:discovery-transition
bun run smoke:path-auth
bun run smoke:transport-reserves
bun run smoke:connection-budget
bun run smoke:idle-peers
bun run smoke:path-dns
```

`smoke:policy-transition` uses `QBUTT_LAB_PATHS=1` and the selected physical
`QBUTT_LAB_NATIVE_INTERFACE` / `QBUTT_LAB_NATIVE_ADDRESS`. One active public
torrent receives complementary data through Native and two controlled SOCKS
paths, then switches Mixed → Tunnels only → Pinned. It checks OS socket closure,
a reachable Native canary, frozen retired relay counters and exact final hashes
without restarting the torrent. `smoke:policy-transition-utp` exercises the same
transitions with uTP-only peers, a positive UDP canary, retirement of the exact
native UDP endpoint, and frozen retired relay datagrams. A selected SOCKS UDP
association may remain alive for other traffic; it is not a peer connection.

`smoke:server-identity` requires `QBUTT_LAB_PATHS=1`. It verifies that two
protocols on the same configured host share one Edge, rejects duplicate active
paths and arbitrary identity overrides, then checks generation renewal and exact
payload hashes after changing the transport. Different configured hostnames do
not prove independent physical exits.

`smoke:transport-reserves` requires `QBUTT_LAB_PATHS=1` and the protocol-8 child.
One generated private torrent starts
on a primary transport, that endpoint closes, and an explicitly selected
same-server reserve must continue under a new generation with exact final hashes.
The child process remains active, and a slow public peer on an independent path
must retain its local port and generation while receiving payload. Private peers
must retain their original edge. The separate component fixture covers TCP/UDP health and
cancellation. These are local fixtures, without public outage or egress claims.

`smoke:discovery-transition` uses the same interface settings and three controlled
IPv4 DHT/HTTP/UDP tracker responders. It observes positive discovery traffic on
all three paths, then requires the retained paths to stay active while retired
responders receive no more packets through Mixed → Tunnels only → Pinned. UDP
phases use fresh tracker URLs with wire-visible markers; they do not establish
response acceptance or reannounce completion for an earlier pending request.
The same torrent finishes with exact hashes. Webseed transitions remain separate.

`smoke:idle-peers` holds two real TCP peers through separate managed paths for
12 seconds: one advertises wanted pieces but stays choked, the other is unchoked
with an empty bitfield. Neither may cause connection churn, route failures,
payload demand time or verified credit. Only the choked peer accrues choke time.
Releasing it must complete the exact payload over the original connection. This
bounded fixture does not establish idle-timeout or long-term stability behavior.

`smoke:direct-transition` and `smoke:direct-transition-utp` use the same physical
interface settings. An active torrent starts with a managed Native identity,
adds a competing relay in Mixed, and must retain its productive Native TCP or
uTP connection and UDP listener while verifying new pieces. Both variants then
enter fail-closed Tunnels only and return to Native with a fresh generation.
A reachable Native canary must receive no Tunnels-only connection. Returning to
Native must complete a new handshake with the original peer, receive at least
16 KiB of fresh payload, and verify another piece within 20 seconds without
adding the peer again. Final sizes and hashes must match; the torrent is never
stopped or restarted. Native remains visible in the path API after relay
retirement. An idle DNS helper may remain; its PID does not imply an active relay.

The shared lab selects its peer port by checking both UDP and TCP binds:
Windows can reserve different port ranges for each protocol.

`smoke:staging-mappings` covers manual and AutoTMM staged repair with separate
download/save directories and incomplete-file suffixes. It checks preview
cancel/restart, commit and native final relocation, rollback, recovery after an
actual resume-file write conflict, and migration of an older v2 journal whose
resume data names temporary storage. Use the Legacy resume backend for this
migration fixture. Sources and unknown files must survive; generated data is
removed after the owned app and lock helper stop.
Matching completion rules must remain held before commit, after rollback, and
while native resume persistence fails; after recovery they use the final native
destination and do not replay after restart. To repeat only one case, set
`QBUTT_STAGING_MAPPING_CASE` to `v1-manual`, `v2-autotmm-recovery`,
`hybrid-autotmm-rollback` or `v1-legacy`.

`smoke:storage-faults` creates owned 96 MiB expandable VHDX files and mounts them
under its temporary directory without drive letters. It verifies full storage,
missing/replaced volumes, long case-sensitive paths, third-party file handles,
and cleanup after a killed volume owner. Successful runs detach volumes and
remove generated data. `QBUTT_STORAGE_CASE` can select comma-separated cases:
`orphan`, `disk-full`, `detached-volume`, `replaced-volume`, `long-case`,
`sharing-conflict`. Case sensitivity is proved by distinct file contents, not
localized command output.

`smoke:native-inbound` starts the app with uTP only and no managed paths or
discovery. An independent checked libtorrent seed initiates the sole connection
to the app's ordinary IPv4 UDP listener; the app never receives `addPeers`. The fixture
checks the process's actual UDP bind, incoming/uTP peer flags, exact payload
sizes/hashes, seed upload accounting and zero seed downloads. Generated data and
profiles are removed after shutdown. For comparison with unchanged upstream,
set `QBUTT_LAB_APP_NAME=qBittorrent`, select its executable and run
`bun tests/network-lab/native-inbound.ts --baseline`; only qbutt's Paths API checks
are omitted. This checks local Native ingress, not gateway or Internet inbound.

Build and run the process-level Qt acceptance executable from the same configured
tree and deployed bundle:

```powershell
cmake -S . -B "$env:LOCALAPPDATA/qbutt/build" -DQBUTT_QT_ACCEPTANCE=ON
cmake --build "$env:LOCALAPPDATA/qbutt/build" --target qbutt-qt-acceptance
$env:QBUTT_QT_ACCEPTANCE_EXE = "$env:LOCALAPPDATA/qbutt/build/qbutt-qt-acceptance.exe"
bun run smoke:qt
```

For torrent playback integration, point the runner to the deployed Qt driver,
the hash-pinned fixture Python environment, a complete qplay portable runtime,
and the native qplay acceptance and Matroska fixture executables built from the
accepted qplay source. Supply the expected acceptance executable SHA-256 from
its build receipt; the runner verifies the actual file before starting:

```powershell
$env:QBUTT_QT_ACCEPTANCE_EXE = 'C:/path/to/deployed/qbutt-qt-acceptance.exe'
$env:QBUTT_LAB_PYTHON = 'C:/path/to/pinned-python/Scripts/python.exe'
$env:QBUTT_LAB_QPLAY = 'C:/path/to/qplay/portable/qplay.exe'
$env:QBUTT_LAB_QPLAY_NATIVE_ACCEPTANCE = 'C:/path/to/qplay-native-acceptance.exe'
$env:QBUTT_LAB_QPLAY_MATROSKA_FIXTURE = 'C:/path/to/subtitles-matroska-fixture.exe'
$env:QBUTT_LAB_QPLAY_NATIVE_ACCEPTANCE_SHA256 = '<approved build SHA-256>'
bun tests/media-streaming.ts
```

The generated v1/v2/hybrid torrents contain faststart MP4, trailing-moov MP4
and indexed Matroska with an authored ASS track. The suite verifies exact Range
bytes during incomplete downloads, seek/cancellation and priority restoration,
then full hashes/sizes and playback without peers. Strong ETags must survive
capability replacement; matching/list/wildcard If-Match requests succeed, while
stale or weak tags return 412 without payload. Numbered immutable command
responses avoid replacing a file while the Windows runner reads it. The native
qplay check uses the real torrent StreamServer URL and libmpv from the supplied
portable runtime to render ASS translation pixels in a hidden Qt scene, verify
authored style and restore Original while the torrent is incomplete. Accepted
translations are preseeded in the isolated cache; this does not exercise live
inference or the qbutt product-window handoff. The other mpv checks use null
audio/video outputs.

For a short appearance-only acceptance on the same executable, use
`bun run smoke:appearance`. It needs no Python, torrent data or transport child.
Three offscreen app processes check a fresh System-theme profile under a
controlled dark Qt color scheme against `docs/ui-default-layout.json`, change
layout and select Light through Qt
controls, verify those settings after restart, and check an independent
functional Light/Fusion profile. Header order, hidden state, logical widths,
Files tab, sidebar action, palette and options controls are asserted; the
stretched last Files column adapts to its viewport. PNGs and JSON evidence stay
in the printed temporary directory; successful profiles and the copied Qt
runtime are removed.

For `bun run smoke:diagnostics`, copy the Qt driver into an isolated copy of the
deployed `portable` bundle and point `QBUTT_QT_ACCEPTANCE_EXE` to that driver.
Set `QBUTT_LAB_PYTHON` on Windows x64. The raw CMake build executable above is
the source for `smoke:qt`, which stages its own runtime. The diagnostics lab
holds a generated file with a Windows oplock to block a real asynchronous disk
write, and separately applies a real per-torrent bandwidth limit. Production
counters and Qt reasons must identify each wait, clear after release, remain
responsive, and finish with exact file
sizes and hashes over one peer connection. It keeps screenshots and compact
evidence, then releases the lock and removes the owned profile and payload.
This is process-level Light/Fusion acceptance, not physical desktop interaction.

The `qbutt-update-acceptance` CMake target builds the production update service,
dialog and a Qt process driver. `bun run smoke:qt` stages its Qt runtime before
running `tests/qt-acceptance/release-fixture.py`. For a manual fixture run, put
the driver in an isolated copy of the deployed `portable` bundle and pass the
driver and OpenSSL executable paths. Register driver/Python with
`bun tests/windows-firewall.ts <driver> <python>` before the socket fixture.

The fixture checks version ordering, GitHub metadata, complete ZIP downloads,
corruption, cancellation, interrupted transfers, redirects and certificate
rejection. Failed downloads preserve an existing destination. Test CA trust is
limited to the driver process; generated keys and payloads are cleaned afterward.
No release sidecar files are used. The driver is not shipped. To include installed
update cases, pass a third argument: a copy of the driver named `qbutt.exe` in an
isolated runtime without an adjacent `profile` directory. Register that stable
path in the firewall before running. Its installation marker is redirected to a
temporary process-local registry key; the real uninstall key is untouched. Cases
cover automatic EXE downloads, cache reuse and corruption, cancellation, repeated
checks, and refusal to execute an installer changed after downloading.

`<driver> live <temporary-output-path>` checks the real GitHub API. A driver built
with an older qbutt version can use `live-download <temporary-zip-path>` after a
new release. Portable ZIP installation remains manual. Installed builds download
the EXE in the background and expose an update-and-restart action. The full Qt
driver's `installed-update` mode exercises this main-window action with an isolated
profile and stopped torrent; `QBUTT_UPDATE_APPLICATION_ARGS` (JSON argv) and
`QBUTT_UPDATE_FIXTURE_SETUP` let the HTTPS fixture serve a locally built installer.
Use an installer fixture with its own AppId for this process-restart scenario;
never replace a live installation. `qbutt-version.txt` controls the independent qbutt
version, UI and archive name.

`tests/qt-acceptance/real-installer-update.py` checks the full native Windows
handoff with Python `psutil`, the pinned ISCC and the HTTPS fixture. Pass builds of
`qbutt-qt-acceptance` from v1.0.1 and the fixed source, plus their portable
bundles, with `--old-driver`, `--fixed-driver`, `--old-bundle`, `--fixed-bundle`,
`--source`, `--iscc` and `--openssl`. It expects the old tray-active process to
miss the automatic exit, then verifies that the fixed producer exits for a
genuine Inno replacement and automatic relaunch. Each setup has a unique test
AppId. Successful runs remove their install, profile and cache while retaining
compact evidence; failed runs retain the fixture for diagnosis.

`smoke:qt` launches the real application offscreen with a new profile. It adds
existing files through the normal torrent dialog, checks them before completion,
then exercises stopped-torrent repair with source mappings, explicit consent and
staged commit. It also covers selected managed nodes beside Direct, the master
network switch and its saved state across disabled and enabled restarts,
completion policies, bounded
diagnostics export, and a 2,000-row transfer list. The runner creates and cancels
a 30,000-file source search, measures event-loop response, checks payload snapshots
before consent, and verifies final bytes and preserved unknown files. Set
`QBUTT_QT_ACCEPTANCE_BUNDLE` only when the deployed runtime is
not in the build tree's `portable` directory. This is process/UI-model acceptance,
not physical desktop interaction or public-network evidence.

To exercise the real qbutt-net child, place its pinned binary beside the app and
run `smoke:network` with `$env:QBUTT_LAB_PATHS = '1'`. This adds node listing,
session transition guards, termination of the owned child PID, blocked-path
retention, a new generation on retry, and explicit stop/return to Native. The
adapter connects only to the lab SOCKS server through the Windows loopback
interface; this is lifecycle evidence, not a VPS egress probe.

The qbutt-net component integration covers configured server identity,
DNS case/IDNA, IPv4-mapped/IPv6 normalization and configuration changes before
opening a path. Protocols on the same configured hostname or IP share one server;
different hostnames remain distinct identifiers and do not prove independent
physical servers or public exits. `smoke:qt` covers grouped server selection,
preferred protocols and actual route generations in the desktop interface.

`smoke:wan` uses an explicitly selected SSH observer (`QBUTT_WAN_OBSERVER`,
`QBUTT_WAN_OBSERVER_IP`), three standalone subscription nodes
(`QBUTT_WAN_PROXY_CONFIG`, pipe-separated `QBUTT_WAN_PROXY_NAMES`), and the physical
`QBUTT_LAB_NATIVE_INTERFACE` / `QBUTT_LAB_NATIVE_ADDRESS`. It transfers one generated
16 MiB torrent through four simultaneous TCP paths with disjoint piece sets.
The observer records the actual source IPs; all four must differ, and the local
file must match its exact size and SHA-256. SSH runs a temporary stdlib Python
peer with a 240-second watchdog; no production configuration is modified.
Payload, the private node copy and remote script are removed after the run;
compact evidence and torrent metadata remain. This proves controlled WAN TCP,
not discovery, UDP or a general speedup.

`benchmark:public-swarm --qbutt-only` compares qbutt Native, one tunnel and Mixed
without an upstream control executable. Such reports explicitly omit any
upstream performance claim. The default comparison still requires the pinned
upstream executable. Both successful and failed windows clean their payload.
For a focused reproduction, `QBUTT_PUBLIC_SWARM_MODES` selects a comma-separated
subset of the available mode names; it does not establish comparisons with
omitted modes. `QBUTT_PUBLIC_SWARM_PROTOCOL=both|tcp|utp` applies the same transport
setting to every selected mode (default `both`). Failed runs retain bounded
route/candidate observations before stopping the application, without node names
or credentials.

`smoke:native` checks selective download, pause/resume, clean process restart,
recheck, exact lengths/hashes and payload preservation when removing a torrent.
It also records the existing recheck limitation: an overlong file can be fully
hash-valid while retaining its extra tail. Every native launch uses offscreen Qt,
an explicit fresh profile, ephemeral WebUI credentials and loopback-only WebUI.
Public discovery, port forwarding, update checks and GeoIP updates are disabled
in that profile before launch.

`smoke:repair` requires the qbutt Repair API. It checks read-only snapshots,
explicit consent and operation identity, held write exclusion, corruption,
extra tails, truncation, renamed mappings, inserted bytes, mutation after an
index snapshot, preservation of unknown files, and hardlink/reparse rejection.
It checks authentication even with localhost exemption enabled, exact verified-byte
accounting, and missing nonzero and zero-length targets. An absent empty target,
including its absent parent directories, is created only after consent under the
same exclusive guard used for apply; a target or mapped parent directory appearing
after analysis is rejected.
It drives the real session through standard recheck and download after apply.

`smoke:repair-mappings` checks selected in-place repair with separate download/save
directories and native `.!qB`/`.unwanted` mappings for v1/v2/hybrid. Analysis and
cancel/restart preserve the data; apply/recheck leaves ignored tails and missing
ignored empty files unchanged. Ordinary resumed download restores normal file
names/location and verifies selected sizes/hashes after restart. Its usual v1
boundary-piece writes are distinct from the selected-only repair operation.
An AutoTMM variant also freezes shared default/category roots during ownership,
preserves AutoTMM across cancel/restart and allows ordinary category relocation
again after repair finishes.

`smoke:repair-index` reuses five renamed source files after changing the target's
names, order and piece length from 16 to 64 KiB. The production metadata index
selects the sources, target-layout hashing verifies every piece, and staged
prepare/commit/restart must preserve sources and unknown files while completing
exact sizes/SHA-256 with zero downloaded payload. No peer or webseed is supplied.
`bun tests/repair/index-layout.ts --v2` runs the same lifecycle with a pure v2
target. Unchanged file roots are preserved while target piece layers are rebuilt
for 64 KiB pieces. Preview verifies whole-file roots; native staging recheck
verifies the target pieces. This does not claim v1-style cross-file piece reuse.

`smoke:staging` uses independent copies and the same native downloader for full
and selected v1/v2/hybrid targets. It checks a read-only plan, renamed-source
indexing, source write exclusion, allocation estimates, exact selected commit,
unknown-file preservation, restart, and ordinary download after selecting an
uncommitted file. Source hardlinks/reparse points and new staging hardlinks are
rejected. A same-size mutation with the original Windows mtime restored must fail
the commit's hash verification while preserving the original installation.
Full target, source, unknown and retained transaction files are checked again
after torrent removal and clean app shutdown. Only the current operation's
directory is excluded when comparing the original installation.

`smoke:staging-faults` requires a separate integration build configured with
`-DQBUTT_STAGING_FAULTS=ON` (default OFF; never enable it in a release). It terminates
the actual app after journal publication and before, during and after each file
rename. The 68 restart cases and 34 forward-recovery cases use the normal
profile, keep ordinary writers suspended, and verify hashes, exact
sizes, original files and unknown files. Four additional cases cover absent
targets and v2/hybrid restart. After each successful case, the stopped fixture's
generated target is removed; compact checks remain in the lab evidence.
A failed case retains its profile and payload for diagnosis.
Writer rejection is observed across more than two native info-cache refreshes.
`QBUTT_STAGING_CASE` selects a single
checkpoint, for example `committing:0:backed_up:renamed` or
`committing:0:backed_up:renamed@commit`. Fault builds exit 197 at that checkpoint.
Finalization means the stopped destination resume data received a durable storage
receipt and the active journal was retired, not merely that file renames ended.
The finished manifest remains in the profile's staging directory, and original
backups remain in the destination's operation directory. No recursive cleanup
is part of these operations.

`smoke:storage-faults` uses two isolated 96 MiB NTFS VHDX files under its fresh
temporary lab, mounted one at a time at the same private directory without a
drive letter. It verifies that newly exhausted space cannot select in-place or
create staging, a detached target fails closed, and a different VHDX mounted at
the same private mount path is rejected by the planned volume/directory/file
identity before the first write. A real Windows sharing conflict must stop commit
before any rename and remain recoverable after the handle is released. The same
suite commits through a path longer than 260 characters in a case-sensitive
directory while preserving two unknown files whose names differ only by case.
Run it for both Legacy and SQLite resume backends from an elevated shell. The
fixture also kills the process owning a third, 32 MiB VHDX and proves that the
next preflight removes only the exact marked mount and image. Every path is
validated before creation, detachment or deletion.

Set `QBUTT_LAB_RESUME_BACKEND` to `Legacy` (default) or `SQLite` before a suite
to exercise that native resume store. Startup verifies the requested preference.
The following additional integration fixtures use the ordinary production build:

```powershell
$env:QBUTT_LAB_RESUME_BACKEND = 'SQLite' # repeat with Legacy
bun tests/repair/staging-journal.ts
bun tests/repair/staging-receipt.ts
```

The journal fixture corrupts persisted selection, identities, mappings and state,
then requires rejection without changing the active journal or any payload after
shutdown. It restores the valid journal and checks recovery and replay rejection.
Version 1 journals remain readable for rollback, but cannot resume a forward
commit without the volume, directory and file identities recorded by version 2.
The receipt fixture holds a real Windows sharing conflict on `.fastresume`, or a
SQLite writer transaction on the owned profile database. Commit must retain the
journal and suspend ordinary writers when final resume persistence fails. After
the lock is released, restart and recovery must complete the receipt and retire
the active journal. These fixtures never lock an existing user profile.

`smoke:completion` proves that normal completion exits an isolated app when that
action is enabled, while a held repair prevents auto-exit after another torrent
finishes. It does not inject the queued-signal or nested-dialog race windows.

`smoke:profile` uses the actual native resume stores and startup import service.
Build its standalone driver against the same configured application build, with
`CMAKE_EXPORT_COMPILE_COMMANDS=ON`. Point both DLL and plugin lookup at the deployed
application bundle so the separate driver can load the same SQLite plugin:

```powershell
bun tests/profile-lab/build.ts C:/path/to/build C:/path/to/drivers C:/path/to/msvc-env.cmd
$env:QBUTT_PROFILE_DRIVER = 'C:/path/to/drivers/service.exe'
$env:PATH = 'C:/path/to/portable;' + $env:PATH
$env:QT_PLUGIN_PATH = 'C:/path/to/portable'
bun run smoke:profile
```

The MSVC environment script must initialize the x64 compiler and linker. The
profile lab covers Bencode/SQLite source and destination combinations, portable
paths, incomplete filename mappings, writer exclusion, native recheck/restart,
and a forced process exit during installation. An incomplete rollback backup
must fail before removing current metadata. Source bytes and modification times
must remain unchanged. The same imported records exercise matching completion
rules across native recheck and restart: only an explicitly acknowledged task
may be removed, its data must remain, and the action must not replay. Successful
fixtures remove their generated profiles and payloads while retaining compact
evidence. Use ordinary physical temporary paths for payloads;
repair ownership guards deliberately reject paths through junctions or symlinks.

The optional `gui-smoke` argument to the driver builder produces a real Qt
offscreen import dialog driver. Pass generated source settings, data directory,
source profile base, and a fresh output directory. It exercises file selection,
preview, destination editing, consent, asynchronous preparation and close guards,
then saves PNGs and reads back the staged native records. Neither driver needs a
live profile. Windows external SQLite import supports schema 9 and snapshots
the source DB/WAL under write-excluding handles; unsupported schemas fail before
installation. Torrents must have complete metadata before importing them.

`smoke:profile-qt` joins these boundaries in four real application processes:
the Tools import action rejects unsupported SQLite schema and incomplete torrent
metadata, then prepares three generated native tasks; restart imports them stopped
with manual management; native recheck and another restart preserve their policy
preview requirement; the real Policies dialog acknowledges exactly one task.
Its matching Remove torrent rule keeps payload files, the other tasks remain held,
and a final restart must retain the same action receipt without replay. Qt property
checks and offscreen Light/Fusion PNGs accompany byte/size/mtime checks of every
source profile and payload. Set `QBUTT_QT_ACCEPTANCE_EXE` to the current
`qbutt-qt-acceptance` build, `QBUTT_QT_ACCEPTANCE_BUNDLE` to its deployed Qt runtime,
`QBUTT_PROFILE_DRIVER` to the service driver above, and `QBUTT_LAB_PYTHON` to the
fixture interpreter. Run `bun run smoke:profile-qt`; no live profile is used.

`smoke:proxy` exercises the bounded authenticated TCP fixture relay using real
sockets. `smoke:network` drives the native client through that relay to a seed and
HTTP tracker whose synthetic endpoints have no direct listener, kills the relay
mid-download, observes a drained stall, restarts it, and verifies the payload.
Relay stream counters include BitTorrent/HTTP protocol bytes; they are neither
unique verified payload bytes nor packet-level wire counters. The final SHA-256
and exact-size checks count verified payload separately.

`smoke:gateway` requires `QBUTT_LAB_GATEWAY_SOURCE` to name a qbutt-net checkout
whose exact `HEAD` equals `upstream-lock.json.qbuttNet.commit`. The driver builds
the real qbutt-net and qbutt-gateway executables from that checkout, copies the
portable application to a fresh runtime, and places a transparent recorder in
front of the real child. The recorder verifies exact protocol 7 request, result,
error, `incomingTcp` and terminal `gatewayClosed` shapes without storing relay tokens, certificate paths or
proxy credentials. A generated mTLS identity, controlled SOCKS route and local
gateway use an ActiveStore `/32` on the Windows loopback interface; the address is
validated before assignment and removed during cleanup. Persistent firewall rules
are registered for every executable before a listener starts.

The gateway configuration is applied through the authenticated `qbuttPaths/gateway`
product API before the path opens. The source checkout must be fully clean, including
untracked files, before either Go binary is built. Do not run this fixture concurrently
with other socket labs: a 2026-09-13 parallel run exhausted all 16 old ephemeral
TCP/UDP port probes before qbutt started; the isolated rerun passed. The fixture
now probes up to 128 explicit dynamic-port candidates. This test-only port handoff
is outside qbutt-net's `listenLease` retry boundary.

The independent generated libtorrent seed initiates the only peer connection to
the leased endpoint. The test requires trusted route path/generation telemetry,
payload counters and final exact hashes/sizes, then observes renewal without a
generation or endpoint change. An asynchronous carrier loss must emit the bounded
terminal event and retire public route descriptors before reconnecting a later
generation. The leased address family is explicit. The real
child's relay and carrier byte counters must advance independently from verified
bytes, while its UDP packet and fanout counters stay zero in this TCP-only scenario.
It explicitly closes and reopens the selected edge,
stops the gateway carrier, requires the terminal `gatewayClosed` event, and verifies that the
application retires the old descriptor before opening a later outgoing-only
generation. This proves application integration through a controlled host-local
gateway. It does not prove public-Internet reachability or NAT/firewall traversal.

`smoke:gateway-utp` runs the same lifecycle with a UDP-only public lease and
uTP-only peers. It requires exact payload hashes/sizes, the seed's original
source port and path/generation, gateway datagram counters and selected SOCKS
adapter traffic, with no `incomingTcp` event. The single gateway port accepts
plain uTP; TLS-uTP stays outgoing-only because its initial SYN cannot distinguish
TLS from plain uTP on the same public port. Run TCP and uTP variants sequentially.
Both remove their generated payload, credentials and copied runtime after stopping
owned processes; compact evidence remains. These local fixtures do not establish
Internet UDP reachability or IPv6 support.

`smoke:gateway-ipv6` and `smoke:gateway-ipv6-utp` exercise the same TCP and uTP
checks with owned temporary IPv6 /128 aliases for gateway and seed on Windows
loopback. They assert bracketed endpoint preservation and IPv6 route family,
then remove both addresses. The control/carrier sockets remain IPv4 loopback; these scenarios
prove the IPv6 payload endpoint contract, not public Internet IPv6 reachability.

`bun tests/network-lab/gateway.ts --utp --trackers` additionally checks HTTP and
UDP announces against the actual leased public socket. A numeric IPv4 HTTP
tracker retains only its matching family even with dual-family policy. The first
UDP connect/announce must leave the leased socket without prior inbound contact.
After lease retirement, both tracker protocols advertise outgoing-only port 1
without a public address; a preflighted Native UDP canary must remain untouched.

`bun run smoke:gateway-wan` is a separate controlled TCP ingress probe. Set
`QBUTT_WAN_OBSERVER`, `QBUTT_WAN_OBSERVER_IP`, `QBUTT_LAB_NATIVE_INTERFACE`,
`QBUTT_LAB_GATEWAY_SOURCE` and `QBUTT_LAB_EXE`. The qbutt-net source must be
fully clean at the lock revision. The driver cross-builds the gateway server
locally, copies it and one-hour generated TLS credentials into a unique
`/tmp/qbutt-gateway-*` directory on the observer, and runs only high-port
gateway and Python peer processes there, each with a 240-second watchdog. It
changes no remote firewall, routing, service or production configuration.
The observer's peer initiates the only BitTorrent connection to the leased
public IPv4 endpoint. The home application must attribute its exact original
source IP and port to the active path generation and verify the generated
payload's size and SHA-256. The test also checks real relay/carrier counters
and terminal lease retirement. Both gateway and peer run on the same remote
host, so this proves remote-host ingress to home qbutt through its gateway
carrier, not reachability from an independent third-party Internet host.
Blocked remote high ports fail explicitly; the fixture does not open them.
Set `QBUTT_GATEWAY_WAN_PROXY_CONFIG` to a local ordinary Mihomo YAML and
`QBUTT_GATEWAY_WAN_PROXY_NAME` to one node in it to run the same bounded peer
on the home machine through a separate, authenticated qbutt-net path. The
fixture imports only that node into an ephemeral configuration and does not
change the running Mihomo or its routing. An owned one-request HTTP listener
on the observer first checks that the selected node's public source IP differs
from home and observer. Only then does the gateway probe start. Before
uploading, it reads the exact accepted source IP and port from a scoped
observer TCP SYN capture for the owned lease, requiring that source IP to differ
from the observer and the home SSH origin, and checks that qbutt reports the
same endpoint on the active path generation. The peer exchanges BitTorrent
handshakes first to trigger lazy VPN dialing, but remains choked until the
kernel observation succeeds. This proves TCP ingress from an
independent VPN exit, with the peer process still physically local; it does not
prove a physically separate third-party host or UDP ingress.

With the independent source configured, `bun tests/network-lab/gateway-wan.ts
--utp --dht` uses an owned UDP lease and a pinned native uTP source. It requires
the exact source endpoint observed at the public port, a hash-checked 512 KiB
download, and correlated inbound DHT `ping/get_peers` replies. The bounded
uTP capture retains only fixed headers in both directions. Before using a real
VPN, `bun tests/network-lab/gateway-wan.ts --source-preflight` isolates the same
source behind authenticated loopback SOCKS and checks exact uTP delivery.
These probes do not establish physical carrier bypass around a system VPN or
BEP42 node-ID enforcement.

For public IPv6 ingress, also set `QBUTT_WAN_OBSERVER_IPV6` to the observer's
numeric global IPv6 address and run `bun tests/network-lab/gateway-wan.ts --ipv6
--system-source`, then the same command with `--utp --dht`. Keep the IPv4
observer address for authenticated control/carrier sockets. The source peer
uses an owned authenticated SOCKS relay restricted to that one IPv6 lease;
the relay follows the existing OS route, including an already active VPN.
This mode cannot be combined with a selected source adapter or `--trackers`.
It does not change system interfaces, VPN configuration or routing.

The IPv6 scenarios require a remote source in scoped packet evidence, original
endpoint/path/generation in qbutt, exact 512 KiB payload and SHA-256, and terminal
lease retirement. UDP also checks correlated DHT replies and uTP headers on the
public interface. They establish public IPv6 ingress through the current route,
not a physical system-VPN bypass, ISP CGNAT, IPv6 tracker announces or another
self-rejection test. Those contracts have their own scenarios above.

Add `--trackers` with a different SSH host in `QBUTT_WAN_TRACKER` and its numeric
IPv4 address in `QBUTT_WAN_TRACKER_IP` to verify announcements on an independent
public tracker. A bounded temporary Python process serves HTTP and UDP on two
random high ports. The same tracker URLs must advertise the active gateway
endpoint, then clear it after gateway retirement and report the new generation.
UDP's observed source must match the active lease exactly; the retired announce
must use a different source and outgoing-only identity. Both protocols must agree
on peer ID/key and have exactly one accepted endpoint in the current generation.
Before the independent seed starts, both trackers return the client's own leased
endpoint. The app must explicitly reject it without payload, live peer or route
failure credit. The trackers then clear that peer, and the independent ingress
must still deliver the complete hash-verified payload. DHT packet attribution
excludes only replies from the fixture's exact UDP tracker endpoint.
The fixture enables all-tracker announces explicitly and removes its profile,
payload, temporary bundle and remote processes/files after either outcome.

The final JSON line points to `evidence.json`, containing exit outcome and checked
observations; native and seed logs remain beside it. Failed and unsupported
filesystem scenarios are explicit. Credentials are never printed. Generated
profiles, payloads and logs are temporary artifacts, not repository source.

`smoke:mixed` requires the route-aware application, bundled qbutt-net,
`QBUTT_LAB_NATIVE_INTERFACE`, and `QBUTT_LAB_NATIVE_ADDRESS`. Three exclusive
authenticated routes and one physical Native route expose complementary subsets
from independent partial files. One application torrent must receive all four
subsets concurrently, report the original peer/path/generation through native
telemetry, and finish with exact hashes and sizes. The Native seed accepts only
clients from the configured physical address and must observe that address. The
test establishes every exact peer/path/generation while the seeds are rate-limited,
proves all four counters advance in one snapshot, restarts the app, and requires
the persisted fail-closed policy. This proves application binding on the host;
public egress and physical-wire routing require a separate environment.

`smoke:tunnels` uses the same complementary peers without a Native route. It
starts a public torrent while Pinned, proves that the first edge can obtain only
its own subset, changes the live job to Tunnels only, and verifies completion
through both authenticated routes. It then repeats concurrent and failed-route
retry coverage with the persisted Tunnels-only policy.

`smoke:native-route` isolates that physical binding from route selection. It
requires the same Native interface variables, transfers a generated public
torrent through only that route, verifies exact payload, and requires the seed
to observe the selected source address.

`smoke:managed-native-inbound` and `smoke:managed-native-inbound:utp` keep that
public torrent in Mixed mode while an independent seed on the selected local
physical address initiates the only peer connection. They inspect the app's
actual physical TCP and UDP listeners, require incoming Native route attribution
and exact verified payload, then require Tunnels Only to retire both listeners.

`smoke:route-policy` drives the standalone libtorrent integration executable.
Set `QBUTT_POLICY_EXE` to the built `route-policy-integration.exe` and
`QBUTT_PUBLIC_IPV4` to a synthetic public IPv4 identity; this local
fixture does not contact that address. Set `QBUTT_LAB_NATIVE_ADDRESS` to a local
non-loopback IPv4 address when automatic interface selection is unsuitable. It
checks live policy replacement for HTTP/UDP trackers and DHT generations against
physical Native listeners. Managed DHT answers on its physical UDP socket; DHT
and UDP tracker packets leave the corresponding listener port. A same-generation
preferred-port rebind closes the old socket, moves DHT and tracker traffic to the
new port, and preserves the Native route. After managed routes are detached, the
retired scoped DHT context rejects a new node and ordinary DHT answers on both
physical listeners. The fixture also rejects a stale tracker reply, a hostile
session-wide announce address, and mismatched tracker peer IDs or keys. Anonymous
announces suppress addresses while preserving the rebound peer port. It checks
an authenticated SOCKS webseed, an unaffected default session, and managed uTP
with exact payload bytes and the physical UDP source port. The local DHT packet
source is loopback; public reachability requires the external gateway scenario.
Two dual-family SOCKS contexts also announce a hostname tracker exactly once per
context for each start/stop, retaining the leased public listener and the separate
outgoing-only context. The hostname maps deterministically to a loopback IPv4
server in the SOCKS fixture; this does not measure DNS queries or Internet egress.

`smoke:ssl-utp` uses the standalone `ssl-utp-integration` target from
`tests/network-lab/CMakeLists.txt`. Set `QBUTT_SSL_UTP_EXE` to that executable;
Go and OpenSSL must be on PATH or supplied through `QBUTT_GO` and
`QBUTT_OPENSSL`. It generates a torrent-local CA and peer certificate, transfers
512 KiB through authenticated SOCKS UDP, retires the first live generation,
rejects that stale generation, and completes through its replacement. It checks
`utp_ssl`, per-generation verified bytes, exact payload, and a direct UDP canary.
This controlled IPv4 scenario covers outgoing SSL-uTP, not public inbound or WAN.
Temporary keys and payload are removed; compact evidence stays in the lab folder.

`benchmark:network` runs three or more interleaved rounds for the validated
unchanged upstream executable, qbutt Native, one authenticated tunnel, and
RouteSelector Mixed with two tunnels plus the selected physical Native route.
Set `QBUTT_BENCH_BASELINE_EXE`, `QBUTT_BENCH_QBUTT_EXE`,
`QBUTT_LAB_PYTHON`, `QBUTT_LAB_NATIVE_INTERFACE`, and
`QBUTT_LAB_NATIVE_ADDRESS`. The control executable must match the pinned hash in
`docs/baseline.md`; override the default four counterbalanced rounds with
`QBUTT_BENCH_ROUNDS=3..9`. Every route starts at a 1 KiB/s warmup cap, then
uses the same acknowledged `QBUTT_BENCH_ROUTE_RATE` cap (32–512 KiB/s).
Evidence records connection setup and end-to-end completion separately from timed
goodput, exact verified bytes, per-route seed and relay counters, redundant
payload, and WebUI response latency. The transfer window also records CPU time,
sampled working-set/private-memory peaks, and process I/O for the exact app and
qbutt-net process handles. CPU percentages use one core as 100%; memory samples
are taken every 250 ms and I/O includes network operations, not only disk. The
receipt includes sampler boundary skew and gaps; fixture processes are excluded.
Relay stream bytes include protocol data and are not wire bytes. The local
single-host TCP topology proves only the stated controlled comparison; its
evidence lists the untested public, UDP, inbound and physical last-mile cases.

`QBUTT_BENCH_SCENARIO=shared-network-cap` sends every controlled Native and SOCKS
peer through one shared downstream TCP byte budget outside the application.
qbutt's application download limits must remain disabled, and the same rotating
Native/Mixed windows verify exact payload and source/relay counters. This emulates
a common downstream bottleneck; it does not modify a physical router or Koala.

`QBUTT_BENCH_SCENARIO=shared-cap` runs only qbutt Native and Mixed, with a single
torrent-wide application download limit shared across every path and faster
fixture sources. Three or more rounds require both median useful rates to reach
70–110% of that limit and Mixed to exceed Native by no more than 10%. This models
an aggregate application bottleneck; it does not emulate a physical last mile.
`QBUTT_BENCH_SCENARIO=failed-path QBUTT_BENCH_ROUNDS=1` runs one Mixed window:
after warmup the first relay closes, while another relay can reach the same
peer. The peer must reconnect automatically through the healthy path without
policy changes or peer reinsertion, preserve the unrelated Native connection,
and complete the exact payload. Interrupted-block retransmission bytes are
reported separately. Each new fixture removes its generated payload, profile
and proxy config after its owned processes stop; compact evidence remains.

`QBUTT_BENCH_SCENARIO=static-comparison` compares a separately built static
selector with the normal Mixed selector. Set `QBUTT_BENCH_STATIC_EXE` and its
`QBUTT_BENCH_STATIC_RECEIPT`, plus the usual qbutt/Python/Native settings. The
receipt must identify the actual static and normal executable SHA-256 values and
the static patch, plus the shared compatible `qbutt-net.exe` SHA-256; both app
binaries and both companion children are rejected unless they match that exact
pair. Keep each app beside its pinned child instead of borrowing a newer runtime.
The experimental binary replaces only public route selection with FNV-1a over
both infohash slots and the numeric peer endpoint, modulo eligible routes; keep
that patch and build provenance with the local receipt, outside production code.
Within each window, six full peers use the same endpoints through Native and both
SOCKS paths; the fixture allocates new endpoint ports for each window.
The fixture chooses two endpoints per static bucket and waits for useful payload
from every peer before applying an equal 8 KiB/s per-peer limit. Four rounds
alternate mode order; actual assignments, verified goodput, resource counters and
redundant payload are recorded. Equal peer limits make this a neutral comparison
with no expected adaptive speedup; latency and loss are not controlled, and the
fixture does not test all 18 possible peer/path pairs.

`QBUTT_BENCH_SCENARIO=static-unequal` reuses that pinned static-selector binary
and compares only route choices for fresh dials after controlled training. Three
partial peers start at 96 KiB/s from their first dial and cover the three routes
once; the training counters span their full history through disconnect.
Independent downstream stream
limiters at 48, 16 and 8 KiB/s then provide at least 64 KiB of the same
verified-bytes/demand signal consumed by RouteSelector, after which those
connections close. The fixture rechecks, removes the torrent without deleting
data, re-adds it and rechecks again. It requires identical verified piece maps,
one application session, unchanged path generations and exactly nine subsequent
admissions. These successive torrent instances isolate fresh dials from training
peer retries; they do not model disappearing peers in a live swarm. Nine fresh
full peers, three from each static hash bucket,
are reachable through every route and use independent 8 KiB/s source caps.
Evidence records the training signal, per-route limiter deltas, initial path for
every measured peer, assignment-derived throughput ceilings and paired goodput.
The static control must keep a 3/3/3 assignment. A successful fixture receipt
means the topology, counters, stable assignments, exact payload and hashes were
valid; `adaptiveAssignmentObserved` and `adaptiveSpeedupProven` remain explicit
summary results rather than pass conditions. The latter requires at least six of
nine peers on the fast path and a 1.2x assignment ceiling in every round, sane
70–110% utilization of both assignment ceilings, at least 10% paired goodput gain
in three of the four default rounds, and at least 15% median goodput gain.
This scenario tests later connection selection, not migration of a live slow
peer, UDP, WAN or a physical last mile. Preserve the static patch/build receipt
with both executable hashes so unrelated application changes cannot be mistaken
for selector performance.

Warmup and final byte counts use verified pieces and the exact last-piece length;
the Web API's completed counter can also include unverified partial pieces.
Limiter accounting starts before measurement dials to include warmup blocks that
may only become hash-verified during the timed window. A one-round diagnostic
never sets `adaptiveSpeedupProven`, even when that pair meets the numeric gates.

`benchmark:public-swarm` uses the pinned official Ubuntu 24.04.5 live-server
torrent and a separate empty profile for every bounded window. Set
`QBUTT_PUBLIC_SWARM_CONTROL_EXE`, `QBUTT_PUBLIC_SWARM_NATIVE_INTERFACE`, and
`QBUTT_PUBLIC_SWARM_NATIVE_ADDRESS`. Add `QBUTT_PUBLIC_SWARM_QBUTT_EXE` for the
counterbalanced upstream/qbutt Native comparison. The executable paths must be
stable: firewall rules are registered before launch and remain keyed to those
exact files. Optional one-tunnel and Mixed windows require an ordinary Mihomo
file in `QBUTT_PUBLIC_SWARM_PROXY_CONFIG` and pipe-separated node names in
`QBUTT_PUBLIC_SWARM_PROXY_NAMES`; credentials and node names are not copied to
the fixture JSON.

All sequential windows use the same `QBUTT_PUBLIC_SWARM_PEER_PORT` (default
45123), checking TCP/UDP availability before every launch. Changing the port
between modes can change actual network reachability and invalidate a comparison.

The default suite runs three upstream-only windows or four rotating comparative
rounds. A logical window may make up to three attempts; a timeout or WebUI failure
is recorded in `rejectedAttempts` and never enters the summary. Each accepted
window stops and flushes the client, reads every completed piece from disk, and
checks its SHA-1 against the exact pinned torrent. The full ISO SHA-256 is only
recorded as the expected upstream value because the bounded fixture deliberately
does not download the full image. Client transfer and qbutt path payload counters
are reported separately and are not packet-level wire bytes. The verified-rate
metric counts pieces that become verified during the window; any partial blocks
received during warmup are not separable and this limit is recorded in evidence.
Public results show external applicability and variability; release thresholds
still come from the controlled benchmark.

`--shared-peers` captures up to 64 connected public IPv4 seeds from the first
accepted upstream window and offers that same candidate list before starting
each later window. DHT and trackers remain active and can discover more peers.
The receipt records the selection rule, count, hash and accepted candidate count;
raw endpoints stay in a separate private local artifact. This compares transfer
with shared candidates, not discovery: the first upstream window discovers them
naturally, and peer availability can still change between sequential windows.

For one focused startup investigation, pass `--diagnostic` and select exactly
one `QBUTT_PUBLIC_SWARM_MODES` value; leave `QBUTT_PUBLIC_SWARM_ROUNDS` unset
or set it to `1`. The receipt is marked `public-swarm-diagnostic`, not a repeated
comparison. Begin any capture before launching the app and restrict it to the
fixture's chosen peer port on each selected interface.

Run `bun run smoke:mixed-baseline` against the unchanged upstream control or
alpha application to verify the negative control: each single proxy obtains
exactly its available subset and cannot finish the target. A timeout is a failure;
the negative result requires a checked native piece bitmap and matching bytes.

`smoke:path-auth` copies only executable/runtime files into a temporary bundle and
compiles a small fake child there. It verifies that the application does not install
a session-wide SOCKS proxy, then tests managed peer-socket negotiation against
no-auth downgrade and rejected credentials and checks incompatible child hello.
DNS cases use the exact protocol 7 handshake and exercise
numeric/family/bounds checks, request errors with retry, child timeout/crash,
generation admission, authentication and redacted public results. It also rejects
the wrong pinned upstream revision, extra handshake/envelope/method fields and
malformed error text, and accepts only the seven-field transport counter allowlist
for the exact active path generation.
Delayed telemetry, a malformed delayed snapshot and decreasing counters prove that
foreground operations queue behind low-priority status polling while every snapshot
is still validated and monotonic. A child which cannot retire a path is terminated
rather than left with a hidden live listener.
The deterministic gateway rollover case injects terminal events during a two-path
configuration rollover and while a different path is explicitly stopping. It
requires every still-desired path to reopen at a later generation, rejects a
non-canonical stop identity without changing active paths, and proves the stopped
path does not return.
These path-only cases reject unexpected gateway requests; public gateway leases
and serialized inbound events are covered by qbutt-net's gateway-client integration
and the application-level `smoke:gateway` scenario.
The normal bundle and profiles are untouched. The fake child records protocol
method/command numbers, never authentication payload. All scenarios produce their
own evidence; any failed scenario makes the suite fail.

`smoke:path-dns` uses the real bundled child with two generated SOCKS nodes and
two local TCP DNS servers returning different A/AAAA answers for the same name.
It checks immutable DNS policy per opened generation, address families, stale
generation rejection, stopping one path while a lookup is pending without
terminating its healthy neighbour, global stop, and saved settings after restart.
`smoke:path-dns:native` adds a physical Native route in Mixed mode. Set
`QBUTT_LAB_NATIVE_INTERFACE` and `QBUTT_LAB_NATIVE_ADDRESS` to an active IPv4
adapter and its address. An owned DNS server binds that address; the scenario
checks its observed physical source, exact Native answer/path generation,
retirement of the old generation, and continued health of both SOCKS paths and
the child. It restores the Pinned policy and prior DNS setting before cleanup.
This checks the application's DNS control boundary; it does not prove that every
libtorrent tracker, peer, webseed or discovery operation uses that boundary.

`smoke:discovery` uses two managed tunnel paths with exact-target local SOCKS
relays. Their DHT responders return different peer subsets; HTTP and UDP trackers
provide two more peers. Four libtorrent seeds own disjoint pieces of one public
torrent. No peer is injected through `addPeers`: completion requires discovery,
concurrent payload through both paths and exact file sizes/hashes. The torrent
is already active when DHT is enabled. The fixture checks read-only DHT messages,
absence of `announce_peer`, separate node IDs and current peer path generations.
Both relays allow all four seed endpoints; only discovery responses are route-local.
HTTP and IPv4 UDP replies must update separate tracker endpoints identified by
path/generation, even when their displayed local addresses coincide.
`smoke:discovery-dedup` additionally advertises the same original peer endpoint
through both DHT paths and the HTTP/UDP trackers. Four complementary seeds must
remain exactly four active connections without duplicate endpoints across paths,
then complete the same hash-checked payload. This observes outgoing connections;
incoming handshakes and reconnect races require separate scenarios.
`smoke:discovery-magnet` starts without local metadata and holds peer responses
to observe pinned discovery. After standard peer `ut_metadata`, the same public
torrent must automatically discover and receive useful payload through both
paths before any additional tracker mutation. It then completes the same four
complementary seeds and exact hashes. This mode uses TCP and cannot be combined
with PEX, duplicate-discovery or automatic uTP retry modes.
Set `QBUTT_DISCOVERY_PROTOCOL=both` to check automatic uTP-to-TCP retry against the
same TCP-only seeds; the default is explicit TCP.
PEX, cross-generation DHT identity changes and real-network discovery remain
unverified by `smoke:discovery`. `smoke:pex` reuses its two paths and one public
torrent with two disjoint, checked partial seeds. Only seed A is supplied through
`addPeers`; A's controlled libtorrent neighbor link advertises seed B by PEX.
DHT, LSD and trackers are disabled. B must appear with WebUI source flag `X`,
deliver payload through the other path generation, and complete exact file
sizes and SHA-256 hashes with A. Both seeds must retain their original piece
subsets and report zero payload download from one another. This proves local
PEX admission and cross-path selection, not Internet peer discovery.
`smoke:discovery-policy` enables DHT and PEX globally with two allowed paths.
Route-local tracker responders and peer accounting must keep a private torrent
on its pinned path, with no private infohash in either live DHT responder.
A controlled peer inspects the extended handshake and requires that `ut_pex`
is absent; this checks private PEX negotiation, not forged PEX-message handling.
A second private torrent is added as a magnet while its tracker response is held:
before metadata, its DHT lookup and tracker announce must use only the pinned
path. Importing the matching `.torrent` into this existing magnet supplies private
metadata (private seeds intentionally do not offer `ut_metadata`). Thereafter
no new DHT lookup is permitted, and the tracker response is released.
Both torrents complete through real libtorrent seeds with exact sizes and hashes;
no peer is injected through `addPeers`. This does not promise infohash privacy
before the magnet's metadata is known.

`smoke:webseed` supplies HTTP URL seeds through the ordinary WebUI API, without
BitTorrent peers or trackers. Two authenticated SOCKS routes map the same numeric
URL to separate controlled Range responders; a reachable direct HTTP canary fails
any Native bypass. A private torrent completes exact sizes and hashes on its
pinned route. After that route is retired, a second private torrent stays active
with zero data or HTTP requests for eight seconds, preserving the retired path
identity while the other route remains open. A public torrent then completes via
the surviving route, proving it works while the private torrent stays blocked.
That HTTP scenario does not test TLS or mid-transfer webseed reconnection.

`smoke:webseed-reconnect` retires the active route during a rate-limited public
download; the torrent must finish through the surviving path without restarting.
`smoke:webseed-reconnect-private` instead checks eight seconds of no migration or
direct fallback, then reopens the pinned path with a new generation. Both require
old sockets to close, zero Native-canary requests and exact final sizes/hashes.

`smoke:webseed-https` separately downloads the pinned 893-byte Ubuntu 24.04.5
`SHA256SUMS` resource as a generated single-file torrent with an HTTPS URL seed
and no peer or tracker sources. Supply `QBUTT_HTTPS_PROXY_CONFIG`,
`QBUTT_HTTPS_PROXY_NAME` and `QBUTT_HTTPS_INTERFACE` through local environment
variables for one real subscription route. It requires exact size/SHA-256 and
verified-byte credit on that path generation, with zero other/Native route
payload in engine accounting. It uses normal public certificate trust without
changing system roots; rejection of invalid certificates and independent packet
capture of Native exclusion are not covered by this positive acceptance test.
Generated payloads and profiles are removed after
owned processes stop; compact evidence and logs remain.

`bun tests/network-lab/webseed-https.ts --reconnect` retires the active path after
a verified piece, then completes over a second real path. Add `--private` to
require eight seconds without progress on the surviving route and continuation
only after reopening the original path with a new generation. Also set
`QBUTT_HTTPS_PROXY_NAME_2` and `QBUTT_HTTPS_SOURCE_FILE` to the locally cached
`ninja-win.zip` pinned in `upstream-lock.json`; it supplies metadata hashes only.
qbutt downloads its own 275253-byte copy through public HTTPS and redirects, with
normal certificate validation. Exact final size/hash and both generations'
verified contributions are checked. Native exclusion uses engine accounting;
this scenario does not capture TLS-decrypted Range headers or Native packets.

`smoke:webseed-tls-rejection` checks the negative certificate case locally through
the real app and qbutt-net. It uses Go (`QBUTT_LAB_GO`, otherwise `go`) to generate
a short-lived CA and a server certificate with the correct numeric SAN. Python's
TLS server must receive `TLSV1_ALERT_UNKNOWN_CA` from qbutt, while HTTP requests,
useful/verified payload and connections to a reachable Native canary remain zero.
A separate client with the explicit fixture CA must retrieve the exact body.
No certificate is installed in the system trust store. Certificate expiration,
name mismatch and mid-transfer reconnection remain separate scenarios.

`smoke:connection-budget` runs two infohashes against four rate-limited seeds
through two managed paths. Repeated explicit peer candidates must not create
duplicate original endpoints. It observes a shared session limit of three with
two peers per torrent, then lowers the torrent limit to one and requires two
remaining connections. Both downloads must finish with exact sizes and hashes.
This covers established outgoing TCP peers, not discovery-source deduplication,
incoming connection slack, pending gateway handshakes or all OS sockets.

This local lab does not prove physical VPS egress, throughput gain, complete DNS
isolation, Koala coexistence or public-Internet inbound. It also does not implement network
namespaces/netem or physical source-volume/power failure; those require separate
integration environments. Staging crash tests cover abrupt process termination on
the local Windows filesystem.

`bun tests/network-lab/transport-capabilities.ts` probes selected adapters against
public HTTPS and DNS endpoints. Set `QBUTT_PROBE_EXE`, `QBUTT_PROBE_CONFIG` (a local
Mihomo YAML file), `QBUTT_PROBE_INDICES` (up to eight distinct zero-based indices)
and `QBUTT_LAB_NATIVE_INTERFACE`. It requires HTTPS 200 with an egress address
different from the interface-bound Native reference, a correlated SOCKS UDP DNS
answer, and path DNS over TCP. The source profile remains read-only; ephemeral
selected-node credentials are removed when the child stops. Evidence excludes
credentials and public addresses. These are point-in-time adapter checks, not
throughput, independent-edge, inbound or physical system-VPN bypass proof.
