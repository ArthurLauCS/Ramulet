# Fetches the two bundled open-source fonts into assets/fonts (not in git; about 2.8 MB).
# Run once before a dev run and before building the installer. Files already present with the
# right hash are skipped. Both fonts are SIL OFL 1.1 (see THIRD_PARTY_NOTICES.txt).
# Uses curl.exe because Windows PowerShell 5.1 Invoke-WebRequest fails on GitHub release assets here.
$dir = "$PSScriptRoot\..\assets\fonts"
New-Item -ItemType Directory -Force $dir | Out-Null
$fonts = @(
    # repo names: Smiley Sans (single style, slanted by design), atelier-anchor/smiley-sans v2.0.1
    @{ Name = 'SmileySans-Oblique.ttf'; Entry = 'SmileySans-Oblique.ttf'
       Url = 'https://github.com/atelier-anchor/smiley-sans/releases/download/v2.0.1/smiley-sans-v2.0.1.zip'
       ZipSha256 = '299c0be6c960ae37361762eca76f7d0cd516615435bb96c0d4b98a1e70178a07'
       Sha256 = 'b447d7e781f08bc95c4c9f23ba71ed2b8ebb639aa7184485c71c4ca5afcd25c4' },
    # branch names: IBM Plex Mono Regular, IBM/plex @ibm/plex-mono@2.5.0
    @{ Name = 'IBMPlexMono-Regular.ttf'; Entry = 'ibm-plex-mono/fonts/complete/ttf/IBMPlexMono-Regular.ttf'
       Url = 'https://github.com/IBM/plex/releases/download/%40ibm/plex-mono%402.5.0/ibm-plex-mono.zip'
       ZipSha256 = '6d23f01257663d8cc49a0d64c22ced630b79e0e2a0ac08a0da86e9a38bbc481c'
       Sha256 = '7c6fbddca4b700be918f5f6183d9bd4464fa427fe435f0b480d77fe2bb8c5a43' }
)
Add-Type -AssemblyName System.IO.Compression.FileSystem
foreach ($f in $fonts) {
    $path = Join-Path $dir $f.Name
    if ((Test-Path $path) -and (Get-FileHash $path -Algorithm SHA256).Hash -eq $f.Sha256) { continue }
    $zip = Join-Path ([IO.Path]::GetTempPath()) ("ramulet-font-" + [guid]::NewGuid().ToString('N') + '.zip')
    try {
        & curl.exe -sSL --fail -o $zip $f.Url
        if ($LASTEXITCODE) { throw "download failed: $($f.Url)" }
        $zh = (Get-FileHash $zip -Algorithm SHA256).Hash
        if ($zh -ne $f.ZipSha256) { throw "$($f.Url) SHA-256 mismatch: $zh" }
        $za = [IO.Compression.ZipFile]::OpenRead($zip)
        try {
            $e = $za.GetEntry($f.Entry)
            if (-not $e) { throw "$($f.Entry) not in archive" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($e, $path, $true)
        } finally { $za.Dispose() }
    } finally { if (Test-Path $zip) { [IO.File]::Delete($zip) } }
    $hash = (Get-FileHash $path -Algorithm SHA256).Hash
    if ($hash -ne $f.Sha256) { [IO.File]::Delete($path); throw "$($f.Name) SHA-256 mismatch: $hash" }
}
