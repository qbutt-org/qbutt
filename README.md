# qbutt

qbutt is a native Qt BitTorrent client based on qBittorrent, focused on configurable network paths through an ordinary Mihomo subscription. It is an independent public project: no service account, private API or prescribed VPN provider is required.

This is an experimental Windows x64 implementation. The application keeps qBittorrent's source layout and one libtorrent session; [qbutt-net](https://github.com/qbutt-org/qbutt-net) runs the selected transport in a separate process. The current source baseline is qBittorrent 5.2.3 with libtorrent 2.0.11. Exact dependencies are recorded in [upstream-lock.json](upstream-lock.json).

## Run

Extract the portable archive into a writable directory and start `qbutt.exe`. Keep `qbutt-net.exe`, the Qt libraries and `profile/` beside it. Settings and session state live in that portable profile. Initial builds are unsigned. **Help → Update status** checks GitHub Releases and can download a complete portable ZIP after SHA-256 verification; installation and profile migration remain manual.

For a subscription, open **Tools → Options → Connection → Mihomo subscription**. Paste an HTTPS subscription URL and click **Refresh**, or use **Local file…** for a Mihomo YAML file. Select a node and physical network interface, then **Connect selected node**. Repeat to add more paths; **Disconnect selected path** removes one, and **Use default connection** returns to Native. Subscription retrieval uses the regular network connection. Only node definitions are imported; the subscription's TUN, DNS and routing configuration is not applied.

**Peer connections** offers Pinned (first selected remote edge), Tunnels only (selected remote edges), and Mixed (remote edges plus Native). Paths and policy can change while torrents remain in the session; a lost managed path is blocked until it is restored or replaced. Controlled integration tests cover multiple paths, TCP peers, HTTP/UDP trackers, DHT, PEX and uTP, but the full real-network Tunnels-only, DNS, private-torrent and failure-transition matrix is still incomplete. Treat an unprobed adapter capability as unknown.

**Public gateway settings…** is optional and requires a separately operated authenticated gateway with TLS credentials. Its TCP or UDP listener lease supplies an advertised public endpoint while active. Controlled IPv4 and IPv6 labs verified inbound TCP, uTP and DHT through that lease; a same-observer WAN TCP test verified remote-first ingress. Independent third-party WAN reachability, WAN UDP and the complete public announce/fault matrix remain unverified. A Mihomo subscription alone does not provide a public inbound listener.

For an existing torrent, stop it and open **Smart repair...** from its transfer-list menu. **File → Smart repair from torrent file...** can preview a new torrent before adding it. Choose source directories or map individual files to reuse existing data. Analysis reads target sizes and hashes without changing payload data; review its findings before authorizing a write. **Safe staged update** builds a separate verified payload with a recoverable journal; **Repair in place, without rollback** changes the managed target under exclusive ownership and then uses the normal libtorrent recheck. Start the torrent afterward to download missing pieces. Unknown files are preserved.

In-place repair requires one save directory on a fixed local Windows drive, with incomplete-file extensions and unwanted-file relocation disabled. It rejects hardlink aliases, reparse points and conflicting writers; keep other writers closed throughout. Missing nonempty files can be analyzed and downloaded. After read-only analysis and consent, the exclusive repair guard can create missing zero-byte targets and their directories. Interrupted staged updates have an explicit recovery mode. Neither repair mode treats unverified candidate bytes as completed torrent data.

The default appearance is dark, with the familiar qBittorrent transfer layout. Theme and layout changes are saved in the separate qbutt profile.

## Build and verify

Use Windows x64, Visual Studio 2022 with the C++ desktop workload and Windows SDK, CMake, Git, Python 3.12 and Bun 1.4.0. From PowerShell:

```powershell
./scripts/build-windows.ps1
```

The script retrieves pinned Qt, Boost, libtorrent, OpenSSL, zlib, Go, Ninja and qbutt-net dependencies, then writes the portable ZIP and build manifest under `%LOCALAPPDATA%/qbutt/build`. The manifest identifies the source revision and whether the application checkout was dirty.

[The integration lab](tests/README.md) uses Bun, generated legal v1/v2/hybrid torrents and isolated application profiles for its default local scenarios. Explicit WAN and public-swarm scenarios can contact external hosts; the WAN fixture reads a separately supplied Mihomo configuration without modifying that source. [Transport capabilities](docs/capabilities.md) distinguish available adapters from measured behavior.

## Development

Read [AGENTS.md](AGENTS.md), [the architecture](docs/qbutt-architecture.md), [implementation evidence and remaining gaps](docs/implementation.md), and [the first-slice decisions](docs/adr/0001-first-slice.md). The architecture describes the target product, not proof of complete acceptance.

The public repositories publish only `main`. Upstream updates are reviewed and integrated through a separate `upstream` remote; upstream branches and tags are not mirrored into these repositories. Report qbutt issues [here](https://github.com/qbutt-org/qbutt/issues).

Release builds and validation run locally; finished archives are uploaded to GitHub Releases. GitHub workflows are available only for an explicit manual run and do not start on pushes or pull requests.

qbutt preserves the work and notices of the [qBittorrent contributors](AUTHORS), libtorrent and the other bundled projects. See [COPYING](COPYING), [COPYING.GPLv2](COPYING.GPLv2) and [COPYING.GPLv3](COPYING.GPLv3); portable archives include dependency notices and source references. The optional IP-to-country data comes from [DB-IP](https://db-ip.com/db/download/ip-to-country-lite) under CC BY 4.0.
