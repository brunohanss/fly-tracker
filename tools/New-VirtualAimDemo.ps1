# Extend an existing labelled, repeated-image fixture. No image clearance is inferred.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Sequence,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(10,10000)][int]$Frames = 1000
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Output directory must be new.' }
$inputPath = (Resolve-Path -LiteralPath $Sequence).Path
$manifest = Get-Content -Raw -LiteralPath $inputPath | ConvertFrom-Json
$entry = $manifest.frames[0]
if (-not $entry.truth -or $entry.truth.hazard -or $entry.truth.targets.Count -eq 0) {
    throw 'Use a labelled clear test fixture with target annotations.'
}
$directory = [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetFullPath($OutputDirectory))
$image = Join-Path (Split-Path $inputPath -Parent) $entry.file
Copy-Item -LiteralPath $image -Destination (Join-Path $directory.FullName 'frame.pgm')
$manifest.frames = @(for ($index = 0; $index -lt $Frames; $index++) {
    @{ file='frame.pgm'; id=$index; timestamp=($index * 10000); truth=$entry.truth }
})
[System.IO.File]::WriteAllText((Join-Path $directory.FullName 'sequence.json'),
    ($manifest | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
Write-Output (Join-Path $directory.FullName 'sequence.json')
