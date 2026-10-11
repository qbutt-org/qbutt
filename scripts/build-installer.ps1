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
foreach ($required in @('qbutt.exe', 'qbutt-net.exe', 'qbutt-content.exe', 'qbutt-search.exe',
    'Qt6Core.dll', 'platforms/qwindows.dll', 'THIRD-PARTY-NOTICES.txt',
    'browser/private-pipe.ps1', 'components/qplay-setup.exe', 'qplay/qplay.exe', 'qplay/qplay.portable',
    'qplay/build-manifest.json', 'qplay/upstream-lock.json',
    'licenses/qbutt-content-notices.txt', 'licenses/rust/dependencies.json',
    'licenses/rust/standard-library/COPYRIGHT-library.html')) {
    if (-not (Test-Path -LiteralPath (Join-Path $BundleDir $required) -PathType Leaf)) {
        throw "Incomplete application bundle: $required"
    }
}
New-Item -ItemType Directory -Force $OutputDir | Out-Null
& $CompilerPath '/Qp' "/DAppVersion=$version" "/DAppFileVersion=$fileVersion" `
    "/DRequiredCompilerVersion=$($lock.windows.innoSetup.version)" "/DBundleDir=$BundleDir" "/DProjectDir=$SourceDir" `
    "/DQplayVersion=$($lock.products.qplay.version)" "/DQplayProtocol=$($lock.products.qplay.protocolVersion)" `
    "/DQplaySetupSha256=$($lock.products.qplay.artifacts.setup.sha256)" `
    "/O$OutputDir" (Join-Path $SourceDir 'dist/windows/qbutt.iss')
if ($LASTEXITCODE -ne 0) { throw "Installer compilation failed: $LASTEXITCODE" }
Write-Output "Installer: $(Join-Path $OutputDir "qbutt-$version-windows-x64-setup.exe")"
