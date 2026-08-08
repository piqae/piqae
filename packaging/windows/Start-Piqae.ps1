[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$InstallDirectory = $PSScriptRoot
$StateDirectory = Join-Path $env:LOCALAPPDATA "Spool"
$ConfigPath = Join-Path $StateDirectory "config.json"

function Write-BoundedLauncherLog([string]$Message) {
    $logDirectory = Join-Path $StateDirectory "logs"
    New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null
    $logPath = Join-Path $logDirectory "launcher.log"
    $maxBytes = 1MB
    if ((Test-Path -LiteralPath $logPath) -and (Get-Item -LiteralPath $logPath).Length -ge $maxBytes) {
        Remove-Item -LiteralPath "$logPath.2" -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath "$logPath.1") {
            Move-Item -LiteralPath "$logPath.1" -Destination "$logPath.2" -Force
        }
        Move-Item -LiteralPath $logPath -Destination "$logPath.1" -Force
    }
    Add-Content -LiteralPath $logPath -Value "$(Get-Date -Format o) $Message" -Encoding UTF8
}

trap {
    Write-BoundedLauncherLog "Piqae launcher failed: $($_.Exception.Message)"
    throw
}

if (-not (Test-Path -LiteralPath $ConfigPath)) {
    throw "Piqae Node is not configured. Run Configure Piqae Node from the Start menu."
}
New-Item -ItemType Directory -Force -Path (Join-Path $StateDirectory "logs") | Out-Null
$stopPath = Join-Path $StateDirectory "supervisor.stop"
Remove-Item -LiteralPath $stopPath -Force -ErrorAction SilentlyContinue
$supervisorPath = Join-Path $InstallDirectory "Supervise-Piqae.ps1"
if (-not (Test-Path -LiteralPath $supervisorPath)) {
    throw "Piqae supervisor is missing from the installation."
}

# The supervisor uses a user-SID global mutex; concurrent login/manual launches are safe.
Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" `
    -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File", ('"' + $supervisorPath + '"')) `
    -WindowStyle Hidden
