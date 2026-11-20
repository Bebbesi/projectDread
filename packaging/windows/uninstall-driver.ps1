# DREAD — removes the Virtual Display Driver installed by install-driver.ps1. Run elevated.
# STATUS: not yet run on Windows (see docs/PLATFORMS.md).
param([string]$DriverDir = "$PSScriptRoot\driver", [string]$Nefcon = "$PSScriptRoot\nefconw.exe")
$DisplayClass = "{4d36e968-e325-11ce-bfc1-08002be10318}"
& $Nefcon --remove-device-node --hardware-id "Root\MttVDD" --class-guid $DisplayClass
& $Nefcon --uninstall-driver --inf-path (Join-Path $DriverDir "MttVDD.inf")
Remove-Item -Recurse -Force "C:\VirtualDisplayDriver" -ErrorAction SilentlyContinue
Write-Host "DREAD virtual display driver removed."
