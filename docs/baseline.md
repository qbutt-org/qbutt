# Native control baseline

The unchanged control build was restored and exercised on Windows x64 on 20 September 2026 after the original build artifacts were cleaned up. Its source was exported with `git archive` from qBittorrent commit `0b63c3d17373f6132ea211c9dcd4241284ccdfaf` (release 5.2.3), with no qbutt source changes. The original libtorrent revision below was also exported and rebuilt independently of the qbutt fork; its `try_signal` submodule is `105cce59972f925a33aa6b1c3109e4cd3caf583d`. All exported source files were checked byte-for-byte against their archives.

| Input | Verified version |
| --- | --- |
| MSVC | 19.41.34120.0, Visual Studio 2022 |
| CMake / Ninja | 4.4.3 / 1.12.0 |
| Qt / Boost | 6.10.1 / 1.90.0 |
| libtorrent | 2.0.11, `9d7443f467147d1784fb7516d2a882db1abb5a8b` |
| OpenSSL / zlib | 3.6.3 / 1.3.2 |

Upstream and dependency pins are in [upstream-lock.json](../upstream-lock.json); this control always uses the original libtorrent revision in the table, independently of the current qbutt component pin. The compiler environment was initialized by `VsDevCmd.bat -no_logo -arch=x64 -host_arch=x64`. These were the control build commands, using existing dependencies without additional SDK downloads:

```powershell
$deps = "C:/Temp/qbutt-build"
$qt = "$env:LOCALAPPDATA/qbutt/dependencies/Qt/6.10.1/msvc2022_64"
cmake --fresh -S "$deps/libtorrent-control-src" -B "$deps/libtorrent/build" -G Ninja `
  -DCMAKE_CXX_COMPILER=cl -DCMAKE_BUILD_TYPE=RelWithDebInfo -DCMAKE_CXX_STANDARD=20 `
  "-DCMAKE_INSTALL_PREFIX=$deps/libtorrent/install" `
  "-DCMAKE_TOOLCHAIN_FILE=$deps/vcpkg/scripts/buildsystems/vcpkg.cmake" `
  -DVCPKG_TARGET_TRIPLET=x64-windows-static-md-release `
  "-DBOOST_ROOT=$deps/boost_1_90_0/lib/cmake" -DBUILD_SHARED_LIBS=OFF `
  -Ddeprecated-functions=OFF -Dbuild_tests=OFF -Dbuild_examples=OFF -Dbuild_tools=OFF
cmake --build "$deps/libtorrent/build" --parallel 8
cmake --install "$deps/libtorrent/build"
cmake --fresh -S "$deps/baseline-src" -B "$deps/baseline" -G Ninja `
  -DCMAKE_CXX_COMPILER=cl -DCMAKE_BUILD_TYPE=RelWithDebInfo -DCMAKE_EXPORT_COMPILE_COMMANDS=ON `
  "-DCMAKE_TOOLCHAIN_FILE=$deps/vcpkg/scripts/buildsystems/vcpkg.cmake" `
  "-DBOOST_ROOT=$deps/boost_1_90_0/lib/cmake" `
  "-DLibtorrentRasterbar_DIR=$deps/libtorrent/install/lib/cmake/LibtorrentRasterbar" `
  "-DCMAKE_PREFIX_PATH=$qt" `
  -DMSVC_RUNTIME_DYNAMIC=ON -DTESTING=OFF -DVCPKG_TARGET_TRIPLET=x64-windows-static-md-release
cmake --build "$deps/baseline" --parallel 8
```

All 168 libtorrent and 405 application build steps completed. The resulting `qbittorrent.exe` was 22,477,312 bytes, SHA-256 `9393e0c523b35a437fb9b356b4c7c7402dbbd9d97b9c1ae519fd01f1219c471e`. The local control bundle is `C:/Temp/qbutt-build/baseline-runtime`; Qt deployment used `windeployqt --release --compiler-runtime --no-translations`, with the offscreen platform plugin supplied for isolated lab runs. This rebuild replaces the cleaned-up 12 September binary (`70322489c36a613eec5788688355fca26268a520d74e3f41ebb2d90c1c8beb0f`), while retaining its pinned source and dependency versions.

The [native integration suite](../tests/README.md) passed for generated v1, v2 and hybrid torrents: selective download verified 54,149 bytes, then each complete payload verified 2,151,362 bytes with exact lengths. Stop, clean process restart, resume, corruption/recheck detection and payload-preserving torrent removal passed. The application used libtorrent 2.0.11; the separate metadata/seed fixture binding was libtorrent 2.0.14 and is pinned in `tests/fixtures/requirements.txt`.

The negative control retained an extra 8,193-byte tail after an ordinary recheck reported the v1 torrent hash-complete. This establishes why exact-size repair is a separate operation. These local runs establish neither multipath benefit nor DNS/UDP policy, public inbound or production throughput.
