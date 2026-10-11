# Rebuilding the VSOV OpenVINO 2026.2.1 plugin

This directory carries the corresponding source for the conveyed `vsov.dll`:

- the complete upstream vs-mlrt v15.16 source archive;
- the exact local patch and prominent modification notice;
- the deterministic build script and dependency manifest;
- both VapourSynth R54 interface headers used by the compiler;
- the exact vcpkg portfiles and patches;
- content-hashed, no-charge source retrieval directions for every static
  dependency and the required delay-loaded OpenVINO library;
- all applicable license texts.

The upstream source is vs-mlrt v15.16 at commit
`885e8bb827fc431fce8e3109e7d60b0c38aa2035`, licensed GPL-3.0.

## Modification notice

Modified by **mpv-config** on **2026-08-15**. The only source change replaces
the checkout-dependent `git describe` step with the equivalent immutable
`v15.16-0-g885e8bb` identity, allowing the source ZIP to build without a
`.git` directory. Runtime code is unchanged. See
`source/MODIFICATIONS.md` and `source/deterministic-vcs-tag.patch`.

## Prerequisites

- Windows x64;
- Visual Studio 2022 17.11.4, MSVC 19.41.34120.0, toolset v143;
- Windows SDK 10.0.22621.0;
- CMake 3.26.0;
- Git;
- a freshly cloned, bootstrapped vcpkg checkout at
  `62739e5f7b27f2e04abd051a61c4848940204d93`, with empty `buildtrees`,
  `installed` and `packages` directories;
- the official OpenVINO wheel pinned in `PROVENANCE.json`.

`source/SOURCE-DOWNLOADS.json` contains the exact URLs, byte counts,
SHA-256 and SHA-512 hashes for OpenVINO 2026.2.1, vcpkg, ONNX 1.19.0,
protobuf/utf8-range 6.33.4 and Abseil 20260107.1. The bundled
`source/vcpkg-ports` snapshot contains every port patch used by this build.
The OpenVINO entry points to the exact commit source corresponding to the
delay-loaded `openvino.dll` supplied by the pinned official wheel.

## Command

Run from Windows PowerShell:

```powershell
.\source\build-vsov.ps1 `
  -VcpkgRoot C:\src\vcpkg `
  -OpenVinoWheel C:\downloads\openvino-2026.2.1-21919-cp313-cp313-win_amd64.whl `
  -DownloadCache C:\source-cache\vcpkg
```

`-DownloadCache` is optional and defaults to the fresh vcpkg checkout's
`downloads` directory. It may point to a shared cache of source archives and
vcpkg tools: their content hashes are still enforced, and compiled packages
or binary-cache objects are never reused.

The script rejects unpinned inputs, dirty or previously used vcpkg roots, and
ambient compiler flags. It disables vcpkg binary caches, creates separate
target and host overlay triplets, and passes an exact, quoted MSVC `/pathmap`
flag set through controlled per-process `CL` to every vcpkg and top-level
compile. The per-run work root, vcpkg checkout, download cache and
corresponding-source checkout are mapped to stable paths; paths containing
spaces are supported. Static CRT, static ONNX, MSVC
`/experimental:deterministic` and linker `/Brepro` complete the build contract.
The finished DLL is rejected if any real build path remains in ANSI or UTF-16
strings. The internal .NET SHA-256 helper works in Windows PowerShell 5.1
without `Get-FileHash`.

The required result is:

```text
bytes   4231168
sha256  25229F2263D6DFE2794E7140C8CDC6093E40C24F5FC840225B73E974DB65CD22
```

Two builds from fresh dependency roots in different absolute directories,
including one whose vcpkg and work paths contained spaces, produced that exact
byte-identical DLL. They shared only the content-verified source/tool download
cache; no compiled package or binary-cache object was reused. The only
delay-loaded non-system dependency is `openvino.dll`; direct imports are
`KERNEL32.dll` and `dbghelp.dll`.
