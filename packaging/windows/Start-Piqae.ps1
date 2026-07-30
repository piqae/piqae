[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$InstallDirectory = $PSScriptRoot
$StateDirectory = Join-Path $env:LOCALAPPDATA "Spool"
$ConfigPath = Join-Path $StateDirectory "config.json"
if (-not (Test-Path -LiteralPath $ConfigPath)) {
    throw "Piqae Node is not configured. Run Configure Piqae Node from the Start menu."
}
$config = Get-Content -Raw -LiteralPath $ConfigPath | ConvertFrom-Json

function Test-InstalledProcess([string]$Name, [string]$ExpectedPath) {
    foreach ($process in Get-Process -Name $Name -ErrorAction SilentlyContinue) {
        try {
            if ($process.MainModule.FileName -eq $ExpectedPath) {
                return $true
            }
        } catch {
            # A process that cannot be inspected is not assumed to be ours.
        }
    }
    return $false
}

$agentPath = Join-Path $InstallDirectory "piqae-agent.exe"
$shellPath = Join-Path $InstallDirectory "piqae-shell-windows.exe"
$env:PIQAE_AGENT_MODE = $config.mode
$env:PIQAE_DATA_DIR = $config.data_dir
$env:PIQAE_LOCAL_BIND = $config.local_bind
$env:PIQAE_EXECUTOR = "process"
$env:PIQAE_EXECUTOR_PATH = Join-Path $InstallDirectory "piqae-executor-windows.exe"
$env:PIQAE_PROFILE_HOST_PATH = Join-Path $InstallDirectory "piqae-profile-host-windows.exe"
if ($config.allow_private_uri_sources) {
    $env:PIQAE_ALLOW_PRIVATE_URI_SOURCES = "true"
} else {
    $env:PIQAE_ALLOW_PRIVATE_URI_SOURCES = "false"
}
$env:PIQAE_LOCAL_API_URL = "http://$($config.local_bind)"
$env:PIQAE_LOCAL_TOKEN_FILE = Join-Path $config.data_dir "local.token"
$env:PIQAE_CONTROL_PLANE_URL = $null
$env:PIQAE_AGENT_ID = $null
$env:PIQAE_DEVICE_KEY_FILE = $null
$env:PIQAE_DASHBOARD_URL = $null
$env:PIQAE_UPDATE_POLICY = "disabled"
$env:PIQAE_UPDATE_FEED_URL = $null
$env:PIQAE_UPDATE_ED25519_PUBLIC_KEY = $null
$env:PIQAE_UPDATE_RUNTIME_VERSION = $null
$env:PIQAE_UPDATE_RUNTIME_SHA256 = $null

if ($config.control_plane_url) {
    $env:PIQAE_CONTROL_PLANE_URL = $config.control_plane_url
    $env:PIQAE_AGENT_ID = $config.agent_id
    $env:PIQAE_DEVICE_KEY_FILE = $config.device_key_file
}
if ($config.dashboard_url) {
    $env:PIQAE_DASHBOARD_URL = $config.dashboard_url
}

$updateConfigPath = Join-Path $InstallDirectory "update-config.json"
if (Test-Path -LiteralPath $updateConfigPath) {
    $updateConfig = Get-Content -Raw -LiteralPath $updateConfigPath | ConvertFrom-Json
    $updateRegistry = Get-ItemProperty -Path "HKCU:\Software\Spool\Updates" -ErrorAction SilentlyContinue
    $updatePolicy = if ($updateRegistry.Policy) { $updateRegistry.Policy } else { "disabled" }
    if ($updatePolicy -in @("notify", "automatic") -and
        $updateConfig.release_signed -and
        $updateConfig.automatic_checks_supported -and
        $updateConfig.feed_url -and
        $updateConfig.ed25519_public_key -and
        $updateConfig.runtime_version -and
        $updateConfig.runtime_sha256 -and
        (Test-Path -LiteralPath (Join-Path $InstallDirectory "WinSparkle.dll"))) {
        $env:PIQAE_UPDATE_POLICY = $updatePolicy
        $env:PIQAE_UPDATE_FEED_URL = $updateConfig.feed_url
        $env:PIQAE_UPDATE_ED25519_PUBLIC_KEY = $updateConfig.ed25519_public_key
        $env:PIQAE_UPDATE_RUNTIME_VERSION = $updateConfig.runtime_version
        $env:PIQAE_UPDATE_RUNTIME_SHA256 = $updateConfig.runtime_sha256
    }
}

New-Item -ItemType Directory -Force -Path (Join-Path $StateDirectory "logs") | Out-Null
if (-not (Test-InstalledProcess "piqae-agent" $agentPath)) {
    Start-Process -FilePath $agentPath -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $StateDirectory "logs\agent.stdout.log") `
        -RedirectStandardError (Join-Path $StateDirectory "logs\agent.stderr.log")
}
if (-not (Test-InstalledProcess "piqae-shell-windows" $shellPath)) {
    Start-Process -FilePath $shellPath
}
