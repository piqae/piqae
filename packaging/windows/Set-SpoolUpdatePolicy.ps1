[CmdletBinding()]
param(
    [ValidateSet("disabled", "notify", "automatic")]
    [string]$Policy
)

$ErrorActionPreference = "Stop"
$ConfigurationPath = Join-Path $PSScriptRoot "update-config.json"
$RegistryPath = "HKCU:\Software\Spool\Updates"

if (-not (Test-Path -LiteralPath $ConfigurationPath)) {
    throw "This Piqae Node installation has no update configuration."
}
$configuration = Get-Content -Raw -LiteralPath $ConfigurationPath | ConvertFrom-Json
if (-not $Policy) {
    Write-Host ""
    Write-Host "Piqae Node update policy:"
    Write-Host "  1. Disabled"
    Write-Host "  2. Check only when requested from the tray"
    Write-Host "  3. Check automatically (installation still requires confirmation)"
    $choice = Read-Host "Policy [1]"
    $Policy = switch ($choice) {
        "2" { "notify" }
        "3" { "automatic" }
        default { "disabled" }
    }
}

if ($Policy -ne "disabled") {
    if (-not $configuration.release_signed) {
        throw "Updates cannot be enabled for an unsigned preview installation."
    }
    if (-not $configuration.feed_url -or
        -not $configuration.ed25519_public_key -or
        -not $configuration.runtime_version -or
        -not $configuration.runtime_sha256 -or
        -not $configuration.shell_integration_available -or
        -not $configuration.automatic_checks_supported) {
        throw "This installation does not contain a complete signed update configuration."
    }
    $feed = [uri]$configuration.feed_url
    if ($feed.Scheme -ne "https") {
        throw "Update feed must use HTTPS."
    }
    $runtime = Join-Path $PSScriptRoot "WinSparkle.dll"
    if (-not (Test-Path -LiteralPath $runtime)) {
        throw "The pinned WinSparkle runtime is missing."
    }
    $runtimeDigest = (
        Get-FileHash -Algorithm SHA256 -LiteralPath $runtime
    ).Hash.ToLowerInvariant()
    if ($runtimeDigest -cne $configuration.runtime_sha256) {
        throw "The WinSparkle runtime does not match signed package metadata."
    }
}

New-Item -Path $RegistryPath -Force | Out-Null
New-ItemProperty -Path $RegistryPath -Name "Policy" -Value $Policy -PropertyType String -Force | Out-Null
Write-Host "Piqae Node update policy set to '$Policy' for the current Windows user."
