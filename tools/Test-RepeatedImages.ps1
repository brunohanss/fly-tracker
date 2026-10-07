# Test each image separately. Do not combine images or generate motion.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string[]]$Images,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [string]$Config,
    [string]$Annotations
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Test output directory must be a new path.' }
foreach ($imagePath in $Images) {
    if (-not (Test-Path -LiteralPath $imagePath -PathType Leaf)) { throw "Missing image: $imagePath" }
}
if ($Images.Count -eq 0) { throw 'Specify at least one image.' }
$repo = Split-Path $PSScriptRoot -Parent
$app = Join-Path $repo 'target/release/app.exe'
if (-not (Test-Path -LiteralPath $app)) { throw 'Run cargo build --release -p app first.' }
$directory = [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetFullPath($OutputDirectory))
$results = @()
$targets = if ($Annotations) { (Get-Content -Raw -LiteralPath $Annotations | ConvertFrom-Json).images } else { $null }
if ($targets -and $targets.Count -ne $Images.Count) { throw 'Annotation image count differs.' }
for ($index = 0; $index -lt $Images.Count; $index++) {
    $sequence = Join-Path $directory.FullName ("image-{0}" -f ($index + 1))
    & (Join-Path $PSScriptRoot 'Import-ImageSequence.ps1') -Images @($Images[$index], $Images[$index], $Images[$index], $Images[$index], $Images[$index]) -OutputDirectory $sequence -PeriodUs 10000
    # Confirm byte-identical frames before testing detection.
    $hashes = @(Get-ChildItem -LiteralPath $sequence -Filter '*.pgm' | Get-FileHash | Select-Object -ExpandProperty Hash -Unique)
    if ($hashes.Count -ne 1) { throw 'Repeated frame pixels differ.' }
    $manifestPath = Join-Path $sequence 'sequence.json'
    if ($targets) {
        $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
        $targetList = @()
        $targetId = 1
        foreach ($point in $targets[$index]) {
            $targetList += @{ id=$targetId; position=@($point[0],$point[1]); visible=$true }
            $targetId++
        }
        foreach ($entry in $manifest.frames) { $entry.truth = @{targets=$targetList;hazard=$null} }
        [System.IO.File]::WriteAllText($manifestPath,($manifest | ConvertTo-Json -Depth 10),[System.Text.UTF8Encoding]::new($false))
    }
    $reportPath = Join-Path $sequence 'run.json'
    $options = @('--headless','--sequence',$manifestPath,'--report',$reportPath)
    if ($Config) { $options += @('--config',$Config) }
    & $app @options
    if ($LASTEXITCODE -ne 0) { throw 'Replay execution failed.' }
    $report = Get-Content -Raw -LiteralPath $reportPath | ConvertFrom-Json
    if ($report.counters.processed_frames -ne 5 -or $report.counters.issued_aims -ne 0) { throw 'Frame-count or output-suppression check failed.' }
    $status = if ($report.counters.detections -eq 0) {
        'FAIL: no stationary targets detected'
    } elseif ($targets) {
        if ($report.counters.false_positives -eq 0 -and $report.counters.false_negatives -eq 0 -and $report.counters.detections -eq 5 * $targets[$index].Count -and $report.counters.acquired_targets -eq $targets[$index].Count) {
            'PASS: annotated targets detected and tracks confirmed'
        } else { 'FAIL: missed targets, unmatched candidates, or missing tracks' }
    } else { 'Candidates found; target annotations required to verify flies' }
    $results += [pscustomobject]@{
        image = $Images[$index]
        identical_frames = 5
        detections = $report.counters.detections
        confirmed_tracks = $report.counters.acquired_targets
        aiming_commands = $report.counters.issued_aims
        stationary_detection = $status
    }
}
[System.IO.File]::WriteAllText((Join-Path $directory.FullName 'summary.json'), (ConvertTo-Json -InputObject $results -Depth 6), [System.Text.UTF8Encoding]::new($false))
$results | Format-Table identical_frames, detections, confirmed_tracks, aiming_commands, stationary_detection
