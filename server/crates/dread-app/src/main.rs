//! DREAD Server — turns an Android device into a real additional monitor.

// no console window for the control panel on Windows
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod logging;
mod ui;

use anyhow::Result;
use dread_core::config::Config;
use dread_core::Engine;
use tracing::info;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--help") || has("-h") {
        println!(
            "DREAD Server {}\n\nUsage: dread [--headless] [--minimized] [--print-pairing-uri]\n\n  \
             --headless           run without the control panel (logs to stderr, Ctrl-C to quit)\n  \
             --minimized          start with the control panel minimized\n  \
             --print-pairing-uri  headless: print the QR pairing URI on start\n  \
             --page NAME          open the control panel on a page (dashboard, display, streaming, connection, input, advanced)\n\n\
             Configuration: {}",
            env!("CARGO_PKG_VERSION"),
            Config::path().display()
        );
        return Ok(());
    }
    let config = Config::load();
    let logs = logging::init(&config.general.log_level);
    info!("DREAD Server {} starting", env!("CARGO_PKG_VERSION"));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4).thread_name("dread-net").build()?;
    let engine = Engine::new(config, logs, rt.handle().clone())?;
    dread_core::start_services(&engine);

    // SIGTERM / SIGINT: tear the virtual display down before exiting
    #[cfg(unix)]
    {
        let e = engine.clone();
        rt.spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return };
            tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
            e.shutdown();
            std::process::exit(0);
        });
    }

    if has("--headless") {
        if has("--print-pairing-uri") {
            println!("PAIRING_URI {}", engine.pairing_uri());
        }
        println!("fingerprint {}", engine.identity.fingerprint_short());
        // the signal task above performs shutdown and exits the process
        rt.block_on(async {
            #[cfg(unix)]
            std::future::pending::<()>().await;
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
        });
        engine.shutdown();
        rt.shutdown_timeout(std::time::Duration::from_millis(500));
        return Ok(());
    }
    let minimized = has("--minimized") || engine.config.read().general.start_minimized;
    let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).map(|p| ui::page_from_name(p)).unwrap_or(ui::Page::Dashboard);
    let res = ui::run(engine.clone(), minimized, page);
    engine.shutdown();
    // sessions blocked on a USB pipe must not keep the process alive
    rt.shutdown_timeout(std::time::Duration::from_millis(500));
    res
}
