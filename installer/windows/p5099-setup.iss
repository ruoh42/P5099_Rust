#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
#ifndef LauncherExe
  #define LauncherExe "..\..\target\release\p5099-launcher.exe"
#endif
#ifndef OutputDir
  #define OutputDir "out"
#endif

#define AppName "P5099"
#define ExeName "p5099.exe"
#define OldName "kartrider-p5099"

[Setup]
AppId={{6F5C2E0A-8B1D-4C55-9E3B-5099A0C0FFEE}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=P5099
DefaultDirName={commonpf32}\p5099
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=P5099-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#ExeName}
SetupIconFile=..\..\assets\icon.ico

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#LauncherExe}"; DestDir: "{app}"; DestName: "{#ExeName}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#ExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#ExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#ExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[InstallDelete]
Type: files; Name: "{autoprograms}\카트라이더 P5099.lnk"
Type: files; Name: "{autodesktop}\카트라이더 P5099.lnk"
Type: files; Name: "{app}\{#OldName}.exe"
Type: files; Name: "{app}\{#OldName}.exe.old"
Type: files; Name: "{app}\{#OldName}.exe.new"
Type: files; Name: "{app}\{#OldName}.exe.new.p5099-download"

[Dirs]
Name: "{commonappdata}\p5099\Game"; Permissions: users-modify

[UninstallDelete]
Type: filesandordirs; Name: "{commonappdata}\p5099"
Type: filesandordirs; Name: "{commonappdata}\{#OldName}"
Type: files; Name: "{app}\{#ExeName}.old"
Type: files; Name: "{app}\{#ExeName}.new"
Type: files; Name: "{app}\{#ExeName}.new.p5099-download"
