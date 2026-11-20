# DREAD — installs the bundled Virtual Display Driver (IddCx, hardware id Root\MttVDD).
# Run elevated. Called by the DREAD installer; can be run by hand:
#   powershell -ExecutionPolicy Bypass -File install-driver.ps1 -DriverDir .\driver
# STATUS: not yet run on Windows (see docs/PLATFORMS.md).
param([string]$DriverDir = "$PSScriptRoot\driver", [string]$Nefcon = "$PSScriptRoot\nefconw.exe")
$ErrorActionPreference = "Stop"
$Target = "C:\VirtualDisplayDriver"
$DisplayClass = "{4d36e968-e325-11ce-bfc1-08002be10318}"

New-Item -ItemType Directory -Force -Path $Target | Out-Null
Copy-Item -Force "$PSScriptRoot\vdd_settings.xml" "$Target\vdd_settings.xml"

# Trust the driver's publisher certificate (as the driver's own installer does)
$cat = Join-Path $DriverDir "mttvdd.cat"
if (Test-Path $cat) {
    $certs = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2Collection
    $certs.Import($cat)
    foreach ($c in $certs) {
        $store = New-Object System.Security.Cryptography.X509Certificates.X509Store("TrustedPublisher", "LocalMachine")
        $store.Open("ReadWrite"); $store.Add($c); $store.Close()
    }
}

$existing = Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.HardwareID -contains "Root\MttVDD" }
if (-not $existing) {
    & $Nefcon --create-device-node --hardware-id "Root\MttVDD" --class-name Display --class-guid $DisplayClass
    if ($LASTEXITCODE -ne 0) { throw "creating the virtual display device failed ($LASTEXITCODE)" }
}
& $Nefcon --install-driver --inf-path (Join-Path $DriverDir "MttVDD.inf")
if ($LASTEXITCODE -ne 0) { throw "installing the virtual display driver failed ($LASTEXITCODE)" }
Write-Host "DREAD virtual display driver installed."
