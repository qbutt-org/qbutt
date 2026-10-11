# Development on Windows

Install Visual Studio 2022 with **Desktop development with C++**, CMake 3.25 or
newer, and the Rust toolchain recorded in `rust-toolchain.toml` when present.
Then run this once from the checkout:

```powershell
./scripts/configure-dev.ps1
```

The script reuses the project's dependency preparation, selects the x64 MSVC
2022 compiler and configures a local `windows` preset. Qt lives under
`C:/Tools/Qt`; the other dependencies live under `C:/Tools/qbutt/dependencies`.
The terminal and IDE use the same SDK and dependencies. Use `-QtRoot` for an
existing SDK or `-DependencyRoot` for another dependency location.

`CMakePresets.json` contains the shared build settings. The generated
`CMakeUserPresets.json` contains your local paths and compiler environment; it is
ignored by Git and belongs to you. Re-running setup does not overwrite it.
After changing an SDK installation, edit this file or remove it before running
setup again.

## CLion

Open the repository root. Under **Settings / Build, Execution, Deployment /
CMake**, enable the imported **windows** preset and disable the old **Debug**
profile. CLion imports new presets disabled, so this is a one-time IDE action.

Under **Toolchains**, use Visual Studio 2022, architecture `amd64`, toolset
`v143`. If you give that toolchain a name, pass `-ClionToolchain '<name>'` during
setup to bind the preset to it. This keeps CLion's debugger and compiler
selection consistent; the preset also records the exact compiler path.

The development configuration is **RelWithDebInfo**: it has debug symbols and
matches the existing libtorrent, OpenSSL and zlib dependencies built with the
dynamic release MSVC runtime. Do not relabel that dependency set as `Debug` or
link it to `/MDd` and debug STL. A full Debug configuration requires a separate
matching dependency build.

CMake exports `build/dev/compile_commands.json` for header indexing. Attach the
repository's `Cargo.toml` in CLion's Cargo tool window when the Rust workspace
is present. For QML, enable **Languages & Frameworks / QML / QML language
server** and use the SDK's `bin/qmlls.exe`; CMake generates the QML module and
language-server metadata as those modules are configured.

The CXX-Qt headers, including `qbutt_quick/src/controller.cxxqt.h`, are generated
by the first build. Build `qbt_quick` before judging missing generated-header
diagnostics in a fresh checkout.

CLion 2026.2's built-in QML parser incorrectly reports `Singleton expected` for
`pragma ComponentBehavior: Bound` ([CPP-42122](https://youtrack.jetbrains.com/issue/CPP-42122)).
Use Qt's language server for QML diagnostics and completion; this does not
guarantee that the older parser's underline disappears. Under the QML settings,
select the `windows` profile, the SDK's `bin` directory for **Qt binaries** and
its `qml` directory for **QML imports**. Enable completion from the language
server and reopen QML editors.

## Terminal build

The same configuration works from a regular terminal:

```powershell
cmake --preset windows
cmake --build --preset windows --parallel 8
```

This configures and builds source targets. For a deployed runnable bundle with
Qt plugins and companion products, use `scripts/build-windows.ps1`; dependency
preparation alone does not create that bundle. Keep application runs on isolated
profiles, as described in [the integration lab](../tests/README.md).

Before invoking Qt tools directly on Windows, set
`$env:QT_COMMAND_LINE_PARSER_NO_GUI_MESSAGE_BOXES = '1'`. This keeps their help
and command-line errors in the terminal. The generated preset, release builder
and project MCP configuration already set it for their own processes.

References: [CLion presets](https://www.jetbrains.com/help/clion/cmake-presets.html),
[toolchains](https://www.jetbrains.com/help/clion/how-to-create-toolchain-in-clion.html),
[QML support](https://www.jetbrains.com/help/clion/qml-syntax-support.html),
[Rust support](https://www.jetbrains.com/help/clion/rust-plugin.html).
