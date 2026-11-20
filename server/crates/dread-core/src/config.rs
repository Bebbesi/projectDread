//! Persistent configuration (`config.json` in the platform config directory).

use anyhow::{Context, Result};
use dread_platform::display::DisplaySettings;
use dread_platform::encode::{Backend, Codec, Quality};
use dread_platform::input::InputSettings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StreamSettings {
    pub codec: Codec,
    pub backend: Backend,
    pub hardware_acceleration: bool,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub quality: Quality,
    pub keyframe_interval_s: f32,
    pub adaptive_bitrate: bool,
    pub low_latency: bool,
    /// Hard cap on the video bitrate (0 = unlimited).
    pub bandwidth_limit_kbps: u32,
    /// Lower resolution automatically when the link cannot sustain the minimum bitrate.
    pub auto_quality: bool,
    /// Frames that may be sent but not yet displayed by the client.
    pub max_frames_in_flight: u32,
    pub show_cursor: bool,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            codec: Codec::H264,
            backend: Backend::Auto,
            hardware_acceleration: true,
            fps: 60,
            bitrate_kbps: 12_000,
            quality: Quality::Balanced,
            keyframe_interval_s: 5.0,
            adaptive_bitrate: true,
            low_latency: true,
            bandwidth_limit_kbps: 0,
            auto_quality: true,
            max_frames_in_flight: 2,
            show_cursor: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ConnectionSettings {
    pub port: u16,
    pub wifi_enabled: bool,
    pub usb_enabled: bool,
    pub discovery_enabled: bool,
    /// Accept connections from non-private (internet-routable) addresses.
    /// Off by default: DREAD is a LAN product.
    pub allow_public_addresses: bool,
    /// Seconds without any message before a session is dropped.
    pub timeout_s: u32,
    /// Keep the virtual display this long after the last client leaves,
    /// so a reconnect lands on the same layout.
    pub display_linger_s: u32,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            port: dread_proto::DEFAULT_PORT,
            wifi_enabled: true,
            usb_enabled: true,
            discovery_enabled: true,
            allow_public_addresses: false,
            timeout_s: 6,
            display_linger_s: 15,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeneralSettings {
    pub server_name: String,
    pub server_id: String,
    pub autostart: bool,
    pub start_minimized: bool,
    /// Begin streaming as soon as a paired client connects.
    pub auto_stream: bool,
    pub ffmpeg_path: String,
    pub vaapi_device: String,
    pub log_level: String,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            server_name: hostname(),
            server_id: uuid::Uuid::new_v4().to_string(),
            autostart: false,
            start_minimized: false,
            auto_stream: true,
            ffmpeg_path: String::new(),
            vaapi_device: "/dev/dri/renderD128".into(),
            log_level: "info".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Config {
    pub general: GeneralSettings,
    pub display: DisplaySettings,
    pub stream: StreamSettings,
    pub connection: ConnectionSettings,
    pub input: InputSettings,
}

pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: buffer is valid for its length.
        if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            let s = String::from_utf8_lossy(&buf[..end]).to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "DREAD Server".into())
}

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("DREAD_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("app", "dread", "dread")
        .map(|p| p.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".dread"))
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("config.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path()).unwrap_or_default()
    }

    pub fn load_from(p: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(p)?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, p: &Path) -> Result<()> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, p).context("saving config")?;
        Ok(())
    }
}
