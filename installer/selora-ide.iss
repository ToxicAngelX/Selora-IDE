; Selora IDE — Windows installer (RELEASE.md §2).
;
; Build locally:
;     installer\prebuild.bat
; (prebuild reads the workspace version from Cargo.toml and invokes ISCC with
; /DMyAppVersion). CI passes the tag's version the same way:
;     ISCC.exe /DMyAppVersion=X.Y.Z installer\selora-ide.iss
;
; Requires Inno Setup 6 (RELEASE.md §1.1).

#ifndef MyAppVersion
  #error "MyAppVersion is not defined — build via installer\prebuild.bat or pass /DMyAppVersion=X.Y.Z"
#endif

#define RepoUrl "https://github.com/ToxicAngelX/Selora-IDE"

[Setup]
AppId={{7A3E9C41-5D2B-4E8F-B6A1-9C0D2E4F6A8B}
AppName=Selora IDE
AppVersion={#MyAppVersion}
AppVerName=Selora IDE {#MyAppVersion}
AppPublisher=Toxic
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
DefaultDirName={localappdata}\Programs\Selora IDE
; Per-user install: no admin prompt (RELEASE.md §2.1).
PrivilegesRequired=lowest
; Welcome, Install, Finish — no unnecessary pages (RELEASE.md §2.2 item 10).
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
WizardStyle=modern
WizardImageFile={#SourcePath}..\assets\icons\wizard_large.bmp
WizardSmallImageFile={#SourcePath}..\assets\icons\wizard_small.bmp
OutputDir={#SourcePath}output
OutputBaseFilename=selora-installer-v1
Compression=lzma2/ultra64
SolidCompression=yes
SetupIconFile={#SourcePath}..\assets\icons\selora-ide.ico
UninstallDisplayIcon={app}\selora-ide.ico
UninstallDisplayName=Selora IDE
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

[Files]
; The binary is self-contained: the Space Grotesk / JetBrains Mono fonts are
; include_bytes!-embedded (crates/selora-ui/src/theme.rs), so no font files
; ship separately (a deliberate RELEASE.md §2.2-item-4 deviation).
Source: "{#SourcePath}..\target\release\selora-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourcePath}..\assets\icons\selora-ide.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userappdata}\Microsoft\Windows\Start Menu\Programs\Selora IDE.lnk"; Filename: "{app}\selora-app.exe"; IconFilename: "{app}\selora-ide.ico"

[Run]
Filename: "{app}\selora-app.exe"; Description: "{cm:LaunchProgram,Selora IDE}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; The app's per-user data (%APPDATA%\selora-ide: settings.json,
; workspace.json) — RELEASE.md's skeleton named {localappdata}; the app
; writes under Roaming (selora-settings, PLAN.md §7), so that is what a
; clean uninstall removes.
Type: filesandordirs; Name: "{userappdata}\selora-ide"

[Registry]
; ".txt opens with Selora IDE" (Lite only; no source-file defaults,
; RELEASE.md §9 item 5). OpenWithProgids adds Selora IDE to the "Open with"
; list without hijacking the user's existing default.
Root: HKCU; Subkey: "Software\Classes\.txt\OpenWithProgids"; ValueType: string; ValueName: "SeloraIDE.txt"; ValueData: ""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\SeloraIDE.txt"; ValueType: string; ValueName: ""; ValueData: "Text Document"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SeloraIDE.txt\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\selora-ide.ico"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SeloraIDE.txt\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\selora-app.exe"" ""%1"""; Flags: uninsdeletekey
