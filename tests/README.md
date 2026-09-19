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
bun run smoke:proxy
bun run smoke:network
bun run smoke:repair
bun run smoke:repair-product
bun run smoke:staging
bun run smoke:completion
bun run smoke:mixed
bun run smoke:mixed-baseline
bun run smoke:tunnels
bun run smoke:native-route
bun run smoke:route-policy
bun run smoke:path-auth
bun run smoke:path-dns
```

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
Three offscreen app processes check a fresh built-in dark profile against
`docs/ui-default-layout.json`, change layout and select Light through Qt
controls, verify those settings after restart, and check an independent
functional Light/Fusion profile. Header order, hidden state, logical widths,
Files tab, sidebar action, palette and options controls are asserted; the
stretched last Files column adapts to its viewport. PNGs and JSON evidence stay
in the printed temporary directory; successful profiles and the copied Qt
runtime are removed. These configure/build/run commands still need validation
against the shared Windows build before reporting the suite as passed.

`smoke:qt` launches the real application offscreen with a new profile. It drives
repair preview, explicit mappings, staged commit, multiple Paths, completion
policies, bounded diagnostics export, and a 2,000-row transfer list. The runner
creates and cancels a 30,000-file source search, measures event-loop response,
checks payload snapshots before consent, and verifies final bytes and preserved
unknown files. Set `QBUTT_QT_ACCEPTANCE_BUNDLE` only when the deployed runtime is
not in the build tree's `portable` directory. This is process/UI-model acceptance,
not physical desktop interaction or public-network evidence.

To exercise the real qbutt-net child, place its pinned binary beside the app and
run `smoke:network` with `$env:QBUTT_LAB_PATHS = '1'`. This adds node listing,
session transition guards, termination of the owned child PID, blocked-path
retention, a new generation on retry, and explicit stop/return to Native. The
adapter connects only to the lab SOCKS server through the Windows loopback
interface; this is lifecycle evidence, not a VPS egress probe.

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

`smoke:repair-product` drives the standalone Smart Repair entry dialog through
Qt's offscreen platform before any torrent exists. Build its process-level driver
against the current production build with `tests/repair-product/build.ts`, then set
`QBUTT_REPAIR_PREVIEW_DRIVER`. It verifies explicit mappings and separate candidate,
hash-verified, target-network and staging-storage summaries, a responsive cancelled
scan, and visible refusal for missing roots, hardlinks and reparse points. Every
source and target tree is hashed before and after; the driver never clicks Apply,
so no torrent, resume record, journal or payload write is created.

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
targets and v2/hybrid restart.
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
must remain unchanged. Use ordinary physical temporary paths for payloads;
repair ownership guards deliberately reject paths through junctions or symlinks.

The optional `gui-smoke` argument to the driver builder produces a real Qt
offscreen import dialog driver. Pass generated source settings, data directory,
source profile base, and a fresh output directory. It exercises file selection,
preview, destination editing, consent, asynchronous preparation and close guards,
then saves PNGs and reads back the staged native records. Neither driver needs a
live profile. Windows external SQLite import supports schema 9 and snapshots
the source DB/WAL under write-excluding handles; unsupported schemas fail before
installation. Torrents must have complete metadata before importing them.

`smoke:proxy` exercises the bounded authenticated TCP fixture relay using real
sockets. `smoke:network` drives the native client through that relay to a seed and
HTTP tracker whose synthetic endpoints have no direct listener, kills the relay
mid-download, observes a drained stall, restarts it, and verifies the payload.
Relay stream counters include BitTorrent/HTTP protocol bytes; they are neither
unique verified payload bytes nor packet-level wire counters. The final SHA-256
and exact-size checks count verified payload separately.

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
`QBUTT_PUBLIC_IPV4` to the currently observed public IPv4. It proves live policy
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
payload, and WebUI response latency.
Relay stream bytes include protocol data and are not wire bytes. The local
single-host TCP topology proves only the stated controlled comparison; its
evidence lists the untested public, UDP, inbound, resource and last-mile cases.

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

Run `bun run smoke:mixed-baseline` against the unchanged upstream control or
alpha application to verify the negative control: each single proxy obtains
exactly its available subset and cannot finish the target. A timeout is a failure;
the negative result requires a checked native piece bitmap and matching bytes.

`smoke:path-auth` copies only executable/runtime files into a temporary bundle and
compiles a small fake child there. It verifies that the application does not install
a session-wide SOCKS proxy, then tests managed peer-socket negotiation against
no-auth downgrade and rejected credentials and checks incompatible child hello.
DNS cases use protocol 2 and exercise
numeric/family/bounds checks, request errors with retry, child timeout/crash,
generation admission, authentication and redacted public results.
The normal bundle and profiles are untouched. The fake child records protocol
method/command numbers, never authentication payload. All scenarios produce their
own evidence; any failed scenario makes the suite fail.

`smoke:path-dns` uses the real bundled child with two generated SOCKS nodes and
two local TCP DNS servers returning different A/AAAA answers for the same name.
It checks immutable DNS policy per opened generation, address families, stale
generation rejection, stopping one path while a lookup is pending without
terminating its healthy neighbour, global stop, and saved settings after restart.
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
Set `QBUTT_DISCOVERY_PROTOCOL=both` to check automatic uTP-to-TCP retry against the
same TCP-only seeds; the default is explicit TCP.
PEX, cross-generation DHT identity changes and real-network discovery remain
unverified by this scenario. Generated payloads and profiles are removed after
owned processes stop; compact evidence and logs remain.

This local lab does not prove physical VPS egress, throughput gain, complete DNS
isolation, Koala coexistence or public inbound. It also does not implement network
namespaces/netem or physical source-volume/power failure; those require separate
integration environments. Staging crash tests cover abrupt process termination on
the local Windows filesystem.
