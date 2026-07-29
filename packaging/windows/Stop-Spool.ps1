[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$InstallDirectory = $PSScriptRoot

foreach ($name in @("spool-shell-windows", "spool-agent", "spool-executor-windows", "spool-profile-host-windows")) {
    $expectedPath = Join-Path $InstallDirectory "$name.exe"
    foreach ($process in Get-Process -Name $name -ErrorAction SilentlyContinue) {
        try {
            if ($process.MainModule.FileName -eq $expectedPath) {
                Stop-Process -Id $process.Id -Force
            }
        } catch {
            # Never terminate a process whose executable path cannot be verified.
        }
    }
}
