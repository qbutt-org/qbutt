[CmdletBinding()]
param(
    [string] $SourceDir = (Split-Path $PSScriptRoot -Parent),
    [string] $BuildRoot = (Join-Path $env:LOCALAPPDATA 'qbutt/build'),
    [string] $DependencyRoot = (Join-Path $env:LOCALAPPDATA 'qbutt/dependencies'),
    [string] $CMakePath = (Join-Path $env:ProgramFiles 'CMake/bin/cmake.exe'),
    [ValidateRange(1, 64)] [int] $Parallel = 8
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

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
$DependencyRoot = [IO.Path]::GetFullPath($DependencyRoot)
New-Item -ItemType Directory -Force $BuildRoot, $DependencyRoot | Out-Null
$lockPath = Join-Path $SourceDir 'upstream-lock.json'
$lock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
$pins = $lock.windows

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
$bun = (Get-Command bun.exe -ErrorAction Stop).Source
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

$qtRoot = Join-Path $DependencyRoot "Qt/$($pins.qt.version)/$($pins.qt.directory)"
$qtArchives = Join-Path $DependencyRoot 'qt-archives'
$missingQtArchives = @($pins.qt.archives | Where-Object { -not (Test-Path (Join-Path $qtArchives $_.file)) })
if (-not (Test-Path "$qtRoot/bin/qmake.exe") -or $missingQtArchives.Count) {
    $python = Join-Path $DependencyRoot 'python/Scripts/python.exe'
    if (-not (Test-Path -LiteralPath $python)) {
        Invoke-Native python @('-m', 'venv', (Join-Path $DependencyRoot 'python'))
    }
    Invoke-Native $python @('-m', 'pip', 'install', "aqtinstall==$($pins.aqtinstall)")
    Push-Location $DependencyRoot
    try {
        Invoke-Native $python @('-m', 'aqt', 'install-qt', 'windows', 'desktop', $pins.qt.version, $pins.qt.architecture,
            '--archives', 'qtbase', 'qtsvg', 'qttools', 'qttranslations', '--modules', 'qtimageformats', '--outputdir', (Join-Path $DependencyRoot 'Qt'),
            '--keep', '--archive-dest', $qtArchives)
    }
    finally { Pop-Location }
}
foreach ($archive in $pins.qt.archives) {
    Assert-Sha256 (Join-Path $qtArchives $archive.file) $archive.sha256
}
$qtLicenses = Join-Path $DependencyRoot 'qt-licenses'
New-Item -ItemType Directory -Force $qtLicenses | Out-Null
$qtLicensePaths = @(foreach ($license in $pins.qt.licenseFiles) {
    if ([IO.Path]::GetFileName($license.file) -ne $license.file) { throw "Unsafe Qt license filename: $($license.file)" }
    $licensePath = Join-Path $qtLicenses $license.file
    Save-VerifiedDownload $licensePath $license.url $license.sha256
    $licensePath
})

$libtorrent = Join-Path $DependencyRoot 'qbutt-libtorrent'
Initialize-Source $libtorrent $pins.libtorrent
$common = @('-G', 'Ninja', '-DCMAKE_BUILD_TYPE=RelWithDebInfo', '-DCMAKE_CXX_COMPILER=cl',
    '-DCMAKE_EXPORT_COMPILE_COMMANDS=ON', "-DCMAKE_TOOLCHAIN_FILE=$vcpkg/scripts/buildsystems/vcpkg.cmake",
    "-DBOOST_ROOT=$boost/lib/cmake", '-DVCPKG_TARGET_TRIPLET=x64-windows-static-md-release')
Invoke-Native $cmake (@('-S', $libtorrent, '-B', "$libtorrent/build") + $common + @(
    '-DCMAKE_C_COMPILER=cl', '-DCMAKE_CXX_STANDARD=20', "-DCMAKE_INSTALL_PREFIX=$libtorrent/install",
    '-DBUILD_SHARED_LIBS=OFF', '-Ddeprecated-functions=OFF', '-Dstatic_runtime=OFF'))
Invoke-Native $cmake @('--build', "$libtorrent/build", '--parallel', "$Parallel")
Invoke-Native $cmake @('--install', "$libtorrent/build")

$net = Join-Path $DependencyRoot 'qbutt-net'
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
    "-DLibtorrentRasterbar_DIR=$libtorrent/install/lib/cmake/LibtorrentRasterbar", "-DCMAKE_PREFIX_PATH=$qtRoot",
    '-DMSVC_RUNTIME_DYNAMIC=ON', '-DTESTING=OFF', '-DQBUTT_STAGING_FAULTS=OFF', '-DQBUTT_COMPLETION_FAULTS=OFF'))
Invoke-Native $cmake @('--build', $BuildRoot, '--parallel', "$Parallel")

$portable = Join-Path $BuildRoot 'portable'
if (Test-Path -LiteralPath $portable) {
    $resolvedPortable = (Resolve-Path -LiteralPath $portable).Path
    if ($resolvedPortable -ne [IO.Path]::GetFullPath($portable) -or
        (Split-Path $resolvedPortable -Parent) -ne (Resolve-Path -LiteralPath $BuildRoot).Path -or
        ((Get-Item -LiteralPath $portable).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Portable output must be a regular directory directly inside BuildRoot.'
    }
    Remove-Item -LiteralPath $resolvedPortable -Recurse -Force
}
New-Item -ItemType Directory -Path $portable | Out-Null
New-Item -ItemType Directory -Path "$portable/profile" | Out-Null
'This directory keeps qbutt settings and session data next to the application.' | Set-Content "$portable/profile/README.txt"
Copy-Item -LiteralPath (Join-Path $BuildRoot 'qbutt.exe'), (Join-Path $BuildRoot 'qbutt-net.exe') -Destination $portable
Invoke-Native "$qtRoot/bin/windeployqt.exe" @('--release', '--no-compiler-runtime', '--include-plugins', 'qoffscreen',
    '--exclude-plugins', 'qsqlibase,qsqlmimer,qsqloci,qsqlodbc,qsqlpsql', '--dir', $portable, (Join-Path $portable 'qbutt.exe'))
Copy-Item -Path (Join-Path $env:VCToolsRedistDir 'x64/Microsoft.VC143.CRT/*.dll') -Destination $portable
Copy-Item -LiteralPath (Join-Path $SourceDir 'COPYING'), (Join-Path $SourceDir 'COPYING.GPLv2'), (Join-Path $SourceDir 'COPYING.GPLv3'),
    (Join-Path $SourceDir 'AUTHORS'), $lockPath -Destination $portable
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
Push-Location $net
try {
    Invoke-Native $bun @('scripts/collect-notices.ts', (Join-Path $portable 'qbutt-net.exe'), $licenses, $go)
}
finally { Pop-Location }
@"
qbutt is a fork of qBittorrent; see COPYING, COPYING.GPLv2, COPYING.GPLv3 and AUTHORS.
Source and build instructions: https://github.com/qbutt-org/qbutt
Exact source revisions and binary archive hashes: upstream-lock.json.

qbutt-net is a separate process under GPLv3 (licenses/qbutt-net.txt).
Source: $($lock.qbuttNet.repository -replace '\.git$', '')/tree/$($lock.qbuttNet.commit)
Linked dependency attributions: licenses/qbutt-net-notices.txt and licenses/qbutt-net-notices.json.
Go runtime license: licenses/go.txt. Pinned toolchain and standard library source:
$($pins.go.url)

Qt $($pins.qt.version) is dynamically linked under LGPLv3 (licenses/qt/qtbase-LGPL-3.0-only.txt).
Qt library and bundled third-party notices are recorded in licenses/*.spdx.json;
the corresponding license texts are included in licenses/qt/.
Corresponding Qt source, including third-party licenses and build instructions:
$($pins.qt.sourceUrl)
The Qt DLLs can be replaced with compatible modified builds of the pinned Qt version.

libtorrent source: $($pins.libtorrent.repository -replace '\.git$', '')/tree/$($pins.libtorrent.commit)
Boost source: $($pins.boost.url)
OpenSSL and zlib source recipes, source checksums, and patches:
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
    sourceRevision = $sourceRevision
    sourceWorktreeChanged = [bool] $sourceChanges
    configuration = 'RelWithDebInfo'
    compiler = (Get-Item (Get-Command cl.exe).Source).VersionInfo.FileVersion
    cmake = (& $cmake --version | Select-Object -First 1)
    ninja = (& "$ninja/ninja.exe" --version)
    go = $goVersion
    bun = (& $bun --version)
    qt = $pins.qt.version
    dependencyLockSha256 = (Get-FileHash -LiteralPath $lockPath -Algorithm SHA256).Hash.ToLowerInvariant()
} | ConvertTo-Json | Set-Content (Join-Path $portable 'build-manifest.json')
Compress-Archive -Path "$portable/*" -DestinationPath (Join-Path $BuildRoot 'qbutt-windows-x64.zip') -Force
Write-Output "Portable build: $portable"
