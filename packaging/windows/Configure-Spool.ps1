[CmdletBinding()]
param(
    [ValidateSet("local", "hosted", "self-hosted")]
    [string]$Mode,
    [uri]$ControlPlaneUrl,
    [uri]$DashboardUrl,
    [string]$NodeName,
    [switch]$AllowInsecureHttp,
    [switch]$DoNotStart
)

$ErrorActionPreference = "Stop"
$InstallDirectory = $PSScriptRoot
$StateDirectory = Join-Path $env:LOCALAPPDATA "Spool"
$ConfigPath = Join-Path $StateDirectory "config.json"
$DeviceKeyPath = Join-Path $StateDirectory "device.key"
$SpoolAgent = Join-Path $InstallDirectory "spool-agent.exe"

function Read-Mode {
    Write-Host ""
    Write-Host "Choose how this Windows node runs:"
    Write-Host "  1. Local only (no server)"
    Write-Host "  2. Hosted Spool control plane"
    Write-Host "  3. Self-hosted Spool control plane"
    $answer = Read-Host "Mode [1]"
    switch ($answer) {
        "2" { return "hosted" }
        "3" { return "self-hosted" }
        default { return "local" }
    }
}

function Protect-UserFile([string]$Path) {
    $identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
    $sid = $identity.User.Value
    & icacls.exe $Path /inheritance:r /grant:r "*$sid`:F" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Could not protect $Path with a current-user-only ACL."
    }
}

New-Item -ItemType Directory -Force -Path $StateDirectory | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $StateDirectory "logs") | Out-Null

if (-not $Mode) {
    $Mode = Read-Mode
}
if (-not $DashboardUrl) {
    $dashboardInput = Read-Host "Dashboard URL (optional; press Enter to skip)"
    if ($dashboardInput) {
        $DashboardUrl = [uri]$dashboardInput
    }
}
if ($DashboardUrl -and $DashboardUrl.Scheme -notin @("http", "https")) {
    throw "Dashboard URL must use HTTP or HTTPS."
}

$agentId = $null
$controlPlane = $null
if ($Mode -ne "local") {
    if (-not $ControlPlaneUrl) {
        $ControlPlaneUrl = [uri](Read-Host "Control-plane URL (for example https://spool.example.com)")
    }
    if ($ControlPlaneUrl.Scheme -ne "https" -and $ControlPlaneUrl.Host -notin @("127.0.0.1", "localhost", "::1")) {
        Write-Warning "HTTP exposes enrolment and print traffic to the local network. Use it only for a short development test on a trusted LAN."
        if (-not $AllowInsecureHttp) {
            $confirmation = Read-Host "Type ALLOW HTTP to continue"
            if ($confirmation -cne "ALLOW HTTP") {
                throw "Connected node configuration cancelled. Use an HTTPS control-plane URL."
            }
        }
    }
    if (Test-Path $DeviceKeyPath) {
        throw "This node already has a device key. Preserve it and edit config.json, or remove the existing enrolment deliberately before enrolling again."
    }

    $secureToken = Read-Host "One-time enrolment token" -AsSecureString
    $tokenPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken)
    try {
        $env:SPOOL_ENROLMENT_TOKEN = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($tokenPointer)
        $arguments = @(
            "--mode", $Mode,
            "--data-dir", $StateDirectory,
            "--local-bind", "127.0.0.1:39100",
            "--control-plane-url", $ControlPlaneUrl.AbsoluteUri,
            "--device-key-file", $DeviceKeyPath
        )
        if ($NodeName) {
            $arguments += @("--enrolment-name", $NodeName)
        }
        $enrolmentJson = (& $SpoolAgent @arguments | Out-String)
        if ($LASTEXITCODE -ne 0) {
            throw "The control plane rejected enrolment."
        }
    } finally {
        $env:SPOOL_ENROLMENT_TOKEN = $null
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($tokenPointer)
    }
    Protect-UserFile $DeviceKeyPath
    Protect-UserFile (Join-Path $StateDirectory "agent-config.json")
    $enrolment = $enrolmentJson | ConvertFrom-Json
    $agentId = $enrolment.agent_id
    $controlPlane = $ControlPlaneUrl.AbsoluteUri.TrimEnd("/")
}

$config = [ordered]@{
    schema_version = 1
    mode = $Mode
    data_dir = $StateDirectory
    local_bind = "127.0.0.1:39100"
    executor_path = (Join-Path $InstallDirectory "spool-executor-windows.exe")
    profile_host_path = (Join-Path $InstallDirectory "spool-profile-host-windows.exe")
    dashboard_url = $(if ($DashboardUrl) { $DashboardUrl.AbsoluteUri.TrimEnd("/") } else { $null })
    control_plane_url = $controlPlane
    agent_id = $agentId
    device_key_file = $(if ($agentId) { $DeviceKeyPath } else { $null })
    allow_private_uri_sources = $false
}
$temporaryConfig = "$ConfigPath.pending"
$config | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $temporaryConfig -Encoding UTF8
Move-Item -Force -LiteralPath $temporaryConfig -Destination $ConfigPath
Protect-UserFile $ConfigPath

Write-Host ""
Write-Host "Spool configuration saved to $ConfigPath"
if (-not $DoNotStart) {
    & (Join-Path $InstallDirectory "Stop-Spool.ps1")
    & (Join-Path $InstallDirectory "Start-Spool.ps1")
    Write-Host "Spool is running. The local API is http://127.0.0.1:39100"
}
