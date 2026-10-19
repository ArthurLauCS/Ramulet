# Build assets/app.ico and assets/app.res from assets/icon.svg (40px and up) and assets/icon-small.svg (16-32px).
# Every size is rasterized on its own by headless Chrome (small icons are drawn, not shrunk), cropped with System.Drawing,
# packed as PNG entries into the .ico, then compiled by rc.exe. Run after editing the SVGs: powershell -File scripts/icon.ps1
# Preview (light and dark rows) goes to target/icon-preview.png.
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $root "assets"
$chrome = "C:\Program Files\Google\Chrome\Application\chrome.exe"
$sizes = 16, 20, 24, 32, 40, 48, 64, 256

# All sizes side by side on one page, one screenshot, then crop by position
$x = 0; $cells = ""
foreach ($s in $sizes) {
    $svg = if ($s -le 32) { "icon-small.svg" } else { "icon.svg" }
    $uri = ([Uri](Join-Path $assets $svg)).AbsoluteUri
    $cells += "<img src='$uri' style='position:absolute;left:${x}px;top:0;width:${s}px;height:${s}px'>"
    $x += $s + 8
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("ramulet-icon-" + [Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
$page = Join-Path $tmp "sheet.html"; $shot = Join-Path $tmp "sheet.png"
[IO.File]::WriteAllText($page, "<body style='margin:0'>$cells</body>")
$max = ($sizes | Measure-Object -Maximum).Maximum
& $chrome --headless=new --hide-scrollbars --force-device-scale-factor=1 --default-background-color=00000000 "--window-size=$x,$max" "--screenshot=$shot" ([Uri]$page).AbsoluteUri 2>$null | Out-Null
$sheet = [System.Drawing.Bitmap]::FromFile($shot)

$pngs = @(); $x = 0
foreach ($s in $sizes) {
    $bmp = $sheet.Clone((New-Object System.Drawing.Rectangle $x, 0, $s, $s), [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $ms = New-Object IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $pngs += , @($s, $ms.ToArray(), $bmp)
    $x += $s + 8
}
$sheet.Dispose()

# ICO: 6-byte header, 16-byte directory entry per image, then the PNG data
$ico = New-Object IO.MemoryStream
$w = New-Object IO.BinaryWriter $ico
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$pngs.Count)
$offset = 6 + 16 * $pngs.Count
foreach ($p in $pngs) {
    $w.Write([byte]($p[0] % 256)); $w.Write([byte]($p[0] % 256)); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$p[1].Length); $w.Write([uint32]$offset)
    $offset += $p[1].Length
}
foreach ($p in $pngs) { $w.Write($p[1]) }
[IO.File]::WriteAllBytes((Join-Path $assets "app.ico"), $ico.ToArray())

# Preview on light and dark backgrounds, to check edges and small-size sharpness
$pw = [int]($sizes | ForEach-Object { $_ + 16 } | Measure-Object -Sum).Sum
$prev = New-Object System.Drawing.Bitmap $pw, ([int](2 * ($max + 16)))
$g = [System.Drawing.Graphics]::FromImage($prev)
$g.Clear([System.Drawing.Color]::FromArgb(0xF3, 0xF4, 0xF6))
$g.FillRectangle((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(0x1F, 0x29, 0x37))), 0, $max + 16, $pw, $max + 16)
$x = 8
foreach ($p in $pngs) {
    foreach ($row in 0, 1) { $g.DrawImageUnscaled($p[2], $x, 8 + $row * ($max + 16)) }
    $x += $p[0] + 16
    $p[2].Dispose()
}
New-Item -ItemType Directory -Force (Join-Path $root "target") | Out-Null
$prev.Save((Join-Path $root "target\icon-preview.png")); $g.Dispose(); $prev.Dispose()

# Resource id 1: Explorer and LoadIcon take the icon group with the lowest id
$rc = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\rc.exe" | Sort-Object FullName | Select-Object -Last 1
[IO.File]::WriteAllText((Join-Path $assets "app.rc"), "1 ICON `"app.ico`"`r`n")
Push-Location $assets
& $rc.FullName /nologo /fo app.res app.rc
Pop-Location
[IO.File]::Delete((Join-Path $assets "app.rc"))
[IO.Directory]::Delete($tmp, $true)
