# Generate placeholder icons for AgentToast
# Run this script to create basic icon files for development

$iconDir = Join-Path $PSScriptRoot ".." "src-tauri" "icons"
New-Item -ItemType Directory -Path $iconDir -Force | Out-Null

# Create a simple 32x32 PNG (minimal valid PNG with a toast emoji concept)
# For now, we'll use a 1x1 transparent PNG as placeholder
# Replace with proper icons before release

Add-Type -AssemblyName System.Drawing

function New-PlaceholderIcon {
    param([string]$Path, [int]$Size)
    
    $bitmap = New-Object System.Drawing.Bitmap($Size, $Size)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    
    # Dark background circle
    $bgBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 30, 30, 46))
    $graphics.FillEllipse($bgBrush, 0, 0, $Size-1, $Size-1)
    
    # Orange accent (toast!)
    $accentBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 250, 179, 135))
    $margin = [int]($Size * 0.25)
    $innerSize = $Size - (2 * $margin)
    $graphics.FillRectangle($accentBrush, $margin, $margin, $innerSize, [int]($innerSize * 0.6))
    
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    
    Write-Host "Created: $Path ($Size x $Size)"
}

# Generate PNGs
New-PlaceholderIcon -Path (Join-Path $iconDir "icon.png") -Size 64
New-PlaceholderIcon -Path (Join-Path $iconDir "32x32.png") -Size 32
New-PlaceholderIcon -Path (Join-Path $iconDir "128x128.png") -Size 128

# Generate ICO from 32x32
$bitmap32 = New-Object System.Drawing.Bitmap(32, 32)
$g = [System.Drawing.Graphics]::FromImage($bitmap32)
$bgBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 30, 30, 46))
$g.FillEllipse($bgBrush, 0, 0, 31, 31)
$accentBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 250, 179, 135))
$g.FillRectangle($accentBrush, 8, 8, 16, 10)
$icon = [System.Drawing.Icon]::FromHandle($bitmap32.GetHicon())
$fs = [System.IO.FileStream]::new((Join-Path $iconDir "icon.ico"), [System.IO.FileMode]::Create)
$icon.Save($fs)
$fs.Close()
$g.Dispose()
$bitmap32.Dispose()
Write-Host "Created: icon.ico"

Write-Host "`nAll placeholder icons generated. Replace with proper branding before release."
