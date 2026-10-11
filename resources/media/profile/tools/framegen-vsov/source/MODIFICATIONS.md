# Local modifications

Modified by **mpv-config** on **2026-08-15**.

The only local source change to vs-mlrt v15.16 replaces the
checkout-dependent `git describe` step in `vsov/CMakeLists.txt` with the
equivalent immutable version string `v15.16-0-g885e8bb`. This allows the
complete tagged source archive to produce a reproducible binary without a
`.git` directory. The exact change is
`deterministic-vcs-tag.patch`; no runtime code was changed.

The modified source and resulting binary remain licensed under GPL-3.0.
