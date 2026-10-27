<#
.SYNOPSIS
    Headless acceptance helper for ROADMAP.md Phase 9 (Agent Workbench).

.DESCRIPTION
    Runs the Phase 9 checks that do not need a live key, a window, or a human:

      * the deterministic acceptance suite in
        crates/selora-ai/tests/phase9_acceptance.rs (planned diff/write,
        manage_todo substrate, the 80% context budget, turn-scoped
        auto-approve);
      * the existing desktop agent tests for the approval card, auto mode,
        and the read-only Explore manifest;
      * the protocol and frontend agent test slices, when their build
        dependencies are present;
      * source/branch capability probes for the four Phase 9 runtime slices
        (plan, subagent, sessions, background).

    Every roadmap step is printed with four honest columns:

      AUTOMATED  PASS | FAIL | PENDING
      OWNER      OWNER (never a pass)
      BLOCKED ON the branch/API that is still missing, when PENDING

    PENDING means the API is not in this checkout. It is never treated as a
    pass and never changes the exit code. When a runtime branch is merged,
    the probes flip and the script runs the matching headless test filter,
    without editing this file. A merged branch whose API/test is still
    missing is a FAIL: the harness must not silently accept a regression.

    Exit code 0 means every available automated check passed. Exit code 1
    means at least one available automated check failed. Owner observations
    and PENDING rows never change the exit code.

.PARAMETER RepoRoot
    Repository root. Defaults to the parent directory of this script.

.PARAMETER SkipFrontend
    Skip the vitest slice even when node_modules/api are present.

.PARAMETER BuildFrontend
    Run `npm ci` (when node_modules is absent) and `npm run build` before the
    desktop tests. The Tauri test binary embeds frontend/dist, so a clean
    checkout without it cannot compile the desktop tests.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase9-acceptance.ps1

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase9-acceptance.ps1 -BuildFrontend

.NOTES
    PowerShell 5.1 compatible. No `??`, no ternary operator, no PS7-only
    cmdlets. Test logs are written under %TEMP%, never into the repository.
    This helper does not modify product behavior and does not mark Phase 9
    accepted.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = '',
    [switch]$SkipFrontend,
    [switch]$BuildFrontend
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
$logDir = Join-Path $env:TEMP ("selora-phase9-acceptance-" + $stamp)
$null = New-Item -ItemType Directory -Path $logDir -Force

$checks = New-Object System.Collections.ArrayList
$capabilities = New-Object System.Collections.ArrayList

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
    if ($Status -eq 'PENDING') { $color = 'DarkYellow' }
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

function Get-GitValue {
    param([string[]]$Arguments)
    Push-Location -LiteralPath $RepoRoot
    try {
        $value = @(& git @Arguments 2>$null | ForEach-Object { $_.ToString() })
        $code = $LASTEXITCODE
    }
    finally {
        Pop-Location
    }
    if ($code -ne 0) { return $null }
    return ($value -join "`n")
}

function Get-BranchState {
    param([string]$Branch)
    $head = Get-GitValue @('rev-parse', '--verify', '--quiet', ("refs/heads/" + $Branch))
    if ([string]::IsNullOrWhiteSpace($head)) { return 'ABSENT' }
    $merged = Get-GitValue @('merge-base', '--is-ancestor', $Branch, 'HEAD')
    if ($null -ne $merged) { return 'MERGED' }
    # `merge-base --is-ancestor` prints nothing on success, so distinguish a
    # clean exit from the failed call by re-running and inspecting the code.
    Push-Location -LiteralPath $RepoRoot
    try {
        $null = & git merge-base --is-ancestor $Branch HEAD 2>$null
        if ($LASTEXITCODE -eq 0) { return 'MERGED' }
    }
    finally {
        Pop-Location
    }
    return 'UNMERGED'
}

function Test-SourceProbe {
    param($Probe)
    foreach ($entry in $Probe) {
        $full = Join-Path $RepoRoot $entry.Path
        if (-not (Test-Path -LiteralPath $full)) { continue }
        $text = Get-Content -LiteralPath $full -Raw
        if ($text -match $entry.Pattern) { return $true }
    }
    return $false
}

function Test-BranchSourceProbe {
    param([string]$Branch, $Probe)
    foreach ($entry in $Probe) {
        $gitPath = $entry.Path -replace '\\', '/'
        Push-Location -LiteralPath $RepoRoot
        try {
            $text = @(& git show ($Branch + ':' + $gitPath) 2>$null | ForEach-Object { $_.ToString() }) -join "`n"
            $code = $LASTEXITCODE
        }
        finally {
            Pop-Location
        }
        if ($code -eq 0 -and $text -match $entry.Pattern) { return $true }
    }
    return $false
}

function Test-CapabilityFilter {
    param([string]$Filter)
    $ai = Invoke-Native -FilePath 'cargo' `
        -Arguments @('test', '-p', 'selora-ai', '--locked', $Filter) `
        -WorkingDirectory $RepoRoot -LogName ("capability-ai-" + $Filter + ".log")
    $aiTotals = Get-RustTestTotals -Lines $ai.Lines
    $passed = $aiTotals.Passed
    $failed = $aiTotals.Failed

    $desktopDir = Join-Path $RepoRoot 'apps\selora-desktop\frontend\dist'
    if (Test-Path -LiteralPath $desktopDir) {
        $desktop = Invoke-Native -FilePath 'cargo' `
            -Arguments @('test', '-p', 'selora-desktop', '--locked', $Filter) `
            -WorkingDirectory $RepoRoot -LogName ("capability-desktop-" + $Filter + ".log")
        $desktopTotals = Get-RustTestTotals -Lines $desktop.Lines
        $passed += $desktopTotals.Passed
        $failed += $desktopTotals.Failed
        if ($desktop.ExitCode -ne 0) { $failed += 1 }
    }

    return [pscustomobject]@{
        Passed = $passed
        Failed = $failed
    }
}

function Add-CapabilityRow {
    param(
        [string]$Id,
        [string]$Branch,
        $Probe,
        [string]$Filter,
        [string]$Expected
    )
    $state = Get-BranchState -Branch $Branch
    $present = Test-SourceProbe -Probe $Probe
    $branchHasWork = Test-BranchSourceProbe -Branch $Branch -Probe $Probe
    $status = 'PENDING'
    $detail = ''

    if ($present) {
        if ([string]::IsNullOrWhiteSpace($Filter)) {
            $status = 'PASS'
            $detail = 'API present in this checkout'
        }
        else {
            $result = Test-CapabilityFilter -Filter $Filter
            if ($result.Failed -eq 0 -and $result.Passed -gt 0) {
                $status = 'PASS'
                $detail = ("API present; headless filter '{0}': {1} passed" -f $Filter, $result.Passed)
            }
            elseif ($result.Failed -gt 0) {
                $status = 'FAIL'
                $detail = ("API present but headless filter '{0}' failed: {1} passed, {2} failed; see capability logs" -f $Filter, $result.Passed, $result.Failed)
            }
            else {
                $status = 'PENDING'
                $detail = ("API present; no headless test matched '{0}' yet (see {1})" -f $Filter, $Branch)
            }
        }
    }
    elseif ($state -eq 'MERGED' -and $branchHasWork) {
        $status = 'FAIL'
        $detail = ("branch {0} is merged and contains the API, but the worktree does not: {1}" -f $Branch, $Expected)
    }
    elseif ($state -eq 'MERGED') {
        # The branch tip is an ancestor but it never carried this slice (the
        # Phase 9 placeholders all point at the same pre-work commit). That is
        # still PENDING, not a pass and not a regression.
        $status = 'PENDING'
        $detail = ("branch {0} is an ancestor but carries no implementation; expected {1}" -f $Branch, $Expected)
    }
    else {
        $status = 'PENDING'
        $detail = ("branch {0} is {1}; expected {2}" -f $Branch, $state, $Expected)
    }

    $displayState = $state
    if ($state -eq 'MERGED' -and -not $branchHasWork -and -not $present) {
        $displayState = 'NO-WORK'
    }
    if ($state -eq 'UNMERGED' -and $branchHasWork) {
        $displayState = 'UNMERGED+WORK'
    }
    $null = $capabilities.Add([pscustomobject]@{
        Id     = $Id
        Branch = $Branch
        State  = $displayState
        Status = $status
        Detail = $detail
    })
    Add-Check -Id $Id -Status $status -Detail $detail
}

Write-Host ''
Write-Host 'Selora IDE - Phase 9 (Agent Workbench) headless acceptance' -ForegroundColor Cyan
Write-Host ("Repository : " + $RepoRoot)
Write-Host ("Branch     : " + (Get-GitValue @('rev-parse', '--abbrev-ref', 'HEAD')))
Write-Host ("HEAD       : " + (Get-GitValue @('rev-parse', 'HEAD')))
Write-Host ("Started    : " + (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'))
Write-Host ("Logs       : " + $logDir)
Write-Host ''

# ---------------------------------------------------------------------------
# 0. Optional frontend bootstrap. The Tauri test binary embeds frontend/dist.
# ---------------------------------------------------------------------------
$frontendDir = Join-Path $RepoRoot 'apps\selora-desktop\frontend'
$distDir = Join-Path $frontendDir 'dist'
$nodeModules = Join-Path $frontendDir 'node_modules'

if ($BuildFrontend) {
    if (-not (Test-Path -LiteralPath $nodeModules)) {
        Write-Host 'Build: npm ci (frontend)' -ForegroundColor Cyan
        $install = Invoke-Native -FilePath 'npm.cmd' `
            -Arguments @('--prefix', $frontendDir, 'ci', '--no-audit', '--no-fund') `
            -WorkingDirectory $RepoRoot -LogName 'frontend-install.log'
        Write-Host ("       exit {0}, {1}s" -f $install.ExitCode, $install.Seconds)
    }
    Write-Host 'Build: npm run build (frontend)' -ForegroundColor Cyan
    $front = Invoke-Native -FilePath 'npm.cmd' `
        -Arguments @('--prefix', $frontendDir, 'run', 'build') `
        -WorkingDirectory $RepoRoot -LogName 'frontend-build.log'
    Write-Host ("       exit {0}, {1}s" -f $front.ExitCode, $front.Seconds)
    if ($front.ExitCode -ne 0) {
        Get-Tail -Lines $front.Lines -Count 20 | ForEach-Object { Write-Host ("       " + $_) }
    }
}

# ---------------------------------------------------------------------------
# 1. Runtime capability probes.
# ---------------------------------------------------------------------------
Write-Host 'Phase 9 runtime capabilities' -ForegroundColor Cyan

Add-CapabilityRow -Id 'plan.api' -Branch 'selora-p3-plan' -Filter 'plan_mode' `
    -Expected 'enter_plan_mode/exit_plan_mode or AgentModeDto::Plan' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\router.rs'
            Pattern = 'enter_plan_mode|exit_plan_mode|plan_mode'
        },
        [pscustomobject]@{
            Path = 'crates\selora-protocol\src\lib.rs'
            Pattern = '(?s)enum AgentModeDto\s*\{(?:(?!\}).)*?\bPlan\s*,'
        }
    )

Add-CapabilityRow -Id 'todos.authoritative' -Branch 'selora-p3-plan' -Filter 'todo' `
    -Expected 'todo_read/todo_write tools and ChatStateDto.todos' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\router.rs'
            Pattern = 'todo_read|todo_write'
        },
        [pscustomobject]@{
            Path = 'crates\selora-protocol\src\lib.rs'
            Pattern = 'todo_read|todo_write|TodoDto|pub todos'
        }
    )

Add-CapabilityRow -Id 'questions.structured' -Branch 'selora-p3-subagent' -Filter 'question' `
    -Expected 'ask_user_question / decision_prompt on the approval channel' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\router.rs'
            Pattern = 'ask_user_question'
        },
        [pscustomobject]@{
            Path = 'crates\selora-protocol\src\lib.rs'
            Pattern = 'ask_user_question|DecisionPrompt|decision_prompt'
        }
    )

Add-CapabilityRow -Id 'subagents.tool' -Branch 'selora-p3-subagent' -Filter 'subagent' `
    -Expected 'spawn_subagent plus a structural SubagentPolicy' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\subagent.rs'
            Pattern = 'SubagentPolicy'
        },
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\router.rs'
            Pattern = 'spawn_subagent'
        }
    )

Add-CapabilityRow -Id 'sessions.resume' -Branch 'selora-p3-sessions' -Filter 'resume' `
    -Expected '.selora/agent-sessions persistence and a resume path' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\turn.rs'
            Pattern = 'agent-sessions|resume_session|ResumeSession'
        },
        [pscustomobject]@{
            Path = 'apps\selora-desktop\src-tauri\src\sessions.rs'
            Pattern = 'agent-sessions|resume_session|ResumeSession|agent_sessions'
        }
    )

Add-CapabilityRow -Id 'compaction.at80' -Branch 'selora-p3-sessions' -Filter 'compact' `
    -Expected 'a summarize/compaction round at the 80% context budget' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\turn.rs'
            Pattern = 'compaction|summarize_round|summarise_round|compact_round'
        },
        [pscustomobject]@{
            Path = 'apps\selora-desktop\src-tauri\src\sessions.rs'
            Pattern = 'compaction|summarize_round|summarise_round|compact_round'
        }
    )

Add-CapabilityRow -Id 'auto.session-flag' -Branch 'integration' -Filter '' `
    -Expected 'auto_approve_writes on the turn/session path' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\turn.rs'
            Pattern = 'auto_approve_writes'
        },
        [pscustomobject]@{
            Path = 'apps\selora-desktop\src-tauri\src\sessions.rs'
            Pattern = 'AgentModeDto::Auto'
        }
    )

Add-CapabilityRow -Id 'background.completion' -Branch 'selora-p3-background' -Filter 'subagent' `
    -Expected 'SubagentRuntime/SubagentCompleted plus the wait tool' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-protocol\src\lib.rs'
            Pattern = 'SubagentCompleted'
        },
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\subagent.rs'
            Pattern = 'SubagentRuntime|wait_tool_spec'
        }
    )

Add-CapabilityRow -Id 'plan.write-rejection' -Branch 'selora-p3-plan' -Filter 'plan_mode' `
    -Expected 'a router/session plan gate that rejects writes structurally' `
    -Probe @(
        [pscustomobject]@{
            Path = 'crates\selora-ai\src\agent\router.rs'
            Pattern = 'plan_mode|PlanMode|plan_only'
        },
        [pscustomobject]@{
            Path = 'apps\selora-desktop\src-tauri\src\sessions.rs'
            Pattern = 'PlanMode|plan_mode|plan_only'
        }
    )

# ---------------------------------------------------------------------------
# 2. Deterministic acceptance suite (mock gateway / fake hosts, no network).
# ---------------------------------------------------------------------------
Write-Host ''
Write-Host 'Tests' -ForegroundColor Cyan

$suite = Invoke-Native -FilePath 'cargo' `
    -Arguments @('test', '-p', 'selora-ai', '--test', 'phase9_acceptance', '--locked') `
    -WorkingDirectory $RepoRoot -LogName 'test-phase9-suite.log'
$suiteTotals = Get-RustTestTotals -Lines $suite.Lines
if ($suite.ExitCode -eq 0 -and $suiteTotals.Failed -eq 0 -and $suiteTotals.Passed -ge 5) {
    Add-Check -Id 'test.phase9-suite' -Status 'PASS' `
        -Detail ("phase9_acceptance: {0} passed, {1} failed, {2}s" -f $suiteTotals.Passed, $suiteTotals.Failed, $suite.Seconds)
}
else {
    Add-Check -Id 'test.phase9-suite' -Status 'FAIL' `
        -Detail ("phase9_acceptance exit {0}: {1} passed, {2} failed; see test-phase9-suite.log" -f $suite.ExitCode, $suiteTotals.Passed, $suiteTotals.Failed)
    Get-Tail -Lines $suite.Lines -Count 25 | ForEach-Object { Write-Host ("       " + $_) }
}

function Add-RustFilterCheck {
    param(
        [string]$Id,
        [string]$Package,
        [string]$Filter,
        [string]$LogName,
        [string]$Label
    )
    $run = Invoke-Native -FilePath 'cargo' `
        -Arguments @('test', '-p', $Package, '--locked', $Filter) `
        -WorkingDirectory $RepoRoot -LogName $LogName
    $totals = Get-RustTestTotals -Lines $run.Lines
    if ($run.ExitCode -eq 0 -and $totals.Failed -eq 0 -and $totals.Passed -gt 0) {
        Add-Check -Id $Id -Status 'PASS' `
            -Detail ("{0}: {1} passed, {2} failed, {3}s" -f $Label, $totals.Passed, $totals.Failed, $run.Seconds)
    }
    else {
        Add-Check -Id $Id -Status 'FAIL' `
            -Detail ("{0} exit {1}: {2} passed, {3} failed; see {4}" -f $Label, $run.ExitCode, $totals.Passed, $totals.Failed, $LogName)
        Get-Tail -Lines $run.Lines -Count 25 | ForEach-Object { Write-Host ("       " + $_) }
    }
}

Add-RustFilterCheck -Id 'test.step1-plan-diff' -Package 'selora-ai' -Filter 'step1_' `
    -LogName 'test-step1.log' -Label 'step1 write/edit approval fixtures'
Add-RustFilterCheck -Id 'test.step2-todo' -Package 'selora-ai' -Filter 'step2_' `
    -LogName 'test-step2.log' -Label 'step2 manage_todo substrate'
Add-RustFilterCheck -Id 'test.step6-context-budget' -Package 'selora-ai' -Filter 'step6_' `
    -LogName 'test-step6.log' -Label 'step6 80% context budget substrate'
Add-RustFilterCheck -Id 'test.step7-auto-reset' -Package 'selora-ai' -Filter 'step7_' `
    -LogName 'test-step7.log' -Label 'step7 turn-scoped auto-approve'

Add-RustFilterCheck -Id 'test.protocol-agent' -Package 'selora-protocol' -Filter 'agent' `
    -LogName 'test-protocol-agent.log' -Label 'protocol agent wire defaults'

if (Test-Path -LiteralPath $distDir) {
    Add-RustFilterCheck -Id 'test.desktop-write-gate' -Package 'selora-desktop' `
        -Filter 'agent_turn_streams_tools_approvals_and_completes' `
        -LogName 'test-desktop-write-gate.log' -Label 'desktop agent approval vertical'
    Add-RustFilterCheck -Id 'test.desktop-auto' -Package 'selora-desktop' `
        -Filter 'auto_mode_applies_file_writes_without_an_approval_card' `
        -LogName 'test-desktop-auto.log' -Label 'desktop auto mode'
    Add-RustFilterCheck -Id 'test.desktop-explore' -Package 'selora-desktop' `
        -Filter 'explore_mode_arms_the_read_only_manifest' `
        -LogName 'test-desktop-explore.log' -Label 'desktop Explore read-only manifest'
}
else {
    Add-Check -Id 'test.desktop-write-gate' -Status 'SKIP' `
        -Detail 'frontend/dist missing; run with -BuildFrontend to compile the desktop test binary'
    Add-Check -Id 'test.desktop-auto' -Status 'SKIP' `
        -Detail 'frontend/dist missing; run with -BuildFrontend'
    Add-Check -Id 'test.desktop-explore' -Status 'SKIP' `
        -Detail 'frontend/dist missing; run with -BuildFrontend'
}

if (-not $SkipFrontend -and (Test-Path -LiteralPath $nodeModules)) {
    $frontend = Invoke-Native -FilePath 'npm.cmd' `
        -Arguments @('--prefix', $frontendDir, 'run', 'test', '--', 'src/features/agent') `
        -WorkingDirectory $RepoRoot -LogName 'test-frontend-agent.log'
    $frontendTotals = Get-VitestTotals -Lines $frontend.Lines
    if ($frontend.ExitCode -eq 0 -and $frontendTotals.Seen -and $frontendTotals.Failed -eq 0 -and $frontendTotals.Passed -gt 0) {
        Add-Check -Id 'test.frontend-agent' -Status 'PASS' `
            -Detail ("vitest src/features/agent: {0} passed, {1} failed, {2}s" -f $frontendTotals.Passed, $frontendTotals.Failed, $frontend.Seconds)
    }
    else {
        Add-Check -Id 'test.frontend-agent' -Status 'FAIL' `
            -Detail ("vitest src/features/agent exit {0}: {1} passed, {2} failed; see test-frontend-agent.log" -f $frontend.ExitCode, $frontendTotals.Passed, $frontendTotals.Failed)
        Get-Tail -Lines $frontend.Lines -Count 25 | ForEach-Object { Write-Host ("       " + $_) }
    }
}
else {
    Add-Check -Id 'test.frontend-agent' -Status 'SKIP' `
        -Detail 'frontend/node_modules absent (or -SkipFrontend); UI slice not run'
}

# ---------------------------------------------------------------------------
# 3. The .selora/ agent-session path contract (the 9d target).
# ---------------------------------------------------------------------------
$gitignorePath = Join-Path $RepoRoot '.gitignore'
$ignoredOk = $false
if (Test-Path -LiteralPath $gitignorePath) {
    $ignoreText = Get-Content -LiteralPath $gitignorePath -Raw
    if ($ignoreText -match '(?m)^/?\.selora/?\s*$') { $ignoredOk = $true }
}
if ($ignoredOk) {
    Add-Check -Id 'sessions.gitignore' -Status 'PASS' -Detail '.gitignore already ignores /.selora/ (agent-sessions will inherit it)'
}
else {
    Add-Check -Id 'sessions.gitignore' -Status 'FAIL' -Detail '.gitignore does not ignore .selora/'
}

$agentSessions = Join-Path $RepoRoot '.selora\agent-sessions'
if (Test-Path -LiteralPath $agentSessions) {
    $sessionCount = @(Get-ChildItem -LiteralPath $agentSessions -Directory -ErrorAction SilentlyContinue).Count
    Add-Check -Id 'sessions.workspace' -Status 'PASS' `
        -Detail ("local agent sessions present: {0} ({1})" -f $sessionCount, $agentSessions)
}
else {
    Add-Check -Id 'sessions.workspace' -Status 'PENDING' `
        -Detail 'no .selora/agent-sessions yet; session persistence is the 9d slice'
}

# ---------------------------------------------------------------------------
# 4. ROADMAP Phase 9 acceptance steps.
# ---------------------------------------------------------------------------
$steps = @(
    [pscustomobject]@{
        Number   = 1
        Title    = 'Plan -> approve plan -> diff card -> approve -> write (Ctrl+Z in owner run)'
        Requires = @('test.step1-plan-diff', 'test.desktop-write-gate', 'plan.api')
        Owner    = 'New Agent session, ask for a fix, approve the plan card, approve the diff card, confirm the file changed and Ctrl+Z reverts it.'
    },
    [pscustomobject]@{
        Number   = 2
        Title    = 'Live todo list updates as steps complete'
        Requires = @('test.step2-todo', 'todos.authoritative')
        Owner    = 'Watch the todo spine update while a run executes; confirm it survives a session switch.'
    },
    [pscustomobject]@{
        Number   = 3
        Title    = 'Structured question parks the turn and answering resumes it'
        Requires = @('questions.structured')
        Owner    = 'Answer an escalation card (option and free text), confirm the same run continues with the answer.'
    },
    [pscustomobject]@{
        Number   = 4
        Title    = 'Two read-only subagents run and group in the transcript'
        Requires = @('subagents.tool')
        Owner    = 'Ask for two read-only reviewers; confirm both run, group under the parent, and neither can write.'
    },
    [pscustomobject]@{
        Number   = 5
        Title    = 'Close/relaunch resumes the session with its todos'
        Requires = @('sessions.resume', 'todos.authoritative', 'sessions.gitignore')
        Owner    = 'Kill the app mid-run, relaunch, pick Resume, confirm the transcript and todo list return and the run continues.'
    },
    [pscustomobject]@{
        Number   = 6
        Title    = 'A long session compacts at ~80% of the context budget'
        Requires = @('test.step6-context-budget', 'compaction.at80')
        Owner    = 'Drive a session past 80%, confirm one summarize round and a coherent continuation (not a hard truncation).'
    },
    [pscustomobject]@{
        Number   = 7
        Title    = 'Session auto-approve resets on restart (never persisted)'
        Requires = @('test.step7-auto-reset', 'test.desktop-auto', 'auto.session-flag')
        Owner    = 'Enable auto-approve, confirm writes skip cards, kill the app, relaunch, confirm the flag is off and the card returns.'
    },
    [pscustomobject]@{
        Number   = 8
        Title    = 'Background subagent completion notifies and exposes its result'
        Requires = @('background.completion')
        Owner    = 'While editing, let a background child finish; click the toast and confirm its result is in the parent session.'
    },
    [pscustomobject]@{
        Number   = 9
        Title    = 'Plan mode rejects writes structurally'
        Requires = @('plan.write-rejection', 'test.desktop-explore')
        Owner    = 'In Plan mode ask for a write; confirm a plan-first tool error appears and no diff/write card is offered.'
    }
)

function Get-StepAutomatedStatus {
    param([string[]]$Requires)
    $status = 'PASS'
    foreach ($id in $Requires) {
        $value = Get-CheckStatus -Id $id
        if ($value -eq 'FAIL') { return 'FAIL' }
        if ($value -eq 'PENDING' -or $value -eq 'SKIP') { $status = 'PENDING' }
    }
    return $status
}

Write-Host ''
Write-Host 'ROADMAP Phase 9 acceptance steps' -ForegroundColor Cyan
Write-Host 'OWNER rows are never automated passes. They need a live key, a real workspace, and a visible window.' -ForegroundColor DarkGray
Write-Host 'PENDING means the runtime branch/API is not in this checkout; it is not a pass.' -ForegroundColor DarkGray
Write-Host ''

$stepFailures = 0
$stepPending = 0
$stepPasses = 0
foreach ($step in $steps) {
    $automated = Get-StepAutomatedStatus -Requires $step.Requires
    if ($automated -eq 'FAIL') { $stepFailures += 1 }
    if ($automated -eq 'PENDING') { $stepPending += 1 }
    if ($automated -eq 'PASS') { $stepPasses += 1 }
    $color = 'Green'
    if ($automated -eq 'FAIL') { $color = 'Red' }
    if ($automated -eq 'PENDING') { $color = 'Yellow' }
    Write-Host ("Step {0}  AUTOMATED: {1}  OWNER: OWNER  {2}" -f $step.Number, $automated, $step.Title) -ForegroundColor $color
    $blocked = @()
    foreach ($id in $step.Requires) {
        if ((Get-CheckStatus -Id $id) -eq 'PENDING') { $blocked += $id }
    }
    if ($blocked.Count -gt 0) {
        Write-Host ("        PENDING on: " + ($blocked -join ', ')) -ForegroundColor DarkYellow
    }
    Write-Host ("        owner: " + $step.Owner) -ForegroundColor DarkGray
}

# ---------------------------------------------------------------------------
# 5. Summary and exit code.
# ---------------------------------------------------------------------------
$failed = @($checks | Where-Object { $_.Status -eq 'FAIL' })
$passed = @($checks | Where-Object { $_.Status -eq 'PASS' })
$skipped = @($checks | Where-Object { $_.Status -eq 'SKIP' })
$pending = @($checks | Where-Object { $_.Status -eq 'PENDING' })

Write-Host ''
Write-Host 'Summary' -ForegroundColor Cyan
Write-Host ("  automated checks : {0} passed, {1} failed, {2} skipped, {3} pending" -f $passed.Count, $failed.Count, $skipped.Count, $pending.Count)
Write-Host ("  roadmap steps    : {0} automated PASS, {1} FAIL, {2} PENDING; 9 owner observations unrun" -f $stepPasses, $stepFailures, $stepPending)
Write-Host ("  logs             : " + $logDir)
Write-Host ''
Write-Host 'Runtime branches' -ForegroundColor Cyan
foreach ($capability in $capabilities) {
    Write-Host ("  {0,-22} branch {1,-22} {2,-9} {3}" -f $capability.Id, $capability.Branch, $capability.State, $capability.Status) -ForegroundColor DarkGray
}

if ($failed.Count -gt 0) {
    Write-Host ''
    Write-Host 'RESULT: FAIL - an available automated check failed. Do not mark Phase 9 accepted.' -ForegroundColor Red
    foreach ($check in $failed) {
        Write-Host ("  - " + $check.Id + ": " + $check.Detail) -ForegroundColor Red
    }
    exit 1
}

Write-Host ''
if ($pending.Count -gt 0) {
    Write-Host 'RESULT: available automated checks passed. Phase 9 is NOT accepted: PENDING runtime slices and ALL 9 owner observations remain.' -ForegroundColor Yellow
}
else {
    Write-Host 'RESULT: available automated checks passed. Phase 9 is NOT accepted until the OWNER observations run.' -ForegroundColor Yellow
}
exit 0