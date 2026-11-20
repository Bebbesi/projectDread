//! Screen capture.

#[cfg(target_os = "linux")]
pub mod wlr;

/// Pixel layout of captured frames, named after FFmpeg's `pix_fmt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgr0,
    Bgra,
    Rgb0,
    Rgba,
}

impl PixelFormat {
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            PixelFormat::Bgr0 => "bgr0",
            PixelFormat::Bgra => "bgra",
            PixelFormat::Rgb0 => "rgb0",
            PixelFormat::Rgba => "rgba",
        }
    }
}

/// A captured frame, borrowed from the capture buffer.
pub struct Frame<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
    pub y_invert: bool,
    /// Monotonic microseconds when the frame became ready.
    pub ready_us: u64,
}

impl Frame<'_> {
    /// Writes the frame as tightly packed rows (what FFmpeg `rawvideo` expects).
    pub fn write_packed<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        let row = self.width as usize * 4;
        let stride = self.stride as usize;
        if !self.y_invert && stride == row {
            return w.write_all(&self.data[..row * self.height as usize]);
        }
        for i in 0..self.height as usize {
            let r = if self.y_invert { self.height as usize - 1 - i } else { i };
            w.write_all(&self.data[r * stride..r * stride + row])?;
        }
        Ok(())
    }
}

/// Monotonic clock shared by capture, encode and transport (microseconds).
pub fn now_us() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}
