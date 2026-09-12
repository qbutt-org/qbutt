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

## Runtime evidence, 12 September 2026

qbutt-net commit `a268eef9753311478f664fd8b1229e2f40fca686`, built with Go 1.27.1, passed 16 local TCP/UDP and fault scenarios. The binary SHA-256 was `011f1d8ba41f676960cd03cb3a06585887a23e0779cc7ad5394ff3ef351554f7`. TCP payload (98,304 bytes), a 1,024-byte UDP datagram, half-close, SOCKS authentication, generation checks and parent EOF were exercised with controlled local endpoints.

Separate HTTPS probes used one explicitly selected node at a time, bound to the active physical Ethernet interface while the existing Koala TUN was active. They compared the returned public address with both a physical Native probe and the selected node's server address; profiles and production configuration were not changed.

| Selected adapter | Observation |
| --- | --- |
| ShadowQUIC | HTTP 200; observed address matched the selected server and differed from physical Native. |
| VLESS | HTTP 200; observed address matched the selected server and differed from physical Native. |
| Hysteria2 with Gecko | Timed out after 25 seconds; working connectivity was not established. |

The child exited cleanly with empty diagnostics. These are point-in-time outbound HTTPS observations, not throughput benchmarks or proof of complete Koala coexistence. DNS remains `system-unverified`; torrent UDP, stable mappings and inbound remain unverified. No private node addresses, names or subscription credentials belong in this document.
