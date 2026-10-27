; Selora IDE - Windows installer (RELEASE.md section 2).
;
; Build locally:
;     installer\prebuild.bat
; prebuild reads the workspace version from Cargo.toml, stages the MSVC
; runtime DLLs beside the exe, fetches the WebView2 bootstrapper, and invokes
; ISCC with /DMyAppVersion, /DMyTargetDir, and /DMyCrtDlls. CI passes the
; tag's version and runs the same staging steps before calling:
;     ISCC.exe /DMyAppVersion=X.Y.Z installer\selora-ide.iss
;
; Requires Inno Setup 6.3 or newer, or 7.x (x64compatible architectures).

#ifndef MyAppVersion
  #error "MyAppVersion is not defined - build via installer\prebuild.bat or pass /DMyAppVersion=X.Y.Z"
#endif

#define RepoUrl "https://github.com/ToxicAngelX/Selora-IDE"

; Directory holding the built exe and the staged DLLs. prebuild.bat passes an
; absolute path when CARGO_TARGET_DIR is set, so a custom cargo target dir
; works end to end. Override only to compile a smoke installer from a
; non-release binary:
;   ISCC /DMyAppVersion=0.1.0 /DMyTargetDir=..\target\debug ...
; Relative values resolve against this script's directory.
#ifndef MyTargetDir
  #define MyTargetDir "..\target\release"
#endif

#ifndef MyAppExe
  #define MyAppExe MyTargetDir + '\selora-desktop.exe'
#endif

; Comma-separated list of the CRT DLLs staged beside the exe. prebuild.bat and
; the release workflow pass the dumpbin-derived list; the [Code] loop below
; asserts every listed DLL landed in {app}. The default keeps a hand-run ISCC
; (or an older build that imports only the two runtime DLLs) working.
#ifndef MyCrtDlls
  #define MyCrtDlls "vcruntime140.dll,vcruntime140_1.dll"
#endif

[Setup]
AppId={{7A3E9C41-5D2B-4E8F-B6A1-9C0D2E4F6A8B}
AppName=Selora IDE
AppVersion={#MyAppVersion}
AppVerName=Selora IDE {#MyAppVersion}
AppPublisher=Toxic
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
DefaultDirName={localappdata}\Programs\Selora IDE
; Per-user install: no admin prompt (RELEASE.md section 2.1).
PrivilegesRequired=lowest
; Welcome, Install, Finish - no unnecessary pages (RELEASE.md section 2.2).
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
WizardStyle=modern
WizardImageFile={#SourcePath}..\assets\icons\wizard_large.bmp
WizardSmallImageFile={#SourcePath}..\assets\icons\wizard_small.bmp
OutputDir={#SourcePath}output
OutputBaseFilename=Selora-IDE-Setup-{#MyAppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
SetupIconFile={#SourcePath}..\assets\icons\selora-ide.ico
UninstallDisplayIcon={app}\selora-ide.ico
UninstallDisplayName=Selora IDE
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1809 (build 17763) or newer - the floor documented in
; docs/installation.md and supported by WebView2.
MinVersion=10.0.17763
; The .txt entry below is a HKCU OpenWithProgids registration; tell the shell
; to refresh its association cache after install/uninstall.
ChangesAssociations=yes

[Files]
; The Tauri shell is selora-desktop.exe (workspace bin name), built in
; {#MyTargetDir}.
Source: "{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourcePath}..\assets\icons\selora-ide.ico"; DestDir: "{app}"; Flags: ignoreversion

; Dynamic CRT (src-tauri/build.rs sets static_vc_runtime(false)). prebuild.bat
; and the release workflow stage every VCRUNTIME140*/MSVCP140*/CONCRT140/
; VCOMP140 DLL the exe imports beside it, then pass that list as /DMyCrtDlls.
; vcruntime140.dll and vcruntime140_1.dll are always required (the x64 binary
; imports them); the rest are optional at compile time - an offline or
; older-toolset build may not import them, and the [Code] loop only demands
; the DLLs the builder reported as staged.
Source: "{#MyTargetDir}\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyTargetDir}\vcruntime140_1.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyTargetDir}\msvcp140.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#MyTargetDir}\msvcp140_1.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#MyTargetDir}\msvcp140_2.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#MyTargetDir}\msvcp140_atomic_wait.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#MyTargetDir}\msvcp140_codecvt_ids.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#MyTargetDir}\concrt140.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#MyTargetDir}\vcomp140.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

; Evergreen Bootstrapper, staged beside the exe by prebuild.bat / CI. Runs
; only when the [Code] check finds no WebView2 runtime. Optional at compile
; time so an offline local build still works.
Source: "{#MyTargetDir}\MicrosoftEdgeWebview2Setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall skipifsourcedoesntexist

[Icons]
Name: "{userappdata}\Microsoft\Windows\Start Menu\Programs\Selora IDE.lnk"; Filename: "{app}\selora-desktop.exe"; IconFilename: "{app}\selora-ide.ico"; Comment: "Selora IDE"

[Run]
; Do not offer to launch into a missing WebView2 runtime; the post-install
; check below explains what to install instead.
Filename: "{app}\selora-desktop.exe"; Description: "{cm:LaunchProgram,Selora IDE}"; Flags: nowait postinstall skipifsilent; Check: WebView2Available

[UninstallDelete]
; The app's per-user data (%APPDATA%\selora-ide: settings.json,
; workspace.json, sessions.json) - RELEASE.md's skeleton named {localappdata};
; the app writes under Roaming (selora-settings), so that is what a clean
; uninstall removes.
Type: filesandordirs; Name: "{userappdata}\selora-ide"

[Registry]
; ".txt opens with Selora IDE" (Lite only; no source-file defaults,
; RELEASE.md section 9 item 5). OpenWithProgids adds Selora IDE to the
; "Open with" list without hijacking the user's existing default.
; Owner decision: keep the .txt association as OpenWithProgids only - do not
; turn it into the default handler for .txt, and do not add other extensions.
Root: HKCU; Subkey: "Software\Classes\.txt\OpenWithProgids"; ValueType: string; ValueName: "SeloraIDE.txt"; ValueData: ""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\SeloraIDE.txt"; ValueType: string; ValueName: ""; ValueData: "Text Document"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SeloraIDE.txt\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\selora-ide.ico"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SeloraIDE.txt\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\selora-desktop.exe"" ""%1"""; Flags: uninsdeletekey

[Code]
const
  { Microsoft's WebView2 Evergreen Runtime client GUID, from "Detect the
    WebView2 Runtime" in Microsoft's WebView2 documentation. }
  WebView2ClientGuid = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

function ReadPv(const RootKey: Integer; const SubKey: String): String;
var
  Value: String;
begin
  Result := '';
  if RegQueryStringValue(RootKey, SubKey, 'pv', Value) then
    Result := Trim(Value);
end;

{ Returns the installed Evergreen WebView2 version, or '' when absent.
  The runtime writes a "pv" version value under EdgeUpdate\Clients:
    HKCU\Software\Microsoft\EdgeUpdate\Clients\<guid>             per-user
    HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\<guid> 32-bit view
    HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\<guid>             64-bit view
  Inno's HKLM32 root is already the WOW6432Node view, so that path is not
  repeated. The Selora prefix keeps the name clear of any support function
  a future Inno Setup release might add. }
function SeloraWebView2Version(): String;
var
  Version: String;
begin
  Version := ReadPv(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\' + WebView2ClientGuid);
  if Version = '' then
    Version := ReadPv(HKLM32, 'SOFTWARE\Microsoft\EdgeUpdate\Clients\' + WebView2ClientGuid);
  if Version = '' then
    Version := ReadPv(HKLM64, 'SOFTWARE\Microsoft\EdgeUpdate\Clients\' + WebView2ClientGuid);
  Result := Version;
end;

function WebView2Available(): Boolean;
begin
  Result := SeloraWebView2Version() <> '';
end;

{ Next comma-separated item of Rest, or '' when the list is exhausted. }
function SeloraNextCrtDll(var Rest: String): String;
var
  P: Integer;
begin
  P := Pos(',', Rest);
  if P > 0 then
  begin
    Result := Trim(Copy(Rest, 1, P - 1));
    Delete(Rest, 1, P);
  end
  else
  begin
    Result := Trim(Rest);
    Rest := '';
  end;
end;

{ Names from /DMyCrtDlls that did not land in the application directory, one
  per line ('' when the install is complete). Kept in sync with the [Files]
  entries above by the build scripts, which pass exactly the DLLs dumpbin
  reported. }
function SeloraMissingCrtDlls(): String;
var
  Rest: String;
  Dll: String;
begin
  Result := '';
  Rest := '{#MyCrtDlls}';
  while Rest <> '' do
  begin
    Dll := SeloraNextCrtDll(Rest);
    if (Dll <> '') and (not FileExists(ExpandConstant('{app}\' + Dll))) then
      Result := Result + #13#10 + Dll;
  end;
end;

function InstallWebView2(): Boolean;
var
  Bootstrapper: String;
  ResultCode: Integer;
begin
  Result := False;
  Bootstrapper := ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe');
  if not FileExists(Bootstrapper) then
  begin
    MsgBox('Microsoft Edge WebView2 Runtime is not installed, and this build'
      + #13#10 + 'does not include the bootstrapper.' + #13#10 + #13#10
      + 'Install the Evergreen Runtime from:' + #13#10
      + 'https://developer.microsoft.com/microsoft-edge/webview2/'
      + #13#10 + #13#10 + 'then start Selora IDE again.',
      mbError, MB_OK);
    exit;
  end;

  if not Exec(Bootstrapper, '/silent /install', '', SW_SHOW,
    ewWaitUntilTerminated, ResultCode) then
  begin
    { ResultCode is undefined when Exec itself fails, so it must not be
      passed to SysErrorMessage. }
    MsgBox('Could not start the bundled WebView2 bootstrapper' + #13#10
      + '(MicrosoftEdgeWebview2Setup.exe).' + #13#10 + #13#10
      + 'Install the Evergreen Runtime manually from:' + #13#10
      + 'https://developer.microsoft.com/microsoft-edge/webview2/',
      mbError, MB_OK);
    exit;
  end;

  { 3010/1641 mean success but a reboot is required. }
  if (ResultCode <> 0) and (ResultCode <> 3010) and (ResultCode <> 1641) then
  begin
    MsgBox('The WebView2 bootstrapper failed (exit code ' + IntToStr(ResultCode)
      + ').' + #13#10 + #13#10
      + 'Install the Evergreen Runtime manually from:' + #13#10
      + 'https://developer.microsoft.com/microsoft-edge/webview2/',
      mbError, MB_OK);
    exit;
  end;

  Result := SeloraWebView2Version() <> '';
  if not Result then
    MsgBox('The WebView2 bootstrapper finished, but the runtime was not'
      + #13#10 + 'detected. Restart Windows, or install the Evergreen Runtime'
      + #13#10 + 'manually from:' + #13#10
      + 'https://developer.microsoft.com/microsoft-edge/webview2/',
      mbError, MB_OK);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  MissingDlls: String;
begin
  if CurStep = ssPostInstall then
  begin
    { Install-time sanity check: every CRT DLL the build reported as staged
      must be beside the exe. The required [Files] entries make this rare;
      it catches a mismatched /DMyCrtDlls or a hand-edited script. }
    MissingDlls := SeloraMissingCrtDlls();
    if MissingDlls <> '' then
      MsgBox('The bundled Visual C++ runtime is incomplete. Missing:'
        + MissingDlls + #13#10 + #13#10
        + 'Selora IDE needs these DLLs beside selora-desktop.exe. Install the'
        + #13#10 + 'Microsoft Visual C++ 2015-2022 x64 Redistributable, then'
        + #13#10 + 'start Selora IDE.',
        mbInformation, MB_OK);

    { Run the Evergreen bootstrapper only when no runtime is registered.
      It installs per-user when Setup is not elevated, matching the per-user
      application install. }
    if not WebView2Available() then
      InstallWebView2();
  end;
end;