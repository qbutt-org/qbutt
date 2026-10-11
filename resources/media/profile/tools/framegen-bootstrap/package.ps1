[CmdletBinding()]
param(
    [switch]$RegenerateNotices
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$repoRoot = Split-Path (Split-Path $root -Parent) -Parent
$manifest = Join-Path $root 'Cargo.toml'
if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA 'qbutt\build\framegen'
}
$release = Join-Path $env:CARGO_TARGET_DIR 'release\framegen_bootstrap.dll'
$destination = Join-Path (Split-Path $root -Parent) 'bin\framegen-bootstrap.dll'
$notices = Join-Path (Split-Path $root -Parent) 'bin\framegen-bootstrap-THIRD-PARTY-NOTICES.txt'
$noticeScript = Join-Path (Split-Path $root -Parent) 'licenses\generate-rust-notices.ps1'
$rustBuildScript = Join-Path (Split-Path $root -Parent) 'licenses\RustBuild.ps1'
. $rustBuildScript
$noticeArguments = @{
    Component = 'framegen-bootstrap'
    Check = -not $RegenerateNotices
}
& $noticeScript @noticeArguments

$buildContext = Invoke-ReproducibleCargoBuild `
    -RepoRoot $repoRoot `
    -CargoArguments @('build', '--manifest-path', $manifest, '--release', '--locked')
if (-not (Test-Path -LiteralPath $release -PathType Leaf)) {
    throw "Release DLL is missing: $release"
}
Assert-NoPrivateBuildPaths -Path $release -SourcePaths $buildContext.SourcePaths

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
    throw 'Visual Studio vswhere.exe is required for PE QA.'
}
$visualStudio = & $vswhere -latest -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$dumpbin = Get-ChildItem -LiteralPath (Join-Path $visualStudio 'VC\Tools\MSVC') -Directory |
    Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'bin\Hostx64\x64\dumpbin.exe' } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if (-not $dumpbin) {
    throw 'dumpbin.exe is required for PE QA.'
}
$headers = (& $dumpbin /nologo /headers $release | Out-String)
if ($LASTEXITCODE -ne 0 -or $headers -notmatch '(?im)^\s*8664 machine \(x64\)\s*$') {
    throw 'framegen-bootstrap.dll is not an x64 PE DLL.'
}
$exports = (& $dumpbin /nologo /exports $release | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw 'dumpbin export inspection failed.'
}
foreach ($name in @(
    'mpv_open_cplugin',
    'pfn_mpv_get_property',
    'pfn_mpv_set_property_string',
    'pfn_mpv_free_node_contents'
)) {
    if ($exports -notmatch "(?m)\s$([regex]::Escape($name))\s*$") {
        throw "Required mpv C-plugin export is missing: $name"
    }
}
$dependents = (& $dumpbin /nologo /dependents $release | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw 'dumpbin dependency inspection failed.'
}
$allowedDependencies = @(
    'KERNEL32.dll',
    'api-ms-win-core-synch-l1-2-0.dll',
    'ntdll.dll'
)
$dependencies = [regex]::Matches(
    $dependents,
    '(?im)^\s+([a-z0-9_.-]+\.dll)\s*$'
) | ForEach-Object { $_.Groups[1].Value }
$unexpectedDependencies = @($dependencies | Where-Object { $_ -notin $allowedDependencies })
if ($unexpectedDependencies.Count -gt 0) {
    throw "Unexpected non-system DLL dependencies: $($unexpectedDependencies -join ', ')"
}

$staged = Join-Path (Split-Path $destination -Parent) ('.publish-' + [guid]::NewGuid().ToString('N') + '.tmp')
try {
    [IO.File]::Copy($release, $staged, $false)
    if (Test-Path -LiteralPath $destination -PathType Leaf) {
        [IO.File]::Replace(
            $staged,
            $destination,
            [Management.Automation.Language.NullString]::Value
        )
    }
    else {
        [IO.File]::Move($staged, $destination)
    }
}
finally {
    Remove-Item -LiteralPath $staged -Force -ErrorAction SilentlyContinue
}

$stream = [IO.File]::OpenRead($destination)
try {
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        $sha256 = ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}
finally {
    $stream.Dispose()
}

[ordered]@{
    path = $destination
    notices = $notices
    bytes = (Get-Item -LiteralPath $destination).Length
    sha256 = $sha256
    exports = 4
    dependencies = $dependencies
} | ConvertTo-Json
