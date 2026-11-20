//! Windows backend: a real IddCx virtual monitor provided by the bundled,
//! signed, MIT-licensed *Virtual Display Driver* (hardware id `Root\MttVDD`).
//!
//! STATUS: written against the Win32 documentation and the driver's source;
//! it has NOT been compiled or run on Windows yet (see docs/PLATFORMS.md).
//!
//! How it works
//! * The installer installs the driver with one monitor and a mode list that
//!   covers standard desktop modes plus common phone/tablet panels (both
//!   orientations), written to `C:\VirtualDisplayDriver\vdd_settings.xml`.
//! * Extend: the driver's monitor is attached to the desktop with
//!   `ChangeDisplaySettingsExW` (mode, refresh, position, optional primary) —
//!   Windows then treats it as an ordinary monitor (Settings → Display shows it).
//! * Stop: the monitor is detached (zero-size mode), which removes it from the
//!   desktop without uninstalling anything.
//! * Duplicate: any attached monitor is captured with DXGI Desktop Duplication.
//! * If a requested resolution is not in the driver's list, the closest mode
//!   with the same orientation is used.

use super::*;
use anyhow::{bail, Context};
use tracing::{info, warn};
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::*;

const VDD_NAMES: [&str; 3] = ["Virtual Display Driver", "IddSampleDriver", "MttVDD"];
const PIPE: &str = r"\\.\pipe\MTTVirtualDisplayPipe";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(w: &[u16]) -> String {
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

#[derive(Debug, Clone)]
struct Adapter {
    device_name: String,
    device_string: String,
    attached: bool,
    primary: bool,
}

fn adapters() -> Vec<Adapter> {
    let mut out = vec![];
    for i in 0.. {
        let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        // SAFETY: dd is a properly sized DISPLAY_DEVICEW
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut dd, 0) }.as_bool() {
            break;
        }
        out.push(Adapter {
            device_name: from_wide(&dd.DeviceName),
            device_string: from_wide(&dd.DeviceString),
            attached: dd.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0,
            primary: dd.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0,
        });
    }
    out
}

fn current_mode(device: &str) -> Option<DEVMODEW> {
    let name = wide(device);
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    // SAFETY: valid device name and DEVMODEW
    unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm) }.as_bool().then_some(dm)
}

fn modes(device: &str) -> Vec<(u32, u32, u32)> {
    let name = wide(device);
    let mut v = vec![];
    for i in 0.. {
        let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        // SAFETY: valid device name and DEVMODEW
        if !unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm) }.as_bool() {
            break;
        }
        let m = (dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency);
        if !v.contains(&m) {
            v.push(m);
        }
    }
    v
}

fn position(dm: &DEVMODEW) -> (i32, i32) {
    // SAFETY: dmPosition is the active union member for display devices
    let p = unsafe { dm.Anonymous1.Anonymous2.dmPosition };
    (p.x, p.y)
}

fn apply_mode(device: &str, w: u32, h: u32, hz: u32, x: i32, y: i32, primary: bool, detach: bool) -> Result<()> {
    let name = wide(device);
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    dm.dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT;
    if detach {
        dm.dmPelsWidth = 0;
        dm.dmPelsHeight = 0;
    } else {
        dm.dmPelsWidth = w;
        dm.dmPelsHeight = h;
        if hz > 0 {
            dm.dmFields |= DM_DISPLAYFREQUENCY;
            dm.dmDisplayFrequency = hz;
        }
    }
    dm.Anonymous1.Anonymous2.dmPosition = windows::Win32::Foundation::POINTL { x, y };
    let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
    if primary {
        flags |= CDS_SET_PRIMARY;
    }
    // SAFETY: valid device name and DEVMODEW
    let r = unsafe { ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, flags, None) };
    if r != DISP_CHANGE_SUCCESSFUL {
        bail!("ChangeDisplaySettingsEx({device}) failed: {:?}", r);
    }
    // SAFETY: applies all pending changes
    let r = unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) };
    if r != DISP_CHANGE_SUCCESSFUL {
        bail!("applying display changes failed: {:?}", r);
    }
    Ok(())
}

/// Sends one command to the driver's control pipe (e.g. `PING`).
fn pipe_command(cmd: &str) -> Option<String> {
    use std::io::{Read, Write};
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(PIPE).ok()?;
    let msg: Vec<u8> = cmd.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    f.write_all(&msg).ok()?;
    let mut buf = vec![0u8; 512];
    let n = f.read(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf[..n]).trim_matches(char::from(0)).to_string())
}

pub struct WindowsVdd {
    device: Option<String>,
}

impl WindowsVdd {
    pub fn new() -> Result<Self> {
        let device = adapters().into_iter().find(|a| VDD_NAMES.iter().any(|n| a.device_string.contains(n))).map(|a| a.device_name);
        match &device {
            Some(d) => info!(device = %d, "virtual display driver found"),
            None => warn!("Virtual Display Driver not installed: extended mode unavailable (duplicate mode still works)"),
        }
        Ok(Self { device })
    }

    fn vdd(&self) -> Result<&str> {
        self.device.as_deref().context("the DREAD virtual display driver is not installed — run the DREAD installer or packaging/windows/install-driver.ps1 as administrator")
    }

    fn info_for(&self, a: &Adapter, idx: u32) -> Option<OutputInfo> {
        let dm = current_mode(&a.device_name)?;
        let (x, y) = position(&dm);
        let is_dread = self.device.as_deref() == Some(a.device_name.as_str());
        Some(OutputInfo {
            name: a.device_name.clone(),
            description: if is_dread { "DREAD virtual display".into() } else { a.device_string.clone() },
            width: dm.dmPelsWidth,
            height: dm.dmPelsHeight,
            refresh: dm.dmDisplayFrequency as f32,
            x,
            y,
            scale: 1.0,
            transform: 0,
            focused: a.primary,
            is_dread,
        })
        .filter(|_| idx < 64)
    }

    /// DXGI output index used by FFmpeg's ddagrab: order of attached outputs.
    fn capture_index(&self, device: &str) -> Option<u32> {
        adapters().iter().filter(|a| a.attached).position(|a| a.device_name == device).map(|i| i as u32)
    }

    fn best_mode(&self, device: &str, w: u32, h: u32, hz: u32) -> (u32, u32, u32) {
        let all = modes(device);
        let portrait = h > w;
        let mut cands: Vec<&(u32, u32, u32)> = all.iter().filter(|m| (m.1 > m.0) == portrait).collect();
        if cands.is_empty() {
            cands = all.iter().collect();
        }
        let score = |m: &(u32, u32, u32)| {
            let dw = m.0 as i64 - w as i64;
            let dh = m.1 as i64 - h as i64;
            let size = dw * dw + dh * dh;
            let hzd = (m.2 as i64 - hz as i64).abs();
            size * 1000 + hzd
        };
        cands.into_iter().min_by_key(|m| score(m)).copied().unwrap_or((w, h, hz))
    }
}

impl DisplayBackend for WindowsVdd {
    fn name(&self) -> &'static str {
        "Windows"
    }

    fn outputs(&mut self) -> Result<Vec<OutputInfo>> {
        Ok(adapters().iter().enumerate().filter(|(_, a)| a.attached).filter_map(|(i, a)| self.info_for(a, i as u32)).collect())
    }

    fn virtual_modes(&self) -> Vec<(u32, u32)> {
        let Some(d) = self.device.as_deref() else { return vec![] };
        let m = modes(d);
        STANDARD_MODES.iter().copied().filter(|s| m.iter().any(|x| (x.0, x.1) == *s || (x.1, x.0) == *s)).collect()
    }

    fn virtual_refresh_rates(&self) -> Vec<u32> {
        let Some(d) = self.device.as_deref() else { return vec![60] };
        let mut r: Vec<u32> = modes(d).into_iter().map(|m| m.2).filter(|h| *h > 1).collect();
        r.sort();
        r.dedup();
        r
    }

    fn max_virtual_size(&self) -> (u32, u32) {
        let Some(d) = self.device.as_deref() else { return (0, 0) };
        modes(d).into_iter().map(|m| (m.0.max(m.1), m.0.min(m.1))).max_by_key(|m| m.0 * m.1).unwrap_or((1920, 1080))
    }

    fn activate(&mut self, s: &DisplaySettings) -> Result<ActiveDisplay> {
        match s.mode {
            DisplayMode::Extend => {
                let dev = self.vdd()?.to_string();
                let (w, h) = s.effective_size();
                let (mw, mh, hz) = self.best_mode(&dev, w, h, s.refresh);
                let others: Vec<OutputInfo> = self.outputs()?.into_iter().filter(|o| !o.is_dread).collect();
                let (x, y) = place(s, &others, (mw as f64, mh as f64));
                apply_mode(&dev, mw, mh, hz, x, y, s.primary, false)?;
                let o = self.outputs()?.into_iter().find(|o| o.is_dread).context("virtual display did not attach")?;
                let idx = self.capture_index(&dev);
                info!(w = mw, h = mh, hz, x, y, "virtual display attached");
                Ok(ActiveDisplay { output: o, owned: true, capture_index: idx })
            }
            DisplayMode::Duplicate => {
                let outs = self.outputs()?;
                let o = outs
                    .iter()
                    .find(|o| !s.duplicate_source.is_empty() && o.name == s.duplicate_source)
                    .or_else(|| outs.iter().find(|o| o.focused && !o.is_dread))
                    .or_else(|| outs.iter().find(|o| !o.is_dread))
                    .cloned()
                    .context("no monitor to duplicate")?;
                let idx = self.capture_index(&o.name);
                Ok(ActiveDisplay { output: o, owned: false, capture_index: idx })
            }
        }
    }

    fn reconfigure(&mut self, d: &ActiveDisplay, s: &DisplaySettings) -> Result<ActiveDisplay> {
        if !d.owned || s.mode == DisplayMode::Duplicate {
            return self.activate(s);
        }
        self.activate(s)
    }

    fn deactivate(&mut self, d: &ActiveDisplay) -> Result<()> {
        if d.owned {
            apply_mode(&d.output.name, 0, 0, 0, 0, 0, false, true)?;
            info!("virtual display detached");
        }
        Ok(())
    }

    fn bind_input(&mut self, _d: &ActiveDisplay, _devices: &[&str]) -> Result<()> {
        // Synthetic pointer devices are injected in desktop coordinates of the
        // DREAD monitor, so no binding step is required on Windows.
        Ok(())
    }

    fn focused_output(&mut self) -> Option<String> {
        None
    }

    fn driver_status(&mut self) -> DriverStatus {
        let ping = pipe_command("PING");
        match (&self.device, ping) {
            (Some(d), Some(p)) => DriverStatus { backend: "Virtual Display Driver (IddCx)".into(), ok: true, detail: format!("{d}, control pipe: {p}") },
            (Some(d), None) => DriverStatus { backend: "Virtual Display Driver (IddCx)".into(), ok: true, detail: format!("{d}; control pipe not answering") },
            (None, _) => DriverStatus {
                backend: "No virtual display driver".into(),
                ok: false,
                detail: "Install it with the DREAD installer (or packaging/windows/install-driver.ps1 as administrator). Duplicate mode works without it.".into(),
            },
        }
    }
}
