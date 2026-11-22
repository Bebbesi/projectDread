; DREAD Server — Inno Setup script (https://jrsoftware.org/isinfo.php)
; Build: run build-windows.ps1 (it stages files into ..\..\build\windows and calls ISCC).
; STATUS: built and installed on Windows 11 (see docs/TESTING.md).

#define AppVersion "1.0.0"
#define Stage "..\..\build\windows"

[Setup]
AppId={{6C1D1E8E-3F1B-4C55-9E0B-0D7E5A1D2E01}
AppName=DREAD
AppVersion={#AppVersion}
AppPublisher=DREAD
DefaultDirName={autopf}\DREAD
DefaultGroupName=DREAD
OutputDir=..\..\dist
OutputBaseFilename=DREAD-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
MinVersion=10.0.17763
UninstallDisplayIcon={app}\dread.exe
WizardStyle=modern

[Files]
Source: "{#Stage}\dread.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\ffmpeg.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\nefconw.exe"; DestDir: "{app}\driver-tools"; Flags: ignoreversion
Source: "{#Stage}\driver\*"; DestDir: "{app}\driver-tools\driver"; Flags: ignoreversion recursesubdirs
Source: "vdd_settings.xml"; DestDir: "{app}\driver-tools"; Flags: ignoreversion
Source: "install-driver.ps1"; DestDir: "{app}\driver-tools"; Flags: ignoreversion
Source: "uninstall-driver.ps1"; DestDir: "{app}\driver-tools"; Flags: ignoreversion
Source: "{#Stage}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\DREAD"; Filename: "{app}\dread.exe"
Name: "{group}\Uninstall DREAD"; Filename: "{uninstallexe}"

[Tasks]
Name: "driver"; Description: "Install the virtual display driver (needed for Extended mode)"; Flags: checkedonce
Name: "firewall"; Description: "Allow DREAD through Windows Firewall on private networks"; Flags: checkedonce

[Run]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\driver-tools\install-driver.ps1"" -DriverDir ""{app}\driver-tools\driver"" -Nefcon ""{app}\driver-tools\nefconw.exe"""; StatusMsg: "Installing the virtual display driver..."; Tasks: driver; Flags: runhidden waituntilterminated
Filename: "netsh.exe"; Parameters: "advfirewall firewall add rule name=""DREAD (TCP)"" dir=in action=allow protocol=TCP localport=47800 profile=private program=""{app}\dread.exe"""; Tasks: firewall; Flags: runhidden
Filename: "netsh.exe"; Parameters: "advfirewall firewall add rule name=""DREAD (discovery)"" dir=in action=allow protocol=UDP localport=47801 profile=private program=""{app}\dread.exe"""; Tasks: firewall; Flags: runhidden
Filename: "{app}\dread.exe"; Description: "Start DREAD"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\driver-tools\uninstall-driver.ps1"" -DriverDir ""{app}\driver-tools\driver"" -Nefcon ""{app}\driver-tools\nefconw.exe"""; Flags: runhidden waituntilterminated; RunOnceId: "RemoveDriver"
Filename: "netsh.exe"; Parameters: "advfirewall firewall delete rule name=""DREAD (TCP)"""; Flags: runhidden; RunOnceId: "FwTcp"
Filename: "netsh.exe"; Parameters: "advfirewall firewall delete rule name=""DREAD (discovery)"""; Flags: runhidden; RunOnceId: "FwUdp"
Filename: "reg.exe"; Parameters: "delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v DREAD /f"; Flags: runhidden; RunOnceId: "Autostart"
