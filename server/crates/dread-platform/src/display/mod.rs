//! Display management: creation of genuine virtual outputs (extended mode)
//! and selection of existing outputs (duplicate mode).

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
pub mod hyprland;
#[cfg(target_os = "linux")]
pub mod sway;
#[cfg(windows)]
pub mod windows_vdd;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    #[default]
    Extend,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Right,
    Left,
    Above,
    Below,
    /// Explicit layout coordinates (logical pixels).
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Landscape,
    Portrait,
}

/// Desired configuration of the DREAD display (desktop "Display" page).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DisplaySettings {
    pub mode: DisplayMode,
    /// Output to mirror in duplicate mode (empty = primary/first).
    pub duplicate_source: String,
    /// Follow the client's native resolution and orientation.
    pub auto_resolution: bool,
    pub width: u32,
    pub height: u32,
    pub refresh: u32,
    pub scale: f32,
    pub orientation: Orientation,
    pub placement: Placement,
    /// Monitor the placement is relative to (empty = primary).
    pub relative_to: String,
    pub custom_x: i32,
    pub custom_y: i32,
    /// Make the DREAD display the primary monitor where the OS supports it.
    pub primary: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            mode: DisplayMode::Extend,
            duplicate_source: String::new(),
            auto_resolution: true,
            width: 1920,
            height: 1080,
            refresh: 60,
            // 0 = automatic: derived from the client's pixel density
            scale: 0.0,
            orientation: Orientation::Landscape,
            placement: Placement::Right,
            relative_to: String::new(),
            custom_x: 0,
            custom_y: 0,
            primary: false,
        }
    }
}

impl DisplaySettings {
    /// Effective mode size with orientation applied.
    pub fn effective_size(&self) -> (u32, u32) {
        let (a, b) = (self.width.max(self.height), self.width.min(self.height));
        match self.orientation {
            Orientation::Landscape => (a, b),
            Orientation::Portrait => (b, a),
        }
    }
}

/// An output known to the display server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputInfo {
    pub name: String,
    pub description: String,
    /// Mode size in physical pixels.
    pub width: u32,
    pub height: u32,
    pub refresh: f32,
    /// Position in the desktop layout (logical pixels).
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    pub transform: u32,
    pub focused: bool,
    pub is_dread: bool,
}

impl OutputInfo {
    /// Size in layout (logical) pixels, with rotation applied.
    pub fn logical_size(&self) -> (f64, f64) {
        let (w, h) = (self.width as f64 / self.scale as f64, self.height as f64 / self.scale as f64);
        if self.transform % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }
}

/// The display DREAD is currently streaming.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveDisplay {
    pub output: OutputInfo,
    /// True when DREAD created the output and must remove it on stop.
    pub owned: bool,
    /// Index used by capture backends that address outputs by number (Windows ddagrab).
    pub capture_index: Option<u32>,
}

/// Standard modes offered in the UI; backends filter by what they support.
pub const STANDARD_MODES: &[(u32, u32)] = &[
    (640, 480),
    (800, 600),
    (1280, 720),
    (1280, 800),
    (1600, 900),
    (1920, 1080),
    (1920, 1200),
    (2560, 1440),
    (2560, 1600),
    (3840, 2160),
];

pub const STANDARD_REFRESH: &[u32] = &[15, 24, 30, 45, 60, 90, 120];

pub trait DisplayBackend: Send {
    fn name(&self) -> &'static str;
    fn outputs(&mut self) -> Result<Vec<OutputInfo>>;
    /// Modes this backend can create for a virtual output.
    fn virtual_modes(&self) -> Vec<(u32, u32)>;
    fn virtual_refresh_rates(&self) -> Vec<u32>;
    fn max_virtual_size(&self) -> (u32, u32);
    /// Create (extend) or select (duplicate) the display for `s`.
    fn activate(&mut self, s: &DisplaySettings) -> Result<ActiveDisplay>;
    /// Apply new settings to an already-active virtual display.
    fn reconfigure(&mut self, d: &ActiveDisplay, s: &DisplaySettings) -> Result<ActiveDisplay>;
    fn deactivate(&mut self, d: &ActiveDisplay) -> Result<()>;
    /// Bind DREAD's absolute input devices (touch, pen) to the output.
    fn bind_input(&mut self, d: &ActiveDisplay, devices: &[&str]) -> Result<()>;
    /// Output that currently holds keyboard focus, if known.
    fn focused_output(&mut self) -> Option<String>;
    fn driver_status(&mut self) -> DriverStatus;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DriverStatus {
    pub backend: String,
    pub ok: bool,
    pub detail: String,
}

/// Picks the display backend for the running session.
pub fn detect_backend() -> Result<Box<dyn DisplayBackend>> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return Ok(Box::new(hyprland::Hyprland::new()?));
        }
        if std::env::var_os("SWAYSOCK").is_some() {
            return Ok(Box::new(sway::Sway::new()?));
        }
        let desk = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        anyhow::bail!(
            "unsupported desktop session ({desk} / {session}). DREAD currently supports Hyprland and \
             Sway on Linux; GNOME, KDE and X11 support is documented in docs/PLATFORMS.md as not yet implemented"
        )
    }
    #[cfg(windows)]
    {
        return Ok(Box::new(windows_vdd::WindowsVdd::new()?));
    }
    #[allow(unreachable_code)]
    {
        anyhow::bail!("unsupported operating system")
    }
}

/// Computes the layout position for a new output next to `reference`.
pub fn place(s: &DisplaySettings, outputs: &[OutputInfo], logical: (f64, f64)) -> (i32, i32) {
    if s.placement == Placement::Custom {
        return (s.custom_x, s.custom_y);
    }
    let others: Vec<&OutputInfo> = outputs.iter().filter(|o| !o.is_dread).collect();
    let reference = others
        .iter()
        .find(|o| !s.relative_to.is_empty() && o.name == s.relative_to)
        .or_else(|| others.iter().find(|o| o.x == 0 && o.y == 0))
        .or(others.first())
        .copied();
    let Some(r) = reference else { return (0, 0) };
    let (rw, rh) = r.logical_size();
    let (w, h) = logical;
    match s.placement {
        // place outside the whole layout on that side so nothing overlaps
        Placement::Right => {
            let max_x = others.iter().map(|o| o.x as f64 + o.logical_size().0).fold(r.x as f64 + rw, f64::max);
            (max_x.round() as i32, r.y)
        }
        Placement::Left => {
            let min_x = others.iter().map(|o| o.x as f64).fold(r.x as f64, f64::min);
            ((min_x - w).round() as i32, r.y)
        }
        Placement::Above => {
            let min_y = others.iter().map(|o| o.y as f64).fold(r.y as f64, f64::min);
            (r.x + ((rw - w) / 2.0).round() as i32, (min_y - h).round() as i32)
        }
        Placement::Below => {
            let max_y = others.iter().map(|o| o.y as f64 + o.logical_size().1).fold(r.y as f64 + rh, f64::max);
            (r.x + ((rw - w) / 2.0).round() as i32, max_y.round() as i32)
        }
        Placement::Custom => unreachable!(),
    }
}

/// Bounding box of all outputs in layout coordinates.
pub fn layout_box(outputs: &[OutputInfo]) -> (f64, f64, f64, f64) {
    let mut x0 = f64::MAX;
    let mut y0 = f64::MAX;
    let mut x1 = f64::MIN;
    let mut y1 = f64::MIN;
    for o in outputs {
        let (w, h) = o.logical_size();
        x0 = x0.min(o.x as f64);
        y0 = y0.min(o.y as f64);
        x1 = x1.max(o.x as f64 + w);
        y1 = y1.max(o.y as f64 + h);
    }
    if outputs.is_empty() {
        return (0.0, 0.0, 1.0, 1.0);
    }
    (x0, y0, x1 - x0, y1 - y0)
}

/// Absolute-pointer mapping for `display` within the layout.
pub fn abs_map(display: &OutputInfo, outputs: &[OutputInfo]) -> crate::input::AbsMap {
    let (bx, by, bw, bh) = layout_box(outputs);
    let (w, h) = display.logical_size();
    crate::input::AbsMap { x: (display.x as f64 - bx) / bw, y: (display.y as f64 - by) / bh, w: w / bw, h: h / bh }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn out(name: &str, x: i32, w: u32) -> OutputInfo {
        OutputInfo { name: name.into(), description: String::new(), width: w, height: 1080, refresh: 60.0, x, y: 0, scale: 1.0, transform: 0, focused: false, is_dread: false }
    }
    #[test]
    fn placement() {
        let outs = vec![out("eDP-1", 0, 1920), out("HDMI-A-1", 1920, 2560)];
        let s = DisplaySettings::default();
        assert_eq!(place(&s, &outs, (1280.0, 800.0)), (4480, 0));
        let s = DisplaySettings { placement: Placement::Left, ..Default::default() };
        assert_eq!(place(&s, &outs, (1280.0, 800.0)), (-1280, 0));
    }
}
