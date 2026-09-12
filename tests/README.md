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
implementation is used. Each generated torrent is private with no public tracker.

Point the lab at a built, deployed Windows executable with Qt's `qoffscreen.dll`:

```powershell
$env:QBUTT_LAB_EXE = 'C:/path/to/portable/qbutt.exe'
$env:QBUTT_LAB_APP_NAME = 'qbutt' # use qBittorrent for the upstream control build
bun run smoke:native
bun run smoke:proxy
bun run smoke:network
bun run smoke:repair
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

This local TCP lab does not prove physical VPS egress, throughput gain, DNS/UDP/
uTP/DHT isolation, Koala coexistence, public inbound, or Tunnels only. It also does
not implement network namespaces/netem, source-volume failure or safe-update
crash recovery; those require their own integration environments.
