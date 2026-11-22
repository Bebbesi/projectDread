//! Windows only: attaches the DREAD virtual display (Extend), waits, detaches.
//! cargo run -p dread-platform --example win_attach -- [width] [height] [seconds]
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use dread_platform::display;
    let a: Vec<String> = std::env::args().collect();
    let w: u32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1920);
    let h: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1200);
    let secs: u64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    let mut be = display::detect_backend()?;
    println!("driver: {:?}", be.driver_status());
    let s = display::DisplaySettings { width: w, height: h, ..Default::default() };
    let d = be.activate(&s)?;
    println!("attached: {:?} capture_index={:?}", d.output, d.capture_index);
    println!("outputs: {:#?}", be.outputs()?);
    std::thread::sleep(std::time::Duration::from_secs(secs));
    // Second activate = what happens on a reconnect / settings change.
    let d2 = be.reconfigure(&d, &s)?;
    println!("reconfigured: {} {}x{}", d2.output.name, d2.output.width, d2.output.height);
    be.deactivate(&d2)?;
    println!("detached");
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
