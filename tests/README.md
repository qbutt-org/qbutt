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
bun run smoke:completion
bun run smoke:mixed
bun run smoke:path-auth
```

To exercise the real qbutt-net child, place its pinned binary beside the app and
run `smoke:network` with `$env:QBUTT_LAB_PATHS = '1'`. This adds node listing,
session transition guards, termination of the owned child PID, blocked-path
retention, a new generation on retry, and explicit stop/return to Native. The
adapter connects only to the lab SOCKS server through the Windows loopback
interface; this is lifecycle evidence, not a VPS egress probe.

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
accounting, and missing nonzero targets. An absent zero-length target must reject
apply without creating files.
It drives the real session through standard recheck and download after apply.

`smoke:completion` proves that normal completion exits an isolated app when that
action is enabled, while a held repair prevents auto-exit after another torrent
finishes. It does not inject the queued-signal or nested-dialog race windows.

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

`smoke:mixed` requires the route-aware application and bundled qbutt-net. Two
native peers hash-check complementary even/odd pieces stored in independent
physical partial files. Each synthetic peer address has an exclusive authenticated
SOCKS route and rejects direct connections. One application torrent must receive
both subsets concurrently, report the original peer/path/generation through native
telemetry, and finish with exact hashes and sizes. A second run deliberately tries
the wrong routes before automatic retry through the alternatives. The partial
peers cannot download their missing complement; their final bitmaps and physical
files must stay unchanged.

`bun run smoke:mixed --native` extends this fixture to four complementary subsets:
three exclusive tunnels and Native. Set `QBUTT_LAB_NATIVE_INTERFACE` to an active
physical adapter name and `QBUTT_LAB_NATIVE_ADDRESS` to its private local IPv4
address. The native peer binds only that address and accepts only clients from
the same address. All four paths must feed one torrent concurrently; the native
peer must observe that exact source address. This proves application binding on
the host, while public egress and traffic on the physical wire require separate
environments.

Run `bun run smoke:mixed --baseline` against the unchanged upstream control or
alpha application to verify the negative control: each single proxy obtains
exactly its available subset and cannot finish the target. A timeout is a failure;
the negative result requires a checked native piece bitmap and matching bytes.

`smoke:path-auth` copies only executable/runtime files into a temporary bundle and
compiles a small fake child there. It tests the real application's SOCKS negotiation
against no-auth downgrade and rejected credentials for both session and peer
sockets, then incompatible child hello.
The normal bundle and profiles are untouched. The fake child records protocol
method/command numbers, never authentication payload. All scenarios produce their
own evidence; any failed scenario makes the suite fail.

This local TCP lab does not prove physical VPS egress, throughput gain, DNS/UDP/
uTP/DHT isolation, Koala coexistence, public inbound, or Tunnels only. It also does
not implement network namespaces/netem, source-volume failure or safe-update
crash recovery; those require their own integration environments.
