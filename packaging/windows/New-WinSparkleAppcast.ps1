[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [Parameter(Mandatory = $true)]
    [uri]$DownloadUrl,
    [Parameter(Mandatory = $true)]
    [long]$Length,
    [Parameter(Mandatory = $true)]
    [string]$Ed25519Signature,
    [Parameter(Mandatory = $true)]
    [datetime]$PublishedAt,
    [uri]$ReleaseNotesUrl
)

$ErrorActionPreference = "Stop"
$SparkleNamespace = "http://www.andymatuschak.org/xml-namespaces/sparkle"

if ($DownloadUrl.Scheme -ne "https") {
    throw "WinSparkle enclosure URL must use HTTPS."
}
if ($Length -le 0) {
    throw "WinSparkle enclosure length must be positive."
}
if ($Ed25519Signature -notmatch "^[A-Za-z0-9+/]{86}==$") {
    throw "WinSparkle Ed25519 signature must be a base64 64-byte signature."
}
if ($Version -notmatch "^[0-9]+(\.[0-9]+){2}([-.][0-9A-Za-z.-]+)?$") {
    throw "Version must be a SemVer-like value without a leading v."
}

$settings = [System.Xml.XmlWriterSettings]::new()
$settings.Indent = $true
$settings.Encoding = [System.Text.UTF8Encoding]::new($false)
$parent = Split-Path -Parent $OutputPath
if ($parent) {
    New-Item -ItemType Directory -Force $parent | Out-Null
}
$writer = [System.Xml.XmlWriter]::Create($OutputPath, $settings)
try {
    $writer.WriteStartDocument()
    $writer.WriteStartElement("rss")
    $writer.WriteAttributeString("version", "2.0")
    $writer.WriteAttributeString("xmlns", "sparkle", $null, $SparkleNamespace)
    $writer.WriteStartElement("channel")
    $writer.WriteElementString("title", "Piqae Node for Windows")
    $writer.WriteElementString("description", "Signed Piqae Node releases for Windows")
    $writer.WriteStartElement("item")
    $writer.WriteElementString("title", "Piqae Node $Version")
    $writer.WriteElementString("pubDate", $PublishedAt.ToUniversalTime().ToString("r"))
    if ($ReleaseNotesUrl) {
        $writer.WriteElementString("sparkle", "releaseNotesLink", $SparkleNamespace, $ReleaseNotesUrl.AbsoluteUri)
    }
    $writer.WriteStartElement("enclosure")
    $writer.WriteAttributeString("url", $DownloadUrl.AbsoluteUri)
    $writer.WriteAttributeString("length", $Length.ToString([Globalization.CultureInfo]::InvariantCulture))
    $writer.WriteAttributeString("type", "application/octet-stream")
    $writer.WriteAttributeString("sparkle", "version", $SparkleNamespace, $Version)
    $writer.WriteAttributeString("sparkle", "shortVersionString", $SparkleNamespace, $Version)
    $writer.WriteAttributeString("sparkle", "os", $SparkleNamespace, "windows")
    $writer.WriteAttributeString("sparkle", "edSignature", $SparkleNamespace, $Ed25519Signature)
    $writer.WriteEndElement()
    $writer.WriteEndElement()
    $writer.WriteEndElement()
    $writer.WriteEndElement()
    $writer.WriteEndDocument()
} finally {
    $writer.Dispose()
}
