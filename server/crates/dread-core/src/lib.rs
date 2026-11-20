//! DREAD engine.

pub mod autostart;
pub mod config;
pub mod diag;
pub mod engine;
pub mod input_thread;
pub mod net;
pub mod pipeline;
pub mod security;
pub mod session;
pub mod usb;

pub use engine::Engine;

/// Starts network listeners, discovery and the USB watcher.
pub fn start_services(e: &Engine) {
    e.rt.spawn(net::serve(e.clone()));
    e.rt.spawn(net::discovery(e.clone()));
    usb::spawn(e.clone());
}
