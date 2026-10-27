<#
.SYNOPSIS
    Reproducible performance harness for Selora IDE.

.DESCRIPTION
    Measures six things and writes raw results next to the logs:

      1. cold start, debug and release (process spawn -> main window handle);
      2. the Rust bootstrap stages from %TEMP%\selora_boot.log (state init,
         Tauri setup, WebView build, window shown);
      3. idle working set / private bytes for the app and its whole child tree
         (WebView2 helpers included);
      4. typing latency, split into the frontend CodeMirror transaction cost
         (no paint, mocked IPC) and the Rust `ApplyEditorChange` path;
      5. event throughput and soak under 8 concurrent producers, including the
         sequence-ordering invariant the frontend gate depends on;
      6. workflow engine run overhead (`prepare`, empty run, facade steps).

    Debug builds load the Vite dev server (`WebviewUrl::default()`), so the
    script starts it for the debug runs. Release builds serve `frontend/dist`.

    Numbers are informational: the harness fails only when a stage cannot run
    or its correctness assertions fail, never because a machine is slow.

.PARAMETER Runs
    Number of launches per configuration for cold start / idle RSS. Default 3.

.PARAMETER IdleSeconds
    Seconds to wait after the window appears before sampling memory. Default 5.

.PARAMETER SkipBuild
    Use existing binaries and `frontend/dist`.

.PARAMETER SkipApp
    Skip the launch-based measurements (cold start, bootstrap, idle RSS).

.PARAMETER SkipRust
    Skip the Rust microbenchmarks.

.PARAMETER SkipFrontend
    Skip the frontend typing-latency spec.

.PARAMETER ResultsDir
    Override the output directory. Default: <repo>\target\perf\<timestamp>.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\perf.ps1

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\perf.ps1 -Runs 5 -IdleSeconds 10

.NOTES
    PowerShell 5.1 compatible. No `??`, no ternary operator. Requires an
    interactive desktop session: the app opens a real window for the launch
    measurements.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = '',
    [int]$Runs = 3,
    [int]$IdleSeconds = 5,
    [switch]$SkipBuild,
    [switch]$SkipApp,
    [switch]$SkipRust,
    [switch]$SkipFrontend,
    [string]$ResultsDir = ''
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
$frontend = Join-Path $RepoRoot 'apps\selora-desktop\frontend'
$debugExe = Join-Path $RepoRoot 'target\debug\selora-desktop.exe'
$releaseExe = Join-Path $RepoRoot 'target\release\selora-desktop.exe'
$bootLog = Join-Path $env:TEMP 'selora_boot.log'

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if ([string]::IsNullOrWhiteSpace($ResultsDir)) {
    $ResultsDir = Join-Path $RepoRoot ("target\perf\" + $stamp)
}
$null = New-Item -ItemType Directory -Path $ResultsDir -Force
$result = [ordered]@{
    generated_at = (Get-Date).ToString('o')
    machine      = [ordered]@{}
    build        = [ordered]@{}
    cold_start   = (New-Object System.Collections.ArrayList)
    idle_rss     = (New-Object System.Collections.ArrayList)
    benchmarks   = (New-Object System.Collections.ArrayList)
    stages       = (New-Object System.Collections.ArrayList)
    gaps         = (New-Object System.Collections.ArrayList)
}
$failures = New-Object System.Collections.ArrayList

function Add-Stage {
    param([string]$Name, [string]$Status, [string]$Detail)
    $null = $result.stages.Add([pscustomobject]@{ name = $Name; status = $Status; detail = $Detail })
    $color = 'Gray'
    if ($Status -eq 'PASS') { $color = 'Green' }
    if ($Status -eq 'FAIL') { $color = 'Red'; $null = $failures.Add($Name) }
    if ($Status -eq 'SKIP') { $color = 'Yellow' }
    Write-Host ("[{0}] {1} - {2}" -f $Status, $Name, $Detail) -ForegroundColor $color
}

function Median {
    param([double[]]$Values)
    if ($null -eq $Values -or $Values.Count -eq 0) { return 0 }
    $sorted = $Values | Sort-Object
    $mid = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return [double]$sorted[$mid] }
    return ([double]$sorted[$mid - 1] + [double]$sorted[$mid]) / 2.0
}

function Wait-Port {
    param([int]$Port, [int]$TimeoutSeconds)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        # Vite binds `localhost`, which is ::1 on this Windows setup. An HTTP
        # probe is both family-agnostic and proof the server answers, unlike a
        # raw IPv4 TCP connect.
        foreach ($uri in @(('http://127.0.0.1:' + $Port + '/'), ('http://[::1]:' + $Port + '/'))) {
            try {
                $response = Invoke-WebRequest -Uri $uri -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
                if ($response.StatusCode -ge 200 -and $response.StatusCode -lt 500) {
                    return $true
                }
            } catch {
                # not serving on this family yet
            }
        }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

function Stop-Tree {
    param([int]$ProcessId)
    if ($ProcessId -le 0) { return }
    & taskkill.exe /PID $ProcessId /T /F 2>&1 | Out-Null
    Start-Sleep -Milliseconds 250
}

function Stop-PortOwner {
    param([int]$Port)
    # `taskkill /T` on the `cmd /c npm run dev` wrapper can finish before the
    # reparented Vite node process does; find the actual listener as a backstop.
    foreach ($match in @(netstat -ano | Select-String (':' + $Port + '\s'))) {
        if ($match.Line -match 'LISTENING\s+(\d+)') {
            Stop-Tree -ProcessId ([int]$matches[1])
        }
    }
}

function Stop-SeloraProcesses {
    Get-Process -Name 'selora-desktop' -ErrorAction SilentlyContinue | ForEach-Object {
        Stop-Tree -ProcessId $_.Id
    }
    # A WebView2 profile is shared by every instance; wait for the tree to die
    # so the next launch really is a cold process start.
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline) {
        $left = @(Get-Process -Name 'selora-desktop' -ErrorAction SilentlyContinue)
        if ($left.Count -eq 0) { return }
        Start-Sleep -Milliseconds 200
    }
}

function Get-ProcessTreeMemory {
    param([int]$RootProcessId)
    $all = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Select-Object ProcessId, ParentProcessId, Name, WorkingSetSize, PrivatePageCount)
    $byParent = @{}
    foreach ($entry in $all) {
        $key = [int]$entry.ParentProcessId
        if (-not $byParent.ContainsKey($key)) { $byParent[$key] = New-Object System.Collections.ArrayList }
        $null = $byParent[$key].Add($entry)
    }
    $tree = New-Object System.Collections.ArrayList
    $queue = New-Object System.Collections.Queue
    $queue.Enqueue($RootProcessId)
    $seen = @{}
    while ($queue.Count -gt 0) {
        $current = [int]$queue.Dequeue()
        if ($seen.ContainsKey($current)) { continue }
        $seen[$current] = $true
        if ($byParent.ContainsKey($current)) {
            foreach ($child in $byParent[$current]) {
                $null = $tree.Add($child)
                $queue.Enqueue([int]$child.ProcessId)
            }
        }
    }
    $workingSet = 0
    $privateBytes = 0
    $names = @()
    foreach ($entry in $tree) {
        $workingSet += [int64]$entry.WorkingSetSize
        $privateBytes += [int64]$entry.PrivatePageCount
        $names += $entry.Name
    }
    return [pscustomobject]@{
        child_count          = $tree.Count
        child_working_set_mb = [math]::Round($workingSet / 1MB, 1)
        child_private_mb     = [math]::Round($privateBytes / 1MB, 1)
        children             = ($names | Sort-Object -Unique) -join ','
    }
}

function Read-BootStages {
    if (-not (Test-Path -LiteralPath $bootLog)) { return [ordered]@{} }
    $stages = [ordered]@{}
    foreach ($line in (Get-Content -LiteralPath $bootLog -ErrorAction SilentlyContinue)) {
        if ($line -match '\[\+(\d+) ms\]\s+(.+)$') {
            $stages[$matches[2].Trim()] = [int]$matches[1]
        }
    }
    return $stages
}

function Wait-BootMark {
    param([string]$Mark, [int]$TimeoutSeconds)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $bootLog) {
            if (Select-String -LiteralPath $bootLog -SimpleMatch $Mark -Quiet -ErrorAction SilentlyContinue) {
                return $true
            }
        }
        Start-Sleep -Milliseconds 20
    }
    return $false
}

function Measure-Launch {
    param([string]$Label, [string]$Exe, [int]$RunNumber)
    Remove-Item -LiteralPath $bootLog -Force -ErrorAction SilentlyContinue
    Stop-SeloraProcesses

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $Exe -WorkingDirectory (Split-Path -Parent $Exe) -PassThru
    $windowMs = -1
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline) {
        if ($proc.HasExited) { break }
        $proc.Refresh()
        if ($proc.MainWindowHandle -ne 0) {
            $windowMs = [math]::Round($timer.Elapsed.TotalMilliseconds, 1)
            break
        }
        Start-Sleep -Milliseconds 5
    }

    if ($proc.HasExited) {
        $timer.Stop()
        return [pscustomobject]@{ label = $Label; run = $RunNumber; ok = $false; error = ('process exited with ' + $proc.ExitCode) }
    }
    if ($windowMs -lt 0) {
        $timer.Stop()
        Stop-Tree -ProcessId $proc.Id
        return [pscustomobject]@{ label = $Label; run = $RunNumber; ok = $false; error = 'no main window within 60 s' }
    }

    # The window handle exists before WebView2 has loaded the shell. The
    # frontend marks the end of bootstrap + replay through `frontend_ready`.
    $readyMs = -1
    if (Wait-BootMark -Mark 'frontend bootstrap ready' -TimeoutSeconds 30) {
        $readyMs = [math]::Round($timer.Elapsed.TotalMilliseconds, 1)
    }
    $timer.Stop()

    $bootAtWindow = Read-BootStages
    Start-Sleep -Seconds $IdleSeconds
    $proc.Refresh()
    $mainRss = [math]::Round($proc.WorkingSet64 / 1MB, 1)
    $mainPrivate = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1)
    $threads = $proc.Threads.Count
    $handles = $proc.HandleCount
    $tree = Get-ProcessTreeMemory -RootProcessId $proc.Id
    $boot = Read-BootStages

    $record = [pscustomobject]@{
        label                  = $Label
        run                    = $RunNumber
        ok                     = $true
        window_ms              = $windowMs
        ready_ms               = $readyMs
        main_rss_mb            = $mainRss
        main_private_mb        = $mainPrivate
        tree_rss_mb            = [math]::Round($mainRss + $tree.child_working_set_mb, 1)
        tree_private_mb        = [math]::Round($mainPrivate + $tree.child_private_mb, 1)
        child_count            = $tree.child_count
        children               = $tree.children
        threads                = $threads
        handles                = $handles
        boot_window            = $bootAtWindow
        boot_idle              = $boot
        idle_seconds           = $IdleSeconds
    }
    Stop-Tree -ProcessId $proc.Id
    Stop-SeloraProcesses
    return $record
}

# ---------------------------------------------------------------------------
# Metadata
# ---------------------------------------------------------------------------

$cpu = @(Get-CimInstance Win32_Processor -ErrorAction SilentlyContinue | Select-Object -First 1)
$os = Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue
$cs = Get-CimInstance Win32_ComputerSystem -ErrorAction SilentlyContinue
$cpuName = 'unknown'
$cpuCores = 0
$cpuThreads = 0
if ($cpu.Count -gt 0) {
    $cpuName = $cpu[0].Name
    $cpuCores = $cpu[0].NumberOfCores
    $cpuThreads = $cpu[0].NumberOfLogicalProcessors
}
$ramGb = 0
if ($null -ne $cs) { $ramGb = [math]::Round($cs.TotalPhysicalMemory / 1GB, 1) }
$osName = 'unknown'
if ($null -ne $os) { $osName = $os.Caption + ' ' + $os.Version + ' (build ' + $os.BuildNumber + ')' }
$dirtyCount = @(& git -C $RepoRoot status --porcelain 2>&1).Count
$result.machine = [ordered]@{
    cpu                 = $cpuName
    cores               = $cpuCores
    logical_processors  = $cpuThreads
    ram_gb              = $ramGb
    os                  = $osName
    powershell          = $PSVersionTable.PSVersion.ToString()
    rustc               = (& rustc --version 2>&1 | Select-Object -First 1)
    cargo               = (& cargo --version 2>&1 | Select-Object -First 1)
    node                = (& node --version 2>&1 | Select-Object -First 1)
    git_commit          = (& git -C $RepoRoot rev-parse HEAD 2>&1 | Select-Object -First 1)
    git_branch          = (& git -C $RepoRoot rev-parse --abbrev-ref HEAD 2>&1 | Select-Object -First 1)
    git_dirty           = ($dirtyCount -gt 0)
}

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

$viteProcess = $null
try {
    if (-not $SkipBuild) {
        Write-Host 'Building frontend (dist) ...'
        Push-Location $frontend
        try {
            & cmd.exe /c 'npm.cmd run build' 2>&1 | Tee-Object -FilePath (Join-Path $ResultsDir 'npm-build.log')
            if ($LASTEXITCODE -ne 0) { throw "npm run build failed ($LASTEXITCODE)" }
        } finally {
            Pop-Location
        }

        Write-Host 'Building debug binary ...'
        & cargo build -p selora-desktop 2>&1 | Tee-Object -FilePath (Join-Path $ResultsDir 'cargo-build-debug.log')
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }

        Write-Host 'Building release binary (thin LTO, codegen-units=1) ...'
        & cargo build -p selora-desktop --release 2>&1 | Tee-Object -FilePath (Join-Path $ResultsDir 'cargo-build-release.log')
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed ($LASTEXITCODE)" }
        Add-Stage -Name 'build' -Status 'PASS' -Detail 'frontend dist + debug + release'
    } else {
        $missing = @()
        if (-not (Test-Path -LiteralPath $debugExe)) { $missing += 'target\debug\selora-desktop.exe' }
        if (-not (Test-Path -LiteralPath $releaseExe)) { $missing += 'target\release\selora-desktop.exe' }
        if (-not (Test-Path -LiteralPath (Join-Path $frontend 'dist\index.html'))) { $missing += 'frontend\dist' }
        if ($missing.Count -gt 0) { throw ('missing build artifacts: ' + ($missing -join ', ')) }
        Add-Stage -Name 'build' -Status 'SKIP' -Detail 'existing artifacts (-SkipBuild)'
    }

    foreach ($pair in @(@('debug', $debugExe), @('release', $releaseExe))) {
        $label = $pair[0]
        $path = $pair[1]
        if (Test-Path -LiteralPath $path) {
            $item = Get-Item -LiteralPath $path
            $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
            $result.build[$label] = [ordered]@{
                path        = $path
                bytes       = $item.Length
                sha256      = $hash
                modified    = $item.LastWriteTime.ToString('o')
            }
        }
    }

    # -----------------------------------------------------------------------
    # Launch measurements
    # -----------------------------------------------------------------------
    if (-not $SkipApp) {
        $configs = @()
        if (Test-Path -LiteralPath $debugExe) { $configs += @('debug') }
        if (Test-Path -LiteralPath $releaseExe) { $configs += @('release') }
        if ($configs.Count -eq 0) { throw 'no app binaries to launch' }

        $viteLog = Join-Path $ResultsDir 'vite-dev.log'
        Write-Host 'Starting Vite dev server for the debug runs ...'
        $viteProcess = Start-Process -FilePath 'cmd.exe' -ArgumentList @('/c', ('npm.cmd run dev > "' + $viteLog + '" 2>&1')) -WorkingDirectory $frontend -PassThru -WindowStyle Hidden
        if (-not (Wait-Port -Port 1420 -TimeoutSeconds 60)) {
            throw 'Vite dev server did not listen on port 1420'
        }

        $launchRecords = @()
        foreach ($config in $configs) {
            $exe = $debugExe
            if ($config -eq 'release') { $exe = $releaseExe }
            Write-Host ("Measuring {0} cold start ({1} runs) ..." -f $config, $Runs)
            for ($run = 1; $run -le $Runs; $run += 1) {
                $record = Measure-Launch -Label $config -Exe $exe -RunNumber $run
                $launchRecords += $record
                if ($record.ok) {
                    Write-Host ("  run {0}: window {1} ms, tree RSS {2} MB" -f $run, $record.window_ms, $record.tree_rss_mb)
                } else {
                    Write-Host ("  run {0}: FAILED - {1}" -f $run, $record.error) -ForegroundColor Red
                }
            }
        }

        foreach ($config in $configs) {
            $ok = @($launchRecords | Where-Object { $_.label -eq $config -and $_.ok })
            $failed = @($launchRecords | Where-Object { $_.label -eq $config -and -not $_.ok })
            if ($failed.Count -gt 0) {
                Add-Stage -Name ("cold-start-" + $config) -Status 'FAIL' -Detail ($failed[0].error)
                continue
            }
            $readyValues = @($ok | Where-Object { $_.ready_ms -gt 0 } | ForEach-Object { $_.ready_ms })
            $readyMedian = -1
            if ($readyValues.Count -gt 0) {
                $readyMedian = [math]::Round((Median -Values $readyValues), 1)
            } else {
                $null = $result.gaps.Add($config + ': frontend bootstrap marker missing')
            }
            $null = $result.cold_start.Add([pscustomobject]@{
                config           = $config
                runs             = $ok.Count
                window_ms_median = [math]::Round((Median -Values @($ok | ForEach-Object { $_.window_ms })), 1)
                window_ms_all    = @($ok | ForEach-Object { $_.window_ms })
                ready_ms_median  = $readyMedian
                ready_ms_all     = $readyValues
                boot_stages      = $ok[0].boot_idle
            })
            $idle = [pscustomobject]@{
                config               = $config
                idle_seconds         = $IdleSeconds
                main_rss_mb_median   = [math]::Round((Median -Values @($ok | ForEach-Object { $_.main_rss_mb })), 1)
                tree_rss_mb_median   = [math]::Round((Median -Values @($ok | ForEach-Object { $_.tree_rss_mb })), 1)
                main_private_mb_med  = [math]::Round((Median -Values @($ok | ForEach-Object { $_.main_private_mb })), 1)
                tree_private_mb_med  = [math]::Round((Median -Values @($ok | ForEach-Object { $_.tree_private_mb })), 1)
                child_processes      = $ok[0].child_count
                children             = $ok[0].children
                samples              = @($ok | ForEach-Object {
                    [pscustomobject]@{
                        run            = $_.run
                        main_rss_mb    = $_.main_rss_mb
                        tree_rss_mb    = $_.tree_rss_mb
                        main_private_mb = $_.main_private_mb
                        tree_private_mb = $_.tree_private_mb
                        threads        = $_.threads
                        handles        = $_.handles
                    }
                })
            }
            $null = $result.idle_rss.Add($idle)
            Add-Stage -Name ("cold-start-" + $config) -Status 'PASS' -Detail ("window median {0} ms, ready median {1} ms over {2} runs" -f $result.cold_start[$result.cold_start.Count - 1].window_ms_median, $readyMedian, $ok.Count)
            Add-Stage -Name ("idle-rss-" + $config) -Status 'PASS' -Detail ("main {0} MB / tree {1} MB after {2} s" -f $idle.main_rss_mb_median, $idle.tree_rss_mb_median, $IdleSeconds)
        }
    } else {
        Add-Stage -Name 'launch-measurements' -Status 'SKIP' -Detail '-SkipApp'
    }

    # -----------------------------------------------------------------------
    # Rust microbenchmarks
    # -----------------------------------------------------------------------
    if (-not $SkipRust) {
        Write-Host 'Running Rust performance tests (release, ignored suite) ...'
        $eventLog = Join-Path $ResultsDir 'cargo-test-perf-desktop.log'
        & cargo test -p selora-desktop --release --bin selora-desktop perf_ -- --ignored --nocapture 2>&1 |
            Tee-Object -FilePath $eventLog
        if ($LASTEXITCODE -ne 0) { throw "selora-desktop perf tests failed ($LASTEXITCODE)" }

        $workflowLog = Join-Path $ResultsDir 'cargo-test-perf-workflow.log'
        & cargo test -p selora-workflow --release --test perf_overhead -- --ignored --nocapture 2>&1 |
            Tee-Object -FilePath $workflowLog
        if ($LASTEXITCODE -ne 0) { throw "selora-workflow perf tests failed ($LASTEXITCODE)" }
    } else {
        Add-Stage -Name 'rust-benchmarks' -Status 'SKIP' -Detail '-SkipRust'
    }

    # -----------------------------------------------------------------------
    # Frontend typing latency
    # -----------------------------------------------------------------------
    if (-not $SkipFrontend) {
        Write-Host 'Running frontend typing-latency spec ...'
        Push-Location $frontend
        try {
            & cmd.exe /c 'npx.cmd vitest run --config perf/vitest.perf.config.ts --reporter=verbose' 2>&1 |
                Tee-Object -FilePath (Join-Path $ResultsDir 'vitest-perf.log')
            if ($LASTEXITCODE -ne 0) { throw "frontend perf spec failed ($LASTEXITCODE)" }
        } finally {
            Pop-Location
        }
    } else {
        Add-Stage -Name 'frontend-typing' -Status 'SKIP' -Detail '-SkipFrontend'
    }
} catch {
    Write-Host ("FAIL " + $_.Exception.Message) -ForegroundColor Red
    Add-Stage -Name 'harness' -Status 'FAIL' -Detail $_.Exception.Message
} finally {
    if ($null -ne $viteProcess) {
        Stop-Tree -ProcessId $viteProcess.Id
        Stop-PortOwner -Port 1420
    }
    Stop-SeloraProcesses
}

# ---------------------------------------------------------------------------
# Parse PERF_JSON lines from every log and persist
# ---------------------------------------------------------------------------

$logFiles = @(Get-ChildItem -LiteralPath $ResultsDir -Filter '*.log' -ErrorAction SilentlyContinue)
foreach ($log in $logFiles) {
    foreach ($line in (Get-Content -LiteralPath $log.FullName -ErrorAction SilentlyContinue)) {
        $index = $line.IndexOf('PERF_JSON ')
        if ($index -ge 0) {
            $jsonText = $line.Substring($index + 'PERF_JSON '.Length).Trim()
            try {
                $metric = $jsonText | ConvertFrom-Json
                $metric | Add-Member -NotePropertyName source -NotePropertyValue $log.Name -Force
                $null = $result.benchmarks.Add($metric)
            } catch {
                $null = $result.gaps.Add(('unparsable benchmark line in ' + $log.Name))
            }
        }
    }
}

$expectedMetrics = @()
if (-not $SkipRust) { $expectedMetrics += 'event_log_concurrent_soak'; $expectedMetrics += 'apply_editor_change'; $expectedMetrics += 'workflow_run_overhead' }
if (-not $SkipFrontend) { $expectedMetrics += 'typing_latency_frontend' }
$foundNames = @($result.benchmarks | ForEach-Object { $_.metric })
foreach ($expected in $expectedMetrics) {
    if ($foundNames -notcontains $expected) {
        $null = $result.gaps.Add(('missing benchmark: ' + $expected))
    }
}
if ($result.benchmarks.Count -gt 0) {
    Add-Stage -Name 'benchmarks' -Status 'PASS' -Detail ("{0} PERF_JSON records" -f $result.benchmarks.Count)
} elseif ($expectedMetrics.Count -gt 0) {
    Add-Stage -Name 'benchmarks' -Status 'FAIL' -Detail 'no PERF_JSON records found'
}

$null = $result.gaps.Add('cold start uses a warm OS file cache; Windows has no supported drop-caches step')
$null = $result.gaps.Add('frontend typing latency excludes paint and the real IPC round trip (happy-dom)')
$null = $result.gaps.Add('bootstrap-ready is the frontend marker after bootstrap + replay; the first painted frame can land a frame later')
$null = $result.gaps.Add('workflow numbers cover the engine only; journal fsync, IPC emits, and agent turns are shell layers')
$null = $result.gaps.Add('event soak measures the in-process log, not the Tauri emit serialization to the webview')

$result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $ResultsDir 'results.json') -Encoding UTF8

# Human summary
$md = New-Object System.Collections.ArrayList
$null = $md.Add('# Selora performance run ' + $stamp)
$null = $md.Add('')
$null = $md.Add('- CPU: ' + $result.machine.cpu + ' (' + $result.machine.cores + 'C/' + $result.machine.logical_processors + 'T)')
$null = $md.Add('- RAM: ' + $result.machine.ram_gb + ' GB')
$null = $md.Add('- OS: ' + $result.machine.os)
$null = $md.Add('- rustc: ' + $result.machine.rustc)
$null = $md.Add('- node: ' + $result.machine.node)
$null = $md.Add('- commit: ' + $result.machine.git_commit + ' (' + $result.machine.git_branch + ')')
$null = $md.Add('')
$null = $md.Add('## Stages')
$null = $md.Add('')
$null = $md.Add('| Stage | Status | Detail |')
$null = $md.Add('|---|---|---|')
foreach ($stage in $result.stages) {
    $null = $md.Add('| ' + $stage.name + ' | ' + $stage.status + ' | ' + $stage.detail + ' |')
}
$null = $md.Add('')
$null = $md.Add('## Cold start and idle RSS')
$null = $md.Add('')
$null = $md.Add('| Config | Window ms (median) | Bootstrap ready ms (median) | Main RSS MB | Tree RSS MB | Main private MB | Tree private MB |')
$null = $md.Add('|---|---|---|---|---|---|---|')
foreach ($cold in $result.cold_start) {
    $idle = @($result.idle_rss | Where-Object { $_.config -eq $cold.config })
    if ($idle.Count -gt 0) {
        $null = $md.Add('| ' + $cold.config + ' | ' + $cold.window_ms_median + ' | ' + $cold.ready_ms_median + ' | ' + $idle[0].main_rss_mb_median + ' | ' + $idle[0].tree_rss_mb_median + ' | ' + $idle[0].main_private_mb_med + ' | ' + $idle[0].tree_private_mb_med + ' |')
    }
}
$null = $md.Add('')
$null = $md.Add('## Benchmarks')
$null = $md.Add('')
foreach ($benchmark in $result.benchmarks) {
    $null = $md.Add('### ' + $benchmark.metric)
    $null = $md.Add('')
    $null = $md.Add('```json')
    $null = $md.Add(($benchmark | ConvertTo-Json -Depth 8))
    $null = $md.Add('```')
    $null = $md.Add('')
}
$null = $md.Add('## Known gaps')
$null = $md.Add('')
foreach ($gap in $result.gaps) { $null = $md.Add('- ' + $gap) }
$null = $md | Set-Content -LiteralPath (Join-Path $ResultsDir 'summary.md') -Encoding UTF8

Write-Host ''
Write-Host ('Results: ' + $ResultsDir) -ForegroundColor Cyan
Write-Host ('Summary: ' + (Join-Path $ResultsDir 'summary.md')) -ForegroundColor Cyan
foreach ($benchmark in $result.benchmarks) {
    Write-Host ('  ' + $benchmark.metric + ': ' + ($benchmark | ConvertTo-Json -Depth 4 -Compress)) -ForegroundColor Gray
}
foreach ($cold in $result.cold_start) {
    $idle = @($result.idle_rss | Where-Object { $_.config -eq $cold.config })
    if ($idle.Count -gt 0) {
        Write-Host ('  ' + $cold.config + ': window ' + $cold.window_ms_median + ' ms, ready ' + $cold.ready_ms_median + ' ms, tree RSS ' + $idle[0].tree_rss_mb_median + ' MB') -ForegroundColor Gray
    }
}

if ($failures.Count -gt 0) {
    Write-Host ('FAILED stages: ' + ($failures -join ', ')) -ForegroundColor Red
    exit 1
}
exit 0