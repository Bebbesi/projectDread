# DREAD — builds dist\DREAD-Setup-<version>.exe on Windows 10/11 (x64).
# Needs: Rust (MSVC toolchain), Inno Setup 6 (ISCC.exe on PATH), internet access.
# Third-party components fetched (pinned):
#   Virtual Display Driver 25.7.23 (MIT)  https://github.com/VirtualDrivers/Virtual-Display-Driver
#   nefcon 1.14.0 (BSD-3-Clause)          https://github.com/nefarius/nefcon
#   FFmpeg essentials build (GPL-3.0)     https://www.gyan.dev/ffmpeg/builds/
# STATUS: not yet run on Windows (see docs/PLATFORMS.md).
$ErrorActionPreference = "Stop"
$Root = Resolve-Path "$PSScriptRoot\..\.."
$Stage = "$Root\build\windows"
$Dl = "$Root\build\downloads"
New-Item -ItemType Directory -Force -Path $Stage, $Dl, "$Stage\licenses" | Out-Null

Push-Location "$Root\server"
cargo build --release -p dread-app
Pop-Location
Copy-Item -Force "$Root\server\target\release\dread.exe" "$Stage\dread.exe"

function Fetch($url, $file) { if (-not (Test-Path $file)) { Invoke-WebRequest -Uri $url -OutFile $file } }

Fetch "https://github.com/VirtualDrivers/Virtual-Display-Driver/releases/download/25.7.23/VirtualDisplayDriver-x86.Driver.Only.zip" "$Dl\vdd.zip"
Expand-Archive -Force "$Dl\vdd.zip" "$Dl\vdd"
$inf = Get-ChildItem -Recurse "$Dl\vdd" -Filter MttVDD.inf | Select-Object -First 1
if (-not $inf) { throw "MttVDD.inf not found in the driver archive" }
New-Item -ItemType Directory -Force "$Stage\driver" | Out-Null
Copy-Item -Force "$($inf.DirectoryName)\*" "$Stage\driver\"

Fetch "https://github.com/nefarius/nefcon/releases/download/v1.14.0/nefcon_v1.14.0.zip" "$Dl\nefcon.zip"
Expand-Archive -Force "$Dl\nefcon.zip" "$Dl\nefcon"
Copy-Item -Force (Get-ChildItem -Recurse "$Dl\nefcon" -Filter nefconw.exe | Where-Object { $_.FullName -match "x64" } | Select-Object -First 1).FullName "$Stage\nefconw.exe"

Fetch "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" "$Dl\ffmpeg.zip"
Expand-Archive -Force "$Dl\ffmpeg.zip" "$Dl\ffmpeg"
Copy-Item -Force (Get-ChildItem -Recurse "$Dl\ffmpeg" -Filter ffmpeg.exe | Select-Object -First 1).FullName "$Stage\ffmpeg.exe"

if (Test-Path "$Root\LICENSE") { Copy-Item -Force "$Root\LICENSE" "$Stage\licenses\DREAD-LICENSE.txt" }
Get-ChildItem -Recurse "$Dl\vdd" -Filter LICENSE* | Select-Object -First 1 | ForEach-Object { Copy-Item $_.FullName "$Stage\licenses\VirtualDisplayDriver-LICENSE.txt" }
Get-ChildItem -Recurse "$Dl\ffmpeg" -Filter LICENSE* | Select-Object -First 1 | ForEach-Object { Copy-Item $_.FullName "$Stage\licenses\FFmpeg-LICENSE.txt" }

ISCC.exe "$PSScriptRoot\dread.iss"
Write-Host "Installer written to $Root\dist"
