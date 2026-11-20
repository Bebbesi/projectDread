//! Diagnostics: in-memory log buffer shown in the control panel and a
//! plain-text diagnostics report.

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct LogLine {
    pub time: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<VecDeque<LogLine>>>);

impl LogBuffer {
    pub const CAP: usize = 2000;

    pub fn push(&self, l: LogLine) {
        let mut q = self.0.lock();
        q.push_back(l);
        while q.len() > Self::CAP {
            q.pop_front();
        }
    }

    pub fn snapshot(&self) -> Vec<LogLine> {
        self.0.lock().iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.0.lock().clear();
    }
}

/// Collects everything useful for a bug report.
pub fn report(e: &crate::Engine) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let cfg = e.config();
    let _ = writeln!(s, "DREAD Server {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    for k in ["XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE", "WAYLAND_DISPLAY", "DISPLAY", "HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK"] {
        if let Ok(v) = std::env::var(k) {
            let _ = writeln!(s, "{k}={v}");
        }
    }
    let d = e.driver_status();
    let _ = writeln!(s, "\nDisplay backend: {} (ok={})\n  {}", d.backend, d.ok, d.detail);
    for o in e.outputs() {
        let _ = writeln!(s, "  output {} {}x{}@{:.2} at {},{} scale {} dread={}", o.name, o.width, o.height, o.refresh, o.x, o.y, o.scale, o.is_dread);
    }
    let _ = writeln!(s, "\nFFmpeg: {}", e.ffmpeg.read().clone().unwrap_or_else(|| "NOT FOUND".into()));
    match e.caps.read().as_ref() {
        None => {
            let _ = writeln!(s, "Encoders: probing…");
        }
        Some(c) if c.is_empty() => {
            let _ = writeln!(s, "Encoders: none working");
        }
        Some(c) => {
            for x in c {
                let _ = writeln!(s, "  encoder {} ({}) max {}x{}", x.encoder, x.backend.label(), x.max_width, x.max_height);
            }
        }
    }
    let st = e.stats.lock().clone();
    let _ = writeln!(s, "\nStats: {}", serde_json::to_string(&st).unwrap_or_default());
    let _ = writeln!(s, "USB: {}", e.usb_status.lock());
    let _ = writeln!(s, "Clients:");
    for c in e.client_views() {
        let _ = writeln!(s, "  {} {} via {} {}x{} rtt {:.1}ms", c.name, c.addr, c.transport.label(), c.width, c.height, c.rtt_ms);
    }
    let _ = writeln!(s, "Paired devices: {}", e.devices().len());
    let _ = writeln!(s, "Server fingerprint: {}", e.identity.fingerprint_hex());
    let _ = writeln!(s, "Port: {} (public addresses allowed: {})", cfg.connection.port, cfg.connection.allow_public_addresses);
    if let Some(err) = e.last_error.lock().clone() {
        let _ = writeln!(s, "Last error: {err}");
    }
    let _ = writeln!(s, "\nRecent log:");
    let logs = e.logs.snapshot();
    for l in logs.iter().rev().take(200).rev() {
        let _ = writeln!(s, "{} {:5} {} {}", l.time, l.level, l.target, l.message);
    }
    s
}
