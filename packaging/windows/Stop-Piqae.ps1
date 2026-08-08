[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$InstallDirectory = $PSScriptRoot
$StateDirectory = Join-Path $env:LOCALAPPDATA "Spool"
$StopPath = Join-Path $StateDirectory "supervisor.stop"
$PidPath = Join-Path $StateDirectory "supervisor.json"

if (Test-Path -LiteralPath $StateDirectory) {
    New-Item -ItemType File -Force -Path $StopPath | Out-Null
}
if (Test-Path -LiteralPath $PidPath) {
    try {
        $metadata = Get-Content -Raw -LiteralPath $PidPath | ConvertFrom-Json
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        while ([DateTime]::UtcNow -lt $deadline -and (Get-Process -Id $metadata.pid -ErrorAction SilentlyContinue)) {
            Start-Sleep -Milliseconds 250
        }
        $supervisor = Get-CimInstance Win32_Process -Filter "ProcessId = $($metadata.pid)" -ErrorAction SilentlyContinue
        $expectedSupervisor = Join-Path $InstallDirectory "Supervise-Piqae.ps1"
        if ($supervisor -and $supervisor.CommandLine.Contains($expectedSupervisor)) {
            Stop-Process -Id $metadata.pid -Force -ErrorAction SilentlyContinue
        }
    } catch {
        # Continue with path-bound child cleanup; stale metadata must not block an update.
    }
}

foreach ($entry in @(
    @{ Name = "piqae-shell-windows"; File = "piqae-shell-windows.exe" },
    @{ Name = "piqae-agent"; File = "piqae-agent.exe" },
    @{ Name = "piqae-executor-windows"; File = "piqae-executor-windows.exe" },
    @{ Name = "piqae-profile-host-windows"; File = "piqae-profile-host-windows.exe" },
    @{ Name = "spool-shell-windows"; File = "spool-shell-windows.exe" },
    @{ Name = "spool-agent"; File = "spool-agent.exe" },
    @{ Name = "spool-executor-windows"; File = "spool-executor-windows.exe" },
    @{ Name = "spool-profile-host-windows"; File = "spool-profile-host-windows.exe" }
)) {
    $expectedPath = Join-Path $InstallDirectory $entry.File
    foreach ($process in Get-Process -Name $entry.Name -ErrorAction SilentlyContinue) {
        try {
            if ($process.MainModule.FileName -eq $expectedPath) {
                Stop-Process -Id $process.Id -Force
            }
        } catch {
            # Never terminate a process whose executable path cannot be verified.
        }
    }
}
Remove-Item -LiteralPath $PidPath -Force -ErrorAction SilentlyContinue
