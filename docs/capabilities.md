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

Mihomo subscriptions can contain complete client profiles. Their node definitions can be reused, but routing groups, DNS settings, provisioning and server lifecycle remain outside this application. Multiple aliases or protocols for one server do not create independent network edges.

## Current adapter verification, 21 September 2026

The pinned qbutt-net revision `678b2caedba68cde52a31d62346ace173e5280e9`
fixes SOCKS replies from adapters that expose no local bound address. Hysteria2
returned an empty domain (`05 00 00 03 00 00 00`), so a standard HTTPS client
waited before sending TLS. The reply now uses an unspecified IPv4 address;
valid bound addresses and adapter implementations are unchanged.

The pre-release binary `98da7cc3034007f4c20e281091877c2400bb0de3e9126a42ccdf322aedccb997`
passed `transport-capabilities-9vdUZU`: Hysteria2 Gecko, Hysteria2 Salamander,
ShadowQUIC and VLESS/gRPC each returned HTTPS 200 with an egress address different
from the interface-bound Native reference, a correlated UDP DNS answer and path
DNS over TCP. The child stopped cleanly, diagnostics were empty, the source
profile was unchanged, and ephemeral credentials were removed. Hysteria2's
previous failure was reproduced on alpha.4; wire observations before and after
the fix confirm the corrected SOCKS address type. These point-in-time results
do not establish throughput, independent edges or physical Koala bypass.

The same binary passed the local TCP/UDP/auth/lifecycle integration (21 checks),
DNS/SNI integration (17 checks) and transport replacement/failure integration
(`transport-reserves-qqjm7h`, nine checks). The reusable live probe is
`tests/network-lab/transport-capabilities.ts`; see `tests/README.md` for inputs.
Clean [alpha.5](https://github.com/qbutt-org/qbutt/releases/tag/v0.1.0-alpha.5)
uses app `6ccde49a` and net `d14c8889` from source `4921d247c`. Final adapter
probe `qoCysY`, app Hysteria2 HTTPS webseed `8Ri1u0` (893 exact verified bytes,
Native route credit zero), and Native v1/v2/hybrid resume/recheck `R9mseq` passed.
The production-trust alpha.3 updater downloaded and verified the published
62,692,085-byte ZIP; SHA-256 `bf5a651690168fb2aa7cc9d6b3ace96140e1886a6ed4b82f56d592095986bfdd`
matches locally. GitHub confirmed the tag/source and all four asset digests.
The release's `verification.json` distinguishes final-bundle checks from retained
checks of unchanged code. Generated payloads, profiles and duplicate download
were recycled after owned processes stopped.

## Historical runtime evidence, 12 September 2026

qbutt-net commit `a268eef9753311478f664fd8b1229e2f40fca686`, built with Go 1.27.1, passed 16 local TCP/UDP and fault scenarios. The binary SHA-256 was `011f1d8ba41f676960cd03cb3a06585887a23e0779cc7ad5394ff3ef351554f7`. TCP payload (98,304 bytes), a 1,024-byte UDP datagram, half-close, SOCKS authentication, generation checks and parent EOF were exercised with controlled local endpoints.

Separate HTTPS probes used one explicitly selected node at a time, bound to the active physical Ethernet interface while the existing Koala TUN was active. They compared the returned public address with both a physical Native probe and the selected node's server address; profiles and production configuration were not changed.

| Selected adapter | Observation |
| --- | --- |
| ShadowQUIC | HTTP 200; observed address matched the selected server and differed from physical Native. |
| VLESS | HTTP 200; observed address matched the selected server and differed from physical Native. |
| Hysteria2 with Gecko | Timed out after 25 seconds; working connectivity was not established. |

The child exited cleanly with empty diagnostics. These are point-in-time outbound HTTPS observations, not throughput benchmarks or proof of complete Koala coexistence. DNS remains `system-unverified`; torrent UDP, stable mappings and inbound remain unverified. No private node addresses, names or subscription credentials belong in this document.
