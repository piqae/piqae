#ifndef SourceDir
  #error SourceDir must point to the staged Windows bundle
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#ifndef AppVersion
  #define AppVersion "0.1.0-dev"
#endif
#ifndef UpdateConfigFile
  #define UpdateConfigFile SourceDir + "\service\update-config.preview.json"
#endif

[Setup]
AppId={{68B68155-B5F3-4F4F-9442-B85F50322F64}
AppName=Spool
AppVersion={#AppVersion}
AppPublisher=Spool contributors
AppPublisherURL=https://github.com/C4CoffeeCo/piqae
DefaultDirName={localappdata}\Programs\Spool
DefaultGroupName=Spool
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=spool-windows-x86_64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\spool-shell-windows.exe
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
Source: "{#SourceDir}\spool-agent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\spoolctl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\spool-executor-windows.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\spool-profile-host-windows.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\spool-shell-windows.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\service\Configure-Spool.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\service\Start-Spool.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\service\Stop-Spool.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\service\Set-SpoolUpdatePolicy.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#UpdateConfigFile}"; DestDir: "{app}"; DestName: "update-config.json"; Flags: ignoreversion
Source: "{#SourceDir}\service\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSES\*"; DestDir: "{app}\LICENSES"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Configure Spool"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Configure-Spool.ps1"""
Name: "{group}\Start Spool"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\Start-Spool.ps1"""
Name: "{group}\Stop Spool"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Stop-Spool.ps1"""
Name: "{group}\Update policy"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Set-SpoolUpdatePolicy.ps1"""
Name: "{group}\Package notes"; Filename: "{app}\README.md"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Spool"; ValueData: """{sys}\WindowsPowerShell\v1.0\powershell.exe"" -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\Start-Spool.ps1"""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Spool\Updates"; ValueType: string; ValueName: "Policy"; ValueData: "disabled"; Flags: createvalueifdoesntexist uninsdeletevalue

[Run]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Configure-Spool.ps1"""; Description: "Configure and start Spool"; Flags: postinstall nowait skipifsilent

[UninstallRun]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\Stop-Spool.ps1"""; Flags: runhidden; RunOnceId: "StopSpool"
