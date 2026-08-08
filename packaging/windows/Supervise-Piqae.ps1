[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$InstallDirectory = $PSScriptRoot
$StateDirectory = Join-Path $env:LOCALAPPDATA "Spool"
$ConfigPath = Join-Path $StateDirectory "config.json"
$StopPath = Join-Path $StateDirectory "supervisor.stop"
$PidPath = Join-Path $StateDirectory "supervisor.json"
$LogDirectory = Join-Path $StateDirectory "logs"
$SupervisorLog = Join-Path $LogDirectory "supervisor.log"
$AgentPath = Join-Path $InstallDirectory "piqae-agent.exe"
$ShellPath = Join-Path $InstallDirectory "piqae-shell-windows.exe"
$Mutex = $null
$ownsMutex = $false
$agent = $null
$CurrentSessionId = (Get-Process -Id $PID).SessionId

function Write-SupervisorLog([string]$Message) {
    $maxBytes = 1MB
    if ((Test-Path -LiteralPath $SupervisorLog) -and
        (Get-Item -LiteralPath $SupervisorLog).Length -ge $maxBytes) {
        Remove-Item -LiteralPath "$SupervisorLog.2" -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath "$SupervisorLog.1") {
            Move-Item -LiteralPath "$SupervisorLog.1" -Destination "$SupervisorLog.2" -Force
        }
        Move-Item -LiteralPath $SupervisorLog -Destination "$SupervisorLog.1" -Force
    }
    $line = "{0:o} {1}" -f [DateTime]::UtcNow, $Message
    Add-Content -LiteralPath $SupervisorLog -Value $line -Encoding UTF8
}

function Test-InstalledProcess([string]$Name, [string]$ExpectedPath, [Nullable[int]]$SessionId = $null) {
    foreach ($process in Get-Process -Name $Name -ErrorAction SilentlyContinue) {
        try {
            if ($process.MainModule.FileName -eq $ExpectedPath -and
                ($null -eq $SessionId -or $process.SessionId -eq $SessionId)) { return $true }
        } catch { }
    }
    return $false
}

function Ensure-ShellRunning {
    if (-not (Test-InstalledProcess "piqae-shell-windows" $ShellPath $CurrentSessionId)) {
        Start-Process -FilePath $ShellPath
    }
}

function New-UserSupervisorMutex {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    if ($null -eq $identity.User) { throw "The current Windows user has no security identifier." }
    $security = [Security.AccessControl.MutexSecurity]::new()
    $security.SetAccessRuleProtection($true, $false)
    $rule = [Security.AccessControl.MutexAccessRule]::new(
        $identity.User,
        [Security.AccessControl.MutexRights]::FullControl,
        [Security.AccessControl.AccessControlType]::Allow
    )
    $security.AddAccessRule($rule)
    $createdNew = $false
    $name = "Global\PiqaeNodeSupervisor-$($identity.User.Value)"
    [Threading.Mutex]::new($false, $name, [ref]$createdNew, $security)
}

function Set-AgentEnvironment($config) {
    $env:PIQAE_LOG_FILE = Join-Path $LogDirectory "agent.log"
    $env:PIQAE_SHELL_LOG_FILE = Join-Path $LogDirectory "shell.log"
    $env:PIQAE_AGENT_MODE = $config.mode
    $env:PIQAE_DATA_DIR = $config.data_dir
    $env:PIQAE_LOCAL_BIND = $config.local_bind
    $env:PIQAE_EXECUTOR = "process"
    $env:PIQAE_EXECUTOR_PATH = Join-Path $InstallDirectory "piqae-executor-windows.exe"
    $env:PIQAE_PROFILE_HOST_PATH = Join-Path $InstallDirectory "piqae-profile-host-windows.exe"
    $env:PIQAE_ALLOW_PRIVATE_URI_SOURCES = if ($config.allow_private_uri_sources) { "true" } else { "false" }
    $env:PIQAE_LOCAL_API_URL = "http://$($config.local_bind)"
    $env:PIQAE_LOCAL_TOKEN_FILE = Join-Path $config.data_dir "local.token"
    foreach ($name in @("PIQAE_CONTROL_PLANE_URL", "PIQAE_AGENT_ID", "PIQAE_DEVICE_KEY_FILE", "PIQAE_DASHBOARD_URL", "PIQAE_UPDATE_FEED_URL", "PIQAE_UPDATE_ED25519_PUBLIC_KEY", "PIQAE_UPDATE_RUNTIME_VERSION", "PIQAE_UPDATE_RUNTIME_SHA256")) {
        Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
    }
    if ($config.control_plane_url) {
        $env:PIQAE_CONTROL_PLANE_URL = $config.control_plane_url
        $env:PIQAE_AGENT_ID = $config.agent_id
        $env:PIQAE_DEVICE_KEY_FILE = $config.device_key_file
    }
    if ($config.dashboard_url) { $env:PIQAE_DASHBOARD_URL = $config.dashboard_url }
    $env:PIQAE_UPDATE_POLICY = "disabled"
    $updateConfigPath = Join-Path $InstallDirectory "update-config.json"
    if (Test-Path -LiteralPath $updateConfigPath) {
        $updateConfig = Get-Content -Raw -LiteralPath $updateConfigPath | ConvertFrom-Json
        $updateRegistry = Get-ItemProperty -Path "HKCU:\Software\Spool\Updates" -ErrorAction SilentlyContinue
        $updatePolicy = if ($updateRegistry.Policy) { $updateRegistry.Policy } else { "disabled" }
        if ($updatePolicy -in @("notify", "automatic") -and $updateConfig.release_signed -and
            $updateConfig.automatic_checks_supported -and $updateConfig.feed_url -and
            $updateConfig.ed25519_public_key -and $updateConfig.runtime_version -and
            $updateConfig.runtime_sha256 -and (Test-Path -LiteralPath (Join-Path $InstallDirectory "WinSparkle.dll"))) {
            $env:PIQAE_UPDATE_POLICY = $updatePolicy
            $env:PIQAE_UPDATE_FEED_URL = $updateConfig.feed_url
            $env:PIQAE_UPDATE_ED25519_PUBLIC_KEY = $updateConfig.ed25519_public_key
            $env:PIQAE_UPDATE_RUNTIME_VERSION = $updateConfig.runtime_version
            $env:PIQAE_UPDATE_RUNTIME_SHA256 = $updateConfig.runtime_sha256
        }
    }
}

try {
    New-Item -ItemType Directory -Force -Path $LogDirectory | Out-Null
    if (-not (Test-Path -LiteralPath $ConfigPath)) { throw "Piqae Node is not configured." }
    $config = Get-Content -Raw -LiteralPath $ConfigPath | ConvertFrom-Json
    Set-AgentEnvironment $config
    $shellMutex = [Threading.Mutex]::new($false, "Local\PiqaeNodeShellLauncher")
    $ownsShellMutex = $false
    try {
        try { $ownsShellMutex = $shellMutex.WaitOne(5000, $false) } catch [Threading.AbandonedMutexException] { $ownsShellMutex = $true }
        if ($ownsShellMutex) { Ensure-ShellRunning }
    } finally {
        if ($ownsShellMutex) { $shellMutex.ReleaseMutex() }
        $shellMutex.Dispose()
    }
    $Mutex = New-UserSupervisorMutex
    while (-not $ownsMutex -and -not (Test-Path -LiteralPath $StopPath)) {
        try { $ownsMutex = $Mutex.WaitOne(1000, $false) } catch [Threading.AbandonedMutexException] { $ownsMutex = $true }
    }
    if ($ownsMutex -and (Test-Path -LiteralPath $StopPath)) {
        $Mutex.ReleaseMutex()
        $ownsMutex = $false
    }
    if (-not $ownsMutex) { exit 0 }
    @{ pid = $PID; started_at = [DateTime]::UtcNow.ToString("o"); script = $MyInvocation.MyCommand.Path } |
        ConvertTo-Json | Set-Content -LiteralPath $PidPath -Encoding UTF8
    $failures = [Collections.Generic.Queue[DateTime]]::new()
    while (-not (Test-Path -LiteralPath $StopPath)) {
        $started = [DateTime]::UtcNow
        Write-SupervisorLog "starting durable agent"
        $agent = Start-Process -FilePath $AgentPath -WindowStyle Hidden -PassThru `
            -RedirectStandardOutput (Join-Path $LogDirectory "agent.stdout.log") `
            -RedirectStandardError (Join-Path $LogDirectory "agent.stderr.log")
        while (-not $agent.HasExited -and -not (Test-Path -LiteralPath $StopPath)) {
            Start-Sleep -Seconds 1
            $agent.Refresh()
            Ensure-ShellRunning
        }
        if (Test-Path -LiteralPath $StopPath) { break }
        $runtime = ([DateTime]::UtcNow - $started).TotalSeconds
        Write-SupervisorLog "durable agent exited code=$($agent.ExitCode) runtime_seconds=$([Math]::Floor($runtime))"
        if ($runtime -ge 600) { $failures.Clear() }
        $now = [DateTime]::UtcNow
        $failures.Enqueue($now)
        while ($failures.Count -gt 0 -and ($now - $failures.Peek()).TotalMinutes -gt 5) { [void]$failures.Dequeue() }
        if ($failures.Count -ge 5) {
            Write-SupervisorLog "crash-loop threshold reached; delaying restart for 300 seconds"
            for ($i = 0; $i -lt 300 -and -not (Test-Path -LiteralPath $StopPath); $i++) { Start-Sleep -Seconds 1 }
            $failures.Clear()
        } else {
            $delay = [Math]::Min(30, [Math]::Pow(2, $failures.Count - 1))
            for ($i = 0; $i -lt $delay -and -not (Test-Path -LiteralPath $StopPath); $i++) { Start-Sleep -Seconds 1 }
        }
    }
} catch {
    try { Write-SupervisorLog "supervisor failure: $($_.Exception.Message)" } catch { }
    exit 1
} finally {
    if ($null -ne $agent -and -not $agent.HasExited) {
        Stop-Process -Id $agent.Id -Force -ErrorAction SilentlyContinue
        try { $agent.WaitForExit(10000) } catch { }
    }
    Remove-Item -LiteralPath $PidPath -Force -ErrorAction SilentlyContinue
    if ($ownsMutex) { $Mutex.ReleaseMutex() }
    if ($null -ne $Mutex) { $Mutex.Dispose() }
}
