# Рисует картинки установщика из логотипа (docs/assets/logo.svg): боковую полосу
# app/installer/sidebar.bmp (164x314, страницы «Добро пожаловать» и «Готово») и шапку
# app/installer/header.bmp (150x57, остальные страницы и удаление).
# NSIS принимает только BMP, поэтому картинки лежат в репозитории готовыми, а этот скрипт
# нужен, чтобы перерисовать их после смены логотипа или цвета:
#
#   pwsh -File tools/installer-art.ps1
#
# Геометрия логотипа - те же прямоугольники, что в logo.svg (поле 512x512), цвета - из ui/app.css.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
Add-Type -AssemblyName System.Drawing

$root = Split-Path $PSScriptRoot -Parent
$out = Join-Path $root 'app\installer'

$accent = [Drawing.ColorTranslator]::FromHtml('#3a61e0')
$accentInk = [Drawing.ColorTranslator]::FromHtml('#2f54c8')
$amber = [Drawing.ColorTranslator]::FromHtml('#ffb020')
$white = [Drawing.Color]::White

function New-RoundRect([single]$x, [single]$y, [single]$w, [single]$h, [single]$r) {
  $p = New-Object Drawing.Drawing2D.GraphicsPath
  $d = 2 * $r
  $p.AddArc($x, $y, $d, $d, 180, 90)
  $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
  $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
  $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
  $p.CloseFigure()
  return $p
}

# логотип в квадрате size x size с левым верхним углом (ox, oy); $plate - рисовать ли синюю плашку
function Draw-Logo($g, [single]$ox, [single]$oy, [single]$size, [bool]$plate) {
  $k = $size / 512
  $rect = { param($x, $y, $w, $h, $r, $color)
    $b = New-Object Drawing.SolidBrush $color
    $g.FillPath($b, (New-RoundRect ($ox + $x * $k) ($oy + $y * $k) ($w * $k) ($h * $k) ($r * $k)))
    $b.Dispose()
  }
  if ($plate) { & $rect 16 16 480 480 112 $accent }
  & $rect 112 144 64 240 14 $white
  & $rect 112 144 80 64 14 $white
  & $rect 336 144 64 240 14 $white
  & $rect 320 144 80 64 14 $white
  & $rect 208 128 96 64 14 $amber
}

function Save-Bmp($bmp, $name) {
  # 24 бита без прозрачности - так NSIS показывает картинку одинаково во всех темах
  $flat = New-Object Drawing.Bitmap $bmp.Width, $bmp.Height, ([Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $g = [Drawing.Graphics]::FromImage($flat)
  $g.DrawImage($bmp, 0, 0, $bmp.Width, $bmp.Height)
  $g.Dispose()
  $path = Join-Path $out $name
  $flat.Save($path, [Drawing.Imaging.ImageFormat]::Bmp)
  $flat.Dispose()
  Write-Output "$path"
}

function New-Canvas([int]$w, [int]$h) {
  $bmp = New-Object Drawing.Bitmap $w, $h
  $g = [Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
  $g.TextRenderingHint = [Drawing.Text.TextRenderingHint]::AntiAliasGridFit
  $g.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  return @($bmp, $g)
}

# боковая полоса: синий фон с лёгким переходом, логотип без плашки, название и строка о деле
$bmp, $g = New-Canvas 164 314
$grad = New-Object Drawing.Drawing2D.LinearGradientBrush (New-Object Drawing.Rectangle -1, -1, 166, 316), $accent, $accentInk, 90
$g.FillRectangle($grad, 0, 0, 164, 314)
Draw-Logo $g 26 34 112 $false
$center = New-Object Drawing.StringFormat
$center.Alignment = [Drawing.StringAlignment]::Center
$title = New-Object Drawing.Font 'Segoe UI Semibold', 17, ([Drawing.FontStyle]::Regular), ([Drawing.GraphicsUnit]::Pixel)
$small = New-Object Drawing.Font 'Segoe UI', 11, ([Drawing.FontStyle]::Regular), ([Drawing.GraphicsUnit]::Pixel)
$g.DrawString('Пульт', $title, (New-Object Drawing.SolidBrush $white), (New-Object Drawing.RectangleF 0, 160, 164, 26), $center)
$soft = New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb(215, 255, 255, 255))
$g.DrawString("команды любого`nпроекта простыми`nсловами", $small, $soft, (New-Object Drawing.RectangleF 0, 192, 164, 60), $center)
$g.Dispose()
Save-Bmp $bmp 'sidebar.bmp'
$bmp.Dispose()

# шапка: белое поле и логотип на плашке; NSIS ставит её слева, а заголовок страницы пишет правее
$bmp, $g = New-Canvas 150 57
$g.Clear($white)
Draw-Logo $g 54 8 41 $true
$g.Dispose()
Save-Bmp $bmp 'header.bmp'
$bmp.Dispose()
