<#
.SYNOPSIS
    Headless acceptance helper for ROADMAP.md Phase 8 (Workflows).

.DESCRIPTION
    Runs the smallest real checks that do not need a live key or a window:

      * the release executable exists (and its size/hash are recorded);
      * targeted Rust tests for the engine, desktop host/dispatch, and protocol;
      * targeted frontend tests for the workflows panel, bridge, and card;
      * deterministic mock-host evidence for ROADMAP steps 1-9, selected by
        exact test name so a renamed or removed test cannot pass silently;
      * the `.selora/workflow-runs/...` journal path contract, the workspace
        `.selora/.gitignore`, and this repository's ignore rule;
      * installer prerequisites (Inno Setup ISCC.exe, WebView2 bootstrapper).

    It then prints the ROADMAP Phase 8 acceptance steps 1-9 with an automated
    status and an explicit OWNER row. A step that needs a real key, a real
    workspace, a visible window, a crash/relaunch, or a human judgement is
    NEVER reported as an automated pass.

    Status vocabulary:

      PASS    - the named automated evidence ran here and passed;
      FAIL    - the named evidence ran and failed, or a required check failed;
      PENDING - the exact evidence is absent from this tree, or the capability
                is a known unimplemented limit. It is never counted as a pass;
      SKIP    - an optional local artifact/environment is absent.

    Exit code 0 means no automated check failed. Pending and owner rows never
    change the exit code, but the summary prints every pending gap so a green
    exit is never mistaken for full Phase 8 acceptance.

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
        [ValidateSet('PASS', 'FAIL', 'SKIP', 'PENDING')]
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
    if ($Status -eq 'PENDING') { $color = 'Magenta' }
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

# Run one Rust test by exact name. Unit tests carry their module path
# (`sandbox::tests::name`), so a short name is first resolved against the test
# binary's `--list` output and then run with `--exact`. A short name that
# matches nothing is reported PENDING, never PASS, so a removed/renamed test or
# an API that does not exist yet cannot fake acceptance.
$script:rustTestListCache = @{}

function Get-RustTestListing {
    param(
        [string]$Crate,
        [string]$Target
    )
    $key = $Crate + '|' + $Target
    if ($script:rustTestListCache.ContainsKey($key)) {
        return $script:rustTestListCache[$key]
    }
    $arguments = @('test', '-p', $Crate, '--locked')
    if (-not [string]::IsNullOrWhiteSpace($Target)) {
        $arguments += @('--test', $Target)
    }
    $arguments += @('--', '--list')
    $logName = 'list-' + ($Crate + '-' + $Target -replace '[^A-Za-z0-9_.-]', '-') + '.log'
    $run = Invoke-Native -FilePath 'cargo' -Arguments $arguments `
        -WorkingDirectory $RepoRoot -LogName $logName
    $names = @()
    foreach ($line in $run.Lines) {
        if ($line -match '^(?<name>\S.*): test$') {
            $names += $Matches['name']
        }
    }
    $listing = [pscustomobject]@{
        Ok      = ($run.ExitCode -eq 0)
        Names   = $names
        Seconds = $run.Seconds
    }
    $script:rustTestListCache[$key] = $listing
    return $listing
}

function Add-NamedRustTestCheck {
    param(
        [string]$Id,
        [string]$Crate,
        [string]$Test,
        [string]$Target = ''
    )
    $where = $Crate
    if (-not [string]::IsNullOrWhiteSpace($Target)) { $where = $Crate + ' --test ' + $Target }
    $listing = Get-RustTestListing -Crate $Crate -Target $Target
    if (-not $listing.Ok) {
        Add-Check -Id $Id -Status 'FAIL' `
            -Detail ("cannot list tests for {0}; the crate or test target does not build (see list log)" -f $where)
        return
    }
    $names = @($listing.Names)
    $matches = @($names | Where-Object { $_ -eq $Test -or $_.EndsWith('::' + $Test) })
    if ($matches.Count -eq 0) {
        Add-Check -Id $Id -Status 'PENDING' `
            -Detail ("{0} is not present in {1}; no evidence claimed" -f $Test, $where)
        return
    }
    if ($matches.Count -gt 1) {
        Add-Check -Id $Id -Status 'FAIL' `
            -Detail ("{0} is ambiguous in {1}: {2}" -f $Test, $where, ($matches -join ', '))
        return
    }
    $fullName = $matches[0]
    $arguments = @('test', '-p', $Crate, '--locked')
    if (-not [string]::IsNullOrWhiteSpace($Target)) {
        $arguments += @('--test', $Target)
    }
    $arguments += @($fullName, '--', '--exact')
    $logName = 'named-' + ($Id -replace '[^A-Za-z0-9_.-]', '-') + '.log'
    $run = Invoke-Native -FilePath 'cargo' -Arguments $arguments `
        -WorkingDirectory $RepoRoot -LogName $logName
    $totals = Get-RustTestTotals -Lines $run.Lines
    if ($run.ExitCode -eq 0 -and $totals.Failed -eq 0 -and $totals.Passed -ge 1) {
        $label = $where + ' ' + $fullName
        Add-Check -Id $Id -Status 'PASS' `
            -Detail ("{0}: {1} passed, {2}s" -f $label, $totals.Passed, $run.Seconds)
    }
    elseif ($run.ExitCode -eq 0 -and $totals.Passed -eq 0) {
        Add-Check -Id $Id -Status 'PENDING' `
            -Detail ("{0} matched no test at run time in {1}; no evidence claimed" -f $fullName, $where)
    }
    else {
        Add-Check -Id $Id -Status 'FAIL' `
            -Detail ("{0} exit {1}: {2} passed, {3} failed; see {4}" -f $fullName, $run.ExitCode, $totals.Passed, $totals.Failed, $logName)
        Get-Tail -Lines $run.Lines -Count 20 | ForEach-Object { Write-Host ("       " + $_) }
    }
}

# Run one frontend test file. Vitest exits non-zero when no test matches, so a
# missing file/test is FAIL here; the evidence file is versioned with this tree.
function Add-NamedFrontendTestCheck {
    param(
        [string]$Id,
        [string]$RelativePath
    )
    $logName = 'named-' + ($Id -replace '[^A-Za-z0-9_.-]', '-') + '.log'
    $run = Invoke-Native -FilePath 'npm.cmd' `
        -Arguments @('--prefix', $frontendDir, 'run', 'test', '--', $RelativePath) `
        -WorkingDirectory $RepoRoot -LogName $logName
    $totals = Get-VitestTotals -Lines $run.Lines
    if ($run.ExitCode -eq 0 -and $totals.Seen -and $totals.Failed -eq 0 -and $totals.Passed -gt 0) {
        Add-Check -Id $Id -Status 'PASS' `
            -Detail ("vitest {0}: {1} passed, 0 failed, {2}s" -f $RelativePath, $totals.Passed, $run.Seconds)
    }
    else {
        Add-Check -Id $Id -Status 'FAIL' `
            -Detail ("vitest {0} exit {1}: {2} passed, {3} failed; see {4}" -f $RelativePath, $run.ExitCode, $totals.Passed, $totals.Failed, $logName)
        Get-Tail -Lines $run.Lines -Count 20 | ForEach-Object { Write-Host ("       " + $_) }
    }
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
# 2b. Deterministic mock-host evidence for ROADMAP Phase 8 steps 1-9.
#
# Every check selects an exact test name. A name that no longer exists matches
# zero tests and is reported PENDING, so a renamed/removed test cannot look
# green. The engine matrix lives in
# crates/selora-workflow/tests/phase8_matrix.rs; approval, process-tree,
# resume-shell and history evidence lives in the desktop suite.
# ---------------------------------------------------------------------------
Write-Host ''
Write-Host 'Phase 8 step evidence (deterministic mock host)' -ForegroundColor Cyan

# Step 1 - confirmation payload.
Add-NamedRustTestCheck -Id 'evidence.step1.confirmation' -Crate 'selora-workflow' `
    -Target 'phase8_matrix' -Test 'phase8_step1_confirmation_payload_matches_the_grant'
Add-NamedRustTestCheck -Id 'evidence.step1.command-extraction' -Crate 'selora-workflow' `
    -Test 'command_set_is_extracted_from_literals_only'
Add-NamedRustTestCheck -Id 'evidence.step1.protocol-wire' -Crate 'selora-protocol' `
    -Test 'p8_workflow_command_wire_shapes_are_stable'
Add-NamedFrontendTestCheck -Id 'test.workflow-card' `
    -RelativePath 'src/features/agent/components/WorkflowCard.test.tsx'

# Step 2 - approve -> phase graph + agent dispatch.
Add-NamedRustTestCheck -Id 'evidence.step2.dispatch' -Crate 'selora-workflow' `
    -Target 'phase8_matrix' -Test 'phase8_step2_approval_starts_the_phase_graph_and_agent_dispatch'
Add-NamedRustTestCheck -Id 'evidence.step2.shell-stream' -Crate 'selora-desktop' `
    -Test 'p8_workflow_run_streams_started_then_phase_step_finished'

# Step 3 - approval parks the run; deny continues; approve writes once.
Add-NamedRustTestCheck -Id 'evidence.step3.deny' -Crate 'selora-desktop' `
    -Test 'proposed_write_parks_deny_does_not_write_and_wrong_ids_reject'
Add-NamedRustTestCheck -Id 'evidence.step3.approve' -Crate 'selora-desktop' `
    -Test 'explicit_approve_writes_once_and_rejects_duplicate_and_stale_ids'
Add-NamedRustTestCheck -Id 'evidence.step3.decision' -Crate 'selora-desktop' `
    -Test 'p8_workflow_approval_maps_exact_diff_and_consumes_only_matching_id'

# Step 4 - resume safe / unsafe / partial effects.
Add-NamedRustTestCheck -Id 'evidence.step4.safe-resume' -Crate 'selora-workflow' `
    -Target 'persisted_execution' `
    -Test 'actual_resume_replays_paid_asks_distinct_reads_commands_and_display_events'
Add-NamedRustTestCheck -Id 'evidence.step4.unsafe-resume' -Crate 'selora-workflow' `
    -Target 'resume_preflight' -Test 'default_resume_is_refused_before_any_effect'
Add-NamedRustTestCheck -Id 'evidence.step4.partial-effects' -Crate 'selora-workflow' `
    -Target 'persisted_execution' `
    -Test 'interrupted_ask_redispatches_but_ambiguous_command_never_does'
Add-NamedRustTestCheck -Id 'evidence.step4.shell-consent' -Crate 'selora-desktop' `
    -Test 'p8_workflow_resume_refuses_until_incomplete_asks_are_confirmed'
Add-Check -Id 'evidence.step4.exactly-once-effects' -Status 'PENDING' `
    -Detail 'Resume re-dispatches unfinished asks; tools in the interrupted turn may already have applied effects. Safe/unsafe/partial policy is automated; exactly-once subagent effects are not claimed (STATUS.md gap 2).'

# Step 5 - failing gate drives the fix-loop rounds.
Add-NamedRustTestCheck -Id 'evidence.step5.gate-loop' -Crate 'selora-workflow' `
    -Target 'phase8_matrix' -Test 'phase8_step5_failing_gate_drives_a_visible_fix_loop_round'
Add-NamedRustTestCheck -Id 'evidence.step5.gate-exit-code' -Crate 'selora-workflow' `
    -Test 'confirmed_command_runs_and_exit_code_reaches_the_script'

# Step 6 - stop + process-tree death.
Add-NamedRustTestCheck -Id 'evidence.step6.stop-engine' -Crate 'selora-workflow' `
    -Target 'phase8_matrix' -Test 'phase8_step6_stop_before_the_next_effect_ends_the_run_stopped'
Add-NamedRustTestCheck -Id 'evidence.step6.stop-shell' -Crate 'selora-desktop' `
    -Test 'p8_workflow_stop_flips_the_active_host_cancel'
Add-NamedRustTestCheck -Id 'evidence.step6.process-tree-cancel' -Crate 'selora-desktop' `
    -Test 'world_run_cancel_kills_detached_grandchildren'
Add-NamedRustTestCheck -Id 'evidence.step6.process-tree-timeout' -Crate 'selora-desktop' `
    -Test 'world_run_timeout_kills_detached_grandchildren'
Add-Check -Id 'evidence.step6.job-startup-race' -Status 'PENDING' `
    -Detail 'Job assignment follows CreateProcess; a descendant spawned in that sub-startup window can escape. Kill-after-assignment is test-covered; the race is a documented limit (STATUS.md gap 1).'

# Step 7 - cost/tokens + known per-phase gap.
Add-NamedRustTestCheck -Id 'evidence.step7.usage' -Crate 'selora-workflow' `
    -Target 'phase8_matrix' -Test 'phase8_step7_cost_and_tokens_match_every_ask'
Add-NamedRustTestCheck -Id 'evidence.step7.shell-summary' -Crate 'selora-desktop' `
    -Test 'p8_workflow_completed_run_is_durable_and_resume_never_reexecutes_it'
Add-NamedRustTestCheck -Id 'evidence.step7.host-receipts' -Crate 'selora-desktop' `
    -Test 'reports_artifacts_logs_and_cost_are_collected'
Add-Check -Id 'evidence.step7.per-phase-cost' -Status 'PENDING' `
    -Detail 'Run totals (tokens + USD + cap) are automated; the protocol DTO carries no per-phase USD, so ROADMAP step 7 per-phase cost is not implemented in this tree.'

# Step 8 - journal path/gitignore + completed history.
Add-NamedRustTestCheck -Id 'evidence.step8.journal-path' -Crate 'selora-workflow' `
    -Target 'phase8_matrix' -Test 'phase8_step8_journal_lands_in_the_gitignored_workspace_path'
Add-NamedRustTestCheck -Id 'evidence.step8.history' -Crate 'selora-desktop' `
    -Test 'p8_workflow_completed_run_history_reconstructs_from_the_journal'
Add-NamedRustTestCheck -Id 'evidence.step8.multi-phase-history' -Crate 'selora-desktop' `
    -Test 'p8_workflow_multi_phase_history_keeps_reports_phases_and_artifact_content'

# Step 9 - facade fetch/fs/process denial.
Add-NamedRustTestCheck -Id 'evidence.step9.facade-denial' -Crate 'selora-workflow' `
    -Target 'phase8_matrix' -Test 'phase8_step9_fetch_fs_process_fail_before_any_host_effect'
Add-NamedRustTestCheck -Id 'evidence.step9.ambient-globals' -Crate 'selora-workflow' `
    -Test 'ambient_globals_are_structurally_absent'
Add-NamedRustTestCheck -Id 'evidence.step9.realm-hardening' -Crate 'selora-workflow' `
    -Test 'realm_hardening_removes_the_ambient_names'

# Cross-cut: the confirmed workflow budget cap.
Add-NamedRustTestCheck -Id 'evidence.budget.matrix' -Crate 'selora-workflow' `
    -Target 'phase8_matrix' -Test 'phase8_budget_cap_stops_the_run_between_steps'
Add-NamedRustTestCheck -Id 'evidence.budget.engine' -Crate 'selora-workflow' `
    -Test 'budget_violation_is_checked_between_steps'
Add-NamedRustTestCheck -Id 'evidence.budget.shell' -Crate 'selora-desktop' `
    -Test 'p9_workflow_budget_exhausted_resume_is_refused_before_worker'

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

# Prove the rule with git itself when this checkout is a repository. A textual
# regex alone cannot show that a nested journal path is ignored.
$gitCommand = Get-Command git -ErrorAction SilentlyContinue
if ($null -eq $gitCommand) {
    Add-Check -Id 'journal.git-check-ignore' -Status 'SKIP' -Detail 'git is not on PATH'
}
elseif (-not (Test-Path -LiteralPath (Join-Path $RepoRoot '.git'))) {
    Add-Check -Id 'journal.git-check-ignore' -Status 'SKIP' -Detail 'this checkout has no .git directory'
}
else {
    $probe = '.selora/workflow-runs/probe/journal.jsonl'
    $ignoreRun = Invoke-Native -FilePath 'git' -Arguments @('check-ignore', '-q', '--', $probe) `
        -WorkingDirectory $RepoRoot -LogName 'git-check-ignore.log'
    if ($ignoreRun.ExitCode -eq 0) {
        Add-Check -Id 'journal.git-check-ignore' -Status 'PASS' `
            -Detail ("git check-ignore confirms " + $probe)
    }
    else {
        Add-Check -Id 'journal.git-check-ignore' -Status 'FAIL' `
            -Detail ("git check-ignore exit {0}: {1} is not ignored; see git-check-ignore.log" -f $ignoreRun.ExitCode, $probe)
    }
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
        Requires = @('evidence.step1.confirmation', 'evidence.step1.command-extraction', 'evidence.step1.protocol-wire', 'test.workflow-card')
        Owner    = 'Send "review every Rust file in src/ for unwrap() calls; report the worst five" and read the confirmation card: script body, command set, cost cap.'
    },
    [pscustomobject]@{
        Number   = 2
        Title    = 'Approve -> phase graph, subagent fan-out, live cards'
        Requires = @('evidence.step2.dispatch', 'evidence.step2.shell-stream', 'test.frontend', 'release-exe')
        Owner    = 'Approve, then watch the Workflows panel: phases appear, subagents fan out, step status changes live.'
    },
    [pscustomobject]@{
        Number   = 3
        Title    = 'Subagent edit parks on an approval card; Deny lets the run continue'
        Requires = @('evidence.step3.deny', 'evidence.step3.approve', 'evidence.step3.decision', 'test.frontend')
        Owner    = 'Wait for a write card, click Deny, confirm the run continues and the file did not change.'
    },
    [pscustomobject]@{
        Number   = 4
        Title    = 'Kill the app mid-run; relaunch offers Resume and finishes'
        Requires = @('evidence.step4.safe-resume', 'evidence.step4.unsafe-resume', 'evidence.step4.partial-effects', 'evidence.step4.shell-consent', 'evidence.step4.exactly-once-effects', 'journal.path-constants')
        Owner    = 'Kill selora-desktop.exe mid-run, relaunch, use Resume, confirm the run finishes with earlier asks not re-billed.'
    },
    [pscustomobject]@{
        Number   = 5
        Title    = 'Failing world.run gate shows the fix-loop round in the panel'
        Requires = @('evidence.step5.gate-loop', 'evidence.step5.gate-exit-code', 'test.frontend')
        Owner    = 'Use a gate that fails (for example cargo test on a broken tree) and watch the retry round in the panel.'
    },
    [pscustomobject]@{
        Number   = 6
        Title    = 'Stop mid-run ends the run and kills in-flight children'
        Requires = @('evidence.step6.stop-engine', 'evidence.step6.stop-shell', 'evidence.step6.process-tree-cancel', 'evidence.step6.process-tree-timeout', 'evidence.step6.job-startup-race')
        Owner    = 'Click Stop during bash work; confirm the run ends in seconds and no child process of that command survives (a child spawned before Job assignment is a documented limit on Windows).'
    },
    [pscustomobject]@{
        Number   = 7
        Title    = 'Cost meter/summary reflect every ask (tokens + USD per phase)'
        Requires = @('evidence.step7.usage', 'evidence.step7.shell-summary', 'evidence.step7.host-receipts', 'evidence.step7.per-phase-cost')
        Owner    = 'Compare the run summary tokens/USD with the receipts; confirm the workflow budget cap is visible.'
    },
    [pscustomobject]@{
        Number   = 8
        Title    = 'Journal lands under .selora/workflow-runs/ and never in git'
        Requires = @('journal.path-constants', 'journal.layout-test', 'journal.gitignore', 'journal.git-check-ignore', 'evidence.step8.journal-path', 'evidence.step8.history', 'evidence.step8.multi-phase-history')
        Owner    = 'After a real run, confirm .selora/workflow-runs/<run_id>/journal.jsonl exists and git status stays clean.'
    },
    [pscustomobject]@{
        Number   = 9
        Title    = 'fetch/fs/process attempts fail at the facade (structurally absent)'
        Requires = @('evidence.step9.facade-denial', 'evidence.step9.ambient-globals', 'evidence.step9.realm-hardening', 'test.sandbox-absence')
        Owner    = 'Paste a script that calls fetch/fs/process and confirm the run fails with a facade error, not an effect.'
    }
)

function Get-StepAutomatedStatus {
    param([string[]]$Requires)
    $status = 'PASS'
    foreach ($id in $Requires) {
        $value = Get-CheckStatus -Id $id
        if ($value -eq 'FAIL') { return 'FAIL' }
        if ($value -eq 'PENDING') { $status = 'PENDING' }
        elseif ($value -eq 'SKIP' -and $status -ne 'PENDING') { $status = 'SKIP' }
    }
    return $status
}

$budgetStatus = Get-StepAutomatedStatus -Requires @('evidence.budget.matrix', 'evidence.budget.engine', 'evidence.budget.shell')

Write-Host ''
Write-Host 'ROADMAP Phase 8 acceptance steps' -ForegroundColor Cyan
Write-Host 'OWNER rows are never automated passes. They require a live key, a real workspace, and a visible window.' -ForegroundColor DarkGray
Write-Host ''
$stepFailures = 0
$stepPending = 0
foreach ($step in $steps) {
    $automated = Get-StepAutomatedStatus -Requires $step.Requires
    if ($automated -eq 'FAIL') { $stepFailures += 1 }
    if ($automated -eq 'PENDING') { $stepPending += 1 }
    $color = 'Green'
    if ($automated -eq 'FAIL') { $color = 'Red' }
    if ($automated -eq 'SKIP') { $color = 'Yellow' }
    if ($automated -eq 'PENDING') { $color = 'Magenta' }
    Write-Host ("Step {0}  AUTOMATED: {1}  OWNER: OWNER (UNRUN)  {2}" -f $step.Number, $automated, $step.Title) -ForegroundColor $color
    Write-Host ("        owner: " + $step.Owner) -ForegroundColor DarkGray
}
$budgetColor = 'Green'
if ($budgetStatus -eq 'FAIL') { $budgetColor = 'Red' }
if ($budgetStatus -eq 'PENDING') { $budgetColor = 'Magenta' }
Write-Host ("Cross-cut  AUTOMATED: {0}  OWNER: OWNER (UNRUN)  workflow budget cap stops the run" -f $budgetStatus) -ForegroundColor $budgetColor

# ---------------------------------------------------------------------------
# 6. Summary and exit code.
# ---------------------------------------------------------------------------
$failed = @($checks | Where-Object { $_.Status -eq 'FAIL' })
$passed = @($checks | Where-Object { $_.Status -eq 'PASS' })
$skipped = @($checks | Where-Object { $_.Status -eq 'SKIP' })
$pending = @($checks | Where-Object { $_.Status -eq 'PENDING' })

Write-Host ''
Write-Host 'Summary' -ForegroundColor Cyan
Write-Host ("  automated checks : {0} passed, {1} failed, {2} pending, {3} skipped" -f $passed.Count, $failed.Count, $pending.Count, $skipped.Count)
Write-Host ("  roadmap steps    : {0} with an automated failure, {1} with pending evidence, 9 owner observations UNRUN" -f $stepFailures, $stepPending)
Write-Host ("  logs             : " + $logDir)

if ($failed.Count -gt 0) {
    Write-Host ''
    Write-Host 'RESULT: FAIL - automated acceptance checks failed. Do not mark Phase 8 accepted.' -ForegroundColor Red
    foreach ($check in $failed) {
        Write-Host ("  - " + $check.Id + ": " + $check.Detail) -ForegroundColor Red
    }
    exit 1
}

if ($pending.Count -gt 0) {
    Write-Host ''
    Write-Host 'PENDING evidence (not passes, not failures):' -ForegroundColor Magenta
    foreach ($check in $pending) {
        Write-Host ("  - " + $check.Id + ": " + $check.Detail) -ForegroundColor Magenta
    }
}

Write-Host ''
Write-Host 'RESULT: no automated check failed. Pending evidence and the 9 OWNER steps still block Phase 8 acceptance.' -ForegroundColor Green
exit 0