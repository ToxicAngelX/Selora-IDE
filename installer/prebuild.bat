@echo off
setlocal EnableExtensions
rem Selora IDE - local installer build (RELEASE.md section 2.3).
rem
rem   installer\prebuild.bat
rem
rem Reads the workspace version from the root Cargo.toml, stages every MSVC
rem runtime DLL the release binary actually imports beside it, fetches the
rem WebView2 Evergreen bootstrapper when it is not already cached, and runs
rem Inno Setup with /DMyAppVersion, /DMyTargetDir, and /DMyCrtDlls.
rem
rem Honors CARGO_TARGET_DIR: the release binary and the staged DLLs are read
rem from %CARGO_TARGET_DIR%\release when it is set (cargo resolves a relative
rem CARGO_TARGET_DIR against the directory it runs in, which is this repo
rem root), otherwise from target\release.
rem
rem Requires: an existing release build, Inno Setup 6.3+ (or 7.x), and the
rem MSVC x64 toolset (dumpbin.exe + the CRT redist directory ship with it).

if /i "%~1"=="--help" goto usage
if /i "%~1"=="/?" goto usage
if not "%~1"=="" (
    echo prebuild: unknown argument "%~1" 1>&2
    goto usage_error
)

cd /d "%~dp0.."
if errorlevel 1 (
    echo prebuild: could not enter the repository root 1>&2
    exit /b 1
)

rem Windows PowerShell 5.1 is the "powershell.exe" the script calls. Pin its
rem module path to the in-box modules: an inherited PSModulePath from a
rem PowerShell 7 prompt (or a vendored runtime) makes Get-AuthenticodeSignature
rem autoload an incompatible module and fail the WebView2 signature check.
set "PSModulePath=%SystemRoot%\System32\WindowsPowerShell\v1.0\Modules"

for /f "usebackq delims=" %%v in (`powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0read-version.ps1"`) do set "VERSION=%%v"
if not defined VERSION (
    echo prebuild: could not read the workspace version from Cargo.toml 1>&2
    exit /b 1
)
echo prebuild: version %VERSION%

rem Resolve the build directory the same way cargo does. %%~fi turns a
rem relative CARGO_TARGET_DIR into an absolute path so the Inno Setup script
rem (whose Source paths are relative to installer\) still finds the files.
set "TARGET_DIR=target"
if defined CARGO_TARGET_DIR set "TARGET_DIR=%CARGO_TARGET_DIR%"
for %%i in ("%TARGET_DIR%\release") do set "RELEASE_DIR=%%~fi"
set "APP_EXE=%RELEASE_DIR%\selora-desktop.exe"

if not exist "%APP_EXE%" (
    echo prebuild: %APP_EXE% is missing 1>&2
    echo prebuild: build the frontend, then the release binary: 1>&2
    echo prebuild:   npm --prefix apps\selora-desktop\frontend run build 1>&2
    echo prebuild:   cargo build --release -p selora-desktop 1>&2
    if defined CARGO_TARGET_DIR echo prebuild: CARGO_TARGET_DIR is "%CARGO_TARGET_DIR%" 1>&2
    exit /b 1
)

set "CRT_DLLS="
call :stage_crt
if errorlevel 1 exit /b 1
if not defined CRT_DLLS (
    echo prebuild: could not determine which CRT DLLs were staged 1>&2
    exit /b 1
)

call :stage_webview2

set "ISCC="
for %%r in ("%ProgramFiles(x86)%" "%ProgramFiles%" "%LOCALAPPDATA%\Programs") do (
    for %%v in (6 7) do (
        if not defined ISCC if exist "%%~r\Inno Setup %%v\ISCC.exe" set "ISCC=%%~r\Inno Setup %%v\ISCC.exe"
    )
)
if not defined ISCC (
    echo prebuild: Inno Setup 6.3 or newer ^(or 7.x^) not found 1>&2
    echo prebuild: install it with: winget install JRSoftware.InnoSetup 1>&2
    exit /b 1
)
echo prebuild: using "%ISCC%"

rem /DMyTargetDir and /DMyCrtDlls keep the installer in step with whatever was
rem actually built and staged; selora-ide.iss defaults them for a hand-run ISCC.
"%ISCC%" "/DMyAppVersion=%VERSION%" "/DMyTargetDir=%RELEASE_DIR%" "/DMyCrtDlls=%CRT_DLLS%" "%~dp0selora-ide.iss"
if errorlevel 1 (
    echo prebuild: ISCC failed 1>&2
    exit /b 1
)

echo prebuild: wrote installer\output\Selora-IDE-Setup-%VERSION%.exe
exit /b 0


:stage_crt
rem The app links the dynamic CRT (src-tauri/build.rs sets
rem static_vc_runtime(false)). Derive the DLL names from dumpbin's import
rem table - VCRUNTIME140*, MSVCP140*, CONCRT140, VCOMP140 - exactly like the
rem release workflow, and stage them beside the exe. dumpbin is required: a
rem silent skip is how a missing msvcp140.dll ships.
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
    echo prebuild: vswhere.exe not found 1>&2
    echo prebuild: install Visual Studio Build Tools with the C++ workload 1>&2
    exit /b 1
)

set "VSPATH="
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSPATH=%%i"
if not defined VSPATH (
    echo prebuild: no Visual Studio installation with the MSVC x64 toolset 1>&2
    exit /b 1
)

set "CRT="
for /f "delims=" %%v in ('dir /b /ad /o-n "%VSPATH%\VC\Redist\MSVC\14.*" 2^>nul') do (
    if not defined CRT (
        for /d %%c in ("%VSPATH%\VC\Redist\MSVC\%%v\x64\Microsoft.VC*.CRT") do (
            if exist "%%~fc\vcruntime140.dll" set "CRT=%%~fc"
        )
    )
)
if not defined CRT (
    echo prebuild: MSVC x64 CRT redist not found under "%VSPATH%\VC\Redist\MSVC" 1>&2
    echo prebuild: install the "MSVC v143 - VS 2022 C++ x64/x86 build tools" component 1>&2
    exit /b 1
)

set "DUMPBIN="
for /f "delims=" %%d in ('dir /b /ad /o-n "%VSPATH%\VC\Tools\MSVC\14.*" 2^>nul') do (
    if not defined DUMPBIN if exist "%VSPATH%\VC\Tools\MSVC\%%d\bin\Hostx64\x64\dumpbin.exe" set "DUMPBIN=%VSPATH%\VC\Tools\MSVC\%%d\bin\Hostx64\x64\dumpbin.exe"
)
if not defined DUMPBIN (
    echo prebuild: dumpbin.exe not found under "%VSPATH%\VC\Tools\MSVC" 1>&2
    echo prebuild: install the "MSVC v143 - VS 2022 C++ x64/x86 build tools" component 1>&2
    echo prebuild: without its import table the CRT staging cannot be verified 1>&2
    exit /b 1
)

set "SELORA_DUMPBIN=%DUMPBIN%"
set "SELORA_EXE=%APP_EXE%"
set "SELORA_OUT=%RELEASE_DIR%"
set "SELORA_CRT=%CRT%"
set "SELORA_LIST=%TEMP%\selora-crt-dlls-%RANDOM%.txt"
if exist "%SELORA_LIST%" del /q "%SELORA_LIST%" >nul 2>&1

rem One PowerShell pass: parse dumpbin, match the CRT import set (the [.] in
rem each pattern escapes the dot so MSVCP140.dll cannot match MSVCP140Xdll),
rem then copy each name from the CRT folder or its Microsoft.VC*.OpenMP
rem sibling (concrt140 / vcomp140 live outside Microsoft.VC*.CRT).
powershell -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $dumpbin=$env:SELORA_DUMPBIN; $exe=$env:SELORA_EXE; $out=$env:SELORA_OUT; $crt=$env:SELORA_CRT; if (-not (Test-Path -LiteralPath $exe)) { throw ('missing ' + $exe) }; $imports = (& $dumpbin /nologo /dependents $exe) -join ' '; if ($imports -notmatch 'VCRUNTIME140[.]dll') { throw 'the exe does not import VCRUNTIME140.dll - if the CRT became static, update installer/selora-ide.iss' }; $pattern = '^(VCRUNTIME140|VCRUNTIME140_1|MSVCP140|MSVCP140_[A-Z0-9_]+|CONCRT140|VCOMP140)[.]dll$'; $names = New-Object System.Collections.Generic.List[string]; foreach ($n in @('vcruntime140.dll','vcruntime140_1.dll')) { if (-not $names.Contains($n)) { $names.Add($n) } }; foreach ($t in ($imports -split '\s+')) { $name = $t.Trim(); if ($name -match $pattern) { $lower = $name.ToLowerInvariant(); if (-not $names.Contains($lower)) { $names.Add($lower) } } }; $dirs = @($crt); Get-ChildItem -LiteralPath (Split-Path -Parent $crt) -Directory | ForEach-Object { $dirs += $_.FullName }; foreach ($name in $names) { $src = $null; foreach ($d in $dirs) { $p = Join-Path $d $name; if (Test-Path -LiteralPath $p) { $src = $p; break } }; if ($null -eq $src) { throw ('the MSVC redist has no ' + $name + ', but the exe imports it') }; Copy-Item -LiteralPath $src -Destination $out -Force }; [IO.File]::WriteAllText($env:SELORA_LIST, ($names -join ','))"
if errorlevel 1 (
    echo prebuild: could not stage the CRT DLLs 1>&2
    if exist "%SELORA_LIST%" del /q "%SELORA_LIST%" >nul 2>&1
    exit /b 1
)

set /p CRT_DLLS=<"%SELORA_LIST%"
if exist "%SELORA_LIST%" del /q "%SELORA_LIST%" >nul 2>&1
if not defined CRT_DLLS (
    echo prebuild: dumpbin reported no CRT imports to stage 1>&2
    exit /b 1
)

echo prebuild: staged CRT DLLs %CRT_DLLS%
exit /b 0


:stage_webview2
rem Microsoft's Evergreen Bootstrapper (x64), staged beside the exe. It
rem downloads the runtime only when the installer actually runs it (the
rem [Code] guard checks first).
set "WV2=%RELEASE_DIR%\MicrosoftEdgeWebview2Setup.exe"
set "SELORA_WEBVIEW2_OUT=%WV2%"
if exist "%WV2%" (
    powershell -NoProfile -ExecutionPolicy Bypass -Command "$sig = Get-AuthenticodeSignature -FilePath $env:SELORA_WEBVIEW2_OUT; if ($sig.Status -eq 'Valid' -and $sig.SignerCertificate.Subject -match 'Microsoft') { exit 0 } else { exit 1 }"
    if not errorlevel 1 (
        echo prebuild: WebView2 bootstrapper already cached
        exit /b 0
    )
    echo prebuild: cached WebView2 bootstrapper failed its signature check; downloading a fresh copy
    del /q "%WV2%" >nul 2>&1
)
echo prebuild: downloading the WebView2 Evergreen bootstrapper...
rem go.microsoft.com redirects to a CDN, and transient connection failures
rem are common. Retry a few times, and never leave a partial download
rem behind: ISCC would happily embed it (the [Files] entry only tests
rem whether the file exists).
set "WV2_ATTEMPT=0"
:webview2_download
set /a WV2_ATTEMPT+=1
powershell -NoProfile -ExecutionPolicy Bypass -Command "$ProgressPreference='SilentlyContinue'; [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; $out = $env:SELORA_WEBVIEW2_OUT; try { Invoke-WebRequest -UseBasicParsing -Uri 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $out -TimeoutSec 120 -ErrorAction Stop; $sig = Get-AuthenticodeSignature -FilePath $out; if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'Microsoft') { throw ('signature check failed: ' + $sig.Status) }; exit 0 } catch { Write-Warning $_.Exception.Message; Remove-Item -LiteralPath $out -Force -ErrorAction SilentlyContinue; exit 1 }"
if not errorlevel 1 goto webview2_downloaded
if %WV2_ATTEMPT% GEQ 3 goto webview2_unavailable
echo prebuild: retrying the WebView2 bootstrapper download (attempt %WV2_ATTEMPT% of 3 failed) 1>&2
ping -n 4 127.0.0.1 >nul
goto webview2_download

:webview2_downloaded
echo prebuild: WebView2 bootstrapper downloaded
exit /b 0

:webview2_unavailable
echo prebuild: WARNING: could not download the WebView2 bootstrapper 1>&2
echo prebuild: the installer will still build, but a machine without WebView2 will be told to install it manually 1>&2
if exist "%WV2%" del /q "%WV2%" >nul 2>&1
exit /b 0


:usage
echo Usage: installer\prebuild.bat
echo.
echo Builds installer\output\Selora-IDE-Setup-^<version^>.exe from an existing
echo release binary: target\release\selora-desktop.exe, or
echo %%CARGO_TARGET_DIR%%\release\selora-desktop.exe when CARGO_TARGET_DIR is set.
echo.
echo Requires Inno Setup 6.3 or newer ^(or 7.x^) and the MSVC x64 toolset.
echo No files are written by this help text; running the script for real
echo builds the installer.
exit /b 0


:usage_error
echo Usage: installer\prebuild.bat [--help]
exit /b 2