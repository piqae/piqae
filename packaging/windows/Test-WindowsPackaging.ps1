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

    $signedPath = Join-Path $TemporaryDirectory "signed.json"
    $publicKey = ("A" * 43) + "="
    & (Join-Path $PSScriptRoot "New-UpdateConfig.ps1") `
        -OutputPath $signedPath `
        -SignedRelease `
        -FeedUrl "https://updates.example.test/windows.xml" `
        -Ed25519PublicKey $publicKey
    $signed = Get-Content -Raw -LiteralPath $signedPath | ConvertFrom-Json
    Assert-True $signed.release_signed "Signed configuration must record release signing."
    Assert-True (-not $signed.shell_integration_available) "Packaging must not claim tray updater integration."
    Assert-True (-not $signed.automatic_checks_supported) "Automatic checks must remain unavailable."

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
        -DownloadUrl "https://downloads.example.test/spool-setup.exe" `
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
    Assert-True ($installer.Contains("createvalueifdoesntexist")) "Installer would overwrite the user update policy."

    Write-Host "Windows packaging static tests passed."
} finally {
    if (Test-Path -LiteralPath $TemporaryDirectory) {
        Remove-Item -LiteralPath $TemporaryDirectory -Recurse -Force
    }
}
