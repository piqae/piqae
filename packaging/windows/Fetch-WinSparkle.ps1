[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DestinationDirectory,
    [string]$SigningToolOutputPath
)

$ErrorActionPreference = "Stop"
$Version = "0.9.4"
$ArchiveSha256 = "6037df37fc263bd1650a1c4949681a9d40ffe991d01f35892a406cb5d103c976"
$RuntimeSha256 = "9b43b1c16ee39fb9a91b5bd75138767898779510e0836be2919250607cdbe8ab"
$Uri = "https://github.com/vslavik/winsparkle/releases/download/v$Version/WinSparkle-$Version.zip"
$TemporaryDirectory = Join-Path (
    [IO.Path]::GetTempPath()
) ("piqae-winsparkle-" + [guid]::NewGuid().ToString("N"))

try {
    New-Item -ItemType Directory -Force -Path $TemporaryDirectory | Out-Null
    $archive = Join-Path $TemporaryDirectory "WinSparkle.zip"
    Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $archive
    $archiveDigest = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    if ($archiveDigest -cne $ArchiveSha256) {
        throw "WinSparkle archive digest mismatch."
    }

    $expanded = Join-Path $TemporaryDirectory "expanded"
    Expand-Archive -LiteralPath $archive -DestinationPath $expanded
    $distribution = Join-Path $expanded "WinSparkle-$Version"
    $runtime = Join-Path $distribution "x64\Release\WinSparkle.dll"
    if (-not (Test-Path -LiteralPath $runtime)) {
        throw "Pinned WinSparkle x64 runtime is missing."
    }
    $runtimeDigest = (Get-FileHash -Algorithm SHA256 -LiteralPath $runtime).Hash.ToLowerInvariant()
    if ($runtimeDigest -cne $RuntimeSha256) {
        throw "WinSparkle runtime digest mismatch."
    }

    $licenses = Join-Path $DestinationDirectory "LICENSES"
    New-Item -ItemType Directory -Force -Path $DestinationDirectory, $licenses | Out-Null
    Copy-Item -LiteralPath $runtime -Destination (
        Join-Path $DestinationDirectory "WinSparkle.dll"
    )
    Copy-Item -LiteralPath (Join-Path $distribution "COPYING") -Destination (
        Join-Path $licenses "WinSparkle-COPYING"
    )
    Copy-Item -LiteralPath (Join-Path $distribution "COPYING.expat") -Destination (
        Join-Path $licenses "WinSparkle-COPYING.expat"
    )
    if ($SigningToolOutputPath) {
        $signingTool = Join-Path $distribution "bin\winsparkle-tool.exe"
        if (-not (Test-Path -LiteralPath $signingTool)) {
            throw "Pinned WinSparkle signing tool is missing."
        }
        $signingToolParent = Split-Path -Parent $SigningToolOutputPath
        if ($signingToolParent) {
            New-Item -ItemType Directory -Force -Path $signingToolParent | Out-Null
        }
        Copy-Item -LiteralPath $signingTool -Destination $SigningToolOutputPath
    }
} finally {
    if (Test-Path -LiteralPath $TemporaryDirectory) {
        Remove-Item -LiteralPath $TemporaryDirectory -Recurse -Force
    }
}
