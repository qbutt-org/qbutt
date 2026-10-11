# Read-only portable shader hardware policy, derived from mpv-config.
$ErrorActionPreference = 'Stop'

function New-ShaderProfileCandidate {
    param(
        [string]$Tier,
        [string]$Quality,
        [bool]$AllowGanAuto,
        [string]$MaxGanAuto,
        [int]$Rank
    )

    return [pscustomobject]@{
        hardware_tier = $Tier
        shader_quality = $Quality
        allow_gan_auto = $AllowGanAuto
        max_gan_auto = $MaxGanAuto
        rank = $Rank
    }
}

function Get-GpuClassFromModel {
    param([int]$Model)

    $suffix = $Model % 1000
    if ($suffix -ge 100) {
        return [int][math]::Floor($suffix / 10)
    }

    return $suffix
}

function ConvertTo-Megabytes {
    param([AllowNull()]$Bytes)

    $value = 0
    if ($null -eq $Bytes -or -not [int64]::TryParse([string]$Bytes, [ref]$value) -or $value -le 0) {
        return $null
    }

    return [int][math]::Round($value / 1MB)
}

function Get-NvidiaGenerationFromModel {
    param([int]$Model)

    return [int][math]::Floor($Model / 100)
}

function Get-PnpVendorId {
    param([string]$PnpDeviceId)

    if ($PnpDeviceId -match 'VEN_([0-9A-Fa-f]{4})') {
        return $Matches[1].ToUpperInvariant()
    }

    return ''
}

function Get-PnpDeviceId {
    param([string]$PnpDeviceId)

    if ($PnpDeviceId -match 'DEV_([0-9A-Fa-f]{4})') {
        return $Matches[1].ToUpperInvariant()
    }

    return ''
}

function Get-GpuVendor {
    param(
        [string]$Name,
        [string]$PnpDeviceId
    )

    $vendorId = Get-PnpVendorId -PnpDeviceId $PnpDeviceId
    if ($vendorId -eq '10DE') {
        return 'nvidia'
    }
    if ($vendorId -eq '1002' -or $vendorId -eq '1022') {
        return 'amd'
    }
    if ($vendorId -eq '8086') {
        return 'intel'
    }

    $text = ([string]$Name).ToLowerInvariant()
    if ($text -match 'nvidia|geforce|rtx|gtx') {
        return 'nvidia'
    }
    if ($text -match 'amd|radeon|rx\s*[0-9]') {
        return 'amd'
    }
    if ($text -match 'intel|iris|uhd|arc') {
        return 'intel'
    }

    return 'unknown'
}

function Test-IntegratedGpu {
    param(
        [string]$Name,
        [string]$Vendor,
        [string]$AdapterDacType
    )

    $text = ([string]$Name).ToLowerInvariant()
    if ($AdapterDacType -match 'internal') {
        return $true
    }

    if ($Vendor -eq 'nvidia') {
        return $false
    }

    if ($Vendor -eq 'amd') {
        if ($text -match '\brx\s*[0-9]{4,5}\b') {
            return $false
        }

        if ($text -match 'radeon\(tm\) graphics|radeon graphics|vega|[78][68]0m') {
            return $true
        }
    }

    if ($Vendor -eq 'intel') {
        if ($text -match 'arc\s*[ab]\s*[0-9]{3,4}') {
            return $false
        }

        return $true
    }

    return $false
}

function Get-DxgiAdapterInfo {
    if (-not ('DxgiGpuInfo' -as [type])) {
        Add-Type -Path (Join-Path $PSScriptRoot 'DxgiGpuInfo.cs')
    }

    try {
        return @([DxgiGpuInfo]::GetAdapters() | ForEach-Object {
            [pscustomobject]@{
                description = [string]$_.Description
                vendor_id = ('{0:X4}' -f $_.VendorId)
                device_id = ('{0:X4}' -f $_.DeviceId)
                dedicated_video_memory_mb = ConvertTo-Megabytes $_.DedicatedVideoMemory
            }
        })
    } catch {
        throw "Could not read DXGI adapter info: $($_.Exception.Message)"
    }
}

function Find-DxgiInfoForGpu {
    param(
        [Parameter(Mandatory=$true)]$Controller,
        [object[]]$DxgiInfo
    )

    $pnpVendorId = Get-PnpVendorId -PnpDeviceId ([string]$Controller.PNPDeviceID)
    $pnpDeviceId = Get-PnpDeviceId -PnpDeviceId ([string]$Controller.PNPDeviceID)
    foreach ($entry in @($DxgiInfo)) {
        if ($pnpVendorId -and $pnpDeviceId -and
            $entry.vendor_id -eq $pnpVendorId -and $entry.device_id -eq $pnpDeviceId) {
            return $entry
        }
    }

    $name = ([string]$Controller.Name).ToLowerInvariant()
    foreach ($entry in @($DxgiInfo)) {
        $description = ([string]$entry.description).ToLowerInvariant()
        if ($description -and ($name -like "*$description*" -or $description -like "*$name*")) {
            return $entry
        }
    }

    return $null
}

function Get-GpuPolicyInput {
    param(
        [Parameter(Mandatory=$true)]$Controller,
        [object[]]$DxgiInfo = @()
    )

    $name = [string]$Controller.Name
    $pnpDeviceId = [string]$Controller.PNPDeviceID
    $vendor = Get-GpuVendor -Name $name -PnpDeviceId $pnpDeviceId
    $adapterDacType = [string]$Controller.AdapterDACType
    $dxgiAdapter = Find-DxgiInfoForGpu -Controller $Controller -DxgiInfo $DxgiInfo
    $integrated = Test-IntegratedGpu -Name $name -Vendor $vendor -AdapterDacType $adapterDacType
    $dedicatedVramMb = if ($dxgiAdapter) { $dxgiAdapter.dedicated_video_memory_mb } else { $null }
    if (-not $dedicatedVramMb -and -not $integrated) {
        $dedicatedVramMb = ConvertTo-Megabytes $Controller.AdapterRAM
    }

    return @{
        name = $name
        dedicated_vram_mb = $dedicatedVramMb
        integrated = $integrated
    }
}

function Get-ShaderProfileCandidateForGpu {
    param(
        [System.Collections.IDictionary]$RawInfo
    )

    $text = ([string]$RawInfo.name).ToLowerInvariant()
    $vramMb = if ($RawInfo.dedicated_vram_mb) { [int]$RawInfo.dedicated_vram_mb } else { 0 }

    if ($text -match '(basic render|microsoft basic|vmware|virtualbox|remote)') {
        return New-ShaderProfileCandidate 'lite' 'fast' $false 'off' 0
    }

    if ($RawInfo.integrated) {
        return New-ShaderProfileCandidate 'balanced' 'fast' $false 'off' 1
    }

    if ($text -match '(?:geforce\s+)?rtx\s*([0-9]{4,5})') {
        $model = [int]$Matches[1]
        $generation = Get-NvidiaGenerationFromModel -Model $model
        $class = $model % 100

        if ($class -ge 90 -or ($generation -ge 40 -and $class -ge 80) -or ($generation -ge 50 -and $class -ge 70) -or $vramMb -ge 16000) {
            return New-ShaderProfileCandidate 'ultra' 'hq' $true 'x4' 3
        }

        if (($generation -ge 30 -and $class -ge 60) -or $vramMb -ge 8000) {
            return New-ShaderProfileCandidate 'high' 'hq' $true 'x2' 2
        }

        return New-ShaderProfileCandidate 'balanced' 'fast' $false 'off' 1
    }

    if ($text -match '(?:geforce\s+)?gtx\s*([0-9]{3,4})') {
        $model = [int]$Matches[1]
        $generation = Get-NvidiaGenerationFromModel -Model $model
        $class = $model % 100

        if (($generation -ge 16 -and $class -ge 60) -or ($generation -ge 10 -and $class -ge 70)) {
            return New-ShaderProfileCandidate 'high' 'hq' $false 'off' 2
        }

        return New-ShaderProfileCandidate 'balanced' 'fast' $false 'off' 1
    }

    if ($text -match '(?:radeon\s+)?rx\s*([0-9]{4,5})\s*(xtx|xt)?') {
        $model = [int]$Matches[1]
        $class = Get-GpuClassFromModel -Model $model
        $suffix = [string]$Matches[2]

        if ($class -ge 90 -or ($class -ge 80 -and $suffix -eq 'xtx') -or $vramMb -ge 16000) {
            return New-ShaderProfileCandidate 'ultra' 'hq' $true 'x4' 3
        }

        if ($class -ge 70 -or $vramMb -ge 8000) {
            return New-ShaderProfileCandidate 'high' 'hq' $true 'x2' 2
        }

        return New-ShaderProfileCandidate 'balanced' 'fast' $false 'off' 1
    }

    if ($text -match 'arc\s*([a-z])\s*([0-9]{3,4})') {
        $model = [int]$Matches[2]

        if ($model -ge 700) {
            return New-ShaderProfileCandidate 'high' 'hq' $true 'x2' 2
        }

        if ($model -ge 500) {
            return New-ShaderProfileCandidate 'high' 'hq' $false 'off' 2
        }

        return New-ShaderProfileCandidate 'balanced' 'fast' $false 'off' 1
    }

    if ($text -match '(intel|iris|uhd|radeon\(tm\) graphics|radeon graphics|vega|[78][68]0m|arc)') {
        return New-ShaderProfileCandidate 'balanced' 'fast' $false 'off' 1
    }

    if ($vramMb -ge 16000) {
        return New-ShaderProfileCandidate 'ultra' 'hq' $true 'x4' 3
    }

    if ($vramMb -ge 8000) {
        return New-ShaderProfileCandidate 'high' 'hq' $true 'x2' 2
    }

    return New-ShaderProfileCandidate 'balanced' 'fast' $false 'off' 1
}

function Get-ShaderHardwareProfile {
    $gpuControllers = @(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue |
        Where-Object { $_.Name })
    $dxgiInfo = Get-DxgiAdapterInfo
    $candidates = @($gpuControllers | ForEach-Object {
        $gpuInput = Get-GpuPolicyInput -Controller $_ -DxgiInfo $dxgiInfo
        Get-ShaderProfileCandidateForGpu -RawInfo $gpuInput
    })
    $bestCandidate = @($candidates | Sort-Object -Property rank -Descending | Select-Object -First 1)[0]

    if (-not $bestCandidate) {
        $bestCandidate = New-ShaderProfileCandidate 'balanced' 'fast' $false 'off' 1
    }

    $profile = [ordered]@{
        hardware_tier = $bestCandidate.hardware_tier
        shader_quality = $bestCandidate.shader_quality
        allow_gan_auto = $bestCandidate.allow_gan_auto
        max_gan_auto = $bestCandidate.max_gan_auto
    }

    return $profile
}

Get-ShaderHardwareProfile | ConvertTo-Json -Compress
