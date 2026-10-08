# Draws the Scratchpad app icon (the shapes in scratchpad.svg) at every size Windows asks for
# and writes them into one PNG-compressed .ico. Needs only Windows PowerShell + System.Drawing.
#
#   powershell -File assets/icons/generate-icon.ps1 [-PreviewDir <dir>]
#
# -PreviewDir also writes one PNG per size, for checking small sizes by eye.
param([string]$PreviewDir)

Add-Type -AssemblyName System.Drawing

$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$out = Join-Path $PSScriptRoot 'scratchpad.ico'

function Rgb([string]$hex) { [System.Drawing.ColorTranslator]::FromHtml($hex) }

# Text lines as (x, y, width, height, colour). Sizes from 40 px up use the SVG's 256-unit grid.
# At 32 px and below anti-aliased lines would blur into grey, so those sizes get a heading and
# two lines laid out on whole pixels instead.
$vectorLines = @(@(90, 92, 70, 14, 'ink'), @(90, 126, 76, 10, 'body'), @(90, 150, 76, 10, 'body'), @(90, 174, 44, 10, 'body'))
$pixelLines = @{
    16 = @(@(5, 5, 5, 2, 'ink'), @(5, 8, 6, 1, 'body'), @(5, 10, 3, 1, 'body'))
    20 = @(@(6, 6, 6, 2, 'ink'), @(6, 9, 8, 1, 'body'), @(6, 12, 4, 1, 'body'))
    24 = @(@(8, 7, 7, 2, 'ink'), @(8, 11, 8, 1, 'body'), @(8, 14, 5, 1, 'body'))
    32 = @(@(11, 11, 9, 3, 'ink'), @(11, 16, 10, 2, 'body'), @(11, 20, 6, 2, 'body'))
}

function RoundedRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 2 * $r
    $path.AddArc($x, $y, $d, $d, 180, 90)
    $path.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $path.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $path.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    $path
}

function DrawIcon([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.ScaleTransform($size / 256.0, $size / 256.0)

    $tile = RoundedRect 6 6 244 244 56
    $gradient = New-Object System.Drawing.Drawing2D.LinearGradientBrush `
        (New-Object System.Drawing.PointF 0, 6), (New-Object System.Drawing.PointF 0, 250), (Rgb '#f7c555'), (Rgb '#d99a00')
    $g.FillPath($gradient, $tile)

    # The sheet: a rounded rectangle with its top-right corner cut off, and the folded flap.
    $sheet = New-Object System.Drawing.Drawing2D.GraphicsPath
    $sheet.AddArc(64, 44, 28, 28, 180, 90)
    $sheet.AddLine(148, 44, 192, 88)
    $sheet.AddLine(192, 88, 192, 198)
    $sheet.AddArc(164, 184, 28, 28, 0, 90)
    $sheet.AddArc(64, 184, 28, 28, 90, 90)
    $sheet.CloseFigure()
    $g.FillPath((New-Object System.Drawing.SolidBrush (Rgb '#ffffff')), $sheet)
    $flap = [System.Drawing.PointF[]]@((New-Object System.Drawing.PointF 148, 44), (New-Object System.Drawing.PointF 192, 88), (New-Object System.Drawing.PointF 148, 88))
    $g.FillPolygon((New-Object System.Drawing.SolidBrush (Rgb '#ecdcae')), $flap)

    if ($pixelLines.ContainsKey($size)) {
        $colors = @{ ink = Rgb '#3a3326'; body = Rgb '#a39a82' }
        $g.ResetTransform()
        $g.SmoothingMode = 'None'
        foreach ($line in $pixelLines[$size]) {
            $g.FillRectangle((New-Object System.Drawing.SolidBrush $colors[$line[4]]), $line[0], $line[1], $line[2], $line[3])
        }
    } else {
        $colors = @{ ink = Rgb '#3a3326'; body = Rgb '#b9b2a0' }
        foreach ($line in $vectorLines) {
            $path = RoundedRect $line[0] $line[1] $line[2] $line[3] ($line[3] / 2.0)
            $g.FillPath((New-Object System.Drawing.SolidBrush $colors[$line[4]]), $path)
        }
    }
    $g.Dispose()
    $bmp
}

$images = foreach ($size in $sizes) {
    $bmp = DrawIcon $size
    $stream = New-Object System.IO.MemoryStream
    $bmp.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    if ($PreviewDir) {
        New-Item -ItemType Directory -Force $PreviewDir | Out-Null
        $bmp.Save((Join-Path $PreviewDir "icon-$size.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    }
    $bmp.Dispose()
    [pscustomobject]@{ Size = $size; Png = $stream.ToArray() }
}

# ICONDIR, then one ICONDIRENTRY per image, then the PNG payloads.
$file = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $file
$w.Write([uint16]0)
$w.Write([uint16]1)
$w.Write([uint16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($image in $images) {
    $dimension = if ($image.Size -ge 256) { 0 } else { $image.Size }  # 0 means 256
    $w.Write([byte]$dimension)
    $w.Write([byte]$dimension)
    $w.Write([byte]0)       # palette colours
    $w.Write([byte]0)       # reserved
    $w.Write([uint16]1)     # colour planes
    $w.Write([uint16]32)    # bits per pixel
    $w.Write([uint32]$image.Png.Length)
    $w.Write([uint32]$offset)
    $offset += $image.Png.Length
}
foreach ($image in $images) { $w.Write($image.Png) }
$w.Flush()
[System.IO.File]::WriteAllBytes($out, $file.ToArray())
Write-Host "Wrote $out ($($file.Length) bytes, sizes: $($sizes -join ', '))"
