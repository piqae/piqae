[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Path,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedSubject,
    [Parameter(Mandatory = $false)]
    [string]$ExpectedThumbprint,
    [Parameter(Mandatory = $false)]
    [switch]$AllowRotatingCertificate
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "Authenticode verification target does not exist."
}
if ([string]::IsNullOrWhiteSpace($ExpectedSubject)) {
    throw "Expected Authenticode certificate subject is missing."
}
$expectedNormalizedThumbprint = $ExpectedThumbprint -replace "\s", ""
if (-not $expectedNormalizedThumbprint -and -not $AllowRotatingCertificate) {
    throw "Expected Authenticode certificate thumbprint is missing."
}
if ($expectedNormalizedThumbprint -and $AllowRotatingCertificate) {
    throw "A fixed thumbprint and rotating-certificate policy are mutually exclusive."
}
if ($expectedNormalizedThumbprint -and $expectedNormalizedThumbprint -notmatch "^[0-9A-Fa-f]{40}$") {
    throw "Expected Authenticode certificate thumbprint must be empty or a 40-character SHA-1 value."
}

$signature = Get-AuthenticodeSignature -LiteralPath $Path
if ($signature.Status -ne "Valid" -or -not $signature.SignerCertificate) {
    throw "Authenticode signature is not valid."
}
if ($signature.SignerCertificate.Subject -cne $ExpectedSubject) {
    throw "Authenticode signer subject does not match the release trust policy."
}
if ($expectedNormalizedThumbprint) {
    $actualThumbprint = $signature.SignerCertificate.Thumbprint -replace "\s", ""
    if ($actualThumbprint -ine $expectedNormalizedThumbprint) {
        throw "Authenticode signer thumbprint does not match the release trust policy."
    }
}

Write-Host "Authenticode signer identity verified for $([IO.Path]::GetFileName($Path))."
