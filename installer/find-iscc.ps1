# Finds the Inno Setup compiler (ISCC.exe) and prints its absolute path.
#
# prebuild.bat and .github/workflows/release.yml both call this instead of
# hard-coding "C:\Program Files (x86)\Inno Setup 6\ISCC.exe". ISCC is not
# always there: winget installs per-user under %LOCALAPPDATA%\Programs, and
# an unpacked/portable Inno Setup registers only an uninstall entry (the
# Phase 8 acceptance run found one under %TEMP%). Hard-coding the path made
# prebuild.bat unusable on that machine, which pushed the installer build
# into a manual ISCC call with an /O override outside installer\output\.
#
# Search order, first hit wins:
#   1. $env:SELORA_ISCC (explicit path to ISCC.exe, for a non-standard install)
#   2. ISCC.exe on PATH
#   3. Inno Setup 6 / 7 under Program Files, Program Files (x86), and the
#      per-user %LOCALAPPDATA%\Programs location
#   4. The uninstall registry entries (HKLM 64-bit, HKLM WOW6432Node, HKCU),
#      reading InstallLocation, DisplayIcon, and "Inno Setup: App Path"
#
# Prints exactly one line - the absolute path - so callers can capture it with
# `for /f` (prebuild.bat). Diagnostics go to stderr. Exits 1 when not found.
#
# PowerShell 5.1 compatible: no ternary operator, no PS7-only cmdlets.

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Write-Diag {
    param([string]$Message)
    [Console]::Error.WriteLine("find-iscc: $Message")
}

function Resolve-IsccPath {
    param([string]$Candidate)

    if ([string]::IsNullOrWhiteSpace($Candidate)) {
        return $null
    }

    $path = $Candidate.Trim().Trim('"')
    if (-not $path) {
        return $null
    }

    # DisplayIcon and "Inno Setup: App Path" sometimes point at an .exe.
    if ($path -match '\.exe$') {
        $path = Split-Path -Parent $path
    }
    if (-not $path) {
        return $null
    }

    $exe = Join-Path $path 'ISCC.exe'
    if (Test-Path -LiteralPath $exe -PathType Leaf) {
        return (Resolve-Path -LiteralPath $exe).Path
    }
    return $null
}

function Write-Result {
    param([string]$IsccPath)

    # ISCC.exe carries no version resource, so there is nothing to compare
    # against the 6.3 floor here. The [Setup] directives that need 6.x fail
    # loudly on an older compiler, which is the check that matters.
    Write-Output $IsccPath
    exit 0
}

# 1. Explicit override.
$found = Resolve-IsccPath -Candidate $env:SELORA_ISCC
if ($found) {
    Write-Result -IsccPath $found
}

# 2. PATH.
$command = Get-Command -Name 'ISCC.exe' -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
if ($command -and (Test-Path -LiteralPath $command.Source -PathType Leaf)) {
    Write-Result -IsccPath (Resolve-Path -LiteralPath $command.Source).Path
}

# 3. Standard install directories.
$localPrograms = ''
if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    $localPrograms = Join-Path $env:LOCALAPPDATA 'Programs'
}
$roots = New-Object System.Collections.Generic.List[string]
foreach ($root in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, $localPrograms)) {
    if (-not [string]::IsNullOrWhiteSpace($root)) {
        $roots.Add($root)
    }
}

$names = @('Inno Setup 7', 'Inno Setup 6', 'Inno Setup')
foreach ($root in $roots) {
    foreach ($name in $names) {
        $found = Resolve-IsccPath -Candidate (Join-Path $root $name)
        if ($found) {
            Write-Result -IsccPath $found
        }
    }
}

# 4. Uninstall registry. A portable/extracted Inno Setup has no Program Files
#    directory but does register DisplayName + InstallLocation.
$uninstallKeys = @(
    'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
)

$entries = New-Object System.Collections.Generic.List[object]
foreach ($key in $uninstallKeys) {
    foreach ($entry in @(Get-ItemProperty -Path $key -ErrorAction SilentlyContinue)) {
        if ($entry.DisplayName -match '^Inno Setup') {
            $entries.Add($entry)
        }
    }
}

# Prefer the newest registered install; a machine can carry 6.x and 7.x.
$entries = @($entries | Sort-Object -Property DisplayVersion -Descending)
foreach ($entry in $entries) {
    $candidates = New-Object System.Collections.Generic.List[string]
    if ($entry.PSObject.Properties.Match('Inno Setup: App Path').Count -gt 0) {
        $candidates.Add($entry.'Inno Setup: App Path')
    }
    if ($entry.PSObject.Properties.Match('InstallLocation').Count -gt 0) {
        $candidates.Add($entry.InstallLocation)
    }
    if ($entry.PSObject.Properties.Match('DisplayIcon').Count -gt 0) {
        $candidates.Add($entry.DisplayIcon)
    }

    foreach ($candidate in $candidates) {
        $found = Resolve-IsccPath -Candidate $candidate
        if ($found) {
            Write-Result -IsccPath $found
        }
    }
}

Write-Diag "Inno Setup (ISCC.exe) was not found on PATH, in the standard install"
Write-Diag "directories, or in the uninstall registry. Install it with:"
Write-Diag "  winget install JRSoftware.InnoSetup"
Write-Diag "or put the compiler path in SELORA_ISCC."
exit 1