[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [switch]$SignedRelease,
    [uri]$FeedUrl,
    [string]$Ed25519PublicKey
)

$ErrorActionPreference = "Stop"
$WinSparkleVersion = "0.9.4"
$WinSparkleRuntimeSha256 = "9b43b1c16ee39fb9a91b5bd75138767898779510e0836be2919250607cdbe8ab"
$runtimePath = Join-Path (Split-Path -Parent $OutputPath) "WinSparkle.dll"

if ($SignedRelease) {
    if (-not $FeedUrl -or $FeedUrl.Scheme -ne "https") {
        throw "Signed update configuration requires an HTTPS appcast URL."
    }
    if (-not $Ed25519PublicKey -or $Ed25519PublicKey -notmatch "^[A-Za-z0-9+/]{43}=$") {
        throw "Signed update configuration requires a base64 Ed25519 public key."
    }
    try {
        $decodedPublicKey = [Convert]::FromBase64String($Ed25519PublicKey)
    } catch {
        throw "Signed update configuration requires a base64 Ed25519 public key."
    }
    if ($decodedPublicKey.Length -ne 32) {
        throw "Signed update configuration requires a 32-byte Ed25519 public key."
    }
    if (-not (Test-Path -LiteralPath $runtimePath)) {
        throw "Signed update configuration requires the pinned WinSparkle runtime."
    }
    $runtimeDigest = (
        Get-FileHash -Algorithm SHA256 -LiteralPath $runtimePath
    ).Hash.ToLowerInvariant()
    if ($runtimeDigest -cne $WinSparkleRuntimeSha256) {
        throw "WinSparkle runtime digest does not match the pinned release."
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
    runtime_file = $(if ($SignedRelease) { "WinSparkle.dll" } else { $null })
    runtime_version = $(if ($SignedRelease) { $WinSparkleVersion } else { $null })
    runtime_sha256 = $(if ($SignedRelease) { $WinSparkleRuntimeSha256 } else { $null })
    shell_integration_available = [bool]$SignedRelease
    automatic_checks_supported = [bool]$SignedRelease
    note = $(if ($SignedRelease) {
        "Signed update checks are available. Installation requires an explicitly paused, idle node and operator confirmation."
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
