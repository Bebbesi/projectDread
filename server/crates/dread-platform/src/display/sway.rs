//! Sway backend: `swaymsg create_output` adds a HEADLESS-n output.
//!
//! Status: implemented against the sway IPC documentation; not yet verified on
//! a Sway session (see docs/PLATFORMS.md).

use super::*;
use anyhow::{bail, Context};
use serde_json::Value;
use std::process::Command;

pub struct Sway {
    created: Option<String>,
}

fn swaymsg(args: &[&str]) -> Result<String> {
    let out = Command::new("swaymsg").args(args).output().context("running swaymsg")?;
    if !out.status.success() {
        bail!("swaymsg {:?}: {}", args, String::from_utf8_lossy(&out.stdout));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

impl Sway {
    pub fn new() -> Result<Self> {
        swaymsg(&["-t", "get_version"])?;
        Ok(Self { created: None })
    }

    fn configure(&mut self, name: &str, s: &DisplaySettings) -> Result<OutputInfo> {
        let (w, h) = s.effective_size();
        let others: Vec<OutputInfo> = self.outputs()?.into_iter().filter(|o| o.name != name).collect();
        let scale = if s.scale > 0.0 { s.scale } else { 1.0 };
        let (x, y) = place(s, &others, (w as f64 / scale as f64, h as f64 / scale as f64));
        swaymsg(&[
            "output",
            name,
            "mode",
            "--custom",
            &format!("{w}x{h}@{}Hz", s.refresh.max(1)),
            "pos",
            &x.to_string(),
            &y.to_string(),
            "scale",
            &scale.to_string(),
            "enable",
        ])?;
        self.outputs()?.into_iter().find(|o| o.name == name).context("output vanished")
    }
}

impl DisplayBackend for Sway {
    fn name(&self) -> &'static str {
        "Sway"
    }

    fn outputs(&mut self) -> Result<Vec<OutputInfo>> {
        let v: Value = serde_json::from_str(&swaymsg(&["-r", "-t", "get_outputs"])?)?;
        Ok(v.as_array()
            .context("get_outputs")?
            .iter()
            .filter(|o| o["active"].as_bool().unwrap_or(false))
            .map(|o| {
                let name = o["name"].as_str().unwrap_or("").to_string();
                let transform = match o["transform"].as_str().unwrap_or("normal") {
                    "90" => 1,
                    "180" => 2,
                    "270" => 3,
                    _ => 0,
                };
                OutputInfo {
                    is_dread: Some(&name) == self.created.as_ref(),
                    description: format!("{} {}", o["make"].as_str().unwrap_or(""), o["model"].as_str().unwrap_or("")),
                    width: o["current_mode"]["width"].as_u64().unwrap_or(0) as u32,
                    height: o["current_mode"]["height"].as_u64().unwrap_or(0) as u32,
                    refresh: o["current_mode"]["refresh"].as_f64().unwrap_or(60000.0) as f32 / 1000.0,
                    x: o["rect"]["x"].as_i64().unwrap_or(0) as i32,
                    y: o["rect"]["y"].as_i64().unwrap_or(0) as i32,
                    scale: o["scale"].as_f64().unwrap_or(1.0) as f32,
                    transform,
                    focused: o["focused"].as_bool().unwrap_or(false),
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
                let before: Vec<String> = self.outputs()?.into_iter().map(|o| o.name).collect();
                swaymsg(&["create_output"])?;
                let after = self.outputs()?;
                let new = after.iter().find(|o| !before.contains(&o.name)).context("create_output produced no output")?;
                let name = new.name.clone();
                self.created = Some(name.clone());
                let o = self.configure(&name, s)?;
                Ok(ActiveDisplay { output: o, owned: true, capture_index: None })
            }
            DisplayMode::Duplicate => {
                let outs = self.outputs()?;
                let o = outs
                    .iter()
                    .find(|o| o.name == s.duplicate_source)
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
        let o = self.configure(&d.output.name, s)?;
        Ok(ActiveDisplay { output: o, owned: true, capture_index: None })
    }

    fn deactivate(&mut self, d: &ActiveDisplay) -> Result<()> {
        if d.owned {
            swaymsg(&["output", &d.output.name, "unplug"])?;
            self.created = None;
        }
        Ok(())
    }

    fn bind_input(&mut self, d: &ActiveDisplay, devices: &[&str]) -> Result<()> {
        let inputs: Value = serde_json::from_str(&swaymsg(&["-r", "-t", "get_inputs"])?)?;
        for i in inputs.as_array().into_iter().flatten() {
            let name = i["name"].as_str().unwrap_or("");
            if devices.contains(&name) {
                if let Some(id) = i["identifier"].as_str() {
                    swaymsg(&["input", id, "map_to_output", &d.output.name])?;
                }
            }
        }
        Ok(())
    }

    fn focused_output(&mut self) -> Option<String> {
        self.outputs().ok()?.into_iter().find(|o| o.focused).map(|o| o.name)
    }

    fn driver_status(&mut self) -> DriverStatus {
        DriverStatus {
            backend: "Sway (headless outputs)".into(),
            ok: true,
            detail: "Sway support is implemented but has not been verified on real hardware yet.".into(),
        }
    }
}
