# qbutt

qbutt is a native Qt BitTorrent client based on qBittorrent, with in-place Smart repair and an optional Mihomo subscription. It is an independent public project: no service account, private API or prescribed VPN provider is required.

This is an experimental Windows x64 implementation. The application keeps qBittorrent's source layout and one libtorrent session; [qbutt-net](https://github.com/qbutt-org/qbutt-net) runs the selected transport in a separate process. The current source baseline is qBittorrent 5.2.3 with libtorrent 2.0.11. Exact dependencies are recorded in [upstream-lock.json](upstream-lock.json).

## Run

Extract the portable archive into a writable directory and start `qbutt.exe`. Keep `qbutt-net.exe`, the Qt libraries and `profile/` beside it. Settings and session state live in that portable profile. These initial builds are unsigned and have no automatic updater.

For a subscription, open **Tools → Options → Connection → Mihomo subscription**. Paste an HTTPS subscription URL and click **Refresh**, or choose a local Mihomo YAML file. Select a node and the physical network interface, then click **Use selected node** before adding torrents. Subscription retrieval uses the regular network connection. Only node definitions are imported; the subscription's TUN, DNS and routing configuration is not applied.

The selected node carries TCP peers and HTTP(S) trackers. Disconnecting or losing the transport leaves transfers blocked; reconnecting preserves the torrent jobs. This first implementation requires an empty session to switch between the selected node and the default connection. It disables DHT, local discovery, uTP, port forwarding and incoming peer connections while the node is selected. It is **not a Tunnels-only mode**: DNS isolation, UDP trackers, public inbound and complete coexistence with another system VPN remain unverified.

For repair, stop a manually managed torrent, select all its files, and use its context menu **Smart repair…**. Analysis reads the current file mappings and target hashes without changing payload data. Review the expected and actual sizes, explicitly consent to in-place changes, then select **Repair in place and recheck**. qbutt removes oversized tails, starts the standard engine recheck and leaves the torrent stopped; use **Start** to download missing data. Unknown files are preserved. In-place repair does not provide rollback.

The initial repair operation requires one save directory on a fixed local Windows drive, with incomplete-file extensions and unwanted-file relocation disabled. It rejects hardlink aliases, reparse points and conflicting writers. Size changes use exclusive file handles, then hand control to normal libtorrent recheck/download; keep other writers closed throughout. Missing nonempty files can be analyzed and downloaded, but a missing zero-byte target permits analysis only in this version. Torrent file renames are supported through the current torrent mappings. Safe staged updates and repair from unrelated torrents are later stages.

## Build and verify

Use Windows x64, Visual Studio 2022 with the C++ desktop workload and Windows SDK, CMake, Git, Python 3.12 and Bun 1.4.0. From PowerShell:

```powershell
./scripts/build-windows.ps1
```

The script retrieves pinned Qt, Boost, libtorrent, OpenSSL, zlib, Go, Ninja and qbutt-net dependencies, then writes the portable ZIP and build manifest under `%LOCALAPPDATA%/qbutt/build`. The manifest identifies the source revision and whether the application checkout was dirty.

[The integration lab](tests/README.md) uses Bun and generated legal v1, v2 and hybrid torrents to exercise the real application, repair, a local seed and transport failure/retry. No public swarm or existing user profile is used by the lab. [Transport capabilities](docs/capabilities.md) distinguish available adapters from measured behavior.

## Development

Read [AGENTS.md](AGENTS.md), [the architecture](docs/qbutt-architecture.md), [the staged implementation plan](docs/implementation.md) and [the first-slice decisions](docs/adr/0001-first-slice.md). The architecture describes the target product; Mixed routing, adaptive selection, safe staging, reverse inbound and signed updates are not implemented by this slice.

The public repositories publish only `main`. Upstream updates are reviewed and integrated through a separate `upstream` remote; upstream branches and tags are not mirrored into these repositories. Report qbutt issues [here](https://github.com/qbutt-org/qbutt/issues).

Release builds and validation run locally; finished archives are uploaded to GitHub Releases. GitHub workflows are available only for an explicit manual run and do not start on pushes or pull requests.

qbutt preserves the work and notices of the [qBittorrent contributors](AUTHORS), libtorrent and the other bundled projects. See [COPYING](COPYING), [COPYING.GPLv2](COPYING.GPLv2) and [COPYING.GPLv3](COPYING.GPLv3); portable archives include dependency notices and source references. The optional IP-to-country data comes from [DB-IP](https://db-ip.com/db/download/ip-to-country-lite) under CC BY 4.0.
