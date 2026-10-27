@echo off
setlocal EnableExtensions
rem Selora IDE - start the desktop app that is already built in this repo.
rem
rem   launch-selora.bat
rem
rem Repo-relative: no absolute build paths. The binary is
rem target\release\selora-desktop.exe, or %CARGO_TARGET_DIR%\release\ when
rem CARGO_TARGET_DIR is set (cargo resolves a relative CARGO_TARGET_DIR
rem against the directory it runs in, which is this repo root).
rem
rem If the binary is missing, print the two build commands and exit 1 - this
rem script never compiles anything.

cd /d "%~dp0"
if errorlevel 1 (
    echo launch-selora: could not enter the repository root 1>&2
    exit /b 1
)

set "TARGET_DIR=target"
if defined CARGO_TARGET_DIR set "TARGET_DIR=%CARGO_TARGET_DIR%"

set "APP_EXE=%TARGET_DIR%\release\selora-desktop.exe"
if not exist "%APP_EXE%" (
    echo launch-selora: selora-desktop.exe is not built yet 1>&2
    echo launch-selora: expected "%CD%\%APP_EXE%" 1>&2
    echo launch-selora: 1>&2
    echo launch-selora: build it first: 1>&2
    echo launch-selora:   npm --prefix apps\selora-desktop\frontend run build 1>&2
    echo launch-selora:   cargo build --release -p selora-desktop 1>&2
    exit /b 1
)

start "" "%APP_EXE%"
if errorlevel 1 (
    echo launch-selora: could not start "%APP_EXE%" 1>&2
    exit /b 1
)
exit /b 0