[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DestinationDirectory
)

$ErrorActionPreference = "Stop"
$PdfiumUri = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/7961/pdfium-win-x64.tgz"
$ExpectedSha256 = "88276459349b291c41f10422dad0210f007c04d919c8fa56472b6b7c6406adf4"
$TemporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ("spool-pdfium-" + [guid]::NewGuid().ToString("N"))
$ArchivePath = Join-Path $TemporaryDirectory "pdfium-win-x64.tgz"
$ExtractedPath = Join-Path $TemporaryDirectory "extracted"

try {
    New-Item -ItemType Directory -Force $TemporaryDirectory, $ExtractedPath | Out-Null
    Invoke-WebRequest -UseBasicParsing -Uri $PdfiumUri -OutFile $ArchivePath
    $actualSha256 = (Get-FileHash -Algorithm SHA256 $ArchivePath).Hash.ToLowerInvariant()
    if ($actualSha256 -cne $ExpectedSha256) {
        throw "PDFium archive digest mismatch: expected $ExpectedSha256, received $actualSha256"
    }
    & tar.exe -xzf $ArchivePath -C $ExtractedPath
    if ($LASTEXITCODE -ne 0) {
        throw "Could not extract the pinned PDFium archive."
    }

    $licenseDirectory = Join-Path $DestinationDirectory "LICENSES\pdfium"
    New-Item -ItemType Directory -Force $DestinationDirectory, $licenseDirectory | Out-Null
    Copy-Item -LiteralPath (Join-Path $ExtractedPath "bin\pdfium.dll") -Destination $DestinationDirectory
    Copy-Item -LiteralPath (Join-Path $ExtractedPath "LICENSE") -Destination (Join-Path $licenseDirectory "pdfium-binaries.MIT.txt")
    Copy-Item -Path (Join-Path $ExtractedPath "licenses\*") -Destination $licenseDirectory
} finally {
    if (Test-Path -LiteralPath $TemporaryDirectory) {
        Remove-Item -LiteralPath $TemporaryDirectory -Recurse -Force
    }
}
