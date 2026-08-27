param(
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$')]
    [string]$PackageVersion = "",
    [string]$OutputDirectory = "artifacts/dotnet"
)

$ErrorActionPreference = "Stop"
$DependencyVersion = "2.6.2"
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("piqae-windows-sdk-release-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force $temporaryRoot | Out-Null

try {
    cargo build --locked --release --target x86_64-pc-windows-msvc -p piqae-node-ffi
    if ($LASTEXITCODE -ne 0) { throw "Native node SDK build failed" }

    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $installation = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $installation) { throw "Visual Studio C++ tools were not found" }
    $developer = Join-Path $installation "Common7\Tools\VsDevCmd.bat"
    $library = (Resolve-Path "target\x86_64-pc-windows-msvc\release\piqae_node_ffi.dll.lib").Path
    cmd /d /s /c "`"$developer`" -arch=x64 -host_arch=x64 >nul && cl /nologo /W4 /WX /I sdk\native\include sdk\native\tests\abi_smoke.c `"$library`" /Fe:target\piqae-node-abi-smoke.exe"
    if ($LASTEXITCODE -ne 0) { throw "C ABI smoke consumer failed to compile" }

    $native = (Resolve-Path "target\x86_64-pc-windows-msvc\release\piqae_node_ffi.dll").Path
    $env:PATH = "$(Split-Path $native);$env:PATH"
    $env:LOCALAPPDATA = Join-Path $temporaryRoot "local-app-data"
    New-Item -ItemType Directory -Force $env:LOCALAPPDATA | Out-Null
    & "target\piqae-node-abi-smoke.exe"
    if ($LASTEXITCODE -ne 0) { throw "C ABI smoke consumer failed at runtime" }

    $env:PIQAE_NODE_NATIVE_TEST = "1"
    dotnet test sdk/dotnet/tests/Piqae.Node.Tests/Piqae.Node.Tests.csproj --configuration Release /p:TreatWarningsAsErrors=true
    if ($LASTEXITCODE -ne 0) { throw ".NET node SDK tests failed" }

    if (-not $PackageVersion) {
        $PackageVersion = dotnet msbuild sdk/dotnet/src/Piqae.Node/Piqae.Node.csproj -getProperty:Version -nologo
        $PackageVersion = $PackageVersion.Trim()
        if ($PackageVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$') {
            throw "The Piqae.Node project did not provide a safe package version"
        }
    }

    New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
    $OutputDirectory = (Resolve-Path $OutputDirectory).Path
    $expectedPackage = Join-Path $OutputDirectory "Piqae.Node.$PackageVersion.nupkg"
    if (Test-Path $expectedPackage) { throw "Refusing to replace an existing staged NuGet package: $expectedPackage" }
    dotnet pack sdk/dotnet/src/Piqae.Node/Piqae.Node.csproj --configuration Release /p:PackageOutputPath="$OutputDirectory" /p:PiqaeNativeLibrary="$native" /p:PackageVersion="$PackageVersion"
    if ($LASTEXITCODE -ne 0) { throw ".NET node SDK package build failed" }
    if (-not (Test-Path $expectedPackage -PathType Leaf)) { throw "The exact Piqae.Node $PackageVersion package was not produced" }
    $unexpectedPackages = @(Get-ChildItem $OutputDirectory -Filter "Piqae.Node.*.nupkg" | Where-Object { $_.FullName -ne (Get-Item $expectedPackage).FullName })
    if ($unexpectedPackages.Count -ne 0) { throw "The staging directory contains an unexpected Piqae.Node package version" }

    $localFeed = Join-Path $temporaryRoot "feed"
    New-Item -ItemType Directory -Force $localFeed | Out-Null
    Copy-Item $expectedPackage $localFeed
    $dependencyPackage = Join-Path $localFeed "BouncyCastle.Cryptography.$DependencyVersion.nupkg"
    Invoke-WebRequest -UseBasicParsing -Uri "https://api.nuget.org/v3-flatcontainer/bouncycastle.cryptography/$DependencyVersion/bouncycastle.cryptography.$DependencyVersion.nupkg" -OutFile $dependencyPackage

    $python = Get-Command python -ErrorAction SilentlyContinue
    if (-not $python) { $python = Get-Command python3 -ErrorAction SilentlyContinue }
    if (-not $python) { throw "Python is required for Windows SDK package and SBOM validation" }
    & $python.Source release/tools/windows_sdk_release.py validate-package --package $expectedPackage --dependency-package $dependencyPackage --version $PackageVersion
    if ($LASTEXITCODE -ne 0) { throw "The staged NuGet package contract is invalid" }

    $consumer = Join-Path $temporaryRoot "consumer"
    New-Item -ItemType Directory -Force (Join-Path $consumer "src") | Out-Null
    $escapedFeed = [System.Security.SecurityElement]::Escape($localFeed)
    @"
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="staged" value="$escapedFeed" />
  </packageSources>
</configuration>
"@ | Set-Content -Encoding utf8 (Join-Path $consumer "NuGet.Config")
    @"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <RuntimeIdentifier>win-x64</RuntimeIdentifier>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
    <TreatWarningsAsErrors>true</TreatWarningsAsErrors>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Piqae.Node" Version="[$PackageVersion]" />
  </ItemGroup>
</Project>
"@ | Set-Content -Encoding utf8 (Join-Path $consumer "Piqae.Node.PackageSmoke.csproj")
    @'
using Org.BouncyCastle.Math.EC.Rfc8032;
using Piqae.Node;

if (typeof(Ed25519).Assembly.GetName().Name != "BouncyCastle.Cryptography")
    throw new InvalidOperationException("The pinned managed dependency was not loaded.");
using var node = new PiqaeNode(new(
    HostMode.EmbeddedApplication,
    AvailabilityClass.ForegroundOnly,
    true,
    "com.piqae.release-smoke",
    "runtime"));
if (!node.Start().GetProperty("started").GetBoolean())
    throw new InvalidOperationException("The packaged native runtime did not start.");
if (node.Snapshot().GetProperty("host_mode").GetString() != "embedded_application")
    throw new InvalidOperationException("The packaged native ABI returned an invalid snapshot.");
if (PiqaeNode.NativeAbiVersion != 1 || PiqaeNode.NativeContractVersion != 2)
    throw new InvalidOperationException("The packaged facade does not require ABI 1 and contract 2.");
var capabilities = node.GetPrintPacketCapabilities();
if (capabilities.Contract != "printpacket/v1" || !capabilities.DirectOfflineRendering)
    throw new InvalidOperationException("The packaged native runtime did not return PrintPacket capabilities.");
if (node.Stop().GetProperty("started").GetBoolean())
    throw new InvalidOperationException("The packaged native runtime did not stop.");
Console.WriteLine("Piqae.Node staged NuGet loaded its managed facade, dependency, and win-x64 ABI.");
'@ | Set-Content -Encoding utf8 (Join-Path $consumer "src\Program.cs")

    dotnet restore (Join-Path $consumer "Piqae.Node.PackageSmoke.csproj") --configfile (Join-Path $consumer "NuGet.Config") --packages (Join-Path $consumer "packages") --no-cache --force
    if ($LASTEXITCODE -ne 0) { throw "The clean consumer could not restore from the isolated local feed" }
    $assets = Get-Content -Raw (Join-Path $consumer "obj\project.assets.json") | ConvertFrom-Json -AsHashtable
    if (-not $assets.libraries.ContainsKey("Piqae.Node/$PackageVersion")) { throw "The clean consumer did not resolve the exact staged Piqae.Node version" }
    if (-not $assets.libraries.ContainsKey("BouncyCastle.Cryptography/$DependencyVersion")) { throw "The clean consumer did not resolve the exact pinned BouncyCastle version" }
    $wrongPiqae = @($assets.libraries.Keys | Where-Object { $_ -like "Piqae.Node/*" -and $_ -ne "Piqae.Node/$PackageVersion" })
    if ($wrongPiqae.Count -ne 0) { throw "The clean consumer resolved an unexpected Piqae.Node version" }

    $published = Join-Path $consumer "published"
    dotnet publish (Join-Path $consumer "Piqae.Node.PackageSmoke.csproj") --configuration Release --runtime win-x64 --self-contained false --no-restore --output $published
    if ($LASTEXITCODE -ne 0) { throw "The clean consumer could not publish the staged package" }
    foreach ($required in @("Piqae.Node.dll", "BouncyCastle.Cryptography.dll", "piqae_node_ffi.dll")) {
        if (-not (Test-Path (Join-Path $published $required) -PathType Leaf)) {
            throw "The clean consumer output is missing $required"
        }
    }
    dotnet (Join-Path $published "Piqae.Node.PackageSmoke.dll")
    if ($LASTEXITCODE -ne 0) { throw "The clean NuGet consumer failed the packaged native ABI smoke" }

    $sbomInput = Join-Path $OutputDirectory "sbom-input"
    New-Item -ItemType Directory -Force $sbomInput | Out-Null
    Copy-Item $dependencyPackage (Join-Path $sbomInput (Split-Path $dependencyPackage -Leaf))
    $sbom = Join-Path $OutputDirectory "Piqae.Node.$PackageVersion.spdx.json"
    & $python.Source release/tools/windows_sdk_release.py generate-sbom --package $expectedPackage --dependency-package $dependencyPackage --version $PackageVersion --output $sbom
    if ($LASTEXITCODE -ne 0) { throw "Windows SDK SPDX SBOM generation failed" }
    & $python.Source release/tools/windows_sdk_release.py validate-sbom --input $sbom --package $expectedPackage --version $PackageVersion
    if ($LASTEXITCODE -ne 0) { throw "Windows SDK SPDX SBOM is incomplete" }
}
finally {
    Remove-Item -Recurse -Force $temporaryRoot -ErrorAction SilentlyContinue
}
