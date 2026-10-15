<p align="center">
  <a href="README.md">Русский</a> / <strong>English</strong>
</p>

<h1 align="center">qbutt</h1>

<p align="center">
  A torrent client with media search, based on qBittorrent and Qt.
</p>

<p align="center">
  <a href="https://github.com/qbutt-org/qbutt/releases/download/v1.5.1/qbutt-1.5.1-windows-x64-setup.exe">
    <img alt="Download qbutt for Windows" src="https://img.shields.io/badge/DOWNLOAD-Windows-009df7?style=for-the-badge">
  </a>
  <a href="https://github.com/qbutt-org/qbutt/releases">
    <img alt="All releases" src="https://img.shields.io/badge/RELEASES-all_versions-24292f?style=for-the-badge&amp;logo=github&amp;logoColor=white">
  </a>
</p>

<p align="center">
  <img alt="Windows x64" src="https://img.shields.io/badge/Windows-x64-0078d4?style=flat-square">
  <a href="https://github.com/qbutt-org/qbutt/releases/latest">
    <img alt="Latest release" src="https://img.shields.io/github/v/release/qbutt-org/qbutt?label=release&amp;color=009df7&amp;style=flat-square">
  </a>
  <a href="COPYING">
    <img alt="GPL" src="https://img.shields.io/badge/license-GPL-blue?style=flat-square">
  </a>
</p>

<p align="center">
  <a href="https://qbutt-org.github.io/qbutt/en/">Website</a> /
  <a href="https://github.com/qbutt-org/qbutt/releases/download/v1.5.1/qbutt-1.5.1-windows-x64.zip">Portable</a>
</p>

## Installation

Download the installer using the button above, or choose a file on the [latest release](https://github.com/qbutt-org/qbutt/releases/latest) page:

- **EXE installer** installs qbutt and adds a Start menu shortcut.
- **Portable ZIP** can be extracted and run using `qbutt.exe`. Settings are stored in the `profile` folder next to the app.

Videos open in [qplay](https://github.com/qbutt-org/qplay), an independent player built on mpv. The qbutt installer installs it alongside the app. [qcatch](https://github.com/qbutt-org/qcatch) connects your browser and can be installed from qbutt when needed. Both projects also work on their own.

qbutt has its own profile. Import settings and torrents from another client through **File → Import profile**. Downloads go to the **Desktop** by default; change the folder in **Settings → Downloads**. Updates preserve your previously selected folder.

## Search and library

**Search** covers games, films, TV series, anime, software, books, and manga. Results for the same work are grouped together. Choose a specific version, edition, or voice track within each result. TV series and anime have separate episode, voice track, and source filters.

Choose an option and click **Download**, **Watch**, or **Read**. The bookmark icon beside the title adds the work to **Favorites**. **History** keeps previously opened items, and **Share** copies a link to the work with the selected version. Open a `qbutt://` link in the installed app or paste it into the search field.

The **Sources** button opens the source catalog and settings. Some sources require a website login or their own access settings.

## Watching and reading

Online video streams to qplay when the source provides a direct stream. You can also watch a torrent before its download finishes: the player receives already verified parts of the file. Open local video and audio through **File → Open media…**. The player includes upscaling and frame generation; available modes depend on your GPU.

qplay settings offer audio and subtitle selection, playback speed, and text subtitle translation into Russian or English. The original remains available, and translated segments are cached. Subtitle height can be adjusted with the mouse or reset to automatic positioning.

Open books and comics from search or through **File → Open book…**. Supported formats include EPUB, FB2, TXT, HTML, CBZ/ZIP archives of images, and individual images. The reader saves your position and caches chapters. Its settings include paged and continuous modes, fit to screen height or width, brightness, page spacing, and reading area width. In paged mode, click the left or right side of the page to turn it.

The reader translates text and manga pages into Russian and English using an installed, signed-in **Codex**. Work context helps preserve names and terminology; the original and cached translation remain available. Choose the model and reasoning effort in translation settings. OpenRouter is also available for text. Text and images are sent to the selected service. Manga translation is experimental: cleaning complex artwork and laying out lettering still need improvement.

## Connecting your browser

qcatch is used for websites with login sessions and video players that require a browser session:

1. Open **Search → Sources → Browser** and select your browser.
2. Click **Install qcatch…**. qcatch will be installed, and the extension folder will open.
3. In Edge, enable developer mode at `edge://extensions`, click **Load unpacked**, and select the folder shown.

Once connected, qbutt uses the relevant website session and can obtain the selected episode and voice track stream for qplay. The browser is still needed when a site requires login, verification, or player selection. Cookies and temporary stream URLs are excluded from **Share** links.

## Downloads and networking

To complete existing files, add the torrent as usual and select the folder containing them: qbutt checks the files and downloads what is missing. For advanced repair, stop the added torrent and choose **Repair files** from its menu.

In **Settings → Downloads**, you can configure automatic opening of `.torrent` files: the Desktop is watched by default, and the original `.torrent` file is deleted after opening. The same page offers a progress bar above other windows and removal of completed torrents from the list while keeping their files.

To use several nodes at once, add an ordinary Mihomo subscription in **Settings → Connection**, select nodes, and enable Mihomo. They work alongside the ordinary connection. No account with a particular service is required, and system VPN settings from the subscription are not applied. Inbound connections through a public gateway require a separately configured gateway.

## Settings and updates

Light, Dark, and System themes are available, with changes applied immediately. Minimizing keeps the window on the taskbar, closing sends it to the tray, and **Exit** quits the app. Startup at Windows logon does not open a window.

Updates are available from the **Help** menu. The installed version downloads an EXE in the background and offers to update and restart; the portable ZIP is updated manually.

## Development

Windows x64, Visual Studio 2022 with C++ and the Windows SDK, CMake, Git, and Python 3.12:

```powershell
./scripts/build-windows.ps1
```

The script downloads pinned dependencies, including Bun and Go, and builds the portable ZIP and EXE installer locally. `-BuildRoot` sets the build directory; `-ArtifactRoot` sets the output directory. The bundle includes `qbutt-net` and a pinned qplay build. Bun is used only during the build; Python and Bun are not required to run the finished app.

`upstream-lock.json` pins component Git revisions, protocol versions, and SHA-256 hashes of installers, portable archives, and third-party runtime sources. `scripts/package-products.ts` verifies the finished artifacts before including them in the build. The portable bundle contains its own qplay in the `qplay/` subfolder. qcatch is downloaded from a pinned URL only when its install button is clicked.

The player interface, upscaling, frame generation, mpv, media tools, and subtitle translation belong to the qplay repository. The extension, its Windows helper, and the browser connection belong to qcatch. qbutt owns torrent Range streaming and viewing history. The apps pass media and playback options to each other.

Component sources are available at the pinned Git revisions in the [qplay](https://github.com/qbutt-org/qplay) and [qcatch](https://github.com/qbutt-org/qcatch) repositories; third-party runtime sources are published in separate technical releases of those products. The build verifies their archives and keeps them in the dependency cache. Downloaded packages, generated GPU engines, and user caches are stored outside the source tree.

The former Bun implementation in `components/content/` remains for separate integration checks, but is excluded from the product build. The bundle records the source revision, working tree dirty state, and SHA-256 hashes of released files in `build-manifest.json`, along with their dependency licenses and notices.

| Path | Responsibility |
|---|---|
| `src/base/`, `src/gui/` | Native services, libtorrent integration, and the Qt interface |
| `src/content/`, `src/gui/quick/bridge/` | Native sources, library, downloads, and reading |
| `components/content/` | Former implementation for separate integration checks |
| `src/base/media/` | qplay integration and torrent Range streaming |
| `scripts/package-products.ts`, `upstream-lock.json` | Verification and packaging of pinned product components |

See the [Rust/QML and redesign plan](docs/implementation.md), [project rules](AGENTS.md), [content contracts](docs/agents/content.md), [architecture](docs/qbutt-architecture.md), and [integration checks](tests/README.md). Windows releases are built locally, without automatic builds in GitHub Actions.

The upstream [authors](AUTHORS), [licenses](COPYING), and dependency notices are preserved. IP country data: [DB-IP](https://db-ip.com/db/download/ip-to-country-lite), CC BY 4.0.
