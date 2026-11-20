//! Hyprland backend.
//!
//! Extended mode creates a *headless output* inside the compositor
//! (`hyprctl output create headless DREAD-1`). Hyprland treats it like any
//! other monitor: it has its own workspaces, windows can be dragged onto it,
//! and it is listed by `hyprctl monitors`. Mode, position, scale and
//! transform are applied through the Lua config API (`hl.monitor`) on
//! Hyprland ≥ 0.50, or the legacy `keyword monitor` command on older builds.

use super::*;
use anyhow::{bail, Context};
use serde_json::Value;
use std::process::Command;
use tracing::{debug, info};

pub const OUTPUT_NAME: &str = "DREAD-1";

pub struct Hyprland {
    lua: bool,
    version: String,
}

fn hyprctl(args: &[&str]) -> Result<String> {
    let out = Command::new("hyprctl").args(args).output().context("running hyprctl")?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    debug!(?args, reply = %s, "hyprctl");
    Ok(s)
}

fn expect_ok(r: String, what: &str) -> Result<()> {
    if r == "ok" || r.is_empty() || r.ends_with("ok") {
        Ok(())
    } else {
        bail!("hyprctl {what}: {r}")
    }
}

/// Hyprland normalises device names: lowercase, spaces become dashes.
pub fn device_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

impl Hyprland {
    pub fn new() -> Result<Self> {
        let v: Value = serde_json::from_str(&hyprctl(&["-j", "version"])?).context("hyprctl version")?;
        let version = v.get("tag").and_then(Value::as_str).unwrap_or("?").to_string();
        let lua = hyprctl(&["eval", "return 1"]).map(|r| !r.contains("unknown request")).unwrap_or(false);
        info!(%version, lua, "Hyprland display backend");
        Ok(Self { lua, version })
    }

    fn set_monitor(&self, name: &str, w: u32, h: u32, refresh: u32, x: i32, y: i32, scale: f32, transform: u32) -> Result<()> {
        if self.lua {
            let code = format!(
                "hl.monitor({{ output = \"{name}\", mode = \"{w}x{h}@{refresh}\", position = \"{x}x{y}\", scale = {scale}, transform = {transform} }})"
            );
            expect_ok(hyprctl(&["eval", &code])?, "monitor")
        } else {
            let v = format!("{name},{w}x{h}@{refresh},{x}x{y},{scale},transform,{transform}");
            expect_ok(hyprctl(&["keyword", "monitor", &v])?, "monitor")
        }
    }

    /// (monitor, active workspace id, focused) for every monitor.
    fn active_workspaces(&self) -> Vec<(String, i64, bool)> {
        let Ok(v) = hyprctl(&["-j", "monitors"]).and_then(|s| Ok(serde_json::from_str::<Value>(&s)?)) else { return vec![] };
        v.as_array()
            .map(|a| {
                a.iter()
                    .map(|m| (m["name"].as_str().unwrap_or("").to_string(), m["activeWorkspace"]["id"].as_i64().unwrap_or(0), m["focused"].as_bool().unwrap_or(false)))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn focus_workspace(&self, id: i64) {
        let _ = if self.lua {
            hyprctl(&["eval", &format!("hl.dispatch(hl.dsp.focus({{ workspace = \"{id}\" }}))")])
        } else {
            hyprctl(&["dispatch", "workspace", &id.to_string()])
        };
    }

    fn move_workspace(&self, id: i64, monitor: &str) {
        if self.lua {
            // hl.dsp.workspace.move acts on the focused workspace
            self.focus_workspace(id);
            let _ = hyprctl(&["eval", &format!("hl.dispatch(hl.dsp.workspace.move({{ monitor = \"{monitor}\" }}))")]);
        } else {
            let _ = hyprctl(&["dispatch", "moveworkspacetomonitor", &id.to_string(), monitor]);
        }
    }

    /// Adding an output must never change what the existing monitors show.
    /// Hyprland moves workspaces that once lived on an output with the same
    /// name back onto it; a workspace that is currently on screen elsewhere
    /// is returned to its monitor here (hidden ones may stay on DREAD-1, like
    /// windows returning to a re-plugged monitor).
    fn keep_workspaces(&self, before: &[(String, i64, bool)]) {
        let now = self.active_workspaces();
        let mut changed = false;
        for (mon, ws, _) in before.iter().filter(|(m, _, _)| m != OUTPUT_NAME) {
            let cur = now.iter().find(|(m, _, _)| m == mon).map(|x| x.1);
            if cur.is_some() && cur != Some(*ws) {
                info!(workspace = ws, monitor = %mon, "returning workspace to its monitor");
                self.move_workspace(*ws, mon);
                changed = true;
            }
        }
        if changed {
            for (_, ws, _) in before.iter().filter(|(m, _, _)| m != OUTPUT_NAME) {
                self.focus_workspace(*ws);
            }
            if let Some((_, ws, _)) = before.iter().find(|(_, _, f)| *f) {
                self.focus_workspace(*ws);
            }
        }
    }

    fn wait_for(&mut self, name: &str) -> Result<OutputInfo> {
        for _ in 0..40 {
            if let Some(o) = self.outputs()?.into_iter().find(|o| o.name == name) {
                return Ok(o);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        bail!("output {name} did not appear")
    }

    fn apply(&mut self, s: &DisplaySettings) -> Result<OutputInfo> {
        let (w, h) = s.effective_size();
        let others: Vec<OutputInfo> = self.outputs()?.into_iter().filter(|o| o.name != OUTPUT_NAME).collect();
        let scale = if s.scale > 0.0 { s.scale } else { 1.0 };
        let (x, y) = place(s, &others, (w as f64 / scale as f64, h as f64 / scale as f64));
        // Pin the other monitors where they are: Hyprland re-flows outputs that
        // use "auto" placement when a new output is positioned explicitly.
        for o in &others {
            self.set_monitor(&o.name, o.width, o.height, o.refresh.round() as u32, o.x, o.y, o.scale, o.transform)?;
        }
        self.set_monitor(OUTPUT_NAME, w, h, s.refresh.max(1), x, y, scale, 0)?;
        std::thread::sleep(std::time::Duration::from_millis(60));
        let o = self.wait_for(OUTPUT_NAME)?;
        if o.width != w || o.height != h {
            bail!("Hyprland applied {}x{} instead of {w}x{h}", o.width, o.height);
        }
        Ok(o)
    }
}

impl DisplayBackend for Hyprland {
    fn name(&self) -> &'static str {
        "Hyprland"
    }

    fn outputs(&mut self) -> Result<Vec<OutputInfo>> {
        let v: Value = serde_json::from_str(&hyprctl(&["-j", "monitors"])?).context("hyprctl monitors")?;
        let arr = v.as_array().context("monitors is not an array")?;
        Ok(arr
            .iter()
            .map(|m| {
                let name = m["name"].as_str().unwrap_or("").to_string();
                OutputInfo {
                    is_dread: name == OUTPUT_NAME,
                    description: m["description"].as_str().unwrap_or("").to_string(),
                    width: m["width"].as_u64().unwrap_or(0) as u32,
                    height: m["height"].as_u64().unwrap_or(0) as u32,
                    refresh: m["refreshRate"].as_f64().unwrap_or(60.0) as f32,
                    x: m["x"].as_i64().unwrap_or(0) as i32,
                    y: m["y"].as_i64().unwrap_or(0) as i32,
                    scale: m["scale"].as_f64().unwrap_or(1.0) as f32,
                    transform: m["transform"].as_u64().unwrap_or(0) as u32,
                    focused: m["focused"].as_bool().unwrap_or(false),
                    name,
                }
            })
            .collect())
    }

    fn virtual_modes(&self) -> Vec<(u32, u32)> {
        STANDARD_MODES.to_vec()
    }

    fn virtual_refresh_rates(&self) -> Vec<u32> {
        STANDARD_REFRESH.to_vec()
    }

    fn max_virtual_size(&self) -> (u32, u32) {
        (7680, 4320)
    }

    fn activate(&mut self, s: &DisplaySettings) -> Result<ActiveDisplay> {
        match s.mode {
            DisplayMode::Extend => {
                let before = self.active_workspaces();
                if self.outputs()?.iter().any(|o| o.name == OUTPUT_NAME) {
                    info!("reusing existing {OUTPUT_NAME}");
                } else {
                    expect_ok(hyprctl(&["output", "create", "headless", OUTPUT_NAME])?, "output create")?;
                    self.wait_for(OUTPUT_NAME)?;
                }
                let o = self.apply(s)?;
                self.keep_workspaces(&before);
                info!(name = %o.name, w = o.width, h = o.height, x = o.x, y = o.y, "virtual display active");
                Ok(ActiveDisplay { output: o, owned: true, capture_index: None })
            }
            DisplayMode::Duplicate => {
                let outs = self.outputs()?;
                let o = outs
                    .iter()
                    .find(|o| !s.duplicate_source.is_empty() && o.name == s.duplicate_source)
                    .or_else(|| outs.iter().find(|o| !o.is_dread))
                    .cloned()
                    .context("no output to duplicate")?;
                Ok(ActiveDisplay { output: o, owned: false, capture_index: None })
            }
        }
    }

    fn reconfigure(&mut self, d: &ActiveDisplay, s: &DisplaySettings) -> Result<ActiveDisplay> {
        if !d.owned || s.mode == DisplayMode::Duplicate {
            return self.activate(s);
        }
        let before = self.active_workspaces();
        let o = self.apply(s)?;
        self.keep_workspaces(&before);
        Ok(ActiveDisplay { output: o, owned: true, capture_index: None })
    }

    fn deactivate(&mut self, d: &ActiveDisplay) -> Result<()> {
        if d.owned {
            expect_ok(hyprctl(&["output", "remove", &d.output.name])?, "output remove")?;
            info!("removed {}", d.output.name);
        }
        Ok(())
    }

    fn bind_input(&mut self, d: &ActiveDisplay, devices: &[&str]) -> Result<()> {
        for dev in devices {
            let id = device_id(dev);
            if self.lua {
                let code = format!("hl.device({{ name = \"{id}\", output = \"{}\" }})", d.output.name);
                expect_ok(hyprctl(&["eval", &code])?, "device")?;
            } else {
                expect_ok(hyprctl(&["keyword", &format!("device[{id}]:output"), &d.output.name])?, "device")?;
            }
        }
        Ok(())
    }

    fn focused_output(&mut self) -> Option<String> {
        let v: Value = serde_json::from_str(&hyprctl(&["-j", "activewindow"]).ok()?).ok()?;
        let mon = v.get("monitor")?.as_i64()?;
        let mons: Value = serde_json::from_str(&hyprctl(&["-j", "monitors"]).ok()?).ok()?;
        mons.as_array()?
            .iter()
            .find(|m| m["id"].as_i64() == Some(mon))
            .and_then(|m| m["name"].as_str())
            .map(str::to_string)
    }

    fn driver_status(&mut self) -> DriverStatus {
        DriverStatus {
            backend: format!("Hyprland {} (headless outputs, {} config API)", self.version, if self.lua { "Lua" } else { "keyword" }),
            ok: true,
            detail: "Virtual outputs are created inside the compositor; no kernel driver is needed.".into(),
        }
    }
}
