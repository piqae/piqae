[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$InstallDirectory = $PSScriptRoot

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
