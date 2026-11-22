//! Windows only: same probe as win_touch, but with the legacy InjectTouchInput API.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use dread_platform::display;
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::UI::Input::Pointer::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    let cursor = || { let mut p = POINT::default(); unsafe { GetCursorPos(&mut p) }.ok(); (p.x, p.y) };
    let mut be = display::detect_backend()?;
    let d = be.activate(&display::DisplaySettings { width: 1920, height: 1200, ..Default::default() })?;
    let o = d.output.clone();
    std::thread::sleep(std::time::Duration::from_millis(800));
    unsafe { InitializeTouchInjection(10, TOUCH_FEEDBACK_DEFAULT) }?;
    let ti = |p: POINT, flags: POINTER_FLAGS| POINTER_TOUCH_INFO {
        pointerInfo: POINTER_INFO { pointerType: PT_TOUCH, pointerId: 0, pointerFlags: flags, ptPixelLocation: p, ..Default::default() },
        touchFlags: 0, touchMask: TOUCH_MASK_CONTACTAREA | TOUCH_MASK_PRESSURE, pressure: 512,
        rcContact: RECT { left: p.x - 4, top: p.y - 4, right: p.x + 4, bottom: p.y + 4 }, ..Default::default()
    };
    for (fx, fy) in [(0.1f64, 0.1f64), (0.5, 0.5), (0.9, 0.9)] {
        let p = POINT { x: o.x + (fx * (o.width - 1) as f64).round() as i32, y: o.y + (fy * (o.height - 1) as f64).round() as i32 };
        let r1 = unsafe { InjectTouchInput(&[ti(p, POINTER_FLAG_DOWN | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT)]) };
        std::thread::sleep(std::time::Duration::from_millis(80));
        let r2 = unsafe { InjectTouchInput(&[ti(p, POINTER_FLAG_UP)]) };
        std::thread::sleep(std::time::Duration::from_millis(250));
        println!("touch {fx},{fy}: expect {},{} -> after {:?} ({:?} {:?})", p.x, p.y, cursor(), r1.is_ok(), r2.is_ok());
    }
    be.deactivate(&d)?;
    Ok(())
}
#[cfg(not(windows))]
fn main() {}
