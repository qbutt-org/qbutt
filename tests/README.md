# Integration lab

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
bun run smoke:discovery-transition
bun run smoke:path-auth
bun run smoke:server-identity
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

`smoke:transport-reserves` requires `QBUTT_LAB_PATHS=1` and the protocol-7 child.
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

`smoke:direct-transition` uses the same physical-interface settings. One active
torrent starts with ordinary Direct networking and no child, switches to
fail-closed Tunnels only, then returns to Direct. It verifies useful TCP payload
in each mode, closure of the original native socket, a reachable native canary
that receives no managed-mode connection, child/listener retirement, and exact
final hashes. There is no torrent stop/start between policy changes.

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

`bun run smoke:diagnostics` uses the same Qt executable and
`QBUTT_LAB_PYTHON` on Windows x64. It holds a generated file with a Windows
oplock to block a real asynchronous disk write, and separately applies a real
per-torrent bandwidth limit. Production counters and Qt reasons must identify
each wait, clear after release, remain responsive, and finish with exact file
sizes and hashes over one peer connection. It keeps screenshots and compact
evidence, then releases the lock and removes the owned profile and payload.
This is process-level Light/Fusion acceptance, not physical desktop interaction.

The `qbutt-update-acceptance` CMake target builds the production update service,
dialog and a Qt process driver. Run `tests/qt-acceptance/release-fixture.py` with
the driver and OpenSSL executable paths. Put the Qt SDK `bin` on `PATH`, set
`QT_PLUGIN_PATH` to its `plugins` directory, and register driver/Python with
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

`smoke:server-identity` uses the real child to list and open three subscription
entries on two configured loopback addresses. Names and ports on one address
must share an Edge: duplicate active admission is rejected without restarting
the child; stopping and selecting the alias preserves Path ID and advances its
generation. The replacement downloads a hash-verified payload. Caller-supplied
`edgeId` is rejected. A second phase explicitly groups two distinct DNS aliases,
preserves an unselected server sharing their IP, and checks closed path records,
duplicate admission, profile restart, renamed nodes, reset and a reserve switch
with exact payload hashes. Group changes must be rejected while a path is open.
A 1,024-entry subscription exercises bounded selected-node opening, not full-list
UI capacity. Different configured hostnames still do not prove distinct physical
servers or public exits. The component integration suite covers DNS
case/IDNA, IPv4-mapped/IPv6 normalization and configuration changes before open.

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

`smoke:route-policy` drives the standalone libtorrent integration executable.
Set `QBUTT_POLICY_EXE` to the built `route-policy-integration.exe` and
`QBUTT_PUBLIC_IPV4` to the public IPv4 identity to advertise; this local fixture
does not contact that address. It proves live policy
replacement for HTTP/UDP trackers and DHT generations, an authenticated SOCKS
webseed, an unaffected default session, and automatic managed uTP with exact
payload bytes and source binding. HTTP and UDP tracker captures require the
configured public address and generic peer port, reject a hostile session-wide
announce address, and verify identical peer IDs and keys. DHT uses a distinct UDP
listener port; an outgoing-only route performs `get_peers` without
`announce_peer`, and anonymous announces suppress addresses while preserving the
peer port. The local DHT packet source is loopback, so this proves route-local
node identity and announced port behavior. Public address correctness and
reachability require the external gateway scenario.
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
