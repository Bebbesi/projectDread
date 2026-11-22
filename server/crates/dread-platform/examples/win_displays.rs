//! Windows only: lists display adapters, their monitors and modes (diagnostics).
#[cfg(windows)]
fn main() {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::*;
    fn s(w: &[u16]) -> String {
        String::from_utf16_lossy(&w[..w.iter().position(|c| *c == 0).unwrap_or(w.len())])
    }
    for i in 0.. {
        let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut dd, 0) }.as_bool() {
            break;
        }
        let name: Vec<u16> = dd.DeviceName.iter().copied().take_while(|c| *c != 0).chain([0]).collect();
        let mut mons = vec![];
        for j in 0.. {
            let mut m = DISPLAY_DEVICEW { cb: dd.cb, ..Default::default() };
            if !unsafe { EnumDisplayDevicesW(PCWSTR(name.as_ptr()), j, &mut m, 0) }.as_bool() {
                break;
            }
            mons.push(format!("{} (flags {:#x})", s(&m.DeviceString), m.StateFlags.0));
        }
        let mut cur = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        let has_cur = unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut cur) }.as_bool();
        let mut modes = vec![];
        for k in 0.. {
            let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
            if !unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(k), &mut dm) }.as_bool() {
                break;
            }
            let m = format!("{}x{}@{}", dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency);
            if !modes.contains(&m) {
                modes.push(m);
            }
        }
        if mons.is_empty() && modes.is_empty() && !s(&dd.DeviceString).contains("Virtual Display") {
            continue;
        }
        let pos = unsafe { cur.Anonymous1.Anonymous2.dmPosition };
        println!(
            "{} | {} | flags {:#x} | current: {} {}x{}@{} at {},{}\n    monitors: {:?}\n    {} modes: {}",
            s(&dd.DeviceName),
            s(&dd.DeviceString),
            dd.StateFlags.0,
            has_cur,
            cur.dmPelsWidth,
            cur.dmPelsHeight,
            cur.dmDisplayFrequency,
            pos.x,
            pos.y,
            mons,
            modes.len(),
            modes.iter().take(40).cloned().collect::<Vec<_>>().join(" ")
        );
    }
}

#[cfg(not(windows))]
fn main() {}
