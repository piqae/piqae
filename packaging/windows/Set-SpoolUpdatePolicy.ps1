[CmdletBinding()]
param(
    [ValidateSet("disabled", "notify", "automatic")]
    [string]$Policy
)

$ErrorActionPreference = "Stop"
$ConfigurationPath = Join-Path $PSScriptRoot "update-config.json"
$RegistryPath = "HKCU:\Software\Spool\Updates"

if (-not (Test-Path -LiteralPath $ConfigurationPath)) {
    throw "This Spool installation has no update configuration."
}
$configuration = Get-Content -Raw -LiteralPath $ConfigurationPath | ConvertFrom-Json
if (-not $Policy) {
    Write-Host ""
    Write-Host "Spool update policy:"
    Write-Host "  1. Disabled"
    Write-Host "  2. Notify before downloading or installing"
    Write-Host "  3. Automatic checks (installation still requires updater confirmation)"
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
        -not $configuration.ed25519_public_key) {
        throw "This installation does not contain a complete signed update configuration."
    }
    $feed = [uri]$configuration.feed_url
    if ($feed.Scheme -ne "https") {
        throw "Update feed must use HTTPS."
    }
}

New-Item -Path $RegistryPath -Force | Out-Null
New-ItemProperty -Path $RegistryPath -Name "Policy" -Value $Policy -PropertyType String -Force | Out-Null
Write-Host "Spool update policy set to '$Policy' for the current Windows user."
if ($Policy -ne "disabled") {
    if (-not $configuration.shell_integration_available) {
        Write-Warning "Policy is stored, but this shell build does not initialize WinSparkle; no update check will run."
    }
}
