# Generate the AgentToast application and tray icons.
#
# Tray icons are drawn at 16-24px against a taskbar that may be light or dark,
# and Windows does not recolour them. A dark shape therefore disappears into a
# dark taskbar, which is exactly what the old placeholder did. The mark here is
# a mid-tone indigo card with light bars, so it keeps contrast either way.

Add-Type -AssemblyName System.Drawing

$iconDir = Join-Path $PSScriptRoot ".." | Join-Path -ChildPath "src-tauri" | Join-Path -ChildPath "icons"
New-Item -ItemType Directory -Path $iconDir -Force | Out-Null

# Accent indigo, between the light and dark theme accents so one mark works on
# both. The bars are near-white for maximum contrast against it.
$accent = [System.Drawing.Color]::FromArgb(255, 91, 110, 225)
$bar    = [System.Drawing.Color]::FromArgb(255, 245, 246, 252)

function New-RoundedPath {
    param([single]$X, [single]$Y, [single]$W, [single]$H, [single]$R)
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $R * 2
    $path.AddArc($X, $Y, $d, $d, 180, 90)
    $path.AddArc($X + $W - $d, $Y, $d, $d, 270, 90)
    $path.AddArc($X + $W - $d, $Y + $H - $d, $d, $d, 0, 90)
    $path.AddArc($X, $Y + $H - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    return $path
}

function New-Icon {
    param([int]$Size)

    $bmp = New-Object System.Drawing.Bitmap($Size, $Size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    # The card fills almost the whole canvas: at 16px every pixel counts.
    $pad = [single]($Size * 0.06)
    $side = [single]($Size - ($pad * 2))
    $radius = [single]([Math]::Max(2, $Size * 0.22))

    $card = New-RoundedPath -X $pad -Y $pad -W $side -H $side -R $radius
    $brush = New-Object System.Drawing.SolidBrush($accent)
    $g.FillPath($brush, $card)

    # Two bars reading as a notification card. Below ~20px the second bar turns
    # to mush, so draw one bolder bar instead.
    $barBrush = New-Object System.Drawing.SolidBrush($bar)
    $left = [single]($Size * 0.28)
    $width = [single]($Size * 0.44)

    if ($Size -ge 20) {
        $h1 = [single][Math]::Max(1, $Size * 0.09)
        $h2 = [single][Math]::Max(1, $Size * 0.09)
        $r1 = [single]($h1 / 2)
        $top = New-RoundedPath -X $left -Y ([single]($Size * 0.34)) -W $width -H $h1 -R $r1
        $bot = New-RoundedPath -X $left -Y ([single]($Size * 0.55)) -W ([single]($width * 0.62)) -H $h2 -R $r1
        $g.FillPath($barBrush, $top)
        $g.FillPath($barBrush, $bot)
        $top.Dispose(); $bot.Dispose()
    } else {
        $h = [single][Math]::Max(2, $Size * 0.16)
        $one = New-RoundedPath -X $left -Y ([single](($Size - $h) / 2)) -W $width -H $h -R ([single]($h / 2))
        $g.FillPath($barBrush, $one)
        $one.Dispose()
    }

    $card.Dispose(); $brush.Dispose(); $barBrush.Dispose(); $g.Dispose()
    return $bmp
}

# --- PNGs -------------------------------------------------------------------

$pngSizes = @{ "icon.png" = 32; "32x32.png" = 32; "128x128.png" = 128; "[email protected]" = 256 }
foreach ($name in $pngSizes.Keys) {
    $bmp = New-Icon -Size $pngSizes[$name]
    $path = Join-Path $iconDir $name
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "wrote $name ($($pngSizes[$name])x$($pngSizes[$name]))"
}

# --- Multi-size .ico --------------------------------------------------------
#
# Built by hand because System.Drawing cannot save a multi-resolution icon.
# Vista and later accept PNG-compressed entries, so each size is embedded as a
# PNG rather than a BMP.

$icoSizes = @(16, 24, 32, 48, 64, 128, 256)
$images = @()
foreach ($size in $icoSizes) {
    $bmp = New-Icon -Size $size
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $images += , @{ Size = $size; Bytes = $ms.ToArray() }
    $ms.Dispose(); $bmp.Dispose()
}

$icoPath = Join-Path $iconDir "icon.ico"
$fs = [System.IO.File]::Create($icoPath)
$bw = New-Object System.IO.BinaryWriter($fs)

$bw.Write([uint16]0)                    # reserved
$bw.Write([uint16]1)                    # type: icon
$bw.Write([uint16]$images.Count)

# Directory entries come first, so image data starts after all of them.
$offset = 6 + (16 * $images.Count)
foreach ($img in $images) {
    # 256 is encoded as 0 in the directory.
    $dim = if ($img.Size -ge 256) { 0 } else { $img.Size }
    $bw.Write([byte]$dim)               # width
    $bw.Write([byte]$dim)               # height
    $bw.Write([byte]0)                  # palette size
    $bw.Write([byte]0)                  # reserved
    $bw.Write([uint16]1)                # colour planes
    $bw.Write([uint16]32)               # bits per pixel
    $bw.Write([uint32]$img.Bytes.Length)
    $bw.Write([uint32]$offset)
    $offset += $img.Bytes.Length
}
foreach ($img in $images) { $bw.Write($img.Bytes) }

$bw.Flush(); $bw.Dispose(); $fs.Dispose()
Write-Host "wrote icon.ico ($($images.Count) sizes: $($icoSizes -join ', '))"
