# Media tools

`scripts/package-media.ts` bundles the pinned mpv, FFmpeg, ffprobe and yt-dlp executables with the qbutt profile and verifies archives and files against `resources/media/component-lock.json`. `components/content/media-runtime.ts` verifies the bundled executables before their first use. Neither path runs upstream installers or uses media tools from PATH.

`ffprobe.exe` is used for video analysis and shader benchmarks. The pinned MSYS2 CLANG64 FFmpeg 9.0.2-6 package supplies both FFmpeg and ffprobe with their shared DLLs at `media/`. Every helper invocation uses that explicit working directory. Player supplies the verified probe path through `QBUTT_FFPROBE`. Original package licenses are retained under `media/licenses/msys2`; exact package recipes, patches, source archives and dependency sources are in the release media-sources ZIP.

The prepared mpv profile keeps certificate and host-name verification enabled. `profile/certs/cacert.pem` is the pinned certifi Mozilla CA collection, used together with native Windows trust. Its source and license are recorded in `component-lock.json` and `media/licenses/`.

`framegen-manager.exe` selects the available NVIDIA/TensorRT or Intel/OpenVINO backend and prepares its pinned runtime in qbutt's data directory. Its source, Cargo lock, package recipe and runtime catalog are maintained in the source distribution under `resources/media/profile/tools/framegen-manager`.

`framegen-bootstrap.dll` is the mpv C plugin that validates the selected manager manifest and loads its portable VSScript into the running player. Its source and package recipe are under `resources/media/profile/tools/framegen-bootstrap` in the source distribution.

`framegen-vsov.dll` is the modified GPL-3.0 OpenVINO plugin. Corresponding source, build instructions, modification notice and dependency licenses are included under `media/profile/tools/framegen-vsov`.

The manager and bootstrap third-party notices are generated from their exact Cargo locks. `Rust-Standard-Library-COPYRIGHT.html` records the licenses from their pinned Rust toolchain. The component package recipes check these notices and compiler identity before producing a binary.
