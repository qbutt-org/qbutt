# Transport capabilities

The public source baseline is [Mihomo d3ec342d](https://github.com/MetaCubeX/mihomo/tree/d3ec342d441b086ec4318332f59dd05d8a2b5697). Source inspection and runtime evidence are separate.

| Mechanism available in source | Location | What it establishes |
| --- | --- | --- |
| TCP dial and UDP packet adapter interfaces | `constant/adapters.go` | The caller can request transport operations; success still needs a probe. |
| Listener-specific proxy | `listener/inbound/base.go` | A SOCKS listener can use one selected adapter. Its default wildcard address must be overridden with loopback. |
| Windows interface binding | `component/dialer/bind_windows.go` | Socket options exist for IPv4 and IPv6; actual routing through another installed TUN requires measurement. |
| Hysteria2 Gecko | `adapter/outbound/hysteria2.go` | Node fields and adapter implementation are available. |
| ShadowQUIC v2/JLS | `adapter/outbound/shadowquic.go` | QUIC version selection, JLS authentication and keepalive fields are available. |
| Sudoku | `adapter/outbound/sudoku.go`, `transport/sudoku/` | The transport is available in the pinned public source. |
| VLESS/REALITY | `adapter/outbound/vless.go` | Node TLS/REALITY transport configuration is available. |

The first child reports source-supported TCP/UDP independently of measured public reachability. An unprobed external address, stable UDP mapping, TCP inbound or UDP inbound is **unknown**. SOCKS UDP support does not establish inbound reachability. Controlled TCP proxy tests do not establish torrent UDP, tracker identity, Koala coexistence or Tunnels-only behavior.

The existing subscription service emits complete client profiles. Its transport objects can be reused, but its groups, routing policies, DNS settings, provisioning and server lifecycle remain outside this application. Multiple aliases or protocols for one server do not create independent network edges.
