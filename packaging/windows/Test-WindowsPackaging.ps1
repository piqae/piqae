[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$TemporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ("spool-windows-tests-" + [guid]::NewGuid().ToString("N"))

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

    $installer = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Spool.iss")
    Assert-True ($installer.Contains("update-config.json")) "Installer does not stage update configuration."
    Assert-True ($installer.Contains("WinSparkle.dll")) "Installer does not stage WinSparkle when present."
    Assert-True ($installer.Contains("createvalueifdoesntexist")) "Installer would overwrite the user update policy."
    Assert-True ($installer.Contains("Check: NeedsInitialConfiguration")) "Installer reopens first-run configuration during upgrades."
    Assert-True ($installer.Contains("Check: HasExistingConfiguration")) "Installer does not restart an already configured node after upgrade."

    $policyScript = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Set-SpoolUpdatePolicy.ps1")
    Assert-True ($policyScript.Contains("spool-shell-windows")) "Update-policy changes do not restart the tray."
    Assert-True ($policyScript.Contains("spool-profile-host-windows")) "Update-policy changes do not defer for native driver settings."
    Assert-True ($policyScript.Contains("Start-Spool.ps1")) "Update-policy changes do not relaunch the tray."
    Assert-True (-not $policyScript.Contains("Stop-Spool.ps1")) "Update-policy changes would stop the durable agent."

    $signerScript = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot "Assert-AuthenticodeSigner.ps1")
    Assert-True ($signerScript.Contains("SignerCertificate.Subject")) "Authenticode verification does not bind the signer subject."
    Assert-True ($signerScript.Contains("SignerCertificate.Thumbprint")) "Authenticode verification does not bind the signer thumbprint."

    $releaseWorkflow = Get-Content -Raw -LiteralPath (Join-Path $RepositoryRoot ".github\workflows\windows-release.yml")
    foreach ($component in @(
        "spool-agent",
        "spoolctl",
        "spool-executor-windows",
        "spool-shell-windows"
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
