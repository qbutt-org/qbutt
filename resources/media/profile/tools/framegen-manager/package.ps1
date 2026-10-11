[CmdletBinding()]
param(
    [switch]$RegenerateNotices
)

$ErrorActionPreference = 'Stop'

function Get-Sha256 {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $stream = [IO.File]::OpenRead($Path)
    try {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try {
            return ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
        }
        finally {
            $algorithm.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Assert-BundledAsset {
    param(
        [Parameter(Mandatory)]
        [string]$Path,
        [Parameter(Mandatory)]
        [long]$Bytes,
        [Parameter(Mandatory)]
        [string]$Sha256
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Bundled frame-generation asset is missing: $Path"
    }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Bundled frame-generation asset must be a normal file: $Path"
    }
    if ($item.Length -ne $Bytes) {
        throw "Bundled frame-generation asset has $($item.Length) bytes; expected ${Bytes}: $Path"
    }
    if ((Get-Sha256 -Path $Path) -ne $Sha256) {
        throw "Bundled frame-generation asset failed SHA-256 verification: $Path"
    }
}

function Assert-BundledTree {
    param(
        [Parameter(Mandatory)]
        [string]$Root,
        [Parameter(Mandatory)]
        [string]$Manifest,
        [Parameter(Mandatory)]
        [int]$ExpectedFiles
    )

    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        throw "Bundled source tree is missing: $Root"
    }
    $rootItem = Get-Item -LiteralPath $Root -Force
    if (($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Bundled source tree root must be a normal directory: $Root"
    }

    $lines = @(Get-Content -LiteralPath $Manifest)
    if ($lines.Count -ne $ExpectedFiles) {
        throw "Bundled source manifest has $($lines.Count) entries; expected ${ExpectedFiles}: $Manifest"
    }
    $expectedHashes = [Collections.Generic.Dictionary[string, string]]::new(
        [StringComparer]::Ordinal
    )
    $expectedPaths = [Collections.Generic.List[string]]::new()
    foreach ($line in $lines) {
        $match = [regex]::Match($line, '^([0-9a-f]{64})  ([A-Za-z0-9._/-]+)$')
        if (-not $match.Success) {
            throw "Bundled source manifest has a noncanonical entry: $line"
        }
        $relative = $match.Groups[2].Value
        $segments = @($relative.Split('/'))
        if ($relative.StartsWith('/') -or
            $relative.EndsWith('/') -or
            $relative.Contains('//') -or
            [IO.Path]::IsPathRooted($relative) -or
            @($segments | Where-Object { $_ -eq '.' -or $_ -eq '..' }).Count -gt 0) {
            throw "Bundled source manifest has an unsafe path: $relative"
        }
        if ($expectedHashes.ContainsKey($relative)) {
            throw "Bundled source manifest repeats a path: $relative"
        }
        $expectedHashes.Add($relative, $match.Groups[1].Value)
        $expectedPaths.Add($relative)
    }

    $sortedExpected = $expectedPaths.ToArray()
    [Array]::Sort($sortedExpected, [StringComparer]::Ordinal)
    for ($index = 0; $index -lt $expectedPaths.Count; $index++) {
        if ($expectedPaths[$index] -cne $sortedExpected[$index]) {
            throw 'Bundled source manifest paths are not in canonical ordinal order.'
        }
    }

    $rootFull = $rootItem.FullName.TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    $prefix = $rootFull + [IO.Path]::DirectorySeparatorChar
    $manifestFull = (Get-Item -LiteralPath $Manifest -Force).FullName
    $entries = @(Get-ChildItem -LiteralPath $Root -Force -Recurse)
    $reparseEntries = @($entries | Where-Object {
        ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    })
    if ($reparseEntries.Count -gt 0) {
        throw "Bundled source tree contains a reparse point: $($reparseEntries[0].FullName)"
    }
    $actualPaths = @($entries | Where-Object {
        -not $_.PSIsContainer -and $_.FullName -cne $manifestFull
    } | ForEach-Object {
        if (-not $_.FullName.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Bundled source file resolves outside its root: $($_.FullName)"
        }
        $_.FullName.Substring($prefix.Length).Replace(
            [IO.Path]::DirectorySeparatorChar,
            '/'
        )
    })
    [Array]::Sort($actualPaths, [StringComparer]::Ordinal)
    if ($actualPaths.Count -ne $sortedExpected.Count) {
        throw "Bundled source tree has $($actualPaths.Count) files; expected $($sortedExpected.Count)."
    }
    for ($index = 0; $index -lt $sortedExpected.Count; $index++) {
        if ($actualPaths[$index] -cne $sortedExpected[$index]) {
            throw "Bundled source tree does not match its manifest: expected $($sortedExpected[$index]), found $($actualPaths[$index])."
        }
        $source = Join-Path $rootFull $actualPaths[$index].Replace(
            '/',
            [IO.Path]::DirectorySeparatorChar
        )
        if ((Get-Sha256 -Path $source) -ne $expectedHashes[$actualPaths[$index]]) {
            throw "Bundled source file failed SHA-256 verification: $source"
        }
    }
}

$managerRoot = $PSScriptRoot
$repoRoot = Split-Path (Split-Path $managerRoot -Parent) -Parent
$manifest = Join-Path $managerRoot 'src-tauri\Cargo.toml'
if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = Join-Path $env:LOCALAPPDATA 'qbutt\build\framegen'
}
$release = Join-Path $env:CARGO_TARGET_DIR 'release\framegen-manager.exe'
$destination = Join-Path $repoRoot 'tools\bin\framegen-manager.exe'
$notices = Join-Path $repoRoot 'tools\bin\framegen-manager-THIRD-PARTY-NOTICES.txt'
$noticeScript = Join-Path $repoRoot 'tools\licenses\generate-rust-notices.ps1'
$rustBuildScript = Join-Path $repoRoot 'tools\licenses\RustBuild.ps1'
. $rustBuildScript
$noticeArguments = @{
    Component = 'framegen-manager'
    Check = -not $RegenerateNotices
}
& $noticeScript @noticeArguments

$vsovSourceRoot = Join-Path $repoRoot 'tools\framegen-vsov'
$vsovSourceManifest = Join-Path $vsovSourceRoot 'MANIFEST.sha256'
Assert-BundledAsset `
    -Path $vsovSourceManifest `
    -Bytes 4262 `
    -Sha256 '5549597303b5dab51be002ebe617fca9bed4ea51e3e477204d95d9c931b62d34'
Assert-BundledTree `
    -Root $vsovSourceRoot `
    -Manifest $vsovSourceManifest `
    -ExpectedFiles 40
Assert-BundledAsset `
    -Path (Join-Path $repoRoot 'tools\bin\framegen-vsov.dll') `
    -Bytes 4231168 `
    -Sha256 '25229f2263d6dfe2794e7140c8cdc6093e40c24f5fc840225b73e974db65cd22'
Assert-BundledAsset `
    -Path (Join-Path $repoRoot 'framegen\models\openvino\rife_v2\rife_v4.26.onnx') `
    -Bytes 22751073 `
    -Sha256 '3a958289369a5645339b2fbd5f3a228bc82075866c2b4723e4ca485fda72fc6b'

$buildContext = Invoke-ReproducibleCargoBuild `
    -RepoRoot $repoRoot `
    -CargoArguments @(
        'build',
        '--manifest-path', $manifest,
        '--release',
        '--locked',
        '--bin', 'framegen-manager'
    )
if (-not (Test-Path -LiteralPath $release -PathType Leaf)) {
    throw "Release binary is missing: $release"
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
    throw 'framegen-manager.exe is not an x64 PE executable.'
}
$dependents = (& $dumpbin /nologo /dependents $release | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw 'dumpbin dependency inspection failed.'
}
$dependencies = [regex]::Matches(
    $dependents,
    '(?im)^\s+([a-z0-9_.-]+\.dll)\s*$'
) | ForEach-Object { $_.Groups[1].Value }
$forbiddenDependencies = @($dependencies | Where-Object {
    $_ -match '(?i)^(?:(?:msvcp|msvcr|vcruntime|concrt|vcomp)[a-z0-9_]*|ucrtbase|api-ms-win-crt-[a-z0-9-]+)\.dll$'
})
if ($forbiddenDependencies.Count -gt 0) {
    throw "Dynamic Visual C++/UCRT dependencies are forbidden: $($forbiddenDependencies -join ', ')"
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

$sha256 = Get-Sha256 -Path $destination

[ordered]@{
    path = $destination
    notices = $notices
    bytes = (Get-Item -LiteralPath $destination).Length
    sha256 = $sha256
    dependencies = $dependencies
} | ConvertTo-Json
