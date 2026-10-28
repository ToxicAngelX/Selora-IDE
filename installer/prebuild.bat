@echo off
setlocal
rem Selora IDE — installer build (RELEASE.md §2.3).
rem Reads the workspace version from the root Cargo.toml (read-version.ps1)
rem and invokes ISCC with it.

cd /d "%~dp0\.."

for /f "delims=" %%v in ('powershell -NoProfile -ExecutionPolicy Bypass -File installer\read-version.ps1') do set "VERSION=%%v"

if "%VERSION%"=="" (
    echo prebuild: could not read the workspace version from Cargo.toml 1>&2
    exit /b 1
)

echo prebuild: version %VERSION%

if not exist "target\release\selora-app.exe" (
    echo prebuild: target\release\selora-app.exe missing - run: cargo build --workspace --release 1>&2
    exit /b 1
)

set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%ProgramFiles%\Inno Setup 6\ISCC.exe"
rem winget installs Inno per-user by default:
if not exist "%ISCC%" set "ISCC=%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" (
    echo prebuild: Inno Setup 6 not found - install: winget install JRSoftware.InnoSetup 1>&2
    exit /b 1
)

"%ISCC%" /DMyAppVersion=%VERSION% installer\selora-ide.iss
if errorlevel 1 exit /b 1
echo prebuild: wrote installer\output\selora-installer-v1.exe
