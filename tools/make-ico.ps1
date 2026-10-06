param([string]$Dir)
Add-Type -AssemblyName System.Drawing

$src = [System.Drawing.Image]::FromFile((Join-Path $Dir 'novars-512.png'))

function ScaleToPngBytes($img, [int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.DrawImage($img, 0, 0, $size, $size)
    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return ,$ms.ToArray()
}

$sizes = @(256, 48, 32, 16)
$data = @()
foreach ($s in $sizes) { $data += , (ScaleToPngBytes $src $s) }
$src.Dispose()

$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($ms)
$bw.Write([UInt16]0)                     # reserved
$bw.Write([UInt16]1)                     # type: icon
$bw.Write([UInt16]$sizes.Count)          # image count
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $b = $data[$i]
    $w = if ($s -ge 256) { 0 } else { $s }
    $bw.Write([byte]$w)                 # width (0 = 256)
    $bw.Write([byte]$w)                 # height
    $bw.Write([byte]0)                  # palette
    $bw.Write([byte]0)                  # reserved
    $bw.Write([UInt16]1)                # planes
    $bw.Write([UInt16]32)               # bpp
    $bw.Write([UInt32]$b.Length)        # data size
    $bw.Write([UInt32]$offset)          # data offset
    $offset += $b.Length
}
foreach ($b in $data) { $bw.Write($b) }
$bw.Flush()

$outPath = Join-Path $Dir 'novars.ico'
[System.IO.File]::WriteAllBytes($outPath, $ms.ToArray())
$bw.Dispose()
Write-Output ('novars.ico written: ' + (Get-Item $outPath).Length + ' bytes')
