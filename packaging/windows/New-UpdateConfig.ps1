[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [switch]$SignedRelease,
    [uri]$FeedUrl,
    [string]$Ed25519PublicKey
)

$ErrorActionPreference = "Stop"

if ($SignedRelease) {
    if (-not $FeedUrl -or $FeedUrl.Scheme -ne "https") {
        throw "Signed update configuration requires an HTTPS appcast URL."
    }
    if (-not $Ed25519PublicKey -or $Ed25519PublicKey -notmatch "^[A-Za-z0-9+/]{43}=$") {
        throw "Signed update configuration requires a base64 Ed25519 public key."
    }
} elseif ($FeedUrl -or $Ed25519PublicKey) {
    throw "Unsigned preview configuration must not contain an update feed or verification key."
}

$configuration = [ordered]@{
    schema_version = 1
    provider = "winsparkle"
    release_signed = [bool]$SignedRelease
    feed_url = $(if ($FeedUrl) { $FeedUrl.AbsoluteUri } else { $null })
    ed25519_public_key = $(if ($Ed25519PublicKey) { $Ed25519PublicKey } else { $null })
    shell_integration_available = $false
    automatic_checks_supported = $false
    note = $(if ($SignedRelease) {
        "WinSparkle-compatible signed feed configuration is present, but this shell does not initialize WinSparkle yet."
    } else {
        "Unsigned preview build. Automatic update checks are disabled."
    })
}

$parent = Split-Path -Parent $OutputPath
if ($parent) {
    New-Item -ItemType Directory -Force $parent | Out-Null
}
$temporary = "$OutputPath.pending"
$configuration | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $temporary -Encoding UTF8
Move-Item -Force -LiteralPath $temporary -Destination $OutputPath
