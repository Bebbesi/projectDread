# DREAD — removes the Virtual Display Driver installed by install-driver.ps1. Run elevated.
param([string]$DriverDir = "$PSScriptRoot\driver", [string]$Nefcon = "$PSScriptRoot\nefconw.exe")
$DisplayClass = "{4d36e968-e325-11ce-bfc1-08002be10318}"
# nefconw.exe is a GUI-subsystem program: wait for it explicitly.
Start-Process -FilePath $Nefcon -ArgumentList @("--remove-device-node", "--hardware-id", "Root\MttVDD", "--class-guid", $DisplayClass) -Wait -NoNewWindow
# Remove the package from the driver store (published name is oemNN.inf).
$pub = (pnputil.exe /enum-drivers | Out-String) -split "(\r?\n){2,}" | Where-Object { $_ -match "(?i)mttvdd\.inf" } |
    ForEach-Object { if ($_ -match "(?m)^[^:]*:\s+(oem\d+\.inf)") { $Matches[1] } }
foreach ($p in $pub) { & pnputil.exe /delete-driver $p /uninstall /force | Out-Host }
Remove-Item -Recurse -Force "C:\VirtualDisplayDriver" -ErrorAction SilentlyContinue
Write-Host "DREAD virtual display driver removed."
