param([Parameter(Mandatory)][string]$Helper)

$ErrorActionPreference = 'Stop'
$fixtureDirectory = Join-Path ([IO.Path]::GetTempPath()) ('qbutt-webview-certificate-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
$certificatePath = Join-Path $fixtureDirectory 'certificate.pem'
$keyPath = Join-Path $fixtureDirectory 'key.pem'
$rsa = [Security.Cryptography.RSA]::Create(2048)
$certificate = $null
$store = [Security.Cryptography.X509Certificates.X509Store]::new('Root', 'CurrentUser')

try {
    $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
        'CN=qbutt generated WebView fixture', $rsa,
        [Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pkcs1)
    $san = [Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
    $san.AddDnsName('fixture.example')
    $request.CertificateExtensions.Add($san.Build())
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($true, $false, 0, $true))
    $usages = [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature -bor
        [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::KeyCertSign
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new($usages, $true))
    $purposes = [Security.Cryptography.OidCollection]::new()
    $purposes.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.1')) | Out-Null
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($purposes, $false))
    $certificate = $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-5), [DateTimeOffset]::UtcNow.AddHours(1))
    [IO.File]::WriteAllText($certificatePath, $certificate.ExportCertificatePem())
    [IO.File]::WriteAllText($keyPath, $rsa.ExportPkcs8PrivateKeyPem())
    $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
    $store.Add($certificate)

    & bun (Join-Path $PSScriptRoot 'webview-proxy-auth.ts') $Helper $certificatePath $keyPath
    if ($LASTEXITCODE -ne 0) {
        throw "Generated HTTPS proxy authentication fixture failed with exit $LASTEXITCODE"
    }
}
finally {
    try {
        if ($certificate) {
            $store.Remove($certificate)
        }
    }
    finally {
        try {
            if ($certificate) {
                $certificate.Dispose()
            }
            $store.Dispose()
            $rsa.Dispose()
        }
        finally {
            $fixture = Get-Item -LiteralPath $fixtureDirectory -Force
            $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar)
            if (($fixture.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
                -not [string]::Equals($fixture.Parent.FullName, $temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) {
                throw 'Fixture cleanup rejected a path outside Temp or a reparse point'
            }
            [IO.Directory]::Delete($fixture.FullName, $true)
        }
    }
}
