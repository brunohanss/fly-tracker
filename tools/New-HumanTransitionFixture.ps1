# Import the supplied room pair without modifying the source images.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ClearImage,
    [Parameter(Mandatory)][string]$HumanImage,
    [Parameter(Mandatory)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'Import-ImageSequence.ps1') -Images @($ClearImage,$HumanImage) -OutputDirectory $OutputDirectory -PeriodUs 200000
$directory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$base = Get-Content -Raw -LiteralPath (Join-Path $directory 'sequence.json') | ConvertFrom-Json
if (($base.size -join ',') -ne '1448,1086') { throw 'Expected 1448 x 1086 supplied images.' }
Copy-Item -LiteralPath $ClearImage -Destination (Join-Path $directory 'clear.png')
Copy-Item -LiteralPath $HumanImage -Destination (Join-Path $directory 'human.png')
# Approximate visual annotations. They are not inputs to insect detection.
$points = @(@(514,164),@(1027,170),@(811,222),@(1204,227),@(590,251),@(725,346),@(1001,401),@(504,424),@(1251,441))
$targets = @(for ($index = 0; $index -lt $points.Count; $index++) {
    @{id=($index+1);position=$points[$index];visible=$true}
})
$phases = @{
    'human-entry.json' = @($false,$false,$false,$false,$false,$true,$true,$true,$true,$true)
    'human-exit.json' = @($true,$true,$true,$true,$true,$false,$false,$false,$false,$false)
    'human-cycle.json' = @($false,$false,$false,$false,$false,$true,$true,$true,$true,$true,$false,$false,$false,$false,$false)
}
foreach ($name in $phases.Keys) {
    $frames = @(for ($index = 0; $index -lt $phases[$name].Count; $index++) {
        $positive = $phases[$name][$index]
        @{file=$(if ($positive) {'000001.pgm'} else {'000000.pgm'});id=$index;timestamp=($index*200000);
            truth=@{targets=$targets;hazard=$(if ($positive) {'Human'} else {$null})}}
    })
    $manifest = @{version=1;size=$base.size;frames=$frames}
    [System.IO.File]::WriteAllText((Join-Path $directory $name), ($manifest | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
}
$settings = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '../config/stationary-room.json') | ConvertFrom-Json
$settings.processing.detection_region = @{x=470;y=100;width=850;height=360}
$settings | Add-Member -NotePropertyName safety_backend -NotePropertyValue @{backend='fixture'}
$settings.processing | Add-Member -NotePropertyName virtual_aim_dwell_us -NotePropertyValue 0
[System.IO.File]::WriteAllText((Join-Path $directory 'config.json'), ($settings | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
Write-Output 'Created human entry, exit, and cycle fixtures with nine fixed target annotations.'
