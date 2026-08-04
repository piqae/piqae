#ifndef SourceDir
  #error SourceDir must point to the staged Windows bundle
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#ifndef AppVersion
  #define AppVersion "0.1.1-dev"
#endif
#ifndef UpdateConfigFile
  #define UpdateConfigFile SourceDir + "\service\update-config.preview.json"
#endif

[Setup]
AppId={{68B68155-B5F3-4F4F-9442-B85F50322F64}
AppName=Piqae Node
AppVersion={#AppVersion}
AppPublisher=Piqae contributors
AppPublisherURL=https://github.com/C4CoffeeCo/piqae
DefaultDirName={localappdata}\Programs\Piqae
DefaultGroupName=Piqae
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=piqae-windows-x86_64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\piqae-shell-windows.exe
ChangesEnvironment=no
CloseApplications=yes
RestartApplications=no
#ifdef InnoSignToolName
SignTool={#InnoSignToolName}
SignedUninstaller=yes
#else
SignedUninstaller=no
#endif

[Files]
Source: "{#SourceDir}\piqae-agent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\piqaectl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\piqae-executor-windows.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\piqae-profile-host-windows.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\piqae-shell-windows.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion
#if FileExists(SourceDir + "\WinSparkle.dll")
Source: "{#SourceDir}\WinSparkle.dll"; DestDir: "{app}"; Flags: ignoreversion
#endif
Source: "{#SourceDir}\service\Configure-Piqae.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\service\Start-Piqae.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\service\Stop-Piqae.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\service\Set-PiqaeUpdatePolicy.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#UpdateConfigFile}"; DestDir: "{app}"; DestName: "update-config.json"; Flags: ignoreversion
Source: "{#SourceDir}\service\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSES\*"; DestDir: "{app}\LICENSES"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Configure Piqae Node"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Configure-Piqae.ps1"""
Name: "{group}\Start Piqae Node"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\Start-Piqae.ps1"""
Name: "{group}\Stop Piqae Node"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Stop-Piqae.ps1"""
Name: "{group}\Update policy"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Set-PiqaeUpdatePolicy.ps1"""
Name: "{group}\Package notes"; Filename: "{app}\README.md"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Piqae"; ValueData: """{sys}\WindowsPowerShell\v1.0\powershell.exe"" -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\Start-Piqae.ps1"""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Spool"; Flags: deletevalue
Root: HKCU; Subkey: "Software\Spool\Updates"; ValueType: string; ValueName: "Policy"; ValueData: "disabled"; Flags: createvalueifdoesntexist uninsdeletevalue

[Run]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Configure-Piqae.ps1"""; Description: "Configure and start Piqae Node"; Flags: postinstall nowait skipifsilent; Check: NeedsInitialConfiguration
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\Stop-Piqae.ps1"""; Flags: runhidden waituntilterminated; Check: HasExistingConfiguration
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\Start-Piqae.ps1"""; Flags: runhidden waituntilterminated; Check: HasExistingConfiguration

[UninstallRun]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Stop-Piqae.ps1"""; Flags: runhidden; RunOnceId: "StopPiqae"

[Code]
function HasExistingConfiguration(): Boolean;
begin
  Result := FileExists(ExpandConstant('{localappdata}\Spool\config.json'));
end;

function NeedsInitialConfiguration(): Boolean;
begin
  Result := not HasExistingConfiguration();
end;
