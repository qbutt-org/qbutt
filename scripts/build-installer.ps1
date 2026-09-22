[CmdletBinding()]
param(
    [string] $SourceDir = (Split-Path $PSScriptRoot -Parent),
    [Parameter(Mandatory)] [string] $BundleDir,
    [Parameter(Mandatory)] [string] $OutputDir,
    [Parameter(Mandatory)] [string] $CompilerPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$SourceDir = (Resolve-Path -LiteralPath $SourceDir).Path
$BundleDir = (Resolve-Path -LiteralPath $BundleDir).Path
$CompilerPath = (Resolve-Path -LiteralPath $CompilerPath).Path
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$version = (Get-Content -LiteralPath (Join-Path $SourceDir 'qbutt-version.txt') -Raw).Trim()
if ($version -notmatch '^(\d+\.\d+\.\d+)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
    throw 'Invalid qbutt release version.'
}
$fileVersion = $matches[1]
$lock = Get-Content -LiteralPath (Join-Path $SourceDir 'upstream-lock.json') -Raw | ConvertFrom-Json
foreach ($required in @('qbutt.exe', 'qbutt-net.exe', 'Qt6Core.dll', 'platforms/qwindows.dll', 'THIRD-PARTY-NOTICES.txt')) {
    if (-not (Test-Path -LiteralPath (Join-Path $BundleDir $required) -PathType Leaf)) {
        throw "Incomplete application bundle: $required"
    }
}
New-Item -ItemType Directory -Force $OutputDir | Out-Null
& $CompilerPath '/Qp' "/DAppVersion=$version" "/DAppFileVersion=$fileVersion" `
    "/DRequiredCompilerVersion=$($lock.windows.innoSetup.version)" "/DBundleDir=$BundleDir" "/DProjectDir=$SourceDir" `
    "/O$OutputDir" (Join-Path $SourceDir 'dist/windows/qbutt.iss')
if ($LASTEXITCODE -ne 0) { throw "Installer compilation failed: $LASTEXITCODE" }
Write-Output "Installer: $(Join-Path $OutputDir "qbutt-$version-windows-x64-setup.exe")"
