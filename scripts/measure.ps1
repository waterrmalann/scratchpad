# Measures startup time and idle memory of the release build (PLAN §37-38, docs/performance.md).
#
#   powershell -File scripts/measure.ps1 [-Runs 7] [-Note path\to\note.md] [-Build] [-Exe other.exe]
#                                        [-Tests]
#
# Each run starts target/release/scratchpad.exe (or -Exe) on a scratch notes and config folder
# (never the user's own), reads the startup timings from its log, samples its memory after
# -IdleSeconds and closes it. -Note copies a note into the scratch folder first, e.g. a large
# document. Prints every run and the median. -Tests first runs the timing guards (ADR 0114).
param(
    [int]$Runs = 7,
    [double]$IdleSeconds = 3,
    [string]$Note,
    [switch]$Build,
    [string]$Exe,
    [switch]$Tests
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if ($Build) {
    cargo build --release -p scratchpad --manifest-path "$root/Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}
if ($Tests) {
    cargo test --release --manifest-path "$root/Cargo.toml" -p scratchpad-editor -p scratchpad-core `
        --test perf -- --nocapture --test-threads 1
    if ($LASTEXITCODE -ne 0) { throw 'a timing guard failed' }
}
$exe = if ($Exe) { (Resolve-Path $Exe).Path } else { Join-Path $root 'target/release/scratchpad.exe' }
if (-not (Test-Path $exe)) { throw "No release build at $exe; pass -Build." }

$scratch = Join-Path ([IO.Path]::GetTempPath()) "scratchpad-measure-$PID"
$notes = Join-Path $scratch 'notes'
$config = Join-Path $scratch 'config'
New-Item -ItemType Directory -Force $notes, $config | Out-Null
if ($Note) {
    Copy-Item $Note $notes
} else {
    Set-Content -Encoding utf8 (Join-Path $notes 'Welcome.md') "# Welcome`n`nA short note.`n"
}
$env:SCRATCHPAD_NOTES_DIR = $notes
$env:SCRATCHPAD_CONFIG_DIR = $config
$log = Join-Path $config 'scratchpad.log'

# Milliseconds from process creation to the log line containing $text, or $null.
function Get-Elapsed([string[]]$lines, [string]$text, [datetime]$start) {
    $line = $lines | Where-Object { $_ -like "*$text*" } | Select-Object -First 1
    if (-not $line) { return $null }
    $stamp = [datetime]::Parse($line.Split(' ')[0]).ToUniversalTime()
    [math]::Round(($stamp - $start).TotalMilliseconds)
}

function Get-Median([double[]]$values) {
    $sorted = $values | Sort-Object
    if ($sorted.Count -eq 0) { return $null }
    $sorted[[math]::Floor(($sorted.Count - 1) / 2)]
}

$results = foreach ($run in 1..$Runs) {
    Remove-Item $log -ErrorAction SilentlyContinue
    $process = Start-Process $exe -PassThru
    $start = $process.StartTime.ToUniversalTime()
    $deadline = (Get-Date).AddSeconds(15)
    $lines = @()
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 100
        $lines = @(Get-Content $log -ErrorAction SilentlyContinue)
        if ($lines -match 'first note shown') { break }
    }
    Start-Sleep -Seconds $IdleSeconds
    $process.Refresh()
    [pscustomobject]@{
        Run          = $run
        MainMs       = Get-Elapsed $lines 'starting Scratchpad' $start
        DeviceMs     = Get-Elapsed $lines 'Created device' $start
        PlatformMs   = Get-Elapsed $lines 'platform initialised' $start
        WindowMs     = Get-Elapsed $lines 'main window created' $start
        FirstFrameMs = Get-Elapsed $lines 'first frame shown' $start
        NoteShownMs  = Get-Elapsed $lines 'first note shown' $start
        WorkingSetMB = [math]::Round($process.WorkingSet64 / 1MB, 1)
        PrivateMB    = [math]::Round($process.PrivateMemorySize64 / 1MB, 1)
    }
    Stop-Process -Id $process.Id -Force
    $process.WaitForExit()
}

$results | Format-Table -AutoSize
$columns = 'MainMs', 'DeviceMs', 'PlatformMs', 'WindowMs', 'FirstFrameMs', 'NoteShownMs', 'WorkingSetMB', 'PrivateMB'
$median = [ordered]@{ Run = 'median' }
foreach ($column in $columns) { $median[$column] = Get-Median ($results.$column | Where-Object { $_ -ne $null }) }
[pscustomobject]$median | Format-Table -AutoSize
Write-Host "Times are from process creation. Scratch folder: $scratch"
Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
