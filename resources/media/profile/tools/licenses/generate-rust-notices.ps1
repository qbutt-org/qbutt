[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('framegen-manager', 'framegen-bootstrap')]
    [string]$Component,

    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$cargoAboutVersion = '0.9.1'
$rustRelease = '1.95.0'
$rustCommit = '59807616e1fa2540724bfbac14d7976d7e4a3860'
$target = 'x86_64-pc-windows-msvc'
$rustStdlibCopyrightBytes = 425661
$rustStdlibCopyrightSha256 = '90567e2718bf7fd65a71a3a43c5596488e80e5f51ed02bfea6fec54458b5f3d1'
$repoRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$projectLicense = Join-Path $repoRoot 'tools\framegen-manager\LICENSE'
$config = Join-Path $PSScriptRoot 'rust-about.toml'
$template = Join-Path $PSScriptRoot 'rust-notices.hbs'
$rustStdlibNoticeRelative = 'tools/bin/Rust-Standard-Library-COPYRIGHT.html'
$rustStdlibNotice = Join-Path $repoRoot ($rustStdlibNoticeRelative -replace '/', '\')

function Get-FileSha256 {
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

switch ($Component) {
    'framegen-manager' {
        $manifest = Join-Path $repoRoot 'tools\framegen-manager\src-tauri\Cargo.toml'
        $lockRelative = 'tools/framegen-manager/src-tauri/Cargo.lock'
    }
    'framegen-bootstrap' {
        $manifest = Join-Path $repoRoot 'tools\framegen-bootstrap\Cargo.toml'
        $lockRelative = 'tools/framegen-bootstrap/Cargo.lock'
    }
}

$lock = Join-Path $repoRoot ($lockRelative -replace '/', '\')
$destination = Join-Path $repoRoot "tools\bin\$Component-THIRD-PARTY-NOTICES.txt"
foreach ($path in @($manifest, $lock, $projectLicense, $config, $template)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required notice input is missing: $path"
    }
}

$rustVersionOutput = (& rustc -Vv 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "rustc -Vv failed with exit code $LASTEXITCODE"
}
$actualRustRelease = [regex]::Match($rustVersionOutput, '(?m)^release:\s*(\S+)\s*$').Groups[1].Value
$actualRustCommit = [regex]::Match($rustVersionOutput, '(?m)^commit-hash:\s*([0-9a-f]+)\s*$').Groups[1].Value
$actualRustHost = [regex]::Match($rustVersionOutput, '(?m)^host:\s*(\S+)\s*$').Groups[1].Value
if ($actualRustRelease -ne $rustRelease -or $actualRustCommit -ne $rustCommit -or $actualRustHost -ne $target) {
    throw "rustc $rustRelease ($rustCommit) for $target is required to build notices and release binaries. Found:`n$rustVersionOutput"
}

$rustSysroot = (& rustc --print sysroot 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "rustc --print sysroot failed with exit code $LASTEXITCODE"
}
$rustStdlibCopyright = Join-Path $rustSysroot 'share\doc\rust\COPYRIGHT-library.html'
if (-not (Test-Path -LiteralPath $rustStdlibCopyright -PathType Leaf)) {
    throw "Rust standard-library copyright file is missing from the exact toolchain: $rustStdlibCopyright"
}
$actualRustStdlibCopyrightBytes = (Get-Item -LiteralPath $rustStdlibCopyright).Length
$actualRustStdlibCopyrightSha256 = Get-FileSha256 -Path $rustStdlibCopyright
if (
    $actualRustStdlibCopyrightBytes -ne $rustStdlibCopyrightBytes -or
    $actualRustStdlibCopyrightSha256 -ne $rustStdlibCopyrightSha256
) {
    throw "Rust standard-library copyright file does not match rustc $rustRelease ($rustCommit): $rustStdlibCopyright"
}

$installedVersion = (& cargo about --version 2>&1 | Out-String).Trim()
if ($installedVersion -ne "cargo-about $cargoAboutVersion") {
    throw "cargo-about $cargoAboutVersion is required. Install it with: cargo install cargo-about --version $cargoAboutVersion --locked --features cli"
}

& cargo fetch --manifest-path $manifest --locked
if ($LASTEXITCODE -ne 0) {
    throw "cargo fetch failed with exit code $LASTEXITCODE"
}

$generatedDependencies = [IO.Path]::GetTempFileName()
try {
    & cargo about generate `
        --manifest-path $manifest `
        --config $config `
        --target $target `
        --frozen `
        --fail `
        --output-file $generatedDependencies `
        $template
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-about failed with exit code $LASTEXITCODE"
    }

    $utf8NoBom = [Text.UTF8Encoding]::new($false)
    $licenseText = [IO.File]::ReadAllText($projectLicense).Replace("`r`n", "`n").TrimEnd()
    $dependencyText = [regex]::Replace(
        [IO.File]::ReadAllText($generatedDependencies).Replace("`r`n", "`n"),
        '(?m)[\t ]+$',
        ''
    ).TrimEnd()
    $lockSha256 = Get-FileSha256 -Path $lock
    $expected = @"
$Component

This binary is part of qbutt and is licensed under the MIT License.
Its third-party dependency notices are generated from the exact Cargo.lock
normal/build dependency graph for $target; dev-only dependencies are excluded.

Lockfile: $lockRelative
Lockfile SHA-256: $lockSha256
Generator: cargo-about $cargoAboutVersion
Rust compiler: $rustRelease ($rustCommit)
Rust target: $target
Rust standard-library copyright: $rustStdlibNoticeRelative
Rust standard-library copyright bytes: $rustStdlibCopyrightBytes
Rust standard-library copyright SHA-256: $rustStdlibCopyrightSha256

PROJECT LICENSE

$licenseText

$dependencyText
"@.Replace("`r`n", "`n").TrimEnd() + "`n"

    if ($Check) {
        if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
            throw "Rust third-party notices are missing: $destination. Regenerate them with the component package.ps1 -RegenerateNotices."
        }

        $actual = [IO.File]::ReadAllText($destination).Replace("`r`n", "`n")
        if ($actual -ne $expected) {
            throw "Rust third-party notices are stale: $destination. Regenerate them with the component package.ps1 -RegenerateNotices."
        }

        if (-not (Test-Path -LiteralPath $rustStdlibNotice -PathType Leaf)) {
            throw "Rust standard-library copyright is missing: $rustStdlibNotice. Regenerate notices with the component package.ps1 -RegenerateNotices."
        }
        $actualNoticeBytes = (Get-Item -LiteralPath $rustStdlibNotice).Length
        $actualNoticeSha256 = Get-FileSha256 -Path $rustStdlibNotice
        if ($actualNoticeBytes -ne $rustStdlibCopyrightBytes -or $actualNoticeSha256 -ne $rustStdlibCopyrightSha256) {
            throw "Rust standard-library copyright is stale: $rustStdlibNotice. Regenerate notices with the component package.ps1 -RegenerateNotices."
        }

        Write-Host "Rust third-party notices and standard-library copyright are current: $destination"
        return
    }

    $stdlibStaged = Join-Path (Split-Path $rustStdlibNotice -Parent) ('.rust-stdlib-' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::Copy($rustStdlibCopyright, $stdlibStaged, $false)
        if (Test-Path -LiteralPath $rustStdlibNotice -PathType Leaf) {
            [IO.File]::Replace(
                $stdlibStaged,
                $rustStdlibNotice,
                [Management.Automation.Language.NullString]::Value
            )
        }
        else {
            [IO.File]::Move($stdlibStaged, $rustStdlibNotice)
        }
    }
    finally {
        Remove-Item -LiteralPath $stdlibStaged -Force -ErrorAction SilentlyContinue
    }

    $staged = Join-Path (Split-Path $destination -Parent) ('.notices-' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::WriteAllText($staged, $expected, $utf8NoBom)
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

    Write-Host "Updated Rust third-party notices: $destination"
}
finally {
    Remove-Item -LiteralPath $generatedDependencies -Force -ErrorAction SilentlyContinue
}
