[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$VcpkgRoot,

    [Parameter(Mandatory = $true)]
    [string]$OpenVinoWheel,

    [string]$OutputDirectory,

    [string]$DownloadCache
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Expected = @{
    VcpkgCommit = '62739e5f7b27f2e04abd051a61c4848940204d93'
    OpenVinoWheel = '197435104D18657A7CC4D05850615B913618EBB0D3BAAEC052FDA25A2D831C74'
    SourceArchive = '1BE96BA225BABDC28212CC7695B7538001AE4ABBC0117E8C72C8720C38C1032F'
    VapourSynthHeader = 'FAE4948F3FA8317A55287FB7D6586A73BD579933D595D376B60536CDD04CCAED'
    VapourSynthHelper = '79037D7E012295E67A0572BCDEA372B50D635479D3FD281764306A9DD49622BF'
    VcsPatch = '36878605EAA5C08008EB810942B284A2B82B4BE6D583743AE6A3FFC55EDF7A4B'
    VcpkgManifest = '55FE5AA5FAF2ACFC9CC543B90CD6A53BC3229F17EF07C8F051CB44C130EB70E3'
    SourceDownloads = '4EF875BE86E1C5CDCF8515D0FC590533D503D0B7711BB5C0D0E746C78FB77BA7'
    Modifications = '5A268C095FFA42D5B06FE5CB71712DAE148CE174E20337173FFC5CAFFDE14A06'
    Output = '25229F2263D6DFE2794E7140C8CDC6093E40C24F5FC840225B73E974DB65CD22'
}

function Get-Sha256([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try {
        return (($algorithm.ComputeHash($stream) | ForEach-Object { $_.ToString('x2') }) -join '').ToUpperInvariant()
    }
    finally {
        $algorithm.Dispose()
        $stream.Dispose()
    }
}

function Assert-Sha256([string]$Path, [string]$Sha256) {
    $actual = Get-Sha256 $Path
    if ($actual -ne $Sha256) {
        throw "SHA-256 mismatch for $Path`: expected $Sha256, got $actual"
    }
}

function Expand-Zip([string]$Archive, [string]$Destination) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($Archive, $Destination)
}

function Assert-FreshVcpkgRoot([string]$Root) {
    foreach ($relative in 'buildtrees', 'installed', 'packages') {
        $path = Join-Path $Root $relative
        if (Test-Path -LiteralPath $path -PathType Container) {
            $entry = Get-ChildItem -LiteralPath $path -Force | Select-Object -First 1
            if ($entry) {
                throw "vcpkg $relative must be empty for a clean reproduction: $path"
            }
        }
    }
}

function Assert-NoEmbeddedPath([string]$Path, [string[]]$SourcePaths) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $texts = @(
        [Text.Encoding]::ASCII.GetString($bytes),
        [Text.Encoding]::Unicode.GetString($bytes)
    )
    foreach ($sourcePath in $SourcePaths) {
        if ([string]::IsNullOrWhiteSpace($sourcePath)) {
            continue
        }
        $normalized = [IO.Path]::GetFullPath($sourcePath).TrimEnd(
            [char[]]@('\', '/')
        )
        foreach ($candidate in @(
            $normalized,
            $normalized.Replace('\', '/'),
            $normalized.Replace('/', '\')
        ) | Select-Object -Unique) {
            foreach ($text in $texts) {
                if ($text.IndexOf($candidate, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                    throw "VSOV output contains a private build path: $candidate"
                }
            }
        }
    }
}

function ConvertTo-MsvcPathMapFlag([string]$Source, [string]$Destination) {
    if ($Source.IndexOf('"') -ge 0 -or $Destination.IndexOf('"') -ge 0) {
        throw 'MSVC pathmap paths cannot contain a double quote.'
    }
    return '"/pathmap:' + $Source + '=' + $Destination + '"'
}

$sourceArchive = Join-Path $PSScriptRoot 'vs-mlrt-v15.16-source.zip'
$vapourSynthHeader = Join-Path $PSScriptRoot 'VapourSynth-R54-VapourSynth.h'
$vapourSynthHelper = Join-Path $PSScriptRoot 'VapourSynth-R54-VSHelper.h'
$vcpkgManifest = Join-Path $PSScriptRoot 'vcpkg.json'
$vcsPatch = Join-Path $PSScriptRoot 'deterministic-vcs-tag.patch'
$sourceDownloads = Join-Path $PSScriptRoot 'SOURCE-DOWNLOADS.json'
$modifications = Join-Path $PSScriptRoot 'MODIFICATIONS.md'
$ports = Join-Path $PSScriptRoot 'vcpkg-ports'
$repoRoot = Split-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) -Parent

Assert-Sha256 $sourceArchive $Expected.SourceArchive
Assert-Sha256 $vapourSynthHeader $Expected.VapourSynthHeader
Assert-Sha256 $vapourSynthHelper $Expected.VapourSynthHelper
Assert-Sha256 $OpenVinoWheel $Expected.OpenVinoWheel
Assert-Sha256 $vcsPatch $Expected.VcsPatch
Assert-Sha256 $vcpkgManifest $Expected.VcpkgManifest
Assert-Sha256 $sourceDownloads $Expected.SourceDownloads
Assert-Sha256 $modifications $Expected.Modifications
foreach ($port in 'onnx', 'protobuf', 'abseil', 'utf8-range') {
    if (-not (Test-Path -LiteralPath (Join-Path $ports "$port\portfile.cmake") -PathType Leaf)) {
        throw "Bundled vcpkg port snapshot is incomplete: $port"
    }
}

$VcpkgRoot = [IO.Path]::GetFullPath($VcpkgRoot)
foreach ($name in @('CL', '_CL_', 'CFLAGS', 'CXXFLAGS', 'LDFLAGS', 'VCPKG_DOWNLOADS')) {
    if (-not [string]::IsNullOrWhiteSpace(
        [Environment]::GetEnvironmentVariable($name, 'Process')
    )) {
        throw "$name must be unset for a reproducible VSOV build."
    }
}
$vcpkgCommit = (& git -C $VcpkgRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $vcpkgCommit -ne $Expected.VcpkgCommit) {
    throw "vcpkg must be checked out at $($Expected.VcpkgCommit)"
}
if ((& git -C $VcpkgRoot status --short)) {
    throw 'vcpkg checkout must be clean'
}
if (-not (Test-Path -LiteralPath (Join-Path $VcpkgRoot 'vcpkg.exe') -PathType Leaf)) {
    throw 'A bootstrapped vcpkg.exe is required in the clean checkout.'
}
Assert-FreshVcpkgRoot $VcpkgRoot
if (-not $DownloadCache) {
    $DownloadCache = Join-Path $VcpkgRoot 'downloads'
}
$DownloadCache = [IO.Path]::GetFullPath($DownloadCache)
[IO.Directory]::CreateDirectory($DownloadCache) | Out-Null

$cmakeVersion = (& cmake --version | Select-Object -First 1).Trim()
if ($cmakeVersion -ne 'cmake version 3.26.0') {
    throw "Expected CMake 3.26.0, got: $cmakeVersion"
}

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $env:TEMP ('mpv-config-vsov-build-' + [guid]::NewGuid().ToString('N'))
}
$work = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $work) {
    throw "OutputDirectory must not exist: $work"
}

$source = Join-Path $work 'source'
$sourceRoot = Join-Path $source 'vs-mlrt-v15.16'
$openvino = Join-Path $work 'openvino-wheel'
$include = Join-Path $work 'include'
$build = Join-Path $work 'build'
$triplets = Join-Path $work 'triplets'
$vcpkgInstalled = Join-Path $work 'vcpkg-installed'
New-Item -ItemType Directory -Path $source, $openvino, $include, $triplets | Out-Null
Expand-Zip $sourceArchive $source
Expand-Zip $OpenVinoWheel $openvino
if (-not (Test-Path -LiteralPath (Join-Path $sourceRoot 'vsov\CMakeLists.txt') -PathType Leaf)) {
    throw 'Source archive does not contain the expected vs-mlrt-v15.16 tree'
}
Copy-Item -LiteralPath $vapourSynthHeader -Destination (Join-Path $include 'VapourSynth.h')
Copy-Item -LiteralPath $vapourSynthHelper -Destination (Join-Path $include 'VSHelper.h')
Copy-Item -LiteralPath $vcpkgManifest -Destination (Join-Path $sourceRoot 'vsov\vcpkg.json')

& git -C $sourceRoot init --quiet
if ($LASTEXITCODE -ne 0) { throw 'git init failed' }
& git -C $sourceRoot apply --check $vcsPatch
if ($LASTEXITCODE -ne 0) { throw 'deterministic VCS tag patch does not apply' }
& git -C $sourceRoot apply $vcsPatch
if ($LASTEXITCODE -ne 0) { throw 'deterministic VCS tag patch failed' }

$toolchain = Join-Path $VcpkgRoot 'scripts\buildsystems\vcpkg.cmake'
$openvinoConfig = Join-Path $openvino 'openvino\cmake'
$pathMappings = @(
    [pscustomobject]@{
        Source = $DownloadCache
        Destination = 'Z:\mpv-config-vcpkg-downloads'
    }
    [pscustomobject]@{
        Source = $work
        Destination = 'Z:\mpv-config-vsov-build'
    }
    [pscustomobject]@{
        Source = $VcpkgRoot
        Destination = 'Z:\mpv-config-vcpkg'
    }
    [pscustomobject]@{
        Source = $repoRoot
        Destination = 'Z:\mpv-config'
    }
)
$compilerEnvironmentFlags = @(
    '/Brepro',
    '/experimental:deterministic'
)
$compilerEnvironmentFlags += @($pathMappings | ForEach-Object {
    ConvertTo-MsvcPathMapFlag -Source $_.Source -Destination $_.Destination
})
$compilerEnvironmentFlags = $compilerEnvironmentFlags -join ' '
$targetTriplet = @"
set(VCPKG_TARGET_ARCHITECTURE x64)
set(VCPKG_CRT_LINKAGE static)
set(VCPKG_LIBRARY_LINKAGE static)
set(VCPKG_CMAKE_SYSTEM_VERSION 10.0.22621.0)
set(VCPKG_ENV_PASSTHROUGH "CL")
"@
$hostTriplet = @"
set(VCPKG_TARGET_ARCHITECTURE x64)
set(VCPKG_CRT_LINKAGE dynamic)
set(VCPKG_LIBRARY_LINKAGE dynamic)
set(VCPKG_CMAKE_SYSTEM_VERSION 10.0.22621.0)
set(VCPKG_ENV_PASSTHROUGH "CL")
"@
[IO.File]::WriteAllText(
    (Join-Path $triplets 'x64-windows-static-mpv.cmake'),
    $targetTriplet,
    [Text.UTF8Encoding]::new($false)
)
[IO.File]::WriteAllText(
    (Join-Path $triplets 'x64-windows-mpv-host.cmake'),
    $hostTriplet,
    [Text.UTF8Encoding]::new($false)
)
$configure = @(
    '-S', (Join-Path $sourceRoot 'vsov'),
    '-B', $build,
    '-G', 'Visual Studio 17 2022',
    '-A', 'x64',
    '-T', 'v143',
    '-DCMAKE_SYSTEM_VERSION=10.0.22621.0',
    "-DCMAKE_TOOLCHAIN_FILE=$toolchain",
    '-DVCPKG_TARGET_TRIPLET=x64-windows-static-mpv',
    '-DVCPKG_HOST_TRIPLET=x64-windows-mpv-host',
    "-DVCPKG_OVERLAY_TRIPLETS=$triplets",
    "-DVCPKG_OVERLAY_PORTS=$ports",
    "-DVCPKG_INSTALLED_DIR=$vcpkgInstalled",
    "-DOpenVINO_DIR=$openvinoConfig",
    "-DVAPOURSYNTH_INCLUDE_DIRECTORY=$include",
    '-DWIN32_SHARED_OPENVINO=ON',
    '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded',
    '-DCMAKE_C_FLAGS_RELEASE=/O2 /Ob2 /DNDEBUG',
    '-DCMAKE_CXX_FLAGS_RELEASE=/O2 /Ob2 /DNDEBUG',
    '-DCMAKE_SHARED_LINKER_FLAGS_RELEASE=/INCREMENTAL:NO /Brepro'
)
$previousCl = [Environment]::GetEnvironmentVariable('CL', 'Process')
$previousBinarySources = [Environment]::GetEnvironmentVariable(
    'VCPKG_BINARY_SOURCES',
    'Process'
)
$previousDisableMetrics = [Environment]::GetEnvironmentVariable(
    'VCPKG_DISABLE_METRICS',
    'Process'
)
$previousDownloads = [Environment]::GetEnvironmentVariable(
    'VCPKG_DOWNLOADS',
    'Process'
)
try {
    [Environment]::SetEnvironmentVariable(
        'CL',
        $compilerEnvironmentFlags,
        'Process'
    )
    [Environment]::SetEnvironmentVariable('VCPKG_BINARY_SOURCES', 'clear', 'Process')
    [Environment]::SetEnvironmentVariable('VCPKG_DISABLE_METRICS', '1', 'Process')
    [Environment]::SetEnvironmentVariable(
        'VCPKG_DOWNLOADS',
        $DownloadCache,
        'Process'
    )
    & cmake @configure
    if ($LASTEXITCODE -ne 0) { throw 'CMake configure failed' }

    $compilerMetadata = Join-Path $build 'CMakeFiles\3.26.0\CMakeCXXCompiler.cmake'
    $projectFile = Join-Path $build 'vsov.vcxproj'
    $compilerText = [IO.File]::ReadAllText($compilerMetadata)
    $projectText = [IO.File]::ReadAllText($projectFile)
    if ($compilerText -notmatch 'CMAKE_CXX_COMPILER_VERSION "19\.41\.34120\.0"') {
        throw 'Expected MSVC compiler version 19.41.34120.0'
    }
    if ($projectText -notmatch '<PlatformToolset>v143</PlatformToolset>') {
        throw 'Expected Visual Studio platform toolset v143'
    }
    if ($projectText -notmatch '<WindowsTargetPlatformVersion>10\.0\.22621\.0</WindowsTargetPlatformVersion>') {
        throw 'Expected Windows SDK 10.0.22621.0'
    }

    & cmake --build $build --config Release --target vsov --parallel
    if ($LASTEXITCODE -ne 0) { throw 'VSOV build failed' }
}
finally {
    [Environment]::SetEnvironmentVariable('CL', $previousCl, 'Process')
    [Environment]::SetEnvironmentVariable(
        'VCPKG_BINARY_SOURCES',
        $previousBinarySources,
        'Process'
    )
    [Environment]::SetEnvironmentVariable(
        'VCPKG_DISABLE_METRICS',
        $previousDisableMetrics,
        'Process'
    )
    [Environment]::SetEnvironmentVariable(
        'VCPKG_DOWNLOADS',
        $previousDownloads,
        'Process'
    )
}

$output = Join-Path $build 'Release\vsov.dll'
Assert-NoEmbeddedPath `
    -Path $output `
    -SourcePaths @(
        $work,
        $VcpkgRoot,
        $DownloadCache,
        $repoRoot,
        [IO.Path]::GetTempPath(),
        $env:USERPROFILE
    )
$outputSha256 = Get-Sha256 $output
if ($Expected.Output) {
    Assert-Sha256 $output $Expected.Output
}
[pscustomobject]@{
    FullName = (Get-Item -LiteralPath $output).FullName
    Length = (Get-Item -LiteralPath $output).Length
    Sha256 = $outputSha256
    LastWriteTimeUtc = (Get-Item -LiteralPath $output).LastWriteTimeUtc
}
