//! Windows only: attaches the DREAD display, injects a mouse move and a touch tap
//! at the centre of it through WindowsInjector, and reports where the cursor went.
//! cargo run -p dread-platform --example win_touch
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use dread_platform::{display, input::windows::WindowsInjector, input::InputSettings};
    use dread_proto::{Contact, InputEvent, TouchAction};
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let cursor = || {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p) }.ok();
        (p.x, p.y)
    };
    if std::env::args().any(|a| a == "dpi") {
        use windows::Win32::UI::HiDpi::*;
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }.ok();
        println!("process set per-monitor DPI aware v2");
    }
    let mut be = display::detect_backend()?;
    let d = be.activate(&display::DisplaySettings { width: 1920, height: 1200, ..Default::default() })?;
    let o = &d.output;
    let rect = Some((o.x, o.y, o.width, o.height));
    println!("DREAD display {} at {},{} {}x{}; centre should be {},{}", o.name, o.x, o.y, o.width, o.height, o.x + o.width as i32 / 2, o.y + o.height as i32 / 2);
    std::thread::sleep(std::time::Duration::from_millis(800));
    {
        use windows::Win32::UI::HiDpi::*;
        use windows::Win32::UI::WindowsAndMessaging::*;
        let m = unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
        println!("virtual screen metrics: {m:?}; dpi awareness ctx: {:?}", unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) });
        unsafe { SetCursorPos(2880, 600) }.ok();
        std::thread::sleep(std::time::Duration::from_millis(100));
        println!("after SetCursorPos(2880,600): {:?}", cursor());
    }
    let mut inj = WindowsInjector::new()?;
    let s = InputSettings::default();
    inj.handle(&InputEvent::MouseAbs { x: 32768, y: 32768, buttons: 0 }, &s, rect)?;
    std::thread::sleep(std::time::Duration::from_millis(200));
    println!("after MouseAbs centre: cursor at {:?}", cursor());
    for (fx, fy) in [(0.1f64, 0.1f64), (0.5, 0.5), (0.9, 0.9)] {
        let c = Contact { id: 0, x: (fx * 65535.0) as u16, y: (fy * 65535.0) as u16, pressure: 30000, major: 500 };
        inj.handle(&InputEvent::Touch { action: TouchAction::Down, changed: 0, contacts: vec![c] }, &s, rect)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let during = cursor();
        inj.handle(&InputEvent::Touch { action: TouchAction::Up, changed: 0, contacts: vec![c] }, &s, rect)?;
        std::thread::sleep(std::time::Duration::from_millis(250));
        println!("touch {fx},{fy}: expect {},{} -> during {:?} after {:?}", o.x + (fx * (o.width - 1) as f64).round() as i32, o.y + (fy * (o.height - 1) as f64).round() as i32, during, cursor());
    }
    {
        // two fingers: 0 down, 1 down, both move, 1 up, 0 up (a pinch)
        let c = |id: u8, x: f64| Contact { id, x: (x * 65535.0) as u16, y: 32768, pressure: 30000, major: 500 };
        let steps: [(TouchAction, u8, Vec<Contact>); 5] = [
            (TouchAction::Down, 0, vec![c(0, 0.4)]),
            (TouchAction::Down, 1, vec![c(0, 0.4), c(1, 0.6)]),
            (TouchAction::Move, 0, vec![c(0, 0.3), c(1, 0.7)]),
            (TouchAction::Up, 1, vec![c(0, 0.3), c(1, 0.7)]),
            (TouchAction::Up, 0, vec![c(0, 0.3)]),
        ];
        for (a, ch, cs) in steps {
            let r = inj.handle(&InputEvent::Touch { action: a, changed: ch, contacts: cs }, &s, rect);
            println!("two-finger {a:?} changed={ch}: {}", if r.is_ok() { "ok".into() } else { format!("{r:?}") });
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    }
    for (fx, fy) in [(0.1f64, 0.1f64), (0.9, 0.9)] {
        use dread_proto::{PenTool, PEN_CONTACT, PEN_IN_RANGE};
        let (x, y) = ((fx * 65535.0) as u16, (fy * 65535.0) as u16);
        let pen = |flags: u8, pressure: u16| InputEvent::Pen { tool: PenTool::Pen, flags, x, y, pressure, tilt_x: 0, tilt_y: 0 };
        for (f, p) in [(PEN_IN_RANGE, 0), (PEN_IN_RANGE | PEN_CONTACT, 30000), (PEN_IN_RANGE, 0), (0, 0)] {
            inj.handle(&pen(f, p), &s, rect)?;
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        println!("pen {fx},{fy}: expect {},{} -> after {:?}", o.x + (fx * (o.width - 1) as f64).round() as i32, o.y + (fy * (o.height - 1) as f64).round() as i32, cursor());
    }
    be.deactivate(&d)?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
