# First runnable slice

The application stays at the repository root, preserving qBittorrent's source layout and history. The baseline is upstream release 5.2.3, commit `0b63c3d17373f6132ea211c9dcd4241284ccdfaf`. The supported engine starts at libtorrent 2.0.11. The transport executable belongs to `qbutt-org/qbutt-net`; it is built and bundled separately. There is no libtorrent fork until an actual engine change requires one.

The first application operations are managed in-place repair and one explicitly selected proxy path. Mixed routing, safe-update staging, reverse inbound, adaptive selection and signed updates remain later acceptance stages in [the implementation plan](../implementation.md).

Repair means reusing, verifying and completing torrent data, not repairing the qbutt installation. Both it and network behavior are high priorities. Automatic updates come last and should simply retrieve GitHub Release bundles, following the existing `element-max` flow.

## Transport boundary

Public Mihomo commit `d3ec342d441b086ec4318332f59dd05d8a2b5697` already contains the necessary transport adapters, per-listener proxy selection and Windows interface binding. The existing service's additional fallback-group policy is not needed for an explicitly selected node. The public component starts from this upstream commit and carries no private repository history.

The application starts its bundled child with inherited private stdin/stdout pipes. Bounded versioned JSON messages use these handles; SOCKS payload uses a separate authenticated loopback listener. This deliberately replaces the planned named-pipe endpoint for the parent/child-only slice: there is no discoverable control address, extra ACL configuration or background service. EOF closes the child and its sockets. A standalone control client would require a separate design.

Network settings accept an ordinary Mihomo subscription, independent of any service or account system. The application retrieves the profile, then imports only the selected node definition into its transport process. Global client rules, DNS/TUN settings, external providers and automatic cross-server fallback are not part of that import. Edge identity is explicit inside the application; a node's display name does not establish its public IP or inbound capability. The settings UI exposes basic subscription, node and enable/disable choices rather than requiring users to understand these internal contracts.

## Repair boundary

Analysis reads the stopped managed torrent's mapped files and target hashes. It does not invoke engine recheck as a substitute for read-only analysis, because engine checking can allocate or rename files.

In-place application requires explicit consent and validates ownership, mappings and filesystem identity before mutation. The libtorrent `truncate_files` helper reopens each pathname and cannot retain an exclusive handle across validation and truncation on Windows. Where this conflicts with ownership, truncate through the already-owned platform handle. This is a concrete exception to reusing that helper, not a separate storage engine; standard libtorrent recheck and download remain responsible for pieces.

In-place operation does not promise rollback. Unknown files remain untouched. A safe staged update will be a separate operation with its own journal and fault-injection gate.
