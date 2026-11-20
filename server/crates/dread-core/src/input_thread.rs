//! Input injection thread: owns the OS input devices so that injection
//! never waits on the network or the video pipeline.

use crate::engine::{Engine, InputCmd};
use dread_platform::input::{AbsMap, InputSettings};
use dread_proto::InputEvent;
use serde_json::json;
use std::sync::mpsc::Receiver;
use tracing::{error, info, warn};

pub fn spawn(e: Engine, rx: Receiver<InputCmd>, settings: InputSettings) {
    std::thread::Builder::new()
        .name("input".into())
        .spawn(move || run(e, rx, settings))
        .expect("spawn input thread");
}

#[cfg(target_os = "linux")]
fn start_text(e: &Engine) {
    use dread_platform::input::wl_text::{TextCmd, TextEvent, WlText};
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    let (ev_tx, ev_rx) = std::sync::mpsc::channel::<TextEvent>();
    let use_im = e.config.read().input.auto_keyboard;
    match WlText::connect(ev_tx, use_im) {
        Ok(wl) => {
            let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<TextCmd>();
            let (str_tx, str_rx) = std::sync::mpsc::channel::<String>();
            *e.text_tx.lock() = Some(str_tx);
            std::thread::Builder::new().name("wl-text".into()).spawn(move || wl.run(cmd_rx)).ok();
            std::thread::Builder::new()
                .name("wl-text-fwd".into())
                .spawn(move || {
                    while let Ok(s) = str_rx.recv() {
                        if cmd_tx.send(TextCmd::Type(s)).is_err() {
                            break;
                        }
                    }
                })
                .ok();
            let e2 = e.clone();
            std::thread::Builder::new()
                .name("ime-events".into())
                .spawn(move || {
                    while let Ok(TextEvent::FieldFocus(on)) = ev_rx.recv() {
                        if !e2.config.read().input.auto_keyboard {
                            continue;
                        }
                        let show = on && {
                            let active = e2.active_display().map(|d| d.output.name);
                            let focused = e2.display.lock().backend.as_mut().and_then(|b| b.focused_output());
                            active.is_some() && active == focused
                        };
                        e2.send_to_all(&json!({"t":"ime","show":show}));
                    }
                })
                .ok();
        }
        Err(err) => warn!("Unicode text input unavailable: {err:#}"),
    }
}

#[cfg(target_os = "linux")]
fn run(e: Engine, rx: Receiver<InputCmd>, mut settings: InputSettings) {
    use dread_platform::input::linux::LinuxInjector;
    let mut inj = match LinuxInjector::new() {
        Ok(i) => {
            info!("input devices created (touchscreen, pen, pointer, mouse, keyboard)");
            Some(i)
        }
        Err(err) => {
            e.set_error(Some(format!("Input: {err:#}")));
            None
        }
    };
    start_text(&e);
    let mut abs = AbsMap::identity();
    while let Ok(cmd) = rx.recv() {
        match cmd {
            InputCmd::Event(InputEvent::Text(t)) => {
                if settings.keyboard_enabled {
                    e.type_text(t);
                }
            }
            InputCmd::Event(ev) => {
                if let Some(i) = inj.as_mut() {
                    if let Err(err) = i.handle(&ev, &settings, &abs) {
                        error!("input injection failed: {err:#}");
                    }
                }
            }
            InputCmd::AbsMap(m) => abs = m,
            InputCmd::Settings(s) => settings = s,
            InputCmd::ReleaseAll => {
                if let Some(i) = inj.as_mut() {
                    i.release_all();
                }
            }
        }
    }
}

#[cfg(windows)]
fn run(e: Engine, rx: Receiver<InputCmd>, mut settings: InputSettings) {
    use dread_platform::input::windows::WindowsInjector;
    let mut inj = match WindowsInjector::new() {
        Ok(i) => Some(i),
        Err(err) => {
            e.set_error(Some(format!("Input: {err:#}")));
            None
        }
    };
    while let Ok(cmd) = rx.recv() {
        match cmd {
            InputCmd::Event(ev) => {
                if let Some(i) = inj.as_mut() {
                    let rect = e.active_display().map(|d| (d.output.x, d.output.y, d.output.width, d.output.height));
                    if let Err(err) = i.handle(&ev, &settings, rect) {
                        error!("input injection failed: {err:#}");
                    }
                }
            }
            InputCmd::AbsMap(_) => {}
            InputCmd::Settings(s) => settings = s,
            InputCmd::ReleaseAll => {
                if let Some(i) = inj.as_mut() {
                    i.release_all();
                }
            }
        }
    }
}
