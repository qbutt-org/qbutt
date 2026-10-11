param([Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$width = 900
$height = 1000
$bitmap = [Drawing.Bitmap]::new($width, $height)
$g = [Drawing.Graphics]::FromImage($bitmap)
$g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.TextRenderingHint = [Drawing.Text.TextRenderingHint]::AntiAliasGridFit
$g.Clear([Drawing.Color]::FromArgb(248,247,241))
$pen = [Drawing.Pen]::new([Drawing.Color]::FromArgb(44,48,58), 3)
$g.DrawRectangle($pen, 25, 25, 850, 950)
$g.FillEllipse([Drawing.Brushes]::White, 590, 60, 240, 420)
$g.DrawEllipse($pen, 590, 60, 240, 420)
$g.FillEllipse([Drawing.Brushes]::White, 65, 80, 415, 310)
$g.DrawEllipse($pen, 65, 80, 415, 310)
$art = [Drawing.Rectangle]::new(50, 510, 800, 420)
$gradient = [Drawing.Drawing2D.LinearGradientBrush]::new($art, [Drawing.Color]::FromArgb(25,45,85), [Drawing.Color]::FromArgb(80,40,95), 32)
$g.FillRectangle($gradient, $art)
$artPen = [Drawing.Pen]::new([Drawing.Color]::FromArgb(105,80,125), 3)
for ($n=0; $n -lt 8; ++$n) {
    $g.DrawBezier($artPen, 55, (550+$n*42), 320, (660+$n*20), 600, (480+$n*42), 845, (650+$n*35))
}
$g.FillEllipse([Drawing.Brushes]::LightGoldenrodYellow, 675, 760, 105, 105)
$bitmap.Save((Join-Path $OutputDirectory 'truth.png'), [Drawing.Imaging.ImageFormat]::Png)
$inkBitmap = [Drawing.Bitmap]::new($width, $height)
$inkG = [Drawing.Graphics]::FromImage($inkBitmap)
$inkG.Clear([Drawing.Color]::Black)
$inkG.TextRenderingHint = [Drawing.Text.TextRenderingHint]::AntiAliasGridFit
$font = [Drawing.Font]::new('Yu Gothic', 29, [Drawing.FontStyle]::Regular, [Drawing.GraphicsUnit]::Pixel)
$format = [Drawing.StringFormat]::GenericTypographic
$regions = [Collections.Generic.List[object]]::new()
function Draw-TextRegion([string]$text, [int]$x, [int]$y, [int]$columns, [bool]$vertical, [bool]$light, [object]$layout) {
    $boxes = [Collections.Generic.List[object]]::new()
    $characters = $text.ToCharArray()
    for($i=0; $i -lt $characters.Length; ++$i) {
        $xx = $x
        $yy = $y
        if ($vertical) { $xx -= [Math]::Floor($i/$columns)*42; $yy += ($i%$columns)*38 }
        else { $xx += ($i%$columns)*35; $yy += [Math]::Floor($i/$columns)*42 }
        $brush = if($light) { [Drawing.Brushes]::White } else { [Drawing.Brushes]::Black }
        $g.DrawString([string]$characters[$i], $font, $brush, [single]$xx, [single]$yy, $format)
        $inkG.DrawString([string]$characters[$i], $font, [Drawing.Brushes]::White, [single]$xx, [single]$yy, $format)
        $minX=900; $minY=1000; $maxX=-1; $maxY=-1
        for($py=[int]$yy; $py -lt [int]$yy+37; ++$py) {
            for($px=[int]$xx; $px -lt [int]$xx+34; ++$px) {
                if($inkBitmap.GetPixel($px,$py).R -gt 1) {
                    $minX=[Math]::Min($minX,$px); $maxX=[Math]::Max($maxX,$px)
                    $minY=[Math]::Min($minY,$py); $maxY=[Math]::Max($maxY,$py)
                }
            }
        }
        if($maxX -ge 0) { $boxes.Add(@{x=$minX/$width; y=$minY/$height; width=($maxX-$minX+1)/$width; height=($maxY-$minY+1)/$height}) }
    }
    $left = ($boxes | ForEach-Object {$_.x} | Measure-Object -Minimum).Minimum
    $top = ($boxes | ForEach-Object {$_.y} | Measure-Object -Minimum).Minimum
    $right = ($boxes | ForEach-Object {$_.x+$_.width} | Measure-Object -Maximum).Maximum
    $bottom = ($boxes | ForEach-Object {$_.y+$_.height} | Measure-Object -Maximum).Maximum
    $regions.Add(@{id="region-$($regions.Count+1)";text=$text;x=$left;y=$top;width=$right-$left;height=$bottom-$top;
        orientation=$(if($vertical){'vertical'}else{'horizontal'});ink=$(if($light){'light'}else{'dark'});layout=$layout;inkBoxes=@($boxes.ToArray())})
}
Draw-TextRegion 'まだ知らない世界がある。' 739 110 7 $true $false @{x=0.69;y=0.1;width=0.2;height=0.34}
Draw-TextRegion '一緒なら、きっと見つけられる。' 130 150 9 $false $false @{x=0.12;y=0.13;width=0.38;height=0.2}
Draw-TextRegion '凛との約束はここにある。' 160 640 18 $false $true @{x=0.16;y=0.61;width=0.65;height=0.12}
$bitmap.Save((Join-Path $OutputDirectory 'vertical-art.png'), [Drawing.Imaging.ImageFormat]::Png)
$inkBitmap.Save((Join-Path $OutputDirectory 'truth-ink.png'), [Drawing.Imaging.ImageFormat]::Png)
$transparent = [Drawing.Bitmap]::new($width, $height)
for($y=0; $y -lt $height; ++$y) {
    for($x=0; $x -lt $width; ++$x) {
        $transparent.SetPixel($x, $y, [Drawing.Color]::FromArgb($inkBitmap.GetPixel($x,$y).R,0,0,0))
    }
}
$transparent.Save((Join-Path $OutputDirectory 'transparent.png'), [Drawing.Imaging.ImageFormat]::Png)
$transparent.Dispose()
@{regions=@($regions.ToArray())} | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'ocr.json') -Encoding utf8
$inkG.Dispose(); $inkBitmap.Dispose(); $font.Dispose(); $gradient.Dispose(); $artPen.Dispose(); $pen.Dispose(); $g.Dispose(); $bitmap.Dispose()
