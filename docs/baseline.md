# Native control baseline

The unchanged control build was compiled and exercised on Windows x64 on 12 September 2026. Its source was exported with `git archive` from qBittorrent commit `0b63c3d17373f6132ea211c9dcd4241284ccdfaf` (release 5.2.3), with no qbutt source changes.

| Input | Verified version |
| --- | --- |
| MSVC | 19.41.34120.0, Visual Studio 2022 |
| CMake / Ninja | 4.4.3 / 1.12.0 |
| Qt / Boost | 6.10.1 / 1.90.0 |
| libtorrent | 2.0.11, `9d7443f467147d1784fb7516d2a882db1abb5a8b` |
| OpenSSL / zlib | 3.6.3 / 1.3.2 |

Source revisions and archive hashes are in [upstream-lock.json](../upstream-lock.json). The compiler environment was initialized by `VsDevCmd.bat -no_logo -arch=x64 -host_arch=x64`. These were the control build commands, with the temporary dependency directory expressed as `$deps`:

```powershell
$deps = "$env:TEMP/qbutt-build"
cmake -S "$deps/baseline-src" -B "$deps/baseline" -G Ninja `
  -DCMAKE_CXX_COMPILER=cl -DCMAKE_BUILD_TYPE=RelWithDebInfo -DCMAKE_EXPORT_COMPILE_COMMANDS=ON `
  "-DCMAKE_TOOLCHAIN_FILE=$deps/vcpkg/scripts/buildsystems/vcpkg.cmake" `
  "-DBOOST_ROOT=$deps/boost_1_90_0/lib/cmake" `
  "-DLibtorrentRasterbar_DIR=$deps/libtorrent/install/lib/cmake/LibtorrentRasterbar" `
  "-DCMAKE_PREFIX_PATH=$deps/Qt/6.10.1/msvc2022_64" `
  -DMSVC_RUNTIME_DYNAMIC=ON -DTESTING=OFF -DVCPKG_TARGET_TRIPLET=x64-windows-static-md-release
cmake --build "$deps/baseline" --parallel 10
```

All 405 build steps completed. The resulting `qbittorrent.exe` was 22,477,312 bytes, SHA-256 `70322489c36a613eec5788688355fca26268a520d74e3f41ebb2d90c1c8beb0f`. Qt deployment used `windeployqt --release --compiler-runtime`; the offscreen platform plugin was supplied for isolated lab runs.

The [native integration suite](../tests/README.md) passed for generated v1, v2 and hybrid torrents: selective download verified 54,149 bytes, then each complete payload verified 2,151,362 bytes with exact lengths. Stop, clean process restart, resume, corruption/recheck detection and payload-preserving torrent removal passed. The application used libtorrent 2.0.11; the separate metadata/seed fixture binding was libtorrent 2.0.14 and is pinned in `tests/fixtures/requirements.txt`.

The negative control retained an extra 8,193-byte tail after an ordinary recheck reported the v1 torrent hash-complete. This establishes why exact-size repair is a separate operation. These local runs establish neither multipath benefit nor DNS/UDP policy, public inbound or production throughput.
