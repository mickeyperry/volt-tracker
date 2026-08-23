# Installer artwork for the desktop build, drawn from VOLT's own palette.
#
# NSIS (the installer Tauri bundles) wants two BITMAPS and they are fussy: 24-bit BMP, exact
# sizes, no alpha. A PNG here silently produces a blank panel, which is why these are generated
# rather than exported by hand from something.
#
#   header.bmp   150 x 57   the strip along the top of every page after the first
#   sidebar.bmp  164 x 314  the panel down the left of the welcome and finish pages
#
# Run:  powershell -ExecutionPolicy Bypass -File tools/mkinstaller-art.ps1
Add-Type -AssemblyName System.Drawing

$out = Join-Path $PSScriptRoot "..\native\desktop\icons"
$null = New-Item -ItemType Directory -Force -Path $out

# VOLT's palette, straight off :root in beta2.html — the installer should look like the app.
$bg    = [System.Drawing.Color]::FromArgb(10, 12, 18)    # --bg    #0a0c12
$panel = [System.Drawing.Color]::FromArgb(16, 20, 29)    # --panel #10141d
$teal  = [System.Drawing.Color]::FromArgb(63, 224, 180)  # --teal  #3fe0b4
$mag   = [System.Drawing.Color]::FromArgb(255, 77, 157)  # --mag   #ff4d9d
$blue  = [System.Drawing.Color]::FromArgb(106, 174, 255) # --blue  #6aaeff
$dim   = [System.Drawing.Color]::FromArgb(150, 160, 178)

# The bolt from the app icon, as a path in a 0..1 box so it can be drawn at any size.
$boltPts = @(
  @(0.62, 0.03), @(0.24, 0.55), @(0.45, 0.55), @(0.34, 0.97), @(0.78, 0.42), @(0.55, 0.42)
)
function Draw-Bolt {
  param($g, [float]$x, [float]$y, [float]$w, [float]$h, $color, [single]$glow = 0)
  $pts = foreach ($p in $boltPts) {
    New-Object System.Drawing.PointF(($x + $p[0] * $w), ($y + $p[1] * $h))
  }
  if ($glow -gt 0) {
    # a cheap glow: the same shape, larger and translucent, drawn underneath
    $gb = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(38, $color))
    $cx = $x + $w / 2; $cy = $y + $h / 2
    $big = foreach ($p in $pts) {
      New-Object System.Drawing.PointF(($cx + ($p.X - $cx) * 1.22), ($cy + ($p.Y - $cy) * 1.22))
    }
    $g.FillPolygon($gb, [System.Drawing.PointF[]]$big)
    $gb.Dispose()
  }
  $b = New-Object System.Drawing.SolidBrush $color
  $g.FillPolygon($b, [System.Drawing.PointF[]]$pts)
  $b.Dispose()
}

function New-Canvas {
  param([int]$w, [int]$h)
  $bmp = New-Object System.Drawing.Bitmap($w, $h, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
  $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::ClearTypeGridFit
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  return @($bmp, $g)
}

# ---- header: 150 x 57 ------------------------------------------------------------------------
$c = New-Canvas 150 57
$bmp = $c[0]; $g = $c[1]
$rect = New-Object System.Drawing.Rectangle(0, 0, 150, 57)
$grad = New-Object System.Drawing.Drawing2D.LinearGradientBrush($rect, $panel, $bg, 15.0)
$g.FillRectangle($grad, $rect); $grad.Dispose()
Draw-Bolt $g 6 9 22 39 $teal 1
$f1 = New-Object System.Drawing.Font("Consolas", 11, [System.Drawing.FontStyle]::Bold)
$f2 = New-Object System.Drawing.Font("Consolas", 7, [System.Drawing.FontStyle]::Bold)
$bDim = New-Object System.Drawing.SolidBrush $dim
$bTeal = New-Object System.Drawing.SolidBrush $teal
$bMag = New-Object System.Drawing.SolidBrush $mag
# MEASURED, not guessed: the two halves of the wordmark are different colours, so they are two
# draws, and "volt" at 13pt is wider than the gap I first left for it — they overlapped.
$fmt = [System.Drawing.StringFormat]::GenericTypographic
$x = 32.0
$g.DrawString("volt", $f1, $bDim, $x, 11.0, $fmt)
$x += $g.MeasureString("volt", $f1, 999, $fmt).Width
$g.DrawString("Desktop", $f1, $bTeal, $x, 11.0, $fmt)
$g.DrawString("BETA", $f2, $bMag, 33.0, 32.0, $fmt)
# the accent rule along the bottom, teal → blue → magenta, as the dialogs have
$r2 = New-Object System.Drawing.Rectangle(0, 55, 150, 2)
$g2 = New-Object System.Drawing.Drawing2D.LinearGradientBrush($r2, $teal, $mag, 0.0)
$g.FillRectangle($g2, $r2); $g2.Dispose()
$bmp.Save((Join-Path $out "header.bmp"), [System.Drawing.Imaging.ImageFormat]::Bmp)
$g.Dispose(); $bmp.Dispose()

# ---- sidebar: 164 x 314 ----------------------------------------------------------------------
$c = New-Canvas 164 314
$bmp = $c[0]; $g = $c[1]
$rect = New-Object System.Drawing.Rectangle(0, 0, 164, 314)
$grad = New-Object System.Drawing.Drawing2D.LinearGradientBrush($rect, $panel, $bg, 70.0)
$g.FillRectangle($grad, $rect); $grad.Dispose()

# a faint tracker grid behind everything — it is a tracker, after all
$penGrid = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(18, 255, 255, 255)), 1
for ($y = 18; $y -lt 314; $y += 13) { $g.DrawLine($penGrid, 0, $y, 164, $y) }
for ($x = 22; $x -lt 164; $x += 34) { $g.DrawLine($penGrid, $x, 0, $x, 314) }
$penGrid.Dispose()
# a few lit "notes" in the grid, in the app's accent colours
$cells = @(@(22, 44, $teal), @(56, 96, $mag), @(124, 70, $blue), @(90, 174, $teal), @(22, 213, $mag), @(124, 252, $teal))
foreach ($cell in $cells) {
  $b = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(150, $cell[2]))
  $g.FillRectangle($b, [int]$cell[0] + 3, [int]$cell[1] + 2, 26, 9)
  $b.Dispose()
}

Draw-Bolt $g 46 66 72 112 $teal 1

$fBig = New-Object System.Drawing.Font("Consolas", 19, [System.Drawing.FontStyle]::Bold)
$fMid = New-Object System.Drawing.Font("Consolas", 9, [System.Drawing.FontStyle]::Bold)
$fSm  = New-Object System.Drawing.Font("Consolas", 7.5, [System.Drawing.FontStyle]::Regular)
$sf = New-Object System.Drawing.StringFormat
$sf.Alignment = [System.Drawing.StringAlignment]::Center
$g.DrawString("volt", $fBig, $bDim, (New-Object System.Drawing.RectangleF(0, 196, 164, 30)), $sf)
$g.DrawString("Desktop", $fMid, $bTeal, (New-Object System.Drawing.RectangleF(0, 226, 164, 20)), $sf)
$g.DrawString("BETA", $fSm, $bMag, (New-Object System.Drawing.RectangleF(0, 244, 164, 16)), $sf)
$g.DrawString("tracker / DAW", $fSm, $bDim, (New-Object System.Drawing.RectangleF(0, 270, 164, 16)), $sf)
$r3 = New-Object System.Drawing.Rectangle(0, 312, 164, 2)
$g3 = New-Object System.Drawing.Drawing2D.LinearGradientBrush($r3, $teal, $mag, 0.0)
$g.FillRectangle($g3, $r3); $g3.Dispose()
$bmp.Save((Join-Path $out "sidebar.bmp"), [System.Drawing.Imaging.ImageFormat]::Bmp)
$g.Dispose(); $bmp.Dispose()

foreach ($b in @($bDim, $bTeal, $bMag)) { $b.Dispose() }
"wrote header.bmp (150x57) and sidebar.bmp (164x314) to $out"
