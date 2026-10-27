<#
.SYNOPSIS
    Headless acceptance helper for ROADMAP.md Phase 8 (Workflows).

.DESCRIPTION
    Runs the smallest real checks that do not need a live key or a window:

      * the release executable exists (and its size/hash are recorded);
      * targeted Rust tests for the engine, desktop host/dispatch, and protocol;
      * targeted frontend tests for the workflows panel and bridge;
      * the `.selora/workflow-runs/...` journal path contract and .gitignore;
      * installer prerequisites (Inno Setup ISCC.exe, WebView2 bootstrapper).

    It then prints the ROADMAP Phase 8 acceptance steps 1-9 with an automated
    status and an explicit OWNER row. A step that needs a real key, a real
    workspace, a visible window, a crash/relaunch, or a human judgement is
    NEVER reported as an automated pass.

    Exit code 0 means every automated check passed. Exit code 1 means at least
    one automated check failed. Owner/SKIP rows never change the exit code.

.PARAMETER Build
    Build the frontend and the release binary before checking them. Without
    this switch the script only inspects an existing target/release binary.

.PARAMETER RepoRoot
    Repository root. Defaults to the parent directory of this script.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase8-acceptance.ps1 -Build

.NOTES
    PowerShell 5.1 compatible. No `??`, no ternary operator, no PS7-only
    cmdlets. Test logs are written under %TEMP%, never into the repository.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = '',
    [switch]$Build
)

$ErrorActionPreference = 'Continue'
Set-StrictMode -Version 2.0

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}
if (-not (Test-Path -LiteralPath $RepoRoot)) {
    Write-Host "FAIL repository root not found: $RepoRoot" -ForegroundColor Red
    exit 1
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$logDir = Join-Path $env:TEMP ("selora-phase8-acceptance-" + $stamp)
$null = New-Item -ItemType Directory -Path $logDir -Force

$checks = New-Object System.Collections.ArrayList

function Add-Check {
    param(
        [string]$Id,
        [ValidateSet('PASS', 'FAIL', 'SKIP')]
        [string]$Status,
        [string]$Detail
    )
    $null = $checks.Add([pscustomobject]@{
        Id     = $Id
        Status = $Status
        Detail = $Detail
    })
    $color = 'Gray'
    if ($Status -eq 'PASS') { $color = 'Green' }
    if ($Status -eq 'FAIL') { $color = 'Red' }
    if ($Status -eq 'SKIP') { $color = 'Yellow' }
    Write-Host ("[{0}] {1} - {2}" -f $Status, $Id, $Detail) -ForegroundColor $color
}

function Get-CheckStatus {
    param([string]$Id)
    foreach ($check in $checks) {
        if ($check.Id -eq $Id) { return $check.Status }
    }
    return 'SKIP'
}

function Invoke-Native {
    param(
        [string]$FilePath,
        [string[]]$Arguments,
        [string]$WorkingDirectory,
        [string]$LogName
    )
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $lines = @()
    $code = -1
    Push-Location -LiteralPath $WorkingDirectory
    try {
        $lines = @(& $FilePath @Arguments 2>&1 | ForEach-Object { $_.ToString() })
        $code = $LASTEXITCODE
    }
    catch {
        $lines = @($_.Exception.Message)
        $code = -1
    }
    finally {
        Pop-Location
        $watch.Stop()
    }
    if (-not [string]::IsNullOrWhiteSpace($LogName)) {
        $path = Join-Path $logDir $LogName
        Set-Content -LiteralPath $path -Value $lines -Encoding UTF8
    }
    return [pscustomobject]@{
        ExitCode = $code
        Seconds  = [math]::Round($watch.Elapsed.TotalSeconds, 1)
        Lines    = $lines
    }
}

function Get-RustTestTotals {
    param([string[]]$Lines)
    $passed = 0
    $failed = 0
    $ignored = 0
    $suites = 0
    foreach ($line in $Lines) {
        if ($line -match 'test result: (ok|FAILED)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored') {
            $suites += 1
            $passed += [int]$Matches[2]
            $failed += [int]$Matches[3]
            $ignored += [int]$Matches[4]
        }
    }
    return [pscustomobject]@{
        Passed = $passed
        Failed = $failed
        Ignored = $ignored
        Suites = $suites
    }
}

function Get-VitestTotals {
    param([string[]]$Lines)
    $passed = 0
    $failed = 0
    $seen = $false
    foreach ($line in $Lines) {
        # vitest colorizes its summary; strip ANSI escapes before matching.
        $clean = $line
        if ($clean.IndexOf([char]27) -ge 0) {
            $clean = $clean -replace "$([char]27)\[[0-9;]*[A-Za-z]", ''
        }
        if ($clean -match '^\s*Tests\s+(.*)$') {
            $summary = $Matches[1]
            $seen = $true
            if ($summary -match '(\d+) failed') { $failed += [int]$Matches[1] }
            if ($summary -match '(\d+) passed') { $passed += [int]$Matches[1] }
        }
    }
    return [pscustomobject]@{
        Passed = $passed
        Failed = $failed
        Seen   = $seen
    }
}

function Get-Tail {
    param([string[]]$Lines, [int]$Count)
    if ($Lines.Count -le $Count) { return $Lines }
    return $Lines[($Lines.Count - $Count)..($Lines.Count - 1)]
}

function Find-Iscc {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return [pscustomobject]@{ Path = $command.Source; Version = '' }
    }

    $candidates = New-Object System.Collections.ArrayList
    $null = $candidates.Add((Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'))
    $null = $candidates.Add((Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 7\ISCC.exe'))
    $null = $candidates.Add((Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'))
    $null = $candidates.Add((Join-Path $env:ProgramFiles 'Inno Setup 7\ISCC.exe'))
    $pf86 = ${env:ProgramFiles(x86)}
    if (-not [string]::IsNullOrWhiteSpace($pf86)) {
        $null = $candidates.Add((Join-Path $pf86 'Inno Setup 6\ISCC.exe'))
        $null = $candidates.Add((Join-Path $pf86 'Inno Setup 7\ISCC.exe'))
    }

    # The uninstall entry also carries the real version (ISCC.exe's file
    # version resource can be 0.0.0.0 on some Inno builds).
    $uninstallRoots = @(
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )
    $registeredVersion = ''
    foreach ($root in $uninstallRoots) {
        $entries = Get-ItemProperty -Path $root -ErrorAction SilentlyContinue
        foreach ($entry in $entries) {
            $name = $entry.PSObject.Properties['DisplayName']
            if ($null -eq $name -or $name.Value -notlike 'Inno Setup*') { continue }
            $location = $entry.PSObject.Properties['InstallLocation']
            if ($null -ne $location -and -not [string]::IsNullOrWhiteSpace($location.Value)) {
                $null = $candidates.Add((Join-Path $location.Value 'ISCC.exe'))
            }
            $version = $entry.PSObject.Properties['DisplayVersion']
            if ($null -ne $version -and -not [string]::IsNullOrWhiteSpace($version.Value)) {
                $registeredVersion = $version.Value
            }
        }
    }

    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            return [pscustomobject]@{ Path = $candidate; Version = $registeredVersion }
        }
    }
    return $null
}

Write-Host ''
Write-Host 'Selora IDE - Phase 8 (Workflows) headless acceptance' -ForegroundColor Cyan
Write-Host ("Repository : " + $RepoRoot)
Write-Host ("Started    : " + (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'))
Write-Host ("Logs       : " + $logDir)
Write-Host ("PowerShell : " + $PSVersionTable.PSVersion.ToString())
Write-Host ''

# ---------------------------------------------------------------------------
# Optional build (the same commands RELEASE.md documents).
# ---------------------------------------------------------------------------
$frontendDir = Join-Path $RepoRoot 'apps\selora-desktop\frontend'

if ($Build) {
    Write-Host 'Build: npm run build (frontend)' -ForegroundColor Cyan
    $front = Invoke-Native -FilePath 'npm.cmd' -Arguments @('--prefix', $frontendDir, 'run', 'build') `
        -WorkingDirectory $RepoRoot -LogName 'frontend-build.log'
    Write-Host ("       exit {0}, {1}s, log frontend-build.log" -f $front.ExitCode, $front.Seconds)
    if ($front.ExitCode -ne 0) {
        Get-Tail -Lines $front.Lines -Count 20 | ForEach-Object { Write-Host ("       " + $_) }
    }

    Write-Host 'Build: cargo clean -p selora-desktop --release (re-embed current dist)' -ForegroundColor Cyan
    $clean = Invoke-Native -FilePath 'cargo' -Arguments @('clean', '-p', 'selora-desktop', '--release') `
        -WorkingDirectory $RepoRoot -LogName 'release-clean.log'
    Write-Host ("       exit {0}, {1}s" -f $clean.ExitCode, $clean.Seconds)

    Write-Host 'Build: cargo build --release -p selora-desktop --locked' -ForegroundColor Cyan
    $release = Invoke-Native -FilePath 'cargo' `
        -Arguments @('build', '--release', '-p', 'selora-desktop', '--locked') `
        -WorkingDirectory $RepoRoot -LogName 'release-build.log'
    Write-Host ("       exit {0}, {1}s, log release-build.log" -f $release.ExitCode, $release.Seconds)
    if ($release.ExitCode -ne 0) {
        Get-Tail -Lines $release.Lines -Count 30 | ForEach-Object { Write-Host ("       " + $_) }
    }
}

# ---------------------------------------------------------------------------
# 1. Release artifact.
# ---------------------------------------------------------------------------
$targetRoot = Join-Path $RepoRoot 'target'
if (-not [string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    if ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        $targetRoot = $env:CARGO_TARGET_DIR
    }
    else {
        $targetRoot = Join-Path $RepoRoot $env:CARGO_TARGET_DIR
    }
}
$releaseDir = Join-Path $targetRoot 'release'
$exePath = Join-Path $releaseDir 'selora-desktop.exe'

if (Test-Path -LiteralPath $exePath) {
    $exe = Get-Item -LiteralPath $exePath
    $hash = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $sizeMb = [math]::Round($exe.Length / 1MB, 2)
    Add-Check -Id 'release-exe' -Status 'PASS' `
        -Detail ("{0} ({1} MB, SHA256 {2})" -f $exePath, $sizeMb, $hash)
    Add-Check -Id 'release-exe-hash' -Status 'PASS' -Detail ("sha256=" + $hash)
}
else {
    Add-Check -Id 'release-exe' -Status 'FAIL' `
        -Detail ("missing " + $exePath + " - run with -Build or: npm.cmd --prefix apps\selora-desktop\frontend run build; cargo build --release -p selora-desktop --locked")
}

# ---------------------------------------------------------------------------
# 2. Targeted tests.
# ---------------------------------------------------------------------------
Write-Host ''
Write-Host 'Tests' -ForegroundColor Cyan

$engine = Invoke-Native -FilePath 'cargo' -Arguments @('test', '-p', 'selora-workflow', '--locked') `
    -WorkingDirectory $RepoRoot -LogName 'test-engine.log'
$engineTotals = Get-RustTestTotals -Lines $engine.Lines
if ($engine.ExitCode -eq 0 -and $engineTotals.Failed -eq 0 -and $engineTotals.Passed -gt 0) {
    Add-Check -Id 'test.engine' -Status 'PASS' `
        -Detail ("cargo test -p selora-workflow: {0} passed, {1} failed, {2} ignored, {3}s" -f $engineTotals.Passed, $engineTotals.Failed, $engineTotals.Ignored, $engine.Seconds)
}
else {
    Add-Check -Id 'test.engine' -Status 'FAIL' `
        -Detail ("cargo test -p selora-workflow exit {0}: {1} passed, {2} failed; see test-engine.log" -f $engine.ExitCode, $engineTotals.Passed, $engineTotals.Failed)
    Get-Tail -Lines $engine.Lines -Count 25 | ForEach-Object { Write-Host ("       " + $_) }
}

$protocol = Invoke-Native -FilePath 'cargo' -Arguments @('test', '-p', 'selora-protocol', '--locked', 'workflow') `
    -WorkingDirectory $RepoRoot -LogName 'test-protocol.log'
$protocolTotals = Get-RustTestTotals -Lines $protocol.Lines
if ($protocol.ExitCode -eq 0 -and $protocolTotals.Failed -eq 0 -and $protocolTotals.Passed -gt 0) {
    Add-Check -Id 'test.protocol' -Status 'PASS' `
        -Detail ("cargo test -p selora-protocol workflow: {0} passed, {1} failed, {2}s" -f $protocolTotals.Passed, $protocolTotals.Failed, $protocol.Seconds)
}
else {
    Add-Check -Id 'test.protocol' -Status 'FAIL' `
        -Detail ("cargo test -p selora-protocol workflow exit {0}: {1} passed, {2} failed; see test-protocol.log" -f $protocol.ExitCode, $protocolTotals.Passed, $protocolTotals.Failed)
    Get-Tail -Lines $protocol.Lines -Count 20 | ForEach-Object { Write-Host ("       " + $_) }
}

$desktop = Invoke-Native -FilePath 'cargo' -Arguments @('test', '-p', 'selora-desktop', '--locked', 'workflow') `
    -WorkingDirectory $RepoRoot -LogName 'test-desktop.log'
$desktopTotals = Get-RustTestTotals -Lines $desktop.Lines
if ($desktop.ExitCode -eq 0 -and $desktopTotals.Failed -eq 0 -and $desktopTotals.Passed -gt 0) {
    Add-Check -Id 'test.desktop' -Status 'PASS' `
        -Detail ("cargo test -p selora-desktop workflow: {0} passed, {1} failed, {2}s" -f $desktopTotals.Passed, $desktopTotals.Failed, $desktop.Seconds)
}
else {
    Add-Check -Id 'test.desktop' -Status 'FAIL' `
        -Detail ("cargo test -p selora-desktop workflow exit {0}: {1} passed, {2} failed; see test-desktop.log" -f $desktop.ExitCode, $desktopTotals.Passed, $desktopTotals.Failed)
    Get-Tail -Lines $desktop.Lines -Count 25 | ForEach-Object { Write-Host ("       " + $_) }
}

# The sandbox-absence tests are the evidence for step 9. Run them by name so a
# passing engine suite cannot hide their removal.
$sandbox = Invoke-Native -FilePath 'cargo' `
    -Arguments @('test', '-p', 'selora-workflow', '--locked', 'ambient_globals_are_structurally_absent') `
    -WorkingDirectory $RepoRoot -LogName 'test-sandbox-absence.log'
$sandboxTotals = Get-RustTestTotals -Lines $sandbox.Lines
if ($sandbox.ExitCode -eq 0 -and $sandboxTotals.Failed -eq 0 -and $sandboxTotals.Passed -gt 0) {
    Add-Check -Id 'test.sandbox-absence' -Status 'PASS' `
        -Detail ("ambient_globals_are_structurally_absent: {0} passed, {1}s" -f $sandboxTotals.Passed, $sandbox.Seconds)
}
else {
    Add-Check -Id 'test.sandbox-absence' -Status 'FAIL' `
        -Detail ("ambient_globals_are_structurally_absent exit {0}: {1} passed, {2} failed; see test-sandbox-absence.log" -f $sandbox.ExitCode, $sandboxTotals.Passed, $sandboxTotals.Failed)
    Get-Tail -Lines $sandbox.Lines -Count 15 | ForEach-Object { Write-Host ("       " + $_) }
}

$frontend = Invoke-Native -FilePath 'npm.cmd' `
    -Arguments @('--prefix', $frontendDir, 'run', 'test', '--', 'src/features/workflows') `
    -WorkingDirectory $RepoRoot -LogName 'test-frontend.log'
$frontendTotals = Get-VitestTotals -Lines $frontend.Lines
if ($frontend.ExitCode -eq 0 -and $frontendTotals.Seen -and $frontendTotals.Failed -eq 0 -and $frontendTotals.Passed -gt 0) {
    Add-Check -Id 'test.frontend' -Status 'PASS' `
        -Detail ("vitest src/features/workflows: {0} passed, {1} failed, {2}s" -f $frontendTotals.Passed, $frontendTotals.Failed, $frontend.Seconds)
}
else {
    Add-Check -Id 'test.frontend' -Status 'FAIL' `
        -Detail ("vitest src/features/workflows exit {0}: {1} passed, {2} failed; see test-frontend.log" -f $frontend.ExitCode, $frontendTotals.Passed, $frontendTotals.Failed)
    Get-Tail -Lines $frontend.Lines -Count 25 | ForEach-Object { Write-Host ("       " + $_) }
}

# ---------------------------------------------------------------------------
# 3. Journal path contract (.selora/workflow-runs/<run_id>/journal.jsonl).
# ---------------------------------------------------------------------------
$journalSource = Join-Path $RepoRoot 'crates\selora-workflow\src\journal.rs'
$journalOk = $false
if (Test-Path -LiteralPath $journalSource) {
    $journalText = Get-Content -LiteralPath $journalSource -Raw
    $hasDir = $journalText -match 'JOURNAL_DIR:\s*&str\s*=\s*"\.selora/workflow-runs"'
    $hasFile = $journalText -match 'JOURNAL_FILE:\s*&str\s*=\s*"journal\.jsonl"'
    if ($hasDir -and $hasFile) { $journalOk = $true }
}
if ($journalOk) {
    Add-Check -Id 'journal.path-constants' -Status 'PASS' `
        -Detail 'journal.rs declares .selora/workflow-runs + journal.jsonl'
}
else {
    Add-Check -Id 'journal.path-constants' -Status 'FAIL' `
        -Detail 'journal.rs does not declare the expected .selora/workflow-runs/journal.jsonl constants'
}

$layoutTest = Join-Path $RepoRoot 'crates\selora-workflow\tests\persisted_execution.rs'
$layoutOk = $false
if (Test-Path -LiteralPath $layoutTest) {
    $layoutText = Get-Content -LiteralPath $layoutTest -Raw
    if ($layoutText -match '\.selora/workflow-runs/id/journal\.jsonl') { $layoutOk = $true }
}
if ($layoutOk) {
    Add-Check -Id 'journal.layout-test' -Status 'PASS' `
        -Detail 'persisted_execution covers .selora/workflow-runs/<id>/journal.jsonl'
}
else {
    Add-Check -Id 'journal.layout-test' -Status 'FAIL' `
        -Detail 'persisted_execution no longer asserts the journal layout'
}

$gitignorePath = Join-Path $RepoRoot '.gitignore'
$ignoredOk = $false
if (Test-Path -LiteralPath $gitignorePath) {
    $ignoreText = Get-Content -LiteralPath $gitignorePath -Raw
    if ($ignoreText -match '(?m)^/?\.selora/?\s*$') { $ignoredOk = $true }
}
if ($ignoredOk) {
    Add-Check -Id 'journal.gitignore' -Status 'PASS' -Detail '.gitignore ignores /.selora/'
}
else {
    Add-Check -Id 'journal.gitignore' -Status 'FAIL' -Detail '.gitignore does not ignore .selora/'
}

$workspaceJournal = Join-Path $RepoRoot '.selora\workflow-runs'
if (Test-Path -LiteralPath $workspaceJournal) {
    $runCount = @(Get-ChildItem -LiteralPath $workspaceJournal -Directory -ErrorAction SilentlyContinue).Count
    Add-Check -Id 'journal.workspace-runs' -Status 'PASS' `
        -Detail ("local runs present: {0} ({1})" -f $runCount, $workspaceJournal)
}
else {
    Add-Check -Id 'journal.workspace-runs' -Status 'SKIP' `
        -Detail 'no local .selora/workflow-runs yet (expected until an owner test run)'
}

# ---------------------------------------------------------------------------
# 4. Packaging prerequisites. These are informational for the acceptance
#    steps; a missing optional bootstrapper is a SKIP, not a failure.
# ---------------------------------------------------------------------------
Write-Host ''
Write-Host 'Packaging' -ForegroundColor Cyan

$issPath = Join-Path $RepoRoot 'installer\selora-ide.iss'
if (Test-Path -LiteralPath $issPath) {
    Add-Check -Id 'installer.script' -Status 'PASS' -Detail $issPath
}
else {
    Add-Check -Id 'installer.script' -Status 'FAIL' -Detail ('missing ' + $issPath)
}

$iscc = Find-Iscc
if ($null -ne $iscc) {
    $isccVersion = $iscc.Version
    if ([string]::IsNullOrWhiteSpace($isccVersion)) {
        $isccItem = Get-Item -LiteralPath $iscc.Path
        $isccVersion = $isccItem.VersionInfo.ProductVersion
        if ([string]::IsNullOrWhiteSpace($isccVersion)) {
            $isccVersion = $isccItem.VersionInfo.FileVersion
        }
    }
    Add-Check -Id 'installer.iscc' -Status 'PASS' `
        -Detail ("{0} (version {1})" -f $iscc.Path, $isccVersion)
}
else {
    Add-Check -Id 'installer.iscc' -Status 'FAIL' `
        -Detail 'ISCC.exe not found on PATH, in the standard install dirs, or in the uninstall registry (winget install JRSoftware.InnoSetup)'
}

$bootstrapper = Join-Path $releaseDir 'MicrosoftEdgeWebview2Setup.exe'
if (Test-Path -LiteralPath $bootstrapper) {
    $bootItem = Get-Item -LiteralPath $bootstrapper
    Add-Check -Id 'installer.webview2-bootstrapper' -Status 'PASS' `
        -Detail ("cached {0} ({1} bytes)" -f $bootstrapper, $bootItem.Length)
}
else {
    Add-Check -Id 'installer.webview2-bootstrapper' -Status 'SKIP' `
        -Detail ("not cached at " + $bootstrapper + ' (installer still builds; offline installs then need WebView2 installed manually)')
}

$installerOutput = Join-Path $RepoRoot 'installer\output'
if (Test-Path -LiteralPath $installerOutput) {
    $setups = @(Get-ChildItem -LiteralPath $installerOutput -Filter 'Selora-IDE-Setup-*.exe' -File -ErrorAction SilentlyContinue)
    if ($setups.Count -gt 0) {
        $latest = $setups | Sort-Object LastWriteTime -Descending | Select-Object -First 1
        $setupHash = (Get-FileHash -LiteralPath $latest.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        Add-Check -Id 'installer.artifact' -Status 'PASS' `
            -Detail ("{0} ({1} bytes, SHA256 {2})" -f $latest.FullName, $latest.Length, $setupHash)
    }
    else {
        Add-Check -Id 'installer.artifact' -Status 'SKIP' -Detail 'installer/output has no Selora-IDE-Setup-*.exe'
    }
}
else {
    Add-Check -Id 'installer.artifact' -Status 'SKIP' -Detail 'installer/output absent (no installer compiled in this checkout)'
}

# ---------------------------------------------------------------------------
# 5. ROADMAP Phase 8 acceptance steps 1-9.
# ---------------------------------------------------------------------------
$steps = @(
    [pscustomobject]@{
        Number   = 1
        Title    = 'Chat agent proposes run_workflow; confirmation card shows script + commands'
        Requires = @('test.engine', 'test.desktop', 'test.frontend')
        Owner    = 'Send "review every Rust file in src/ for unwrap() calls; report the worst five" and read the confirmation card: script body, command set, cost cap.'
    },
    [pscustomobject]@{
        Number   = 2
        Title    = 'Approve -> phase graph, subagent fan-out, live cards'
        Requires = @('test.engine', 'test.desktop', 'test.frontend', 'release-exe')
        Owner    = 'Approve, then watch the Workflows panel: phases appear, subagents fan out, step status changes live.'
    },
    [pscustomobject]@{
        Number   = 3
        Title    = 'Subagent edit parks on an approval card; Deny lets the run continue'
        Requires = @('test.desktop', 'test.engine')
        Owner    = 'Wait for a write card, click Deny, confirm the run continues and the file did not change.'
    },
    [pscustomobject]@{
        Number   = 4
        Title    = 'Kill the app mid-run; relaunch offers Resume and finishes'
        Requires = @('test.engine', 'journal.path-constants', 'journal.gitignore')
        Owner    = 'Kill selora-desktop.exe mid-run, relaunch, use Resume, confirm the run finishes with earlier asks not re-billed.'
    },
    [pscustomobject]@{
        Number   = 5
        Title    = 'Failing world.run gate shows the fix-loop round in the panel'
        Requires = @('test.engine', 'test.desktop')
        Owner    = 'Use a gate that fails (for example cargo test on a broken tree) and watch the retry round in the panel.'
    },
    [pscustomobject]@{
        Number   = 6
        Title    = 'Stop mid-run ends the run and kills in-flight children'
        Requires = @('test.desktop', 'test.engine')
        Owner    = 'Click Stop during bash work; confirm the run ends in seconds and no child process of that command survives (grandchildren are a documented limit on Windows).'
    },
    [pscustomobject]@{
        Number   = 7
        Title    = 'Cost meter/summary reflect every ask (tokens + USD per phase)'
        Requires = @('test.engine', 'test.desktop')
        Owner    = 'Compare the run summary tokens/USD with the receipts; confirm the workflow budget cap is visible.'
    },
    [pscustomobject]@{
        Number   = 8
        Title    = 'Journal lands under .selora/workflow-runs/ and never in git'
        Requires = @('journal.path-constants', 'journal.layout-test', 'journal.gitignore', 'test.engine')
        Owner    = 'After a real run, confirm .selora/workflow-runs/<run_id>/journal.jsonl exists and git status stays clean (this checkout has no .git).'
    },
    [pscustomobject]@{
        Number   = 9
        Title    = 'fetch/fs/process attempts fail at the facade (structurally absent)'
        Requires = @('test.sandbox-absence', 'test.engine')
        Owner    = 'Paste a script that calls fetch/fs/process and confirm the run fails with a facade error, not an effect.'
    }
)

function Get-StepAutomatedStatus {
    param([string[]]$Requires)
    $status = 'PASS'
    foreach ($id in $Requires) {
        $value = Get-CheckStatus -Id $id
        if ($value -eq 'FAIL') { return 'FAIL' }
        if ($value -eq 'SKIP') { $status = 'SKIP' }
    }
    return $status
}

Write-Host ''
Write-Host 'ROADMAP Phase 8 acceptance steps' -ForegroundColor Cyan
Write-Host 'OWNER rows are never automated passes. They need a live key, a real workspace, and a visible window.' -ForegroundColor DarkGray
Write-Host ''
$stepFailures = 0
foreach ($step in $steps) {
    $automated = Get-StepAutomatedStatus -Requires $step.Requires
    if ($automated -eq 'FAIL') { $stepFailures += 1 }
    $color = 'Green'
    if ($automated -eq 'FAIL') { $color = 'Red' }
    if ($automated -eq 'SKIP') { $color = 'Yellow' }
    Write-Host ("Step {0}  AUTOMATED: {1}  OWNER: OWNER  {2}" -f $step.Number, $automated, $step.Title) -ForegroundColor $color
    Write-Host ("        owner: " + $step.Owner) -ForegroundColor DarkGray
}

# ---------------------------------------------------------------------------
# 6. Summary and exit code.
# ---------------------------------------------------------------------------
$failed = @($checks | Where-Object { $_.Status -eq 'FAIL' })
$passed = @($checks | Where-Object { $_.Status -eq 'PASS' })
$skipped = @($checks | Where-Object { $_.Status -eq 'SKIP' })

Write-Host ''
Write-Host 'Summary' -ForegroundColor Cyan
Write-Host ("  automated checks : {0} passed, {1} failed, {2} skipped" -f $passed.Count, $failed.Count, $skipped.Count)
Write-Host ("  roadmap steps    : {0} with an automated failure, 9 owner observations pending" -f $stepFailures)
Write-Host ("  logs             : " + $logDir)

if ($failed.Count -gt 0) {
    Write-Host ''
    Write-Host 'RESULT: FAIL - automated acceptance checks failed. Do not mark Phase 8 accepted.' -ForegroundColor Red
    foreach ($check in $failed) {
        Write-Host ("  - " + $check.Id + ": " + $check.Detail) -ForegroundColor Red
    }
    exit 1
}

Write-Host ''
Write-Host 'RESULT: automated checks passed. Phase 8 acceptance still requires the OWNER steps.' -ForegroundColor Green
exit 0