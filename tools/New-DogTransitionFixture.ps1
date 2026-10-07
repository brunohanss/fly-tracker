# Build disk replay fixtures from two supplied images. Do not infer safety labels from pixels.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ClearImage,
    [Parameter(Mandatory)][string]$DogImage,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [string]$Config = 'config/stationary-room.json',
    [string]$Annotations = 'config/stationary-room-targets.json'
)
$ErrorActionPreference = 'Stop'
$points = (Get-Content -Raw -LiteralPath $Annotations | ConvertFrom-Json).images[0]
$settings = Get-Content -Raw -LiteralPath $Config | ConvertFrom-Json
& (Join-Path $PSScriptRoot 'Import-ImageSequence.ps1') -Images @($ClearImage,$DogImage) -OutputDirectory $OutputDirectory -PeriodUs 200000
$directory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$base = Get-Content -Raw -LiteralPath (Join-Path $directory 'sequence.json') | ConvertFrom-Json
if (($settings.frame_size -join ',') -ne ($base.size -join ',')) { throw 'Fixture dimensions differ from the test configuration.' }
Copy-Item -LiteralPath $ClearImage -Destination (Join-Path $directory 'clear.png')
Copy-Item -LiteralPath $DogImage -Destination (Join-Path $directory 'dog.png')
$targets = @()
for ($index = 0; $index -lt $points.Count; $index++) {
    $targets += @{id=($index+1);position=@($points[$index][0],$points[$index][1]);visible=$true}
}
$phases = @{
    'dog-entry.json' = @($false,$false,$false,$false,$false,$true,$true,$true,$true,$true)
    'dog-exit.json' = @($true,$true,$true,$true,$true,$false,$false,$false,$false,$false)
    'dog-cycle.json' = @($false,$false,$false,$false,$false,$true,$true,$true,$true,$true,$false,$false,$false,$false,$false)
}
foreach ($name in $phases.Keys) {
    $frames = @(for ($index = 0; $index -lt $phases[$name].Count; $index++) {
        $dog = $phases[$name][$index]
        @{file=$(if ($dog) {'000001.pgm'} else {'000000.pgm'});id=$index;timestamp=($index*200000);
            truth=@{targets=$targets;hazard=$(if ($dog) {'Dog'} else {$null})}}
    })
    $manifest = @{version=1;size=$base.size;frames=$frames}
    [System.IO.File]::WriteAllText((Join-Path $directory $name), ($manifest | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
}
$settings | Add-Member -NotePropertyName safety_backend -NotePropertyValue @{backend='fixture'} -Force
$settings.processing | Add-Member -NotePropertyName virtual_aim_dwell_us -NotePropertyValue 0 -Force
[System.IO.File]::WriteAllText((Join-Path $directory 'config.json'), ($settings | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
Write-Output 'Created entry, exit, and cycle fixtures. Safety labels are explicit test annotations.'
