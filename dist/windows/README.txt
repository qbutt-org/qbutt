qbutt Windows distribution

Windows packaging is implemented by scripts/build-windows.ps1 at the repository
root. See the root README for build prerequisites and usage.

The script creates a portable directory and qbutt-windows-x64.zip under BuildRoot.
The bundle contains qbutt.exe, qbutt-net.exe, Qt, the Visual C++ runtime, pinned
source revisions and license notices. Its profile directory keeps qbutt settings
and session data beside the application.

CMakeLists.txt installs qt.conf. The portable build also copies qt.conf beside
qbutt.exe so Qt can find the bundled translations. windeployqt collects the Qt
runtime and translations.

The inherited qBittorrent NSIS installer has been removed. Windows distribution
uses the portable bundle.
