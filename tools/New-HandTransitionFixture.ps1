# Import the supplied room pair without modifying the source images.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ClearImage,
    [Parameter(Mandatory)][string]$HandImage,
    [Parameter(Mandatory)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'Import-ImageSequence.ps1') -Images @($ClearImage,$HandImage) -OutputDirectory $OutputDirectory -PeriodUs 200000
$directory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$base = Get-Content -Raw -LiteralPath (Join-Path $directory 'sequence.json') | ConvertFrom-Json
if (($base.size -join ',') -ne '1448,1086') { throw 'Expected 1448 x 1086 supplied images.' }
Copy-Item -LiteralPath $ClearImage -Destination (Join-Path $directory 'clear.png')
Copy-Item -LiteralPath $HandImage -Destination (Join-Path $directory 'hand.png')
# Approximate visual annotations. They are not inputs to insect detection.
$points = @(@(441,255),@(559,260),@(761,311),@(777,332),@(546,379))
$targets = @(for ($index = 0; $index -lt $points.Count; $index++) {
    @{id=($index+1);position=$points[$index];visible=$true}
})
$phases = @{
    'clear-only.json' = @($false,$false,$false,$false,$false)
    'hand-only.json' = @($true,$true,$true,$true,$true)
    'hand-entry.json' = @($false,$false,$false,$false,$false,$true,$true,$true,$true,$true)
    'hand-exit.json' = @($true,$true,$true,$true,$true,$false,$false,$false,$false,$false)
    'hand-cycle.json' = @($false,$false,$false,$false,$false,$true,$true,$true,$true,$true,$false,$false,$false,$false,$false)
    'hand-reactivity.json' = @($false,$false,$false,$false,$false,$true,$false,$true,$false,$true)
}
foreach ($name in $phases.Keys) {
    $frames = @(for ($index = 0; $index -lt $phases[$name].Count; $index++) {
        $positive = $phases[$name][$index]
        $period = if ($name -eq 'hand-reactivity.json') {10000} else {200000}
        @{file=$(if ($positive) {'000001.pgm'} else {'000000.pgm'});id=$index;timestamp=($index*$period);
            truth=@{targets=$targets;hazard=$(if ($positive) {'Human'} else {$null})}}
    })
    $manifest = @{version=1;size=$base.size;frames=$frames}
    [System.IO.File]::WriteAllText((Join-Path $directory $name), ($manifest | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
}
$settings = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '../config/stationary-room.json') | ConvertFrom-Json
$settings.processing.detection_region = @{x=420;y=240;width=370;height=150}
$settings | Add-Member -NotePropertyName safety_backend -NotePropertyValue @{backend='fixture'}
$settings.processing | Add-Member -NotePropertyName virtual_aim_dwell_us -NotePropertyValue 0
[System.IO.File]::WriteAllText((Join-Path $directory 'config.json'), ($settings | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
Write-Output 'Created hand entry, exit, and cycle fixtures with five fixed target annotations.'
