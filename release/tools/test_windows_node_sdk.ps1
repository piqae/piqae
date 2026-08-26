$ErrorActionPreference = "Stop"

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
& "target\piqae-node-abi-smoke.exe"
if ($LASTEXITCODE -ne 0) { throw "C ABI smoke consumer failed at runtime" }

$env:PIQAE_NODE_NATIVE_TEST = "1"
dotnet test sdk/dotnet/tests/Piqae.Node.Tests/Piqae.Node.Tests.csproj --configuration Release
if ($LASTEXITCODE -ne 0) { throw ".NET node SDK tests failed" }
dotnet pack sdk/dotnet/src/Piqae.Node/Piqae.Node.csproj --configuration Release --output artifacts/dotnet /p:PiqaeNativeLibrary="$native"
if ($LASTEXITCODE -ne 0) { throw ".NET node SDK package build failed" }
if (-not (Get-ChildItem artifacts/dotnet/Piqae.Node.*.nupkg)) { throw "Piqae.Node package was not produced" }
