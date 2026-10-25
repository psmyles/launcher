; The Windows installer (N13, M10): per user, no admin rights. Built by `cargo xtask
; package-windows`, which passes the defines below from product.json and target\package:
;
;   /DAppVersion=0.1.0 /DAppPublisher=... /DSourceExe=...\launcher.exe /DSourceIcon=...\launcher.ico
;   /DNoticesDir=... /DWizardImages=a.png,b.png,... /DWizardSmallImages=... /DOutputDir=...
;
; State, trust and history live in %LOCALAPPDATA%\Launcher (app data), which neither an upgrade
; nor the uninstaller touches.

#ifndef AppVersion
  #error AppVersion is not defined; build with cargo xtask package-windows
#endif
#if VER < EncodeVer(6, 5, 2)
  #error Inno Setup 6.5.2 or later is needed (PNG wizard images)
#endif

#define AppName "Launcher"
#define ProgId "Launcher.File"
#define AppUserModelId "Launcher.App"

[Setup]
; The installer's identity: an upgrade finds the installed copy by it. Never change it.
AppId={{6F2B7C1E-8A43-4E0B-9C5D-2E7A1B3F4D60}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={localappdata}\Programs\{#AppName}
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename={#AppName}-{#AppVersion}-setup
SetupIconFile={#SourceIcon}
UninstallDisplayIcon={app}\launcher.exe
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Drawn from the app icon for each display scale (xtask icons.rs). Centred, not stretched: the
; large image's tile grey fills any space around it.
WizardImageFile={#WizardImages}
WizardSmallImageFile={#WizardSmallImages}
WizardImageStretch=no
WizardImageBackColor=$393634
ChangesAssociations=yes
ChangesEnvironment=yes
; A running launcher is closed for an upgrade (Restart Manager) and not restarted: its user
; opens the file again.
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "path"; Description: "Add launcher to my PATH (to use launcher check and launcher run in a terminal)"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion
; The licence notices (LICENSE.txt, THIRD-PARTY-NOTICES.md, licenses\): the exe links
; third-party code whose licences require them to travel with it.
Source: "{#NoticesDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\{#AppName}"; Filename: "{app}\launcher.exe"; AppUserModelID: "{#AppUserModelId}"; Comment: "Open .launcher files"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\launcher.exe"; AppUserModelID: "{#AppUserModelId}"; Comment: "Open .launcher files"; Tasks: desktopicon

[Registry]
; .launcher files open in the launcher; "Edit" opens the visual editor. The launcher owns the
; extension, so it is the default, and it stays in "Open with" if the user picks another default.
; Text for everything else (they are KDL text: previewers and "Open with" editors show them).
Root: HKCU; Subkey: "Software\Classes\.launcher"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\.launcher"; ValueType: string; ValueName: "Content Type"; ValueData: "text/plain"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\.launcher"; ValueType: string; ValueName: "PerceivedType"; ValueData: "text"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\.launcher\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueName: ""; ValueData: "Launcher file"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\launcher.exe"",0"
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\launcher.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\shell\edit"; ValueType: string; ValueName: ""; ValueData: "Edit"
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\shell\edit\command"; ValueType: string; ValueName: ""; ValueData: """{app}\launcher.exe"" edit ""%1"""
; The PATH entry (the task), taken out again by the uninstaller (see [Code]).
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: path; Check: NotOnPath(ExpandConstant('{app}'))

[Run]
; With no file, the launcher opens its home window: Make a new tool, Open a tool..., and the tools opened lately.
Filename: "{app}\launcher.exe"; Description: "Open {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
function NotOnPath(Dir: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

procedure RemoveFromPath(Dir: string);
var
  Path, Upper, Needle: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
    exit;
  Upper := ';' + Uppercase(Path) + ';';
  Needle := ';' + Uppercase(Dir) + ';';
  P := Pos(Needle, Upper);
  if P = 0 then
    exit;
  Path := ';' + Path + ';';
  Delete(Path, P, Length(Needle) - 1);
  Path := Copy(Path, 2, Length(Path) - 2);
  RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
