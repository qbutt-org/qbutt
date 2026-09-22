qbutt Windows distribution

Windows packaging is implemented by scripts/build-windows.ps1 at the repository
root. See the root README for build prerequisites and usage.

The script creates a portable directory, qbutt-VERSION-windows-x64.zip and
qbutt-VERSION-windows-x64-setup.exe under ArtifactRoot (BuildRoot by default).
The bundle contains qbutt.exe, qbutt-net.exe, Qt, the Visual C++ runtime and license
notices. Its profile directory keeps qbutt settings and session data beside the
application. Build provenance stays in ArtifactRoot/build-manifest.json.

CMakeLists.txt installs qt.conf. The portable build also copies qt.conf beside
qbutt.exe so Qt can find the bundled translations. windeployqt collects the Qt
runtime and translations.

qbutt.iss defines the per-user Inno Setup installer. The build script obtains the
compiler pinned in upstream-lock.json, or accepts its path via InstallerCompiler.
scripts/build-installer.ps1 can also package an already-built bundle. Installation
excludes the portable profile, creates a Start Menu shortcut, and optionally adds
a desktop shortcut. Uninstall removes installed program files and shortcuts while
preserving the user's profile, downloads and other files.
