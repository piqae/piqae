[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$PrinterName,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$OutputPath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Convert-ToSafeText {
    param([AllowNull()][object]$Value)

    if ($null -eq $Value) { return $null }
    $text = [string]$Value
    if ($text.Length -gt 512) { throw "A driver capability value exceeded 512 characters." }
    if ($text -match '[\x00-\x08\x0B\x0C\x0E-\x1F]') {
        throw "A driver capability value contained control characters."
    }
    return $text
}

function Get-Sha256Hex {
    param([Parameter(Mandatory = $true)][string]$LiteralPath)
    return (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
}

$printer = Get-Printer -Name $PrinterName
$driver = Get-PrinterDriver -Name $printer.DriverName

$candidateFiles = @($driver.DriverPath, $driver.ConfigFile, $driver.DataFile) + @($driver.DependentFiles)
$files = @(
    $candidateFiles |
        Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) -and (Test-Path -LiteralPath $_ -PathType Leaf) } |
        Sort-Object -Unique |
        ForEach-Object {
            $item = Get-Item -LiteralPath $_
            [ordered]@{
                name = Convert-ToSafeText $item.Name
                size_bytes = [long]$item.Length
                sha256 = Get-Sha256Hex $item.FullName
            }
        }
)

if ($files.Count -eq 0) { throw "No installed driver files were available for evidence capture." }
if ($files.Count -gt 256) { throw "The driver exposed more than 256 package files." }
$duplicateNames = @($files | Group-Object name | Where-Object Count -gt 1)
if ($duplicateNames.Count -gt 0) {
    throw "The installed driver exposed duplicate package basenames; evidence would be ambiguous."
}
$files = @($files | Sort-Object { $_.name }, { $_.sha256 })

$inventory = ($files | ForEach-Object { "{0}`0{1}`0{2}" -f $_.name, $_.size_bytes, $_.sha256 }) -join "`n"
$inventoryBytes = [Text.Encoding]::UTF8.GetBytes($inventory)
$hasher = [Security.Cryptography.SHA256]::Create()
try {
    $packageHash = -join ($hasher.ComputeHash($inventoryBytes) | ForEach-Object { $_.ToString("x2") })
} finally { $hasher.Dispose() }

Add-Type -AssemblyName System.Printing
$server = [System.Printing.LocalPrintServer]::new()
try {
    $queue = $server.GetPrintQueue($PrinterName)
    $stream = $queue.GetPrintCapabilitiesAsXml()
    try {
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8, $true, 4096, $true)
        try { $xmlText = $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $stream.Dispose() }
} finally { $server.Dispose() }

if ($xmlText.Length -gt 4MB) { throw "PrintCapabilities XML exceeded 4 MiB." }
[xml]$xml = $xmlText
$features = @(
    $xml.SelectNodes("//*[local-name()='Feature']") |
        Select-Object -First 512 |
        ForEach-Object {
            $featureName = Convert-ToSafeText $_.GetAttribute("name")
            $options = @(
                $_.SelectNodes("./*[local-name()='Option']") |
                    Select-Object -First 512 |
                    ForEach-Object { Convert-ToSafeText ($_.GetAttribute("name")) } |
                    Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
                    Sort-Object -Unique
            )
            if (-not [string]::IsNullOrWhiteSpace($featureName)) {
                [ordered]@{ key = $featureName; choices = $options }
            }
        }
)

$result = [ordered]@{
    schema_version = 1
    captured_at_utc = [DateTime]::UtcNow.ToString("o")
    source = "windows.printing.print_capabilities"
    redacted = $true
    printer = [ordered]@{
        manufacturer = Convert-ToSafeText $driver.Manufacturer
        driver_name = Convert-ToSafeText $driver.Name
        driver_model_version = Convert-ToSafeText $driver.MajorVersion
        driver_file_version = Convert-ToSafeText ([Diagnostics.FileVersionInfo]::GetVersionInfo($driver.DriverPath).FileVersion)
        platform = "windows"
    }
    driver_package = [ordered]@{
        canonical_inventory_sha256 = $packageHash
        files = $files
    }
    capabilities = $features
}

$parent = Split-Path -Parent $OutputPath
if (-not [string]::IsNullOrWhiteSpace($parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
}
$json = $result | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText($OutputPath, $json, [Text.UTF8Encoding]::new($false))
Write-Host "Wrote redacted, non-printing driver evidence to $OutputPath"
