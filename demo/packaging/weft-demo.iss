; Inno Setup script: builds the Windows installer (.exe) from the staged "stage" folder.
#define AppName "WEFT ERP Demo"
#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
[Setup]
AppId={{6B1F5D0C-3E9A-4C57-9C2B-5A0E7D9F1A42}
AppName={#AppName}
AppVersion={#AppVersion}
DefaultDirName={autopf}\WEFT ERP Demo
DefaultGroupName={#AppName}
OutputDir=..\..\out
OutputBaseFilename=WEFT-ERP-Demo-Setup
Compression=lzma2/fast
SolidCompression=yes
PrivilegesRequired=lowest
ArchitecturesInstallIn64BitMode=x64compatible
DisableProgramGroupPage=yes
UninstallDisplayName={#AppName}
[Files]
Source: "..\..\stage\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs
[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\WEFT-ERP-Demo.exe"
Name: "{group}\Reset demo data"; Filename: "{app}\WEFT-ERP-Demo.exe"; Parameters: "--reset"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\WEFT-ERP-Demo.exe"; Tasks: desktopicon
[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
[Run]
Filename: "{app}\WEFT-ERP-Demo.exe"; Description: "Start the demo now"; Flags: nowait postinstall skipifsilent
[UninstallRun]
Filename: "{app}\pg\bin\pg_ctl.exe"; Parameters: "-D ""{localappdata}\WEFT-ERP-Demo\pgdata"" -m immediate stop"; Flags: runhidden; RunOnceId: "StopDb"
[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\WEFT-ERP-Demo"
