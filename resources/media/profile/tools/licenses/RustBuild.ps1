function Get-ReproducibleRustBuildContext {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    if ([string]::IsNullOrWhiteSpace([string]$env:USERPROFILE)) {
        throw 'USERPROFILE is required for a reproducible Rust build.'
    }

    $rustSysroot = (& rustc --print sysroot).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($rustSysroot)) {
        throw 'rustc --print sysroot failed.'
    }

    $cargoHome = if ([string]::IsNullOrWhiteSpace([string]$env:CARGO_HOME)) {
        Join-Path $env:USERPROFILE '.cargo'
    }
    else {
        $env:CARGO_HOME
    }
    $candidates = @(
        [pscustomobject]@{ Source = $rustSysroot; Destination = 'Z:\rust-toolchain' }
        [pscustomobject]@{ Source = $cargoHome; Destination = 'Z:\cargo-home' }
        [pscustomobject]@{ Source = $RepoRoot; Destination = 'Z:\mpv-config' }
        [pscustomobject]@{ Source = [IO.Path]::GetTempPath(); Destination = 'Z:\temp' }
        [pscustomobject]@{ Source = $env:USERPROFILE; Destination = 'Z:\user' }
    )
    $seen = [Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $mappings = [Collections.Generic.List[object]]::new()
    foreach ($candidate in $candidates) {
        $source = [IO.Path]::GetFullPath([string]$candidate.Source).TrimEnd(
            [char[]]@('\', '/')
        )
        if ($seen.Add($source)) {
            $mappings.Add([pscustomobject]@{
                Source = $source
                Destination = [string]$candidate.Destination
            })
        }
    }
    # rustc applies the last matching prefix, so parent paths must precede children.
    $orderedMappings = @($mappings | Sort-Object `
        @{ Expression = { $_.Source.Length }; Ascending = $true }, `
        @{ Expression = { $_.Source }; Ascending = $true })

    return [pscustomobject]@{
        Mappings = $orderedMappings
        SourcePaths = @($orderedMappings | ForEach-Object Source)
    }
}

function Invoke-ReproducibleCargoBuild {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,
        [Parameter(Mandatory = $true)]
        [string[]]$CargoArguments
    )

    $context = Get-ReproducibleRustBuildContext -RepoRoot $RepoRoot
    $previousRustFlags = [Environment]::GetEnvironmentVariable('RUSTFLAGS', 'Process')
    $previousEncodedRustFlags = [Environment]::GetEnvironmentVariable(
        'CARGO_ENCODED_RUSTFLAGS',
        'Process'
    )
    $previousCl = [Environment]::GetEnvironmentVariable('CL', 'Process')
    $previousIncremental = [Environment]::GetEnvironmentVariable(
        'CARGO_INCREMENTAL',
        'Process'
    )
    foreach ($ambient in @(
        [pscustomobject]@{ Name = 'RUSTFLAGS'; Value = $previousRustFlags }
        [pscustomobject]@{
            Name = 'CARGO_ENCODED_RUSTFLAGS'
            Value = $previousEncodedRustFlags
        }
        [pscustomobject]@{ Name = 'CL'; Value = $previousCl }
    )) {
        if (-not [string]::IsNullOrWhiteSpace([string]$ambient.Value)) {
            throw "$($ambient.Name) must be unset for a reproducible Rust package build."
        }
    }

    $rustFlags = [Collections.Generic.List[string]]::new()
    $rustFlags.Add('-C')
    $rustFlags.Add('target-feature=+crt-static')
    $rustFlags.Add('-C')
    $rustFlags.Add('link-arg=/Brepro')
    foreach ($mapping in $context.Mappings) {
        $rustFlags.Add(
            "--remap-path-prefix=$($mapping.Source)=$($mapping.Destination)"
        )
    }

    $clFlags = [Collections.Generic.List[string]]::new()
    foreach ($mapping in $context.Mappings) {
        $flag = "/pathmap:$($mapping.Source)=$($mapping.Destination)"
        if ($flag -match '\s') {
            $flag = '"' + $flag.Replace('"', '\"') + '"'
        }
        $clFlags.Add($flag)
    }

    try {
        [Environment]::SetEnvironmentVariable('RUSTFLAGS', $null, 'Process')
        [Environment]::SetEnvironmentVariable(
            'CARGO_ENCODED_RUSTFLAGS',
            ($rustFlags.ToArray() -join [char]0x1f),
            'Process'
        )
        [Environment]::SetEnvironmentVariable(
            'CL',
            ($clFlags.ToArray() -join ' '),
            'Process'
        )
        [Environment]::SetEnvironmentVariable('CARGO_INCREMENTAL', '0', 'Process')
        & cargo @CargoArguments | Out-Host
        if ($LASTEXITCODE -ne 0) {
            throw "cargo failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        [Environment]::SetEnvironmentVariable('RUSTFLAGS', $previousRustFlags, 'Process')
        [Environment]::SetEnvironmentVariable(
            'CARGO_ENCODED_RUSTFLAGS',
            $previousEncodedRustFlags,
            'Process'
        )
        [Environment]::SetEnvironmentVariable('CL', $previousCl, 'Process')
        [Environment]::SetEnvironmentVariable(
            'CARGO_INCREMENTAL',
            $previousIncremental,
            'Process'
        )
    }

    return $context
}

function Assert-NoPrivateBuildPaths {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [string[]]$SourcePaths
    )

    $bytes = [IO.File]::ReadAllBytes($Path)
    $texts = @(
        [Text.Encoding]::ASCII.GetString($bytes),
        [Text.Encoding]::Unicode.GetString($bytes)
    )
    foreach ($portablePath in @(
        'Z:\user\.cargo',
        'Z:\user\.rustup'
    )) {
        foreach ($candidate in @(
            $portablePath,
            $portablePath.Replace('\', '/')
        )) {
            foreach ($text in $texts) {
                if ($text.IndexOf($candidate, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                    throw "Built artifact contains a shadowed Rust build path: $candidate"
                }
            }
        }
    }
    foreach ($sourcePath in $SourcePaths) {
        foreach ($candidate in @(
            $sourcePath,
            $sourcePath.Replace('\', '/'),
            $sourcePath.Replace('/', '\')
        ) | Select-Object -Unique) {
            foreach ($text in $texts) {
                if ($text.IndexOf($candidate, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                    throw "Built artifact contains a private build path: $candidate"
                }
            }
        }
    }
}
