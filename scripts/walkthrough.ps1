<#
.SYNOPSIS
    Automated functional walkthrough for the Selora IDE desktop app.

.DESCRIPTION
    Launches a real Selora build (release or dev), attaches to its WebView2
    DevTools endpoint, and drives the live UI over CDP:

      * boot + bootstrap state
      * first-run overlay dismissal
      * boot workspace + opening another folder
      * opening a file from the explorer
      * typing into CodeMirror and saving with Ctrl+S (file verified on disk)
      * the dirty-tab close guard (Save / Discard / Cancel)
      * opening, switching, and closing tabs
      * a real settings write, verified in the UI, the bootstrap, and settings.json
      * the Workflows panel and Agent composer smoke checks
      * a renderer-exception gate

    Every screenshot, log, and the machine-readable result land in an artifact
    directory. The app runs with an isolated settings directory and a temporary
    workspace, so the walkthrough never edits the user's settings or files.

    This script never claims human acceptance. Visual judgement, real-gateway
    agent/workflow runs, and OS-level interactions stay on the human checklist
    in docs/WALKTHROUGH.md.

.PARAMETER Mode
    release (default) uses target\release\selora-desktop.exe with the embedded
    frontend. dev uses the Vite dev server on port 1420 plus target\debug,
    matching `npm run dev` + `cargo run` work. dev starts and stops its own Vite
    process.

.PARAMETER Port
    CDP port. Default 9333. Must be free; the script refuses to attach to an
    unknown process.

.PARAMETER ArtifactsDir
    Where screenshots, logs, and summary files are written. Default
    %TEMP%\selora-walkthrough-<stamp>. The directory is created if missing and
    never cleaned automatically.

.PARAMETER Workspace
    Workspace folder for the run. Default: a generated fixture with hello.ts,
    notes.md, and src\util.ts. The walkthrough opens and restores this folder.

.PARAMETER WorkspaceB
    Second folder used for the Open Folder switch check. Default: a generated
    fixture with second-workspace.txt.

.PARAMETER Build
    Build the frontend and the binary before launching. Without it the script
    only uses an existing artifact and fails with the exact build commands when
    the artifact is missing.

.PARAMETER KeepRunning
    Leave the app running after the walkthrough. The default kills only the
    process tree this script started.

.PARAMETER FailOnFindings
    Treat recorded product findings (behaviour the walkthrough observed but
    worked around, for example the Agent activity button no-op) as a failed
    run. Without this switch findings are reported prominently but do not
    change the exit code.

.PARAMETER TimeoutSec
    Overall budget for the CDP driver. Default 900.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\walkthrough.ps1

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\walkthrough.ps1 -Mode dev -Build

.NOTES
    PowerShell 5.1 compatible. Exit codes:
      0 = every automated step passed
      1 = a walkthrough step failed
      2 = environment/launch/attach failure (nothing was driven)

    Safety: refuses to start when another selora-desktop process is running
    (the Tauri single-instance plugin would forward to it). Kills only the
    process tree it started. The app gets SELORA_SETTINGS_DIR pointing inside
    the artifact directory, so settings/layout writes are isolated. No API key
    is read from or written to this script, and the walkthrough never sends a
    gateway request.
#>
[CmdletBinding()]
param(
    [ValidateSet('release', 'dev')]
    [string]$Mode = 'release',
    [int]$Port = 9333,
    [string]$ArtifactsDir = '',
    [string]$Workspace = '',
    [string]$WorkspaceB = '',
    [switch]$Build,
    [switch]$KeepRunning,
    [switch]$FailOnFindings,
    [int]$TimeoutSec = 900
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$RepoRoot = Split-Path -Parent $PSScriptRoot
$Driver = Join-Path $PSScriptRoot 'walkthrough\cdp-walkthrough.mjs'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$startedAt = Get-Date

if ([string]::IsNullOrWhiteSpace($ArtifactsDir)) {
    $ArtifactsDir = Join-Path $env:TEMP ("selora-walkthrough-" + $stamp)
}
$ArtifactsDir = [System.IO.Path]::GetFullPath($ArtifactsDir)
$null = New-Item -ItemType Directory -Path $ArtifactsDir -Force

$summaryJson = Join-Path $ArtifactsDir 'summary.json'
$summaryMd = Join-Path $ArtifactsDir 'summary.md'
$driverLog = Join-Path $ArtifactsDir 'cdp-driver.log'
$buildLog = Join-Path $ArtifactsDir 'build.log'
$settingsDir = Join-Path $ArtifactsDir 'settings'
$null = New-Item -ItemType Directory -Path $settingsDir -Force
$settingsFile = Join-Path $settingsDir 'settings.json'

$script:appProcess = $null
$script:outTask = $null
$script:errTask = $null
$script:viteProcess = $null
$script:driverExit = -1
$script:failure = $null
$script:result = $null
$script:exePath = $null
$script:cdpReady = $false

function Write-Phase {
    param([string]$Message)
    Write-Host ''
    Write-Host ("== " + $Message) -ForegroundColor Cyan
}

function Fail-Launch {
    param([string]$Message)
    Write-Host ("FAIL " + $Message) -ForegroundColor Red
    $script:failure = $Message
    throw (New-Object System.Exception($Message))
}

function Write-TextFile {
    param([string]$Path, [string]$Text)
    $parent = Split-Path -Parent $Path
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        $null = New-Item -ItemType Directory -Path $parent -Force
    }
    [System.IO.File]::WriteAllText($Path, $Text, (New-Object System.Text.UTF8Encoding($false)))
}

function New-Fixtures {
    if ([string]::IsNullOrWhiteSpace($Workspace)) {
        $script:Workspace = Join-Path $ArtifactsDir 'fixture-main'
    }
    if (-not (Test-Path -LiteralPath $script:Workspace)) {
        $null = New-Item -ItemType Directory -Path $script:Workspace -Force
    }
    if ([string]::IsNullOrWhiteSpace($WorkspaceB)) {
        $script:WorkspaceB = Join-Path $ArtifactsDir 'fixture-second'
    }
    if (-not (Test-Path -LiteralPath $script:WorkspaceB)) {
        $null = New-Item -ItemType Directory -Path $script:WorkspaceB -Force
    }
    $script:Workspace = (Resolve-Path -LiteralPath $script:Workspace).Path
    $script:WorkspaceB = (Resolve-Path -LiteralPath $script:WorkspaceB).Path

    # Only seed generated fixtures, never a caller-provided workspace.
    if ($script:Workspace -like (Join-Path $ArtifactsDir 'fixture-main*')) {
        Write-TextFile (Join-Path $script:Workspace 'hello.ts') @'
export const greeting = 'hello from walkthrough';

export function greet(name: string): string {
  return greeting + ', ' + name;
}
'@
        Write-TextFile (Join-Path $script:Workspace 'notes.md') @'
# Walkthrough notes

- automated functional walkthrough
- no gateway request is sent
'@
        Write-TextFile (Join-Path $script:Workspace 'src\util.ts') @'
export function add(left: number, right: number): number {
  return left + right;
}
'@
    }
    if ($WorkspaceB -like (Join-Path $ArtifactsDir 'fixture-second*')) {
        Write-TextFile (Join-Path $script:WorkspaceB 'second-workspace.txt') "opened by the walkthrough Open Folder check`r`n"
    }
}

function Assert-Preconditions {
    if (-not (Test-Path -LiteralPath $Driver)) {
        Fail-Launch ("CDP driver not found: " + $Driver)
    }
    $node = Get-Command node -ErrorAction SilentlyContinue
    if ($null -eq $node) {
        Fail-Launch 'node is not on PATH; the walkthrough driver needs Node 22 or newer'
    }
    $existing = @(Get-Process -Name 'selora-desktop' -ErrorAction SilentlyContinue)
    if ($existing.Count -gt 0) {
        $ids = ($existing | ForEach-Object { $_.Id }) -join ', '
        Fail-Launch ("selora-desktop is already running (pid " + $ids + "). Close it first: the Tauri single-instance plugin would forward the new launch to that process, which has no CDP port.")
    }
    $listener = $null
    try {
        $listener = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
    }
    catch {
        $listener = $null
    }
    if ($null -ne $listener) {
        Fail-Launch ("CDP port " + $Port + " is already in use. Pick another with -Port.")
    }
    if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot 'Cargo.toml'))) {
        Fail-Launch ("repository root not found at " + $RepoRoot)
    }
    if ($Mode -eq 'dev') {
        $viteListener = $null
        try {
            $viteListener = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
        }
        catch {
            $viteListener = $null
        }
        if ($null -ne $viteListener) {
            $owner = ''
            try {
                $ownerProcess = Get-CimInstance Win32_Process -Filter ("ProcessId=" + $viteListener.OwningProcess) -ErrorAction SilentlyContinue
                if ($null -ne $ownerProcess) { $owner = [string]$ownerProcess.CommandLine }
            }
            catch {
                $owner = ''
            }
            Fail-Launch ("Vite port 1420 is already in use by pid " + $viteListener.OwningProcess + ". " + $owner + "`nThe dev walkthrough will not attach to or kill another project's dev server. Stop that server, or use release mode. (The debug binary has its devUrl compiled as http://localhost:1420.)")
        }
    }
}

function Invoke-BuildStep {
    param([string]$Label, [string]$FilePath, [string[]]$Arguments)
    Write-Host ("build: " + $Label) -ForegroundColor DarkGray
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $lines = @()
    $code = -1
    Push-Location -LiteralPath $RepoRoot
    $previousPreference = $ErrorActionPreference
    try {
        # Cargo and npm write normal progress to stderr. Under the script-wide
        # 'Stop' preference PowerShell 5.1 turns that into a terminating
        # NativeCommandError, which would hide the real exit code and the log.
        $ErrorActionPreference = 'Continue'
        $lines = @(& $FilePath @Arguments 2>&1 | ForEach-Object { $_.ToString() })
        $code = $LASTEXITCODE
    }
    catch {
        $lines = @($_.Exception.Message)
        $code = -1
    }
    finally {
        $ErrorActionPreference = $previousPreference
        Pop-Location
        $watch.Stop()
    }
    $block = @("### " + $Label + " (exit " + $code + ", " + [math]::Round($watch.Elapsed.TotalSeconds, 1) + " s)") + $lines + @('')
    Add-Content -LiteralPath $buildLog -Value $block -Encoding UTF8
    $lines | Select-Object -Last 12 | ForEach-Object { Write-Host ("  " + $_) -ForegroundColor DarkGray }
    if ($code -ne 0) {
        Fail-Launch ("build step failed: " + $Label + " (exit " + $code + "). See " + $buildLog)
    }
}

function Resolve-Exe {
    $targetDir = 'target'
    if (-not [string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        $targetDir = $env:CARGO_TARGET_DIR
    }
    if (-not [System.IO.Path]::IsPathRooted($targetDir)) {
        $targetDir = Join-Path $RepoRoot $targetDir
    }
    if ($Mode -eq 'release') {
        $script:exePath = Join-Path $targetDir 'release\selora-desktop.exe'
    }
    else {
        $script:exePath = Join-Path $targetDir 'debug\selora-desktop.exe'
    }
    return $script:exePath
}

function Install-FrontendDeps {
    $nodeModules = Join-Path $RepoRoot 'apps\selora-desktop\frontend\node_modules'
    if (-not (Test-Path -LiteralPath $nodeModules)) {
        Invoke-BuildStep 'npm ci (frontend dependencies)' 'cmd.exe' @('/c', 'npm.cmd --prefix apps\selora-desktop\frontend ci --no-audit --no-fund')
    }
}

function Start-Vite {
    Write-Phase 'Starting Vite dev server on 127.0.0.1:1420'
    $viteLog = Join-Path $ArtifactsDir 'vite-dev.log'
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = 'cmd.exe'
    $psi.Arguments = '/c npm.cmd --prefix apps\selora-desktop\frontend run dev > "' + $viteLog + '" 2>&1'
    $psi.WorkingDirectory = $RepoRoot
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    $null = $proc.Start()
    $script:viteProcess = $proc

    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline) {
        if ($proc.HasExited) {
            $tail = ''
            if (Test-Path -LiteralPath $viteLog) { $tail = (Get-Content -LiteralPath $viteLog -Tail 20) -join ' ' }
            Fail-Launch ('Vite exited early (exit ' + $proc.ExitCode + '): ' + $tail)
        }
        try {
            $response = Invoke-WebRequest -Uri 'http://127.0.0.1:1420/' -UseBasicParsing -TimeoutSec 2
            if ($response.StatusCode -eq 200) {
                Write-Host 'Vite ready' -ForegroundColor DarkGray
                return
            }
        }
        catch {
            Start-Sleep -Milliseconds 500
        }
    }
    Fail-Launch ('Vite did not answer on http://127.0.0.1:1420 within 60 s; see ' + $viteLog)
}

function Start-App {
    Write-Phase ("Launching " + $Mode + " app: " + $script:exePath)
    if (-not (Test-Path -LiteralPath $script:exePath)) {
        if ($Mode -eq 'release') {
            Fail-Launch ("release binary missing: " + $script:exePath + ". Build it with:`n  npm.cmd --prefix apps\selora-desktop\frontend run build`n  cargo build --release -p selora-desktop`nor rerun with -Build.")
        }
        Fail-Launch ("debug binary missing: " + $script:exePath + ". Rerun with -Build.")
    }

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $script:exePath
    $psi.WorkingDirectory = $RepoRoot
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.EnvironmentVariables['SELORA_WORKSPACE_ROOT'] = $Workspace
    $psi.EnvironmentVariables['SELORA_SETTINGS_DIR'] = $settingsDir
    $psi.EnvironmentVariables['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = ('--remote-debugging-port=' + $Port + ' --remote-allow-origins=*')
    $psi.EnvironmentVariables['RUST_BACKTRACE'] = '1'

    # WebView2 keeps its profile locked briefly after the browser process dies
    # (HRESULT 0x800700AA "requested resource is in use"). Retry a clean start
    # a few times instead of reporting a false launch failure.
    $attempts = 3
    for ($attempt = 1; $attempt -le $attempts; $attempt++) {
        $proc = New-Object System.Diagnostics.Process
        $proc.StartInfo = $psi
        $null = $proc.Start()
        $script:appProcess = $proc
        $script:outTask = $proc.StandardOutput.ReadToEndAsync()
        $script:errTask = $proc.StandardError.ReadToEndAsync()
        Write-Host ("pid " + $proc.Id + " (attempt " + $attempt + "/" + $attempts + ")") -ForegroundColor DarkGray

        Start-Sleep -Seconds 2
        if (-not $proc.HasExited) { break }

        $out = ''
        $err = ''
        try { $out = $script:outTask.Result } catch { }
        try { $err = $script:errTask.Result } catch { }
        $attemptLog = Join-Path $ArtifactsDir ('app-launch-attempt-' + $attempt + '.log')
        Write-TextFile $attemptLog ("exit " + $proc.ExitCode + "`r`n" + $err)
        if ($attempt -lt $attempts) {
            Write-Host ("startup exit " + $proc.ExitCode + "; retrying after WebView2 profile release") -ForegroundColor Yellow
            Start-Sleep -Seconds (3 * $attempt)
            continue
        }
        Fail-Launch ("selora-desktop exited during startup on all " + $attempts + " attempts (last exit " + $proc.ExitCode + "). See " + $attemptLog + " and app-panic.log.")
    }

    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline) {
        if ($proc.HasExited) {
            Fail-Launch ("selora-desktop exited before CDP attached (exit " + $proc.ExitCode + "). Check app-boot.log.")
        }
        try {
            $targets = Invoke-RestMethod -Uri ('http://127.0.0.1:' + $Port + '/json/list') -TimeoutSec 2
            $pages = @($targets | Where-Object { $_.type -eq 'page' })
            if ($pages.Count -gt 0) {
                Write-Host ("CDP ready: " + $pages[0].url) -ForegroundColor DarkGray
                $script:cdpReady = $true
                return
            }
        }
        catch {
            Start-Sleep -Milliseconds 400
        }
    }
    Fail-Launch ('CDP endpoint did not expose a page on port ' + $Port + ' within 60 s. WebView2 may ignore WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS on this machine; see docs/WALKTHROUGH.md.')
}

function Invoke-Driver {
    Write-Phase 'Driving the live UI over CDP'
    $env:CDP_PORT = [string]$Port
    $env:WALKTHROUGH_ARTIFACTS = $ArtifactsDir
    $env:WALKTHROUGH_WORKSPACE = $Workspace
    $env:WALKTHROUGH_WORKSPACE_B = $WorkspaceB
    $env:WALKTHROUGH_SETTINGS_FILE = $settingsFile
    $env:WALKTHROUGH_STAMP = $stamp
    $env:WALKTHROUGH_MODE = $Mode
    $env:WALKTHROUGH_BOOT_TIMEOUT_MS = '60000'
    $env:WALKTHROUGH_STEP_TIMEOUT_MS = '20000'
    try {
        $previousPreference = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $lines = @(& node $Driver 2>&1 | ForEach-Object { $_.ToString() })
        $script:driverExit = $LASTEXITCODE
        $ErrorActionPreference = $previousPreference
        $lines | Set-Content -LiteralPath $driverLog -Encoding UTF8
        $lines | ForEach-Object { Write-Host $_ }
    }
    finally {
        $ErrorActionPreference = 'Stop'
        Remove-Item Env:CDP_PORT -ErrorAction SilentlyContinue
        Remove-Item Env:WALKTHROUGH_ARTIFACTS -ErrorAction SilentlyContinue
        Remove-Item Env:WALKTHROUGH_WORKSPACE -ErrorAction SilentlyContinue
        Remove-Item Env:WALKTHROUGH_WORKSPACE_B -ErrorAction SilentlyContinue
        Remove-Item Env:WALKTHROUGH_SETTINGS_FILE -ErrorAction SilentlyContinue
        Remove-Item Env:WALKTHROUGH_STAMP -ErrorAction SilentlyContinue
        Remove-Item Env:WALKTHROUGH_MODE -ErrorAction SilentlyContinue
        Remove-Item Env:WALKTHROUGH_BOOT_TIMEOUT_MS -ErrorAction SilentlyContinue
        Remove-Item Env:WALKTHROUGH_STEP_TIMEOUT_MS -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath (Join-Path $ArtifactsDir 'cdp-result.json')) {
        $script:result = Get-Content -LiteralPath (Join-Path $ArtifactsDir 'cdp-result.json') -Raw | ConvertFrom-Json
    }
}

function Copy-App-Logs {
    $pidText = ''
    if ($null -ne $script:appProcess) { $pidText = '[pid ' + $script:appProcess.Id + ']' }
    $bootLog = Join-Path $env:TEMP 'selora_boot.log'
    if (Test-Path -LiteralPath $bootLog) {
        $lines = @()
        if (-not [string]::IsNullOrWhiteSpace($pidText)) {
            $lines = @(Select-String -LiteralPath $bootLog -SimpleMatch -Pattern $pidText | ForEach-Object { $_.Line })
        }
        if ($lines.Count -eq 0) {
            $lines = @(Get-Content -LiteralPath $bootLog -Tail 80)
        }
        $lines | Set-Content -LiteralPath (Join-Path $ArtifactsDir 'app-boot.log') -Encoding UTF8
    }
    $panicLog = Join-Path $env:TEMP 'selora_last_panic.log'
    if (Test-Path -LiteralPath $panicLog) {
        $info = Get-Item -LiteralPath $panicLog
        if ($info.LastWriteTime -gt $startedAt) {
            Copy-Item -LiteralPath $panicLog -Destination (Join-Path $ArtifactsDir 'app-panic.log') -Force
        }
    }
    if ($null -ne $script:outTask -and $null -ne $script:appProcess -and $script:appProcess.HasExited) {
        try { Write-TextFile (Join-Path $ArtifactsDir 'app-stdout.log') $script:outTask.Result } catch { }
    }
    if ($null -ne $script:errTask -and $null -ne $script:appProcess -and $script:appProcess.HasExited) {
        try { Write-TextFile (Join-Path $ArtifactsDir 'app-stderr.log') $script:errTask.Result } catch { }
    }
}

function Stop-ProcessTree {
    param([System.Diagnostics.Process]$Process, [string]$Label)
    if ($null -eq $Process) { return }
    try {
        if (-not $Process.HasExited) {
            Write-Host ("stopping " + $Label + " tree (pid " + $Process.Id + ")") -ForegroundColor DarkGray
            $null = & taskkill.exe /PID $Process.Id /T /F 2>&1
            $null = $Process.WaitForExit(5000)
        }
    }
    catch {
        Write-Host ("warning: could not stop " + $Label + ": " + $_.Exception.Message) -ForegroundColor Yellow
    }
}

function Write-Summary {
    param([int]$ExitCode)
    $mode = $Mode
    $steps = @()
    if ($null -ne $script:result -and $null -ne $script:result.steps) {
        $steps = @($script:result.steps)
    }
    $passed = @($steps | Where-Object { $_.status -eq 'PASS' }).Count
    $failed = @($steps | Where-Object { $_.status -eq 'FAIL' }).Count
    $findings = @($steps | Where-Object { $_.status -eq 'FINDING' })
    $findingCount = $findings.Count
    $exceptionCount = 0
    if ($null -ne $script:result -and $null -ne $script:result.exceptions) {
        $exceptionCount = @($script:result.exceptions).Count
    }

    $summary = [pscustomobject]@{
        generated_at       = (Get-Date).ToString('o')
        mode               = $mode
        exit_code          = $ExitCode
        automated_pass     = ($ExitCode -eq 0)
        human_acceptance   = 'NOT CLAIMED - see docs/WALKTHROUGH.md checklist'
        driver_exit        = $script:driverExit
        port               = $Port
        app_pid            = $(if ($null -ne $script:appProcess) { $script:appProcess.Id } else { $null })
        exe                = $script:exePath
        workspace          = $Workspace
        workspace_b        = $WorkspaceB
        settings_dir       = $settingsDir
        artifacts          = $ArtifactsDir
        cdp_ready          = $script:cdpReady
        steps_passed       = $passed
        steps_failed       = $failed
        findings_count     = $findingCount
        findings           = $findings
        renderer_exceptions = $exceptionCount
        failure            = $script:failure
        steps              = $steps
        screenshots        = $(if ($null -ne $script:result) { @($script:result.screenshots) } else { @() })
        bootstrap          = $(if ($null -ne $script:result) { $script:result.bootstrap } else { $null })
        diagnostics        = $(if ($null -ne $script:result) { $script:result.diagnostics } else { $null })
    }
    $summary | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $summaryJson -Encoding UTF8

    $lines = New-Object System.Collections.ArrayList
    $null = $lines.Add('# Selora walkthrough - automated run ' + $stamp)
    $null = $lines.Add('')
    $verdict = 'PASS (automated checks only)'
    if ($ExitCode -ne 0) { $verdict = 'FAIL' }
    elseif ($findingCount -gt 0) { $verdict = 'PASS (automated checks only; ' + $findingCount + ' finding(s) reported, not accepted)' }
    $null = $lines.Add('- Verdict: **' + $verdict + '**')
    $null = $lines.Add('- Human acceptance: **NOT CLAIMED**. Visual, real-gateway, and OS-level checks are still on the human checklist in `docs/WALKTHROUGH.md`.')
    $null = $lines.Add('- Mode: `' + $mode + '`; port `' + $Port + '`; app pid `' + $summary.app_pid + '`')
    $null = $lines.Add('- Binary: `' + $script:exePath + '`')
    $null = $lines.Add('- Fixture workspace: `' + $Workspace + '`')
    $null = $lines.Add('- Isolated settings: `' + $settingsDir + '`')
    $null = $lines.Add('- Steps: **' + $passed + ' passed, ' + $failed + ' failed, ' + $findingCount + ' finding(s)**; renderer exceptions: **' + $exceptionCount + '**')
    if ($findingCount -gt 0) {
        $null = $lines.Add('')
        $null = $lines.Add('## Findings')
        $null = $lines.Add('')
        $null = $lines.Add('These are observed product behaviours the walkthrough worked around. They are not accepted as correct; run with `-FailOnFindings` to gate on them.')
        $null = $lines.Add('')
        foreach ($finding in $findings) {
            $null = $lines.Add('- **' + [string]$finding.id + '** - ' + [string]$finding.detail)
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($script:failure)) {
        $null = $lines.Add('')
        $null = $lines.Add('## Failure')
        $null = $lines.Add('')
        $null = $lines.Add('```text')
        $null = $lines.Add($script:failure)
        $null = $lines.Add('```')
    }
    $null = $lines.Add('')
    $null = $lines.Add('## Steps')
    $null = $lines.Add('')
    $null = $lines.Add('| Step | Status | ms | Detail |')
    $null = $lines.Add('|---|---|---|---|')
    foreach ($step in $steps) {
        $detail = ([string]$step.detail).Replace('|', '\|')
        if ($detail.Length -gt 220) { $detail = $detail.Substring(0, 220) + '...' }
        $null = $lines.Add('| `' + $step.id + '` - ' + ([string]$step.title).Replace('|', '\|') + ' | ' + $step.status + ' | ' + $step.ms + ' | ' + $detail + ' |')
    }
    $null = $lines.Add('')
    $null = $lines.Add('## Artifacts')
    $null = $lines.Add('')
    $null = $lines.Add('- `summary.json`, `cdp-result.json`, `cdp-driver.log`, `build.log`')
    $null = $lines.Add('- Screenshots: ' + (@($summary.screenshots | ForEach-Object { '`' + $_ + '`' }) -join ', '))
    $null = $lines.Add('- App logs: `app-boot.log`, `app-stdout.log`, `app-stderr.log`, `app-panic.log` (when present)')
    if ($exceptionCount -gt 0) {
        $null = $lines.Add('')
        $null = $lines.Add('## Renderer exceptions')
        $null = $lines.Add('')
        foreach ($exception in @($script:result.exceptions)) {
            $null = $lines.Add('- ' + [string]$exception.description)
        }
    }
    $null = $lines.Add('')
    $null = $lines.Add('## Human checklist')
    $null = $lines.Add('')
    $null = $lines.Add('Open `docs/WALKTHROUGH.md` and run the visual/judgement checklist. This file is evidence of automated behavior, not acceptance.')
    $lines | Set-Content -LiteralPath $summaryMd -Encoding UTF8
    return $summary
}

$exitCode = 0
$summary = $null
try {
    Write-Host ('Selora walkthrough: ' + $Mode + ' -> ' + $ArtifactsDir) -ForegroundColor White
    Assert-Preconditions
    New-Fixtures
    Write-Host ('workspace: ' + $Workspace) -ForegroundColor DarkGray
    Write-Host ('workspace B: ' + $WorkspaceB) -ForegroundColor DarkGray

    if ($Mode -eq 'release') {
        $exe = Resolve-Exe
        if ($Build) {
            Write-Phase 'Building release artifacts'
            Install-FrontendDeps
            Invoke-BuildStep 'npm run build (frontend)' 'cmd.exe' @('/c', 'npm.cmd --prefix apps\selora-desktop\frontend run build')
            # Force the binary to re-embed frontend\dist; an up-to-date cargo
            # artifact can otherwise keep the previous asset bundle.
            Invoke-BuildStep 'cargo clean -p selora-desktop --release' 'cargo.exe' @('clean', '-p', 'selora-desktop', '--release')
            Invoke-BuildStep 'cargo build --release -p selora-desktop' 'cargo.exe' @('build', '--release', '-p', 'selora-desktop', '--locked')
        }
        if (-not (Test-Path -LiteralPath $exe)) {
            Fail-Launch ('release binary missing: ' + $exe + ". Build it with:`n  npm.cmd --prefix apps\selora-desktop\frontend run build`n  cargo build --release -p selora-desktop`nor rerun with -Build.")
        }
    }
    else {
        Install-FrontendDeps
        if ($Build -or -not (Test-Path -LiteralPath (Resolve-Exe))) {
            Write-Phase 'Building debug binary'
            Invoke-BuildStep 'cargo build -p selora-desktop' 'cargo.exe' @('build', '-p', 'selora-desktop', '--locked')
        }
        $null = Resolve-Exe
        Start-Vite
    }

    Start-App
    Copy-App-Logs
    Invoke-Driver
    Copy-App-Logs

    if ($script:driverExit -eq 0) {
        $exitCode = 0
    }
    elseif ($script:driverExit -eq 2) {
        $exitCode = 2
    }
    else {
        $exitCode = 1
    }
    if ($null -ne $script:result -and -not $script:result.ok) {
        if ($exitCode -eq 0) { $exitCode = 1 }
        $script:failure = [string]$script:result.error
    }
    if ($FailOnFindings -and $null -ne $script:result -and $null -ne $script:result.steps) {
        $findingSteps = @($script:result.steps | Where-Object { $_.status -eq 'FINDING' })
        if ($findingSteps.Count -gt 0) {
            $exitCode = 1
            $script:failure = [string]$findingSteps[0].detail
        }
    }
}
catch {
    $script:failure = $_.Exception.Message
    $exitCode = 2
    Write-Host $_.ScriptStackTrace -ForegroundColor DarkRed
}
finally {
    if (-not $KeepRunning) {
        Stop-ProcessTree $script:appProcess 'app'
        if ($null -ne $script:viteProcess) {
            Stop-ProcessTree $script:viteProcess 'vite'
        }
    }
    else {
        Write-Host 'KeepRunning: app left running; Vite (dev) left running.' -ForegroundColor Yellow
    }
    Copy-App-Logs
    try {
        $summary = Write-Summary $exitCode
    }
    catch {
        Write-Host ('warning: could not write summary: ' + $_.Exception.Message) -ForegroundColor Yellow
    }
}

Write-Phase 'Result'
if ($null -ne $summary) {
    Write-Host ('automated pass: ' + $summary.automated_pass) -ForegroundColor $(if ($summary.automated_pass) { 'Green' } else { 'Red' })
    Write-Host ('steps: ' + $summary.steps_passed + ' passed, ' + $summary.steps_failed + ' failed; renderer exceptions: ' + $summary.renderer_exceptions) -ForegroundColor $(if ($summary.steps_failed -eq 0 -and $summary.renderer_exceptions -eq 0) { 'Green' } else { 'Red' })
    if ($summary.findings_count -gt 0) {
        Write-Host ('findings: ' + $summary.findings_count + ' (reported, not accepted; see summary.md; gate with -FailOnFindings)') -ForegroundColor Yellow
    }
}
Write-Host ('artifacts: ' + $ArtifactsDir)
Write-Host 'human acceptance: NOT CLAIMED - see docs/WALKTHROUGH.md'
if ($exitCode -ne 0) {
    Write-Host ('failure: ' + $script:failure) -ForegroundColor Red
    Write-Host ('diagnostics: ' + $summaryMd)
}
exit $exitCode