[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$TemporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ("piqae-windows-tests-" + [guid]::NewGuid().ToString("N"))

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

try {
    New-Item -ItemType Directory -Force $TemporaryDirectory | Out-Null

    $parseErrors = @()
    foreach ($script in Get-ChildItem $PSScriptRoot -Filter "*.ps1") {
        $tokens = $null
        $errors = $null
        [void][System.Management.Automation.Language.Parser]::ParseFile(
            $script.FullName,
            [ref]$tokens,
            [ref]$errors
        )
        $parseErrors += $errors | ForEach-Object { "$($script.Name): $($_.Message)" }
    }
    Assert-True ($parseErrors.Count -eq 0) "PowerShell parse failures: $($parseErrors -join '; ')"

    $previewPath = Join-Path $TemporaryDirectory "preview.json"
    & (Join-Path $PSScriptRoot "New-UpdateConfig.ps1") -OutputPath $previewPath
    $preview = Get-Content -Raw -LiteralPath $previewPath | ConvertFrom-Json
    Assert-True (-not $preview.release_signed) "Preview must not claim release signing."
    Assert-True (-not $preview.feed_url) "Preview must not contain a feed URL."
    Assert-True (-not $preview.ed25519_public_key) "Preview must not contain an update key."
    Assert-True (-not $preview.shell_integration_available) "Preview must not claim shell integration."
    Assert-True (-not $preview.runtime_sha256) "Preview must not pin an updater runtime."

    $rejectedMissingRuntime = $false
    try {
        & (Join-Path $PSScriptRoot "New-UpdateConfig.ps1") `
            -OutputPath (Join-Path $TemporaryDirectory "signed.json") `
            -SignedRelease `
            -FeedUrl "https://updates.example.test/windows.xml" `
            -Ed25519PublicKey (("A" * 43) + "=")
    } catch {
        $rejectedMissingRuntime = $true
    }
    Assert-True $rejectedMissingRuntime "Signed update configuration accepted a missing runtime."

    $rejectedUnsignedFeed = $false
    try {
        & (Join-Path $PSScriptRoot "New-UpdateConfig.ps1") `
            -OutputPath (Join-Path $TemporaryDirectory "invalid.json") `
            -FeedUrl "https://updates.example.test/windows.xml"
    } catch {
        $rejectedUnsignedFeed = $true
    }
    Assert-True $rejectedUnsignedFeed "Unsigned update configuration accepted a feed URL."

    $appcastPath = Join-Path $TemporaryDirectory "appcast.xml"
    $signature = ("A" * 86) + "=="
    & (Join-Path $PSScriptRoot "New-WinSparkleAppcast.ps1") `
        -OutputPath $appcastPath `
        -Version "1.2.3-preview.1" `
        -DownloadUrl "https://downloads.example.test/piqae-node-setup.exe" `
        -Length 12345 `
        -Ed25519Signature $signature `
        -PublishedAt ([datetime]"2026-01-01T00:00:00Z") `
        -ReleaseNotesUrl "https://downloads.example.test/releases/1.2.3"
    [xml]$appcast = Get-Content -Raw -LiteralPath $appcastPath
    $namespace = [System.Xml.XmlNamespaceManager]::new($appcast.NameTable)
    $namespace.AddNamespace("sparkle", "http://www.andymatuschak.org/xml-namespaces/sparkle")
    $enclosure = $appcast.SelectSingleNode("/rss/channel/item/enclosure", $namespace)
    Assert-True ($null -ne $enclosure) "Generated appcast has no enclosure."
    Assert-True ($enclosure.GetAttribute("edSignature", $namespace.LookupNamespace("sparkle")) -eq $signature) "Appcast signature differs."
    Assert-True ($enclosure.GetAttribute("version", $namespace.LookupNamespace("sparkle")) -eq "1.2.3-preview.1") "Appcast version differs."

    $installer = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Piqae.iss")
    Assert-True ($installer.Contains("update-config.json")) "Installer does not stage update configuration."
    Assert-True ($installer.Contains("WinSparkle.dll")) "Installer does not stage WinSparkle when present."
    Assert-True ($installer.Contains("createvalueifdoesntexist")) "Installer would overwrite the user update policy."
    Assert-True ($installer.Contains("Check: NeedsInitialConfiguration")) "Installer reopens first-run configuration during upgrades."
    Assert-True ($installer.Contains("Check: HasExistingConfiguration")) "Installer does not restart an already configured node after upgrade."
    Assert-True ($installer.Contains("{localappdata}\Spool\config.json")) "Installer moved the shipped durable state path during the Piqae rename."
    Assert-True ($installer.Contains("ValueName: ""Spool""; Flags: deletevalue")) "Installer leaves the legacy startup registration active."
    Assert-True ($installer.Contains("Supervise-Piqae.ps1")) "Installer does not stage the durable-agent supervisor."
    Assert-True ($installer.Contains("runhidden nowait; Check: HasExistingConfiguration")) "Upgrade would wait forever on the long-lived supervisor."

    $startScript = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Start-Piqae.ps1")
    Assert-True ($startScript.Contains('Join-Path $env:LOCALAPPDATA "Spool"')) "Start script moved the shipped durable state path."
    Assert-True ($startScript.Contains('launcher.log')) "Start script does not retain bounded launcher failures."
    Assert-True (-not $startScript.Contains('RedirectStandardOutput')) "Start script still creates an unbounded stdout log."
    Assert-True (-not $startScript.Contains('RedirectStandardError')) "Start script still creates an unbounded stderr log."
    Assert-True ($startScript.Contains("Supervise-Piqae.ps1")) "Start script does not launch the supervisor."

    $supervisorScript = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Supervise-Piqae.ps1")
    Assert-True ($supervisorScript.Contains("piqae-executor-windows.exe")) "Supervisor does not replace the executor path from an existing configuration."
    Assert-True ($supervisorScript.Contains("piqae-profile-host-windows.exe")) "Supervisor does not replace the profile-host path from an existing configuration."
    Assert-True ($supervisorScript.Contains('"Global\PiqaeNodeSupervisor-$($identity.User.Value)"')) "Supervisor is not a cross-session per-user singleton."
    Assert-True ($supervisorScript.Contains("MutexSecurity")) "Supervisor mutex has no explicit user-only security descriptor."
    Assert-True ($supervisorScript.Contains('SetAccessRuleProtection($true, $false)')) "Supervisor mutex inherits broader access rules."
    Assert-True ($supervisorScript.Contains("MutexRights]::FullControl")) "Supervisor mutex does not grant its owning user control."
    Assert-True ($supervisorScript.Contains('"Local\PiqaeNodeShellLauncher"')) "Tray launch is not isolated to each interactive session."
    Assert-True ($supervisorScript.Contains('$process.SessionId -eq $SessionId')) "Tray detection can mistake another session's tray for the current one."
    Assert-True ($supervisorScript.Contains('$Mutex.WaitOne(1000, $false)')) "Another active session cannot take over after the owning session exits."
    Assert-True (([regex]::Matches($supervisorScript, 'Ensure-ShellRunning')).Count -ge 4) "Standby sessions do not independently restore their disposable tray."
    Assert-True (-not $supervisorScript.Contains('Remove-Item -LiteralPath $StopPath')) "A supervisor could erase a stop request before standby sessions observe it."
    Assert-True ($supervisorScript.Contains("crash-loop threshold reached")) "Supervisor has no bounded crash-loop policy."
    Assert-True ($supervisorScript.Contains('$failures.Count -ge 5')) "Supervisor crash-loop threshold changed unexpectedly."
    Assert-True ($supervisorScript.Contains('Test-Path -LiteralPath $StopPath')) "Supervisor does not honor clean stop requests."
    Assert-True ($supervisorScript.Contains("Ensure-ShellRunning")) "Supervisor does not restore the disposable tray after a policy restart or crash."
    Assert-True ($supervisorScript.Contains('PIQAE_LOG_FILE')) "Supervisor does not configure the bounded agent log."
    Assert-True ($supervisorScript.Contains('PIQAE_SHELL_LOG_FILE')) "Supervisor does not configure the bounded shell log."
    Assert-True ($supervisorScript.Contains('$maxBytes = 1MB')) "Supervisor log has no hard rotation threshold."
    Assert-True (-not $supervisorScript.Contains('RedirectStandardOutput')) "Supervisor creates an unbounded stdout log."
    Assert-True (-not $supervisorScript.Contains('RedirectStandardError')) "Supervisor creates an unbounded stderr log."

    $stopScript = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Stop-Piqae.ps1")
    Assert-True ($stopScript.Contains("piqae-agent")) "Stop script does not stop the renamed node."
    Assert-True ($stopScript.Contains("spool-agent")) "Stop script cannot safely hand off an existing pre-rename node."
    Assert-True ($stopScript.Contains("MainModule.FileName")) "Stop script does not bind termination to the installed executable path."
    Assert-True ($stopScript.Contains("Get-CimInstance Win32_Process")) "Stop script does not verify the supervisor command line before forced termination."

    $policyScript = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Set-PiqaeUpdatePolicy.ps1")
    Assert-True ($policyScript.Contains("piqae-shell-windows")) "Update-policy changes do not restart the tray."
    Assert-True ($policyScript.Contains("piqae-profile-host-windows")) "Update-policy changes do not defer for native driver settings."
    Assert-True ($policyScript.Contains("Start-Piqae.ps1")) "Update-policy changes do not relaunch the tray."
    Assert-True (-not $policyScript.Contains("Stop-Piqae.ps1")) "Update-policy changes would stop the durable agent."

    $signerScript = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Assert-AuthenticodeSigner.ps1")
    Assert-True ($signerScript.Contains("SignerCertificate.Subject")) "Authenticode verification does not bind the signer subject."
    Assert-True ($signerScript.Contains("SignerCertificate.Thumbprint")) "Authenticode verification does not bind the signer thumbprint."

    $releaseWorkflow = Get-Content -Raw -LiteralPath (Join-Path $RepositoryRoot ".github\workflows\windows-release.yml")
    foreach ($component in @(
        "piqae-agent",
        "piqaectl",
        "piqae-executor-windows",
        "piqae-shell-windows"
    )) {
        Assert-True ($releaseWorkflow.Contains('"' + $component + '"')) "Release version gate omits $component."
    }
    Assert-True ($releaseWorkflow.Contains("WINDOWS_EXPECTED_CERTIFICATE_SUBJECT")) "Release workflow does not require the expected signer subject."
    Assert-True ($releaseWorkflow.Contains("WINDOWS_EXPECTED_CERTIFICATE_THUMBPRINT")) "Release workflow does not require the expected signer thumbprint."

    Write-Host "Windows packaging static tests passed."
} finally {
    if (Test-Path -LiteralPath $TemporaryDirectory) {
        Remove-Item -LiteralPath $TemporaryDirectory -Recurse -Force
    }
}
