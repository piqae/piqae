[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Path,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedSubject,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedThumbprint
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "Authenticode verification target does not exist."
}
if ([string]::IsNullOrWhiteSpace($ExpectedSubject)) {
    throw "Expected Authenticode certificate subject is missing."
}
$expectedNormalizedThumbprint = $ExpectedThumbprint -replace "\s", ""
if ($expectedNormalizedThumbprint -notmatch "^[0-9A-Fa-f]{40}$") {
    throw "Expected Authenticode certificate thumbprint must be a 40-character SHA-1 value."
}

$signature = Get-AuthenticodeSignature -LiteralPath $Path
if ($signature.Status -ne "Valid" -or -not $signature.SignerCertificate) {
    throw "Authenticode signature is not valid."
}
if ($signature.SignerCertificate.Subject -cne $ExpectedSubject) {
    throw "Authenticode signer subject does not match the release trust policy."
}
$actualThumbprint = $signature.SignerCertificate.Thumbprint -replace "\s", ""
if ($actualThumbprint -ine $expectedNormalizedThumbprint) {
    throw "Authenticode signer thumbprint does not match the release trust policy."
}

Write-Host "Authenticode signer identity verified for $([IO.Path]::GetFileName($Path))."
