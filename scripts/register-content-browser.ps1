param(
    [Parameter(Mandatory)][string]$ContentExecutable,
    [Parameter(Mandatory)][ValidateSet('edge', 'chrome', 'firefox')][string]$Browser,
    [string]$ExtensionDirectory,
    [switch]$OpenSetup
)

$ErrorActionPreference = 'Stop'
$contentPath = (Resolve-Path -LiteralPath $ContentExecutable).ProviderPath
if ([IO.Path]::GetExtension($contentPath) -ne '.exe') {
    throw 'The native messaging host must be qbutt-content.exe.'
}

$nativeDirectory = Join-Path $env:LOCALAPPDATA 'qbutt\content\native-messaging'
[IO.Directory]::CreateDirectory($nativeDirectory) | Out-Null
$manifestPath = Join-Path $nativeDirectory "org.qbutt.content.$Browser.json"
$manifest = [ordered]@{
    name = 'org.qbutt.content'
    description = 'qbutt browser connection'
    path = $contentPath
    type = 'stdio'
}

if ($Browser -eq 'firefox') {
    $manifest.allowed_extensions = @('content@qbutt.org')
    $registryPath = 'Software\Mozilla\NativeMessagingHosts\org.qbutt.content'
    $setupUrl = 'about:debugging#/runtime/this-firefox'
}
else {
    $manifest.allowed_origins = @('chrome-extension://jnmnnglmejlbjnjmmjampnhfncpcbffh/')
    if ($Browser -eq 'edge') {
        $registryPath = 'Software\Microsoft\Edge\NativeMessagingHosts\org.qbutt.content'
        $setupUrl = 'edge://extensions'
    }
    else {
        $registryPath = 'Software\Google\Chrome\NativeMessagingHosts\org.qbutt.content'
        $setupUrl = 'chrome://extensions'
    }
}

[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($registryPath)
try { $key.SetValue('', $manifestPath, [Microsoft.Win32.RegistryValueKind]::String) }
finally { $key.Dispose() }

if ($OpenSetup) {
    $relativePath = switch ($Browser) {
        'edge' { 'Microsoft\Edge\Application\msedge.exe' }
        'chrome' { 'Google\Chrome\Application\chrome.exe' }
        'firefox' { 'Mozilla Firefox\firefox.exe' }
    }
    $browserPath = @($env:LOCALAPPDATA, $env:ProgramFiles, ${env:ProgramFiles(x86)}) |
        Where-Object { $_ } | ForEach-Object { Join-Path $_ $relativePath } |
        Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $browserPath) { throw 'The selected browser is not installed.' }
    Start-Process -FilePath $browserPath -ArgumentList $setupUrl -WindowStyle Normal
    if ($ExtensionDirectory) {
        $extensionPath = (Resolve-Path -LiteralPath $ExtensionDirectory).ProviderPath
        Start-Process -FilePath explorer.exe -ArgumentList ('"' + $extensionPath + '"') -WindowStyle Normal
    }
}

@{ browser = $Browser; manifest = $manifestPath; setupUrl = $setupUrl } | ConvertTo-Json -Compress
