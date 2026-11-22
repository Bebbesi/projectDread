# DREAD — installs the bundled Virtual Display Driver (IddCx, hardware id Root\MttVDD).
# Run elevated. Called by the DREAD installer; can be run by hand:
#   powershell -ExecutionPolicy Bypass -File install-driver.ps1 -DriverDir .\driver
# Writes a log next to this script (install-driver.log).
param([string]$DriverDir = "$PSScriptRoot\driver", [string]$Nefcon = "$PSScriptRoot\nefconw.exe")
$ErrorActionPreference = "Stop"
$Target = "C:\VirtualDisplayDriver"
$DisplayClass = "{4d36e968-e325-11ce-bfc1-08002be10318}"
$Inf = Join-Path $DriverDir "MttVDD.inf"
Start-Transcript -Path "$PSScriptRoot\install-driver.log" -Force | Out-Null

# nefconw.exe is a GUI-subsystem program: PowerShell's `&` does not wait for it and
# $LASTEXITCODE is not set, so always run it through Start-Process -Wait.
function Run($exe, [string[]]$argv) {
    $p = Start-Process -FilePath $exe -ArgumentList $argv -Wait -PassThru -NoNewWindow
    Write-Host "$exe $($argv -join ' ') -> exit $($p.ExitCode)"
    return $p.ExitCode
}

try {
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
        $rc = Run $Nefcon @("--create-device-node", "--hardware-id", "Root\MttVDD", "--class-name", "Display", "--class-guid", $DisplayClass)
        if ($rc -ne 0) { throw "creating the virtual display device failed ($rc)" }
    }

    # Add the package to the driver store and install it on the matching device node.
    $out = & pnputil.exe /add-driver "$Inf" /install 2>&1
    $rc = $LASTEXITCODE
    $out | Out-Host
    # 0 = ok, 3010 = ok but reboot required, 259 = already up to date
    if ($rc -ne 0 -and $rc -ne 3010 -and $rc -ne 259) { throw "installing the virtual display driver failed (pnputil exit $rc)" }

    # Make sure the device actually picked up the driver (restart it if it was created before).
    $dev = Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.HardwareID -contains "Root\MttVDD" } | Select-Object -First 1
    if ($dev) {
        & pnputil.exe /restart-device "$($dev.InstanceId)" 2>&1 | Out-Host
        $inf = (Get-PnpDeviceProperty -InstanceId $dev.InstanceId -KeyName DEVPKEY_Device_DriverInfPath -ErrorAction SilentlyContinue).Data
        if (-not $inf) { throw "the virtual display device exists but no driver is bound to it" }
        Write-Host "Device $($dev.InstanceId) uses $inf"
    }
    Write-Host "DREAD virtual display driver installed."
} catch {
    Write-Host "ERROR: $_"
    Stop-Transcript | Out-Null
    exit 1
}
Stop-Transcript | Out-Null
