//! Input injection: turns DREAD protocol input events into OS input.

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub mod uinput;
#[cfg(target_os = "linux")]
pub mod wl_text;
#[cfg(windows)]
pub mod windows;

/// User-configurable input behaviour (desktop "Input" page).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InputSettings {
    pub touch_enabled: bool,
    pub pen_enabled: bool,
    pub mouse_enabled: bool,
    pub keyboard_enabled: bool,
    /// Exponent applied to normalized pen pressure (1.0 = linear, <1 softer, >1 firmer).
    pub pen_pressure_gamma: f32,
    pub pen_tilt: bool,
    pub mouse_speed: f32,
    pub scroll_speed: f32,
    pub invert_scroll: bool,
    /// Ask the client to open its soft keyboard when a text field on the DREAD display gets focus.
    pub auto_keyboard: bool,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            touch_enabled: true,
            pen_enabled: true,
            mouse_enabled: true,
            keyboard_enabled: true,
            pen_pressure_gamma: 1.0,
            pen_tilt: true,
            mouse_speed: 1.0,
            scroll_speed: 1.0,
            invert_scroll: false,
            auto_keyboard: true,
        }
    }
}

/// Maps display-normalized coordinates onto the coordinate space of an
/// absolute pointer device that spans the whole desktop layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AbsMap {
    /// Display rectangle inside the layout, as fractions of the layout box.
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl AbsMap {
    pub fn identity() -> Self {
        Self { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
    }

    pub fn map(&self, nx: u16, ny: u16) -> (u16, u16) {
        let fx = self.x + self.w * (nx as f64 / 65535.0);
        let fy = self.y + self.h * (ny as f64 / 65535.0);
        ((fx.clamp(0.0, 1.0) * 65535.0).round() as u16, (fy.clamp(0.0, 1.0) * 65535.0).round() as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn abs_map_right_monitor() {
        // layout: 1920 wide primary + 1280 wide DREAD to its right
        let m = AbsMap { x: 1920.0 / 3200.0, y: 0.0, w: 1280.0 / 3200.0, h: 800.0 / 1080.0 };
        let (x, y) = m.map(0, 0);
        assert_eq!((x, y), ((0.6f64 * 65535.0).round() as u16, 0));
        let (x, _) = m.map(65535, 65535);
        assert_eq!(x, 65535);
    }
}
