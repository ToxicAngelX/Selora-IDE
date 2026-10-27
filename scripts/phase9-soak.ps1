<#
.SYNOPSIS
    Phase 9 crash/soak runner.

.DESCRIPTION
    Runs the deterministic crash/soak tests added for Phase 9 and prints the
    PASS / FAIL / PENDING / OWNER matrix documented in docs/PHASE9_SOAK.md.

    The Rust tests use scripted gateways and real process-kill harnesses. They
    do not need a key, a gateway, or a visible window. Frontend coverage runs
    only when apps/selora-desktop/frontend/node_modules exists; otherwise that
    row is PENDING (run `npm install` in the frontend directory to enable it).

    PENDING rows are features whose APIs are not on `integration` yet. They are
    never reported as passes and never change the exit code; the owning branch
    listed in docs/PHASE9_SOAK.md is responsible for turning them into tests.

.PARAMETER RepoRoot
    Repository root. Defaults to the parent directory of this script.

.PARAMETER Full
    Also run the existing durable-replay suite and the full agent-store test
    file. More coverage, more time.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase9-soak.ps1

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase9-soak.ps1 -Full

.NOTES
    PowerShell 5.1 compatible. Test logs go under %TEMP%, never the repo.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = '',
    [switch]$Full
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
$logDir = Join-Path $env:TEMP ("selora-phase9-soak-" + $stamp)
$null = New-Item -ItemType Directory -Path $logDir -Force

$checks = New-Object System.Collections.ArrayList

function Add-Check {
    param(
        [string]$Id,
        [ValidateSet('PASS', 'FAIL', 'SKIP')]
        [string]$Status,
        [string]$Detail
    )
    $null = $checks.Add([pscustomobject]@{ Id = $Id; Status = $Status; Detail = $Detail })
    $color = 'Gray'
    if ($Status -eq 'PASS') { $color = 'Green' }
    if ($Status -eq 'FAIL') { $color = 'Red' }
    if ($Status -eq 'SKIP') { $color = 'Yellow' }
    Write-Host ("[{0}] {1} - {2}" -f $Status, $Id, $Detail) -ForegroundColor $color
}

function Invoke-Logged {
    param(
        [string]$Name,
        [scriptblock]$Command
    )
    $log = Join-Path $logDir ($Name + '.log')
    Write-Host ("    running {0} (log: {1})" -f $Name, $log) -ForegroundColor DarkGray
    & $Command 2>&1 | Out-File -FilePath $log -Encoding utf8
    return $LASTEXITCODE
}

function Test-RustSuite {
    param(
        [string]$Id,
        [string]$Package,
        [string]$TestName,
        [string]$Detail
    )
    $code = Invoke-Logged -Name $Id -Command {
        & cargo test -p $Package --test $TestName -- --test-threads=1
    }
    if ($code -eq 0) {
        Add-Check -Id $Id -Status PASS -Detail $Detail
    } else {
        Add-Check -Id $Id -Status FAIL -Detail ("{0}; see {1}" -f $Detail, $logDir)
    }
}

Write-Host ''
Write-Host 'Phase 9 crash/soak' -ForegroundColor Cyan
Write-Host '=================' -ForegroundColor Cyan

Push-Location $RepoRoot
try {
    Test-RustSuite `
        -Id 'agent-turn-crash' `
        -Package 'selora-ai' `
        -TestName 'phase9_agent_crash' `
        -Detail 'stream/park/execute kills, stale decisions, event order, auto-approve locality'

    Test-RustSuite `
        -Id 'subagent-soak' `
        -Package 'selora-workflow' `
        -TestName 'phase9_subagent_soak' `
        -Detail 'same-agent FIFO, concurrency cap, failure/panic rollback, torn-tail soak'

    Test-RustSuite `
        -Id 'workflow-process-crash' `
        -Package 'selora-workflow' `
        -TestName 'phase9_workflow_crash' `
        -Detail 'real child killed mid-ask; restart dispatches once and accounts once'

    if ($Full) {
        Test-RustSuite `
            -Id 'durable-replay-regression' `
            -Package 'selora-workflow' `
            -TestName 'persisted_execution' `
            -Detail 'existing durable replay suite'
    } else {
        Add-Check -Id 'durable-replay-regression' -Status SKIP -Detail 'use -Full'
    }

    $frontend = Join-Path $RepoRoot 'apps/selora-desktop/frontend'
    $agentsTest = 'src/features/agent/agent-store.test.ts'
    if (-not (Test-Path -LiteralPath (Join-Path $frontend 'node_modules'))) {
        Add-Check -Id 'auto-approve-restart' -Status SKIP -Detail 'frontend node_modules missing; run npm install (row stays PENDING)'
    } else {
        $npm = Get-Command 'npm.cmd' -ErrorAction SilentlyContinue
        if ($null -eq $npm) {
            Add-Check -Id 'auto-approve-restart' -Status SKIP -Detail 'npm.cmd not on PATH (row stays PENDING)'
        } else {
            Push-Location $frontend
            try {
                $code = Invoke-Logged -Name 'auto-approve-restart' -Command {
                    & $npm.Source test -- --run $agentsTest
                }
                if ($code -eq 0) {
                    Add-Check -Id 'auto-approve-restart' -Status PASS -Detail 'restart/reset/new session return to ask_before_change'
                } else {
                    Add-Check -Id 'auto-approve-restart' -Status FAIL -Detail ("agent-store.test.ts failed; see {0}" -f $logDir)
                }
            } finally {
                Pop-Location
            }
        }
    }
} finally {
    Pop-Location
}

Write-Host ''
Write-Host 'PENDING (no API on integration; owner branch must add the test)' -ForegroundColor Yellow
Write-Host '-----------------------------------------------------------------' -ForegroundColor Yellow
$pending = @(
    'background subagent kill / completion notification  -> Phase 9c/9e (selora-p3-background)',
    'parked subagent child cancellation                  -> Phase 9c (selora-p3-subagent)',
    'parent cancellation of a cooperative subagent      -> Phase 9c',
    'late subagent result after cancellation            -> Phase 9c',
    'question-card park / kill                          -> Phase 9c (ask_user_question)',
    'plan-card park / kill                              -> Phase 9b (enter/exit_plan_mode)',
    'session JSONL torn tail / restart                  -> Phase 9d',
    'compaction twice / summary failure / accounting    -> Phase 9d'
)
foreach ($row in $pending) {
    Write-Host ("[PENDING] {0}" -f $row) -ForegroundColor Yellow
}
Write-Host ''
Write-Host ("Test logs: {0}" -f $logDir) -ForegroundColor DarkGray
Write-Host 'Matrix: docs/PHASE9_SOAK.md' -ForegroundColor DarkGray

$failed = @($checks | Where-Object { $_.Status -eq 'FAIL' })
if ($failed.Count -gt 0) {
    Write-Host ''
    Write-Host ("FAIL: {0} automated check(s) failed." -f $failed.Count) -ForegroundColor Red
    exit 1
}
Write-Host ''
Write-Host 'PASS: every automated check passed; PENDING rows remain owned by later branches.' -ForegroundColor Green
exit 0