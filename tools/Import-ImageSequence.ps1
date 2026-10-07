# Convert ordered images to the monochrome replay format. Input images stay unchanged.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string[]]$Images,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(1, 1000000000)][long]$PeriodUs = 1000000
)
$ErrorActionPreference = 'Stop'
if ($Images.Count -eq 0 -or $Images.Count -gt 100000) { throw 'Specify 1 to 100000 ordered images.' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Output directory must be a new path.' }
foreach ($imagePath in $Images) {
    if (-not (Test-Path -LiteralPath $imagePath -PathType Leaf)) { throw "Missing image: $imagePath" }
}
Add-Type -AssemblyName System.Drawing
if (-not ('FlyReplay.ImageImport' -as [type])) {
    Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
namespace FlyReplay {
    public static class ImageImport {
        public static byte[] Gray(Bitmap source) {
            using (var bitmap = new Bitmap(source.Width, source.Height, PixelFormat.Format24bppRgb)) {
                using (var graphics = Graphics.FromImage(bitmap)) { graphics.DrawImageUnscaled(source, 0, 0); }
                var data = bitmap.LockBits(new Rectangle(0, 0, bitmap.Width, bitmap.Height), ImageLockMode.ReadOnly, PixelFormat.Format24bppRgb);
                try {
                    var result = new byte[checked(bitmap.Width * bitmap.Height)];
                    var row = new byte[checked(bitmap.Width * 3)];
                    for (int y = 0; y < bitmap.Height; y++) {
                        Marshal.Copy(IntPtr.Add(data.Scan0, checked(y * data.Stride)), row, 0, row.Length);
                        for (int x = 0; x < bitmap.Width; x++) {
                            int index = x * 3;
                            result[y * bitmap.Width + x] = (byte)((77 * row[index + 2] + 150 * row[index + 1] + 29 * row[index] + 128) >> 8);
                        }
                    }
                    return result;
                } finally { bitmap.UnlockBits(data); }
            }
        }
    }
}
'@
}
$directory = [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetFullPath($OutputDirectory))
$entries = [System.Collections.Generic.List[object]]::new()
$width = 0
$height = 0
for ($index = 0; $index -lt $Images.Count; $index++) {
    $bitmap = [System.Drawing.Bitmap]::new([System.IO.Path]::GetFullPath($Images[$index]))
    try {
        if ($index -eq 0) { $width = $bitmap.Width; $height = $bitmap.Height }
        if ($bitmap.Width -ne $width -or $bitmap.Height -ne $height) { throw 'All images must have the same dimensions.' }
        if ([long]$width * $height -gt 16777216) { throw 'Image exceeds the replay pixel limit.' }
        $pixels = [FlyReplay.ImageImport]::Gray($bitmap)
        $name = '{0:D6}.pgm' -f $index
        $stream = [System.IO.File]::Open((Join-Path $directory.FullName $name), [System.IO.FileMode]::CreateNew)
        try {
            $header = [System.Text.Encoding]::ASCII.GetBytes("P5`n$width $height`n255`n")
            $stream.Write($header, 0, $header.Length)
            $stream.Write($pixels, 0, $pixels.Length)
        } finally { $stream.Dispose() }
        $entries.Add(@{ file = $name; id = $index; timestamp = $index * $PeriodUs; truth = $null })
    } finally { $bitmap.Dispose() }
}
$manifest = @{ version = 1; size = @($width, $height); frames = $entries.ToArray() } | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText((Join-Path $directory.FullName 'sequence.json'), $manifest, [System.Text.UTF8Encoding]::new($false))
Write-Output "Imported $($Images.Count) images. Interval is assumed: $PeriodUs microseconds. Ground truth and safety evidence are unavailable."
Write-Output (Join-Path $directory.FullName 'sequence.json')
