//! DREAD platform layer.
//!
//! Everything that differs between operating systems and display servers
//! lives here behind small interfaces: [`display::DisplayBackend`],
//! [`capture`], [`encode::Encoder`] and the input injectors.

pub mod capture;
pub mod display;
pub mod encode;
pub mod input;
