[CmdletBinding()]
param(
    [string] $SourceDir = (Split-Path $PSScriptRoot -Parent),
    [string] $BuildRoot = 'C:/Tools/qbutt/build',
    [string] $ArtifactRoot,
    [string] $DependencyRoot = 'C:/Tools/qbutt/dependencies',
    [switch] $DependenciesOnly,
    [string] $QtRootOverride,
    [string] $LibtorrentSourceDir,
    [string] $LibtorrentBuildRoot,
    [string] $NetSourceDir,
    [string] $InstallerCompiler,
    [string] $CMakePath = (Join-Path $env:ProgramFiles 'CMake/bin/cmake.exe'),
    [ValidateRange(1, 64)] [int] $Parallel = 8
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$env:QT_COMMAND_LINE_PARSER_NO_GUI_MESSAGE_BOXES = '1'
$releaseVersion = (Get-Content -LiteralPath (Join-Path $SourceDir 'qbutt-version.txt') -Raw).Trim()
if ($releaseVersion -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
    throw 'Invalid qbutt release version.'
}
$expectedFixedVersion = "$($releaseVersion.Split('-')[0]).0"

function Invoke-Native([string] $Program, [string[]] $CommandArguments) {
    & $Program @CommandArguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed with exit code $LASTEXITCODE" }
}

function Assert-Sha256([string] $Path, [string] $Expected) {
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ne $Expected) {
        throw "SHA256 mismatch: $Path"
    }
}

function Save-VerifiedDownload([string] $Path, [string] $Url, [string] $Sha256) {
    if (-not (Test-Path -LiteralPath $Path)) {
        $partial = "$Path.$PID.partial"
        try {
            Invoke-Native curl.exe @('--fail', '--location', '--retry', '3', '--max-time', '600', '--output', $partial, $Url)
            Assert-Sha256 $partial $Sha256
            Move-Item -LiteralPath $partial -Destination $Path -Force
        }
        finally { if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial } }
    }
    Assert-Sha256 $Path $Sha256
}

function Initialize-Qt([string] $Root, $Pin, [string] $ArchiveDirectory, [switch] $External) {
    $required = @('bin/qmake.exe', 'bin/windeployqt.exe', 'bin/lrelease.exe',
        'include/QtGui/qstylehints.h', 'lib/cmake/Qt6/Qt6Config.cmake',
        'lib/cmake/Qt6Quick/Qt6QuickConfig.cmake', 'lib/cmake/Qt6QuickWidgets/Qt6QuickWidgetsConfig.cmake',
        'lib/cmake/Qt6QuickControls2/Qt6QuickControls2Config.cmake',
        'lib/cmake/Qt6ShaderTools/Qt6ShaderToolsConfig.cmake',
        'qml/QtQuick/Controls/qmldir', 'qml/Qt/labs/StyleKit/qmldir',
        'plugins/platforms/qwindows.dll', 'bin/Qt6Core.dll')
    $missing = @($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $Root $_) -PathType Leaf) })

    if (-not $External) {
        New-Item -ItemType Directory -Force $Root, $ArchiveDirectory | Out-Null
        $receipt = Join-Path $Root '.qbutt-qt-archives'
        $expected = ($Pin.archives | ForEach-Object { "$($_.sha256)  $($_.file)" }) -join "`n"
        $installed = if (Test-Path -LiteralPath $receipt) { (Get-Content -LiteralPath $receipt -Raw).TrimEnd() } else { '' }
        foreach ($archive in $Pin.archives) {
            $path = Join-Path $ArchiveDirectory $archive.file
            Save-VerifiedDownload $path $archive.url $archive.sha256
            if ($missing.Count -or $installed -ne $expected) {
                # Qt 6.12 archives contain the kit contents directly, without a version directory.
                Invoke-Native tar.exe @('-xf', $path, '-C', $Root)
            }
        }
        "[Paths]`nPrefix=.." | Set-Content -LiteralPath (Join-Path $Root 'bin/qt.conf') -Encoding ascii
        $expected | Set-Content -LiteralPath $receipt -Encoding ascii
    }

    foreach ($file in $required) {
        if (-not (Test-Path -LiteralPath (Join-Path $Root $file) -PathType Leaf)) {
            throw "Incomplete Qt installation: $file at $Root"
        }
    }
    $version = & (Join-Path $Root 'bin/qmake.exe') -query QT_VERSION
    if ($LASTEXITCODE -ne 0 -or $version -ne $Pin.version) {
        throw "Qt installation does not match upstream-lock.json: $Root"
    }
}

function Initialize-Source([string] $Path, $Pin) {
    if (-not (Test-Path -LiteralPath $Path)) {
        Invoke-Native git @('init', $Path)
        Invoke-Native git @('-C', $Path, 'remote', 'add', 'origin', $Pin.repository)
    }
    $revision = & git -C $Path rev-parse --verify --quiet HEAD
    if ($LASTEXITCODE -eq 1) {
        $remote = & git -C $Path remote get-url origin
        if ($LASTEXITCODE -ne 0 -or $remote -ne $Pin.repository) { throw "Unexpected dependency repository: $Path" }
        Invoke-Native git @('-C', $Path, 'fetch', '--depth', '1', 'origin', $Pin.commit)
        Invoke-Native git @('-C', $Path, 'checkout', '--detach', 'FETCH_HEAD')
        $revision = & git -C $Path rev-parse HEAD
    }
    if ($LASTEXITCODE -ne 0 -or $revision -ne $Pin.commit) {
        throw "Dependency revision differs from upstream-lock.json: $Path"
    }
    $changes = & git -C $Path status --porcelain --untracked-files=no --ignore-submodules=none
    if ($LASTEXITCODE -ne 0 -or $changes) { throw "Dependency source has tracked changes: $Path" }
    $submodules = & git -C $Path submodule status --recursive
    if ($LASTEXITCODE -ne 0 -or @($submodules | Where-Object { $_ -match '^[+U]' }).Count) {
        throw "Dependency submodules differ from pinned revisions: $Path"
    }
    if (@($submodules | Where-Object { $_ -match '^-' }).Count) {
        Invoke-Native git @('-C', $Path, 'submodule', 'update', '--init', '--recursive', '--depth', '1')
    }
}

if ($env:OS -ne 'Windows_NT') { throw 'This build requires Windows x64 and Visual Studio 2022 C++ tools.' }
$SourceDir = (Resolve-Path -LiteralPath $SourceDir).Path
$BuildRoot = [IO.Path]::GetFullPath($BuildRoot)
$ArtifactRoot = if ($ArtifactRoot) { [IO.Path]::GetFullPath($ArtifactRoot) } else { $BuildRoot }
$DependencyRoot = [IO.Path]::GetFullPath($DependencyRoot)
New-Item -ItemType Directory -Force $BuildRoot, $DependencyRoot, $ArtifactRoot | Out-Null
$lockPath = Join-Path $SourceDir 'upstream-lock.json'
$lock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
Assert-Sha256 (Join-Path $SourceDir 'resources/fonts/Neucha.ttf') $lock.fonts.neucha.sha256
$pins = $lock.windows
$qtRoot = if ($QtRootOverride) { [IO.Path]::GetFullPath($QtRootOverride) }
    else { Join-Path 'C:/Tools/Qt' "$($pins.qt.version)/$($pins.qt.directory)" }

$rustToolchainPath = Join-Path $SourceDir 'rust-toolchain.toml'
$rustToolchain = [regex]::Match((Get-Content -LiteralPath $rustToolchainPath -Raw),
    '(?m)^\s*channel\s*=\s*"(\d+\.\d+\.\d+)"\s*$')
if (-not $rustToolchain.Success) { throw 'rust-toolchain.toml must pin an exact Rust release.' }
$env:RUSTUP_TOOLCHAIN = $rustToolchain.Groups[1].Value
$cargo = & rustup which --toolchain $env:RUSTUP_TOOLCHAIN cargo
if ($LASTEXITCODE -ne 0) { throw "Install the Rust $env:RUSTUP_TOOLCHAIN toolchain with rustup." }
$rustc = & rustup which --toolchain $env:RUSTUP_TOOLCHAIN rustc
if ($LASTEXITCODE -ne 0) { throw "Cannot find rustc in the Rust $env:RUSTUP_TOOLCHAIN toolchain." }
$rustVersion = & $rustc --version --verbose
if ($LASTEXITCODE -ne 0 -or $rustVersion -notcontains "release: $env:RUSTUP_TOOLCHAIN" -or
    $rustVersion -notcontains 'host: x86_64-pc-windows-msvc') {
    throw 'Expected the pinned Windows x64 MSVC Rust compiler.'
}
$cargoLockPath = Join-Path $SourceDir 'Cargo.lock'
if (-not (Test-Path -LiteralPath $cargoLockPath -PathType Leaf)) { throw 'Cargo.lock is required for reproducible builds.' }
Push-Location $SourceDir
try { Invoke-Native $cargo @('fetch', '--locked', '--target', 'x86_64-pc-windows-msvc') }
finally { Pop-Location }

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$visualStudio = & $vswhere -latest -version '[17.0,18.0)' -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $visualStudio) { throw 'Install Visual Studio 2022 with the C++ desktop workload.' }
$environmentFile = Join-Path $BuildRoot 'msvc-environment.cmd'
@"
@echo off
call "$visualStudio\Common7\Tools\VsDevCmd.bat" -no_logo -arch=x64 -host_arch=x64 >nul
if errorlevel 1 exit /b %errorlevel%
set
"@ | Set-Content -LiteralPath $environmentFile
$environment = & cmd.exe /d /c "call `"$environmentFile`""
if ($LASTEXITCODE -ne 0) { throw 'MSVC environment initialization failed.' }
foreach ($line in $environment) {
    if ($line -match '^([^=]+)=(.*)$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
    }
}
$env:PATH = "${env:ProgramFiles}\CMake\bin;${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer;$env:PATH"
$cmake = (Get-Command $CMakePath -ErrorAction Stop).Source
$env:VCPKG_MAX_CONCURRENCY = "$Parallel"

$ninjaArchive = Join-Path $DependencyRoot 'ninja-win.zip'
Save-VerifiedDownload $ninjaArchive $pins.ninja.url $pins.ninja.sha256
$ninja = Join-Path $DependencyRoot 'ninja'
if (-not (Test-Path "$ninja/ninja.exe")) { Expand-Archive -LiteralPath $ninjaArchive -DestinationPath $ninja }
$env:PATH = "$ninja;$env:PATH"

$vcpkg = Join-Path $DependencyRoot 'vcpkg'
Initialize-Source $vcpkg $pins.vcpkg
if (-not (Test-Path "$vcpkg/vcpkg.exe")) {
    Invoke-Native "$vcpkg/bootstrap-vcpkg.bat" @('-disableMetrics')
}
$triplets = Join-Path $vcpkg 'triplets_overlay'
New-Item -ItemType Directory -Force $triplets | Out-Null
@'
set(VCPKG_TARGET_ARCHITECTURE x64)
set(VCPKG_LIBRARY_LINKAGE static)
set(VCPKG_CRT_LINKAGE dynamic)
set(VCPKG_BUILD_TYPE release)
'@ | Set-Content "$triplets/x64-windows-static-md-release.cmake"
Invoke-Native "$vcpkg/vcpkg.exe" @('install', 'openssl:x64-windows-static-md-release', 'zlib:x64-windows-static-md-release',
    "--vcpkg-root=$vcpkg", "--overlay-triplets=$triplets", '--disable-metrics')

$boost = Join-Path $DependencyRoot $pins.boost.directory
$boostArchive = "$boost.tar.gz"
Save-VerifiedDownload $boostArchive $pins.boost.url $pins.boost.sha256
if (-not (Test-Path "$boost/lib/cmake/Boost-$($pins.boost.version)/BoostConfig.cmake")) {
    Invoke-Native tar.exe @('-xf', $boostArchive, '-C', $DependencyRoot)
    Push-Location $boost
    try {
        Invoke-Native '.\bootstrap.bat' @()
        Invoke-Native '.\b2.exe' @('stage', 'toolset=msvc', '--stagedir=.', '--with-headers')
    }
    finally { Pop-Location }
}

$qtArchives = Join-Path $DependencyRoot "qt-archives/$($pins.qt.version)"
Initialize-Qt $qtRoot $pins.qt $qtArchives -External:([bool]$QtRootOverride)
$webviewArchive = Join-Path $DependencyRoot "Microsoft.Web.WebView2.$($pins.webview2.version).nupkg"
Save-VerifiedDownload $webviewArchive $pins.webview2.url $pins.webview2.sha256
$webviewSdk = Join-Path $DependencyRoot "WebView2/$($pins.webview2.version)"
Expand-Archive -LiteralPath $webviewArchive -DestinationPath $webviewSdk -Force

$libtorrent = if ($LibtorrentSourceDir) { [IO.Path]::GetFullPath($LibtorrentSourceDir) }
    else { Join-Path $DependencyRoot 'qbutt-libtorrent' }
Initialize-Source $libtorrent $pins.libtorrent
$libtorrentBuild = if ($LibtorrentBuildRoot) { [IO.Path]::GetFullPath($LibtorrentBuildRoot) } else { $libtorrent }
$common = @('-G', 'Ninja', '-DCMAKE_BUILD_TYPE=RelWithDebInfo', '-DCMAKE_CXX_COMPILER=cl',
    '-DCMAKE_EXPORT_COMPILE_COMMANDS=ON', "-DCMAKE_TOOLCHAIN_FILE=$vcpkg/scripts/buildsystems/vcpkg.cmake",
    "-DBOOST_ROOT=$boost/lib/cmake", '-DVCPKG_TARGET_TRIPLET=x64-windows-static-md-release')
Invoke-Native $cmake (@('-S', $libtorrent, '-B', "$libtorrentBuild/build") + $common + @(
    '-DCMAKE_C_COMPILER=cl', '-DCMAKE_CXX_STANDARD=20', "-DCMAKE_INSTALL_PREFIX=$libtorrentBuild/install",
    '-DBUILD_SHARED_LIBS=OFF', '-Ddeprecated-functions=OFF', '-Dstatic_runtime=OFF'))
Invoke-Native $cmake @('--build', "$libtorrentBuild/build", '--parallel', "$Parallel")
Invoke-Native $cmake @('--install', "$libtorrentBuild/build")

if ($DependenciesOnly) { return }

if (-not $InstallerCompiler) {
    $installerTools = Join-Path $DependencyRoot "inno-setup-$($pins.innoSetup.version)"
    $InstallerCompiler = Join-Path $installerTools 'ISCC.exe'
    if (-not (Test-Path -LiteralPath $InstallerCompiler)) {
        $installerDownload = Join-Path $DependencyRoot "innosetup-$($pins.innoSetup.version).exe"
        Save-VerifiedDownload $installerDownload $pins.innoSetup.url $pins.innoSetup.sha256
        $installerProcess = Start-Process -FilePath $installerDownload -ArgumentList @(
            '/CURRENTUSER', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/NOICONS', '/TASKS=',
            "/DIR=`"$installerTools`"") -WindowStyle Hidden -Wait -PassThru
        if ($installerProcess.ExitCode -ne 0) { throw "Inno Setup installation failed: $($installerProcess.ExitCode)" }
    }
}

$bunArchive = Join-Path $DependencyRoot "bun-$($pins.bun.version)-windows-x64-baseline.zip"
Save-VerifiedDownload $bunArchive $pins.bun.url $pins.bun.sha256
$bunToolchain = Join-Path $DependencyRoot "bun-$($pins.bun.version)"
$bun = Join-Path $bunToolchain "$($pins.bun.directory)/bun.exe"
if (-not (Test-Path -LiteralPath $bun)) {
    Expand-Archive -LiteralPath $bunArchive -DestinationPath $bunToolchain
}
$bunVersion = & $bun --version
if ($LASTEXITCODE -ne 0 -or $bunVersion -ne $pins.bun.version) {
    throw 'Bun does not match upstream-lock.json.'
}

$qtLicenses = Join-Path $DependencyRoot "qt-licenses/$($pins.qt.version)"
New-Item -ItemType Directory -Force $qtLicenses | Out-Null
$qtLicensePaths = @(foreach ($license in $pins.qt.licenseFiles) {
    if ([IO.Path]::GetFileName($license.file) -ne $license.file) { throw "Unsafe Qt license filename: $($license.file)" }
    $licensePath = Join-Path $qtLicenses $license.file
    Save-VerifiedDownload $licensePath $license.url $license.sha256
    $licensePath
})

$net = if ($NetSourceDir) { [IO.Path]::GetFullPath($NetSourceDir) } else { Join-Path $DependencyRoot 'qbutt-net' }
Initialize-Source $net $lock.qbuttNet
$goArchive = Join-Path $DependencyRoot "go$($pins.go.version).zip"
Save-VerifiedDownload $goArchive $pins.go.url $pins.go.sha256
$goToolchain = Join-Path $DependencyRoot "go$($pins.go.version)"
$go = Join-Path $goToolchain "$($pins.go.directory)/bin/go.exe"
if (-not (Test-Path -LiteralPath $go)) { Expand-Archive -LiteralPath $goArchive -DestinationPath $goToolchain }
$env:GOTOOLCHAIN = 'local'
$env:CGO_ENABLED = '0'
$env:GOOS = 'windows'
$env:GOARCH = 'amd64'
$env:GOAMD64 = 'v1'
Push-Location $net
try {
    Invoke-Native $go @('build', '-mod=readonly', '-trimpath', '-o', (Join-Path $BuildRoot 'qbutt-net.exe'), './cmd/qbutt-net')
    $goVersion = & $go version
    if ($LASTEXITCODE -ne 0) { throw 'Cannot record Go version.' }
}
finally { Pop-Location }

Invoke-Native $cmake (@('-S', $SourceDir, '-B', $BuildRoot) + $common + @(
    "-DLibtorrentRasterbar_DIR=$libtorrentBuild/install/lib/cmake/LibtorrentRasterbar", "-DCMAKE_PREFIX_PATH=$qtRoot",
    "-DQBUTT_WEBVIEW2_SDK=$webviewSdk", "-DRust_COMPILER=$rustc", "-DRust_CARGO=$cargo",
    '-DMSVC_RUNTIME_DYNAMIC=ON', '-DTESTING=OFF', '-DQBUTT_STAGING_FAULTS=OFF', '-DQBUTT_COMPLETION_FAULTS=OFF'))
Invoke-Native $cmake @('--build', $BuildRoot, '--parallel', "$Parallel")

$binaryVersion = (Get-Item -LiteralPath (Join-Path $BuildRoot 'qbutt.exe')).VersionInfo
$fixedFileVersion = '{0}.{1}.{2}.{3}' -f $binaryVersion.FileMajorPart, $binaryVersion.FileMinorPart,
    $binaryVersion.FileBuildPart, $binaryVersion.FilePrivatePart
$fixedProductVersion = '{0}.{1}.{2}.{3}' -f $binaryVersion.ProductMajorPart, $binaryVersion.ProductMinorPart,
    $binaryVersion.ProductBuildPart, $binaryVersion.ProductPrivatePart
if ($binaryVersion.FileVersion -ne $releaseVersion -or $binaryVersion.ProductVersion -ne $releaseVersion -or
    $fixedFileVersion -ne $expectedFixedVersion -or $fixedProductVersion -ne $expectedFixedVersion) {
    throw "Built qbutt.exe version does not match qbutt-version.txt ($releaseVersion)."
}

$portable = Join-Path $ArtifactRoot 'portable'
if (Test-Path -LiteralPath $portable) {
    $resolvedPortable = (Resolve-Path -LiteralPath $portable).Path
    if ($resolvedPortable -ne [IO.Path]::GetFullPath($portable) -or
        (Split-Path $resolvedPortable -Parent) -ne (Resolve-Path -LiteralPath $ArtifactRoot).Path -or
        ((Get-Item -LiteralPath $portable).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Portable output must be a regular directory directly inside ArtifactRoot.'
    }
    [IO.Directory]::Delete($resolvedPortable, $true)
}
New-Item -ItemType Directory -Path $portable | Out-Null
New-Item -ItemType Directory -Path "$portable/profile" | Out-Null
'This directory keeps qbutt settings and session data next to the application.' | Set-Content "$portable/profile/README.txt"
Copy-Item -LiteralPath (Join-Path $BuildRoot 'qbutt.exe'), (Join-Path $BuildRoot 'qbutt-net.exe'),
    (Join-Path $BuildRoot 'qbutt-webview.exe') -Destination $portable
Invoke-Native "$qtRoot/bin/windeployqt.exe" @('--release', '--quickwidgets', '--no-compiler-runtime', '--include-plugins', 'qoffscreen',
    '--exclude-plugins', 'qsqlibase,qsqlmimer,qsqloci,qsqlodbc,qsqlpsql',
    '--qmldir', (Join-Path $SourceDir 'src/gui/quick/qml'), '--dir', $portable, (Join-Path $portable 'qbutt.exe'))
Copy-Item -Path (Join-Path $env:VCToolsRedistDir 'x64/Microsoft.VC143.CRT/*.dll') -Destination $portable
Copy-Item -LiteralPath (Join-Path $SourceDir 'COPYING'), (Join-Path $SourceDir 'COPYING.GPLv2'), (Join-Path $SourceDir 'COPYING.GPLv3'),
    (Join-Path $SourceDir 'AUTHORS') -Destination $portable
Copy-Item -LiteralPath (Join-Path $SourceDir 'dist/windows/qt.conf') -Destination $portable
$licenses = Join-Path $portable 'licenses'
New-Item -ItemType Directory -Path $licenses | Out-Null
Copy-Item -LiteralPath "$net/LICENSE" -Destination "$licenses/qbutt-net.txt"
Copy-Item -LiteralPath "$goToolchain/$($pins.go.directory)/LICENSE" -Destination "$licenses/go.txt"
Copy-Item -LiteralPath "$libtorrent/LICENSE" -Destination "$licenses/libtorrent.txt"
Copy-Item -LiteralPath "$boost/LICENSE_1_0.txt" -Destination "$licenses/boost.txt"
Copy-Item -LiteralPath "$vcpkg/installed/x64-windows-static-md-release/share/openssl/copyright" -Destination "$licenses/openssl.txt"
Copy-Item -LiteralPath "$vcpkg/installed/x64-windows-static-md-release/share/zlib/copyright" -Destination "$licenses/zlib.txt"
$qtPortableLicenses = Join-Path $licenses 'qt'
New-Item -ItemType Directory -Path $qtPortableLicenses | Out-Null
Copy-Item -LiteralPath $qtLicensePaths -Destination $qtPortableLicenses
Copy-Item -Path "$qtRoot/sbom/*.spdx.json" -Destination $licenses
Copy-Item -LiteralPath (Join-Path $webviewSdk 'LICENSE.txt') -Destination "$licenses/WebView2-SDK.txt"
Copy-Item -LiteralPath (Join-Path $webviewSdk 'NOTICE.txt') -Destination "$licenses/WebView2-NOTICE.txt"
Copy-Item -LiteralPath (Join-Path $SourceDir 'resources/fonts/Neucha-OFL.txt') -Destination "$licenses/Neucha-OFL.txt"
Copy-Item -LiteralPath (Join-Path $SourceDir 'src/icons/LICENSE-Tabler') -Destination "$licenses/Tabler-Icons.txt"
Invoke-Native $bun @((Join-Path $SourceDir 'scripts/package-products.ts'), '--destination', $portable,
    '--cache', (Join-Path $DependencyRoot 'products'))
Invoke-Native $bun @((Join-Path $SourceDir 'scripts/collect-rust-notices.ts'), '--bundle', $portable,
    '--cargo', $cargo, '--rustc', $rustc)
Push-Location $net
try {
    Invoke-Native $bun @('scripts/collect-notices.ts', (Join-Path $portable 'qbutt-net.exe'), $licenses, $go)
}
finally { Pop-Location }
@"
qbutt is a fork of qBittorrent; see COPYING, COPYING.GPLv2, COPYING.GPLv3 and AUTHORS.
Source and build instructions: https://github.com/qbutt-org/qbutt

The native Rust content library and Qt Quick bridge are statically linked into qbutt.
Exact crate versions, source checksums and license files are recorded in
licenses/rust/dependencies.json. Rust standard-library notices are included in
licenses/rust/standard-library/.

Neucha by Jovanny Lemonad is bundled under the SIL Open Font License 1.1.
License: licenses/Neucha-OFL.txt. Pinned source:
$($lock.fonts.neucha.repository -replace '\.git$', '')/blob/$($lock.fonts.neucha.commit)/$($lock.fonts.neucha.path)

Reader interface glyphs from Tabler Icons are bundled under MIT.
License: licenses/Tabler-Icons.txt. Pinned source:
$($lock.assets.tablerIcons.repository -replace '\.git$', '')/tree/$($lock.assets.tablerIcons.commit)/icons/outline

qbutt-net is a separate process under GPLv3 (licenses/qbutt-net.txt).
Source: $($lock.qbuttNet.repository -replace '\.git$', '')/tree/$($lock.qbuttNet.commit)
Linked dependency attributions: licenses/qbutt-net-notices.txt and licenses/qbutt-net-notices.json.
Go runtime license: licenses/go.txt. Pinned toolchain and standard library source:
$($pins.go.url)

qbutt-webview uses the shared installed Microsoft Edge WebView2 Evergreen Runtime
for source verification. Its WebView2 SDK $($pins.webview2.version) loader is linked
statically under BSD-3-Clause (licenses/WebView2-SDK.txt and licenses/WebView2-NOTICE.txt).
Pinned SDK distribution: $($pins.webview2.url)

qplay is an independent mpv-based player installed alongside qbutt. Its prepared
profile, media tools, licenses and corresponding source archives belong to qplay.
qcatch is an optional independent browser extension and native capture application.
Their exact source commits, release versions and artifact SHA256 pins are recorded
in upstream-lock.json products. Compatible newer installations are reused.
The portable bundle contains qplay's independent portable distribution in qplay/.
The installer contains qplay's offline installer and preserves independent products
and their preferences when qbutt is uninstalled.

Qt $($pins.qt.version) is dynamically linked under LGPLv3 (licenses/qt/qtbase-LGPL-3.0-only.txt).
Qt library and bundled third-party notices are recorded in licenses/*.spdx.json;
the corresponding license texts are included in licenses/qt/.
Corresponding Qt source, including third-party licenses and build instructions:
$($pins.qt.sourceUrl)
The Qt DLLs can be replaced with compatible modified builds of the pinned Qt version.

libtorrent source: $($pins.libtorrent.repository -replace '\.git$', '')/tree/$($pins.libtorrent.commit)
Based on upstream libtorrent $($pins.libtorrent.upstream.tag):
$($pins.libtorrent.upstream.repository -replace '\.git$', '')/tree/$($pins.libtorrent.upstream.commit)
Boost source: $($pins.boost.url)
OpenSSL and zlib source recipes and patches:
https://github.com/microsoft/vcpkg/tree/$($pins.vcpkg.commit)/ports/openssl
https://github.com/microsoft/vcpkg/tree/$($pins.vcpkg.commit)/ports/zlib
Their license texts are included in licenses/.

Microsoft Visual C++ runtime libraries are redistributed from Visual Studio's
Microsoft.VC143.CRT directory under the Visual Studio redistributable terms.
"@ | Set-Content (Join-Path $portable 'THIRD-PARTY-NOTICES.txt')
$sourceRevision = & git -C $SourceDir rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot record source revision.' }
$sourceChanges = & git -C $SourceDir status --porcelain
if ($LASTEXITCODE -ne 0) { throw 'Cannot record source status.' }
[ordered]@{
    product = 'qbutt'
    version = $releaseVersion
    platform = 'windows-x64'
    sourceRevision = $sourceRevision
    sourceWorktreeChanged = [bool] $sourceChanges
    configuration = 'RelWithDebInfo'
    compiler = (Get-Item (Get-Command cl.exe).Source).VersionInfo.FileVersion
    cmake = (& $cmake --version | Select-Object -First 1)
    ninja = (& "$ninja/ninja.exe" --version)
    go = $goVersion
    bun = $bunVersion
    rust = $rustVersion -join "`n"
    qt = $pins.qt.version
    innoSetup = $pins.innoSetup.version
    dependencyLockSha256 = (Get-FileHash -LiteralPath $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
    cargoLockSha256 = (Get-FileHash -LiteralPath $cargoLockPath -Algorithm SHA256).Hash.ToLowerInvariant()
    rustToolchainSha256 = (Get-FileHash -LiteralPath $rustToolchainPath -Algorithm SHA256).Hash.ToLowerInvariant()
    products = $lock.products
    binaries = [ordered]@{
        qbutt = (Get-FileHash -LiteralPath (Join-Path $portable 'qbutt.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
        webview = (Get-FileHash -LiteralPath (Join-Path $portable 'qbutt-webview.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
        net = (Get-FileHash -LiteralPath (Join-Path $portable 'qbutt-net.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
    }
} | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $ArtifactRoot 'build-manifest.json')
Copy-Item -LiteralPath (Join-Path $ArtifactRoot 'build-manifest.json'), $lockPath -Destination $portable
$archive = Join-Path $ArtifactRoot "qbutt-$releaseVersion-windows-x64.zip"
$portableFiles = Get-ChildItem -LiteralPath $portable |
    Where-Object Name -ne 'components' | Select-Object -ExpandProperty FullName
Compress-Archive -Path $portableFiles -DestinationPath $archive -Force
& (Join-Path $SourceDir 'scripts/build-installer.ps1') -SourceDir $SourceDir -BundleDir $portable `
    -OutputDir $ArtifactRoot -CompilerPath $InstallerCompiler
Write-Output "Portable build: $portable"
Write-Output "Portable archive: $archive"
