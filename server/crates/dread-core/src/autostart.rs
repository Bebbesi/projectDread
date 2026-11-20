//! "Start with the system" support.
//!
//! * Linux: an XDG autostart entry `~/.config/autostart/dread.desktop`
//!   (honoured by Hyprland via `exec-once`-style autostart helpers, GNOME,
//!   KDE, XFCE, ...). For AppImages the path of the AppImage is used.
//! * Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\DREAD`.

use tracing::warn;

fn exe_path() -> Option<String> {
    if let Ok(a) = std::env::var("APPIMAGE") {
        return Some(a);
    }
    std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned())
}

pub fn is_enabled() -> bool {
    #[cfg(target_os = "linux")]
    {
        return linux_path().map(|p| p.exists()).unwrap_or(false);
    }
    #[cfg(windows)]
    {
        return std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "DREAD"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
    }
    #[allow(unreachable_code)]
    false
}

#[cfg(target_os = "linux")]
fn linux_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart").join("dread.desktop"))
}

pub fn apply(enable: bool) {
    if enable == is_enabled() {
        return;
    }
    let Some(exe) = exe_path() else { return };
    #[cfg(target_os = "linux")]
    {
        let Some(p) = linux_path() else { return };
        let r = if enable {
            std::fs::create_dir_all(p.parent().unwrap()).and_then(|_| {
                std::fs::write(
                    &p,
                    format!("[Desktop Entry]\nType=Application\nName=DREAD Server\nExec=\"{exe}\" --minimized\nIcon=dread\nX-GNOME-Autostart-enabled=true\n"),
                )
            })
        } else {
            std::fs::remove_file(&p)
        };
        if let Err(e) = r {
            warn!("autostart: {e}");
        }
    }
    #[cfg(windows)]
    {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let r = if enable {
            std::process::Command::new("reg").args(["add", key, "/v", "DREAD", "/t", "REG_SZ", "/d", &format!("\"{exe}\" --minimized"), "/f"]).output()
        } else {
            std::process::Command::new("reg").args(["delete", key, "/v", "DREAD", "/f"]).output()
        };
        if let Err(e) = r {
            warn!("autostart: {e}");
        }
    }
    let _ = exe;
}
