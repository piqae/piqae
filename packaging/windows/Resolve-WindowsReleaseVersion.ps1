param(
    [Parameter(Mandatory = $true)]
    [string]$RefType,
    [Parameter(Mandatory = $true)]
    [string]$RefName,
    [Parameter(Mandatory = $true)]
    [string]$RequestedVersion,
    [switch]$UnsignedPreview
)

$ErrorActionPreference = "Stop"
$semverPattern = "^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$"
$previewTagPattern = "^v(?<version>[0-9]+\.[0-9]+\.[0-9]+)-windows-preview\.[0-9]+$"

if ($UnsignedPreview) {
    if ($RefType -cne "tag" -or $RefName -notmatch $previewTagPattern) {
        throw "Unsigned preview publication requires a v<version>-windows-preview.<number> tag."
    }
    $version = $Matches.version
} elseif ($RefType -ceq "tag") {
    $version = $RefName.TrimStart("v")
} else {
    $version = $RequestedVersion
}

if ($version -notmatch $semverPattern) {
    throw "Version must be SemVer-like and must not include a leading v."
}
if ($RefType -ceq "tag" -and $version -cne $RequestedVersion) {
    throw "Tag version '$version' does not match requested version '$RequestedVersion'."
}

$version
