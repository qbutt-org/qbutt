[CmdletBinding()]
param(
    [string] $SourceDir = (Split-Path $PSScriptRoot -Parent),
    [string] $DependencyRoot = (Join-Path $env:SystemDrive 'Tools/qbutt/dependencies'),
    [string] $QtRoot,
    [string] $CMakePath = (Join-Path $env:ProgramFiles 'CMake/bin/cmake.exe'),
    [string] $ClionToolchain
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'This setup requires Windows and Visual Studio 2022 C++ tools.' }

$SourceDir = (Resolve-Path -LiteralPath $SourceDir).Path
$DependencyRoot = [IO.Path]::GetFullPath($DependencyRoot)
$userPresetPath = Join-Path $SourceDir 'CMakeUserPresets.json'
if (Test-Path -LiteralPath $userPresetPath) {
    throw "Local presets already exist: $userPresetPath. Edit that file to preserve your settings."
}

$lock = Get-Content -LiteralPath (Join-Path $SourceDir 'upstream-lock.json') -Raw | ConvertFrom-Json
$prepare = @{
    SourceDir = $SourceDir
    DependencyRoot = $DependencyRoot
    DependenciesOnly = $true
}
if ($QtRoot) { $prepare.QtRootOverride = $QtRoot }
if (-not $QtRoot) {
    $QtRoot = Join-Path $env:SystemDrive "Tools/Qt/$($lock.windows.qt.version)/$($lock.windows.qt.directory)"
}
$QtRoot = [IO.Path]::GetFullPath($QtRoot)
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$visualStudio = & $vswhere -latest -version '[17.0,18.0)' -products '*' `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $visualStudio) {
    throw 'Install Visual Studio 2022 with the Desktop development with C++ workload.'
}

# Reuse the release dependency owner; do not introduce a second package installer.
& (Join-Path $SourceDir 'scripts/build-windows.ps1') @prepare

$libtorrentConfig = Join-Path $DependencyRoot 'qbutt-libtorrent/install/lib/cmake/LibtorrentRasterbar'
$vcpkg = Join-Path $DependencyRoot 'vcpkg'
$boost = Join-Path $DependencyRoot $lock.windows.boost.directory
$ninja = Join-Path $DependencyRoot 'ninja/ninja.exe'
$webviewSdk = Join-Path $DependencyRoot "WebView2/$($lock.windows.webview2.version)"
$buildDir = Join-Path $SourceDir 'build/dev'
New-Item -ItemType Directory -Path $buildDir -Force | Out-Null
$environmentFile = Join-Path $buildDir 'msvc-environment.cmd'
@"
@echo off
call "$visualStudio\Common7\Tools\VsDevCmd.bat" -no_logo -arch=x64 -host_arch=x64 >nul
if errorlevel 1 exit /b %errorlevel%
set
"@ | Set-Content -LiteralPath $environmentFile
$lines = & cmd.exe /d /c "call `"$environmentFile`""
if ($LASTEXITCODE -ne 0) { throw 'Visual Studio 2022 environment initialization failed.' }
$msvc = @{}
foreach ($line in $lines) {
    if ($line -match '^([^=]+)=(.*)$') { $msvc[$matches[1]] = $matches[2] }
}
$compiler = Join-Path $msvc.VCToolsInstallDir 'bin/Hostx64/x64/cl.exe'
if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) { throw 'The x64 MSVC 2022 compiler is missing.' }

$preset = [ordered]@{
    name = 'windows'
    displayName = 'Windows MSVC 2022'
    inherits = 'windows-msvc'
    binaryDir = '${sourceDir}/build/dev'
    cacheVariables = [ordered]@{
        CMAKE_CXX_COMPILER = $compiler.Replace('\', '/')
        CMAKE_MAKE_PROGRAM = $ninja.Replace('\', '/')
        CMAKE_TOOLCHAIN_FILE = (Join-Path $vcpkg 'scripts/buildsystems/vcpkg.cmake').Replace('\', '/')
        CMAKE_PREFIX_PATH = $QtRoot.Replace('\', '/')
        LibtorrentRasterbar_DIR = $libtorrentConfig.Replace('\', '/')
        Boost_DIR = (Join-Path $boost "lib/cmake/Boost-$($lock.windows.boost.version)").Replace('\', '/')
        VCPKG_TARGET_TRIPLET = 'x64-windows-static-md-release'
        QBUTT_WEBVIEW2_SDK = $webviewSdk.Replace('\', '/')
    }
    environment = [ordered]@{
        INCLUDE = $msvc.INCLUDE
        LIB = $msvc.LIB
        LIBPATH = $msvc.LIBPATH
        PATH = "$(Join-Path $QtRoot 'bin');$(Split-Path $ninja);$($msvc.Path)"
        QT_COMMAND_LINE_PARSER_NO_GUI_MESSAGE_BOXES = '1'
    }
}
if ($ClionToolchain) {
    $preset.vendor = @{ 'jetbrains.com/clion' = @{ toolchain = $ClionToolchain } }
}
$presets = [ordered]@{
    version = 6
    configurePresets = @($preset)
    buildPresets = @(@{ name = 'windows'; configurePreset = 'windows' })
}
$presets | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $userPresetPath -Encoding utf8

Push-Location $SourceDir
try {
    & $CMakePath --preset windows
    if ($LASTEXITCODE -ne 0) { throw 'Development configure failed. See the CMake error above.' }
}
finally { Pop-Location }

Write-Host "Ready: $userPresetPath"
Write-Host 'In CLion, enable the windows preset and disable its old default Debug profile.'
