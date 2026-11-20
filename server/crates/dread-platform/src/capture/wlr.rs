//! Capture through `wlr-screencopy-unstable-v1` (Hyprland, Sway, river, ...).
//!
//! Frames are requested with `copy_with_damage`, so the compositor only
//! answers when the output actually changed: an idle desktop costs no CPU,
//! no encoder time and no bandwidth.

use super::{now_us, Frame, PixelFormat};
use anyhow::{bail, Context, Result};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};
use tracing::{debug, info};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1 as frame_v1, zwlr_screencopy_manager_v1 as mgr_v1};

#[derive(Default)]
struct FrameState {
    shm_format: Option<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    y_invert: bool,
    ready: bool,
    failed: bool,
}

struct State {
    shm: Option<wl_shm::WlShm>,
    mgr: Option<mgr_v1::ZwlrScreencopyManagerV1>,
    mgr_version: u32,
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
    frame: FrameState,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(st: &mut Self, reg: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_shm" => st.shm = Some(reg.bind(name, 1, qh, ())),
                "zwlr_screencopy_manager_v1" => {
                    st.mgr_version = version.min(3);
                    st.mgr = Some(reg.bind(name, st.mgr_version, qh, ()));
                }
                "wl_output" if version >= 4 => {
                    let o: wl_output::WlOutput = reg.bind(name, 4, qh, ());
                    st.outputs.push((o, None));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(st: &mut Self, o: &wl_output::WlOutput, ev: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = ev {
            if let Some(e) = st.outputs.iter_mut().find(|(x, _)| x == o) {
                e.1 = Some(name);
            }
        }
    }
}

impl Dispatch<frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(st: &mut Self, _: &frame_v1::ZwlrScreencopyFrameV1, ev: frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            frame_v1::Event::Buffer { format: WEnum::Value(f), width, height, stride } => {
                st.frame.shm_format = Some((f, width, height, stride))
            }
            frame_v1::Event::BufferDone => st.frame.buffer_done = true,
            frame_v1::Event::Flags { flags: WEnum::Value(f) } => st.frame.y_invert = f.contains(frame_v1::Flags::YInvert),
            frame_v1::Event::Ready { .. } => st.frame.ready = true,
            frame_v1::Event::Failed => st.frame.failed = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(State: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(State: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(State: mgr_v1::ZwlrScreencopyManagerV1);

struct ShmBuffer {
    buffer: wl_buffer::WlBuffer,
    pool: wl_shm_pool::WlShmPool,
    _fd: OwnedFd,
    ptr: *mut u8,
    len: usize,
    key: (wl_shm::Format, u32, u32, u32),
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
        // SAFETY: ptr/len come from our own successful mmap.
        unsafe { libc::munmap(self.ptr as *mut _, self.len) };
    }
}

// The raw pointer is only touched from the owning capture thread.
unsafe impl Send for ShmBuffer {}

pub struct WlrCapture {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    output: wl_output::WlOutput,
    pub output_name: String,
    buffer: Option<ShmBuffer>,
    pending: Option<frame_v1::ZwlrScreencopyFrameV1>,
    pending_damage: bool,
    copied: bool,
    overlay_cursor: bool,
}

fn map_format(f: wl_shm::Format) -> Option<PixelFormat> {
    Some(match f {
        wl_shm::Format::Xrgb8888 => PixelFormat::Bgr0,
        wl_shm::Format::Argb8888 => PixelFormat::Bgra,
        wl_shm::Format::Xbgr8888 => PixelFormat::Rgb0,
        wl_shm::Format::Abgr8888 => PixelFormat::Rgba,
        _ => return None,
    })
}

impl WlrCapture {
    pub fn new(output_name: &str, overlay_cursor: bool) -> Result<Self> {
        let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let mut state = State { shm: None, mgr: None, mgr_version: 0, outputs: vec![], frame: FrameState::default() };
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;
        if state.mgr.is_none() {
            bail!("compositor does not support wlr-screencopy-unstable-v1");
        }
        if state.mgr_version < 2 {
            bail!("wlr-screencopy v{} is too old (need v2+)", state.mgr_version);
        }
        let output = state
            .outputs
            .iter()
            .find(|(_, n)| n.as_deref() == Some(output_name))
            .map(|(o, _)| o.clone())
            .with_context(|| format!("output {output_name} not found on the Wayland display"))?;
        info!(output = output_name, version = state.mgr_version, "screencopy capture ready");
        Ok(Self {
            conn,
            queue,
            state,
            output,
            output_name: output_name.to_string(),
            buffer: None,
            pending: None,
            pending_damage: false,
            copied: false,
            overlay_cursor,
        })
    }

    fn ensure_buffer(&mut self, key: (wl_shm::Format, u32, u32, u32)) -> Result<()> {
        if self.buffer.as_ref().map(|b| b.key) == Some(key) {
            return Ok(());
        }
        self.buffer = None;
        let (fmt, w, h, stride) = key;
        let len = (stride * h) as usize;
        // SAFETY: anonymous memfd owned by us.
        let raw = unsafe { libc::memfd_create(c"dread-capture".as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            bail!("memfd_create: {}", std::io::Error::last_os_error());
        }
        // SAFETY: freshly created fd.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: valid fd, positive length.
        if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } < 0 {
            bail!("ftruncate: {}", std::io::Error::last_os_error());
        }
        // SAFETY: mapping the memfd we just sized.
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
        if ptr == libc::MAP_FAILED {
            bail!("mmap: {}", std::io::Error::last_os_error());
        }
        let qh = self.queue.handle();
        let shm = self.state.shm.as_ref().context("no wl_shm")?;
        let pool = shm.create_pool(fd.as_fd(), len as i32, &qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, fmt, &qh, ());
        debug!(w, h, stride, ?fmt, "allocated capture buffer");
        self.buffer = Some(ShmBuffer { buffer, pool, _fd: fd, ptr: ptr as *mut u8, len, key });
        Ok(())
    }

    /// Drops any outstanding request (e.g. before forcing a full frame).
    pub fn cancel(&mut self) {
        if let Some(f) = self.pending.take() {
            f.destroy();
            let _ = self.conn.flush();
        }
    }

    /// Waits up to `timeout` for the next frame.
    ///
    /// With `force == false` the compositor only answers once the output has
    /// changed; `force == true` copies the current contents immediately
    /// (used for the first frame and key-frame requests).
    pub fn next_frame(&mut self, force: bool, timeout: Duration) -> Result<Option<Frame<'_>>> {
        if force && self.pending_damage {
            self.cancel();
        }
        if self.pending.is_none() {
            let qh = self.queue.handle();
            let mgr = self.state.mgr.as_ref().unwrap();
            self.state.frame = FrameState::default();
            self.pending = Some(mgr.capture_output(self.overlay_cursor as i32, &self.output, &qh, ()));
            self.pending_damage = !force;
            self.copied = false;
        }
        let deadline = Instant::now() + timeout;
        loop {
            // once the compositor described the buffer, attach ours and copy
            let described = self.state.frame.buffer_done || (self.state.mgr_version < 3 && self.state.frame.shm_format.is_some());
            if !self.copied && described {
                let key = self.state.frame.shm_format.context("compositor offered no shm buffer format")?;
                if map_format(key.0).is_none() {
                    bail!("unsupported capture pixel format {:?}", key.0);
                }
                self.ensure_buffer(key)?;
                let f = self.pending.as_ref().unwrap();
                let b = &self.buffer.as_ref().unwrap().buffer;
                if self.pending_damage {
                    f.copy_with_damage(b);
                } else {
                    f.copy(b);
                }
                self.copied = true;
            }
            if self.state.frame.failed {
                self.cancel();
                bail!("compositor reported a failed capture of {}", self.output_name);
            }
            if self.state.frame.ready {
                if let Some(f) = self.pending.take() {
                    f.destroy();
                }
                let b = self.buffer.as_ref().unwrap();
                let (fmt, w, h, stride) = b.key;
                // SAFETY: buffer is mapped for `len` bytes and the compositor finished writing it.
                let data = unsafe { std::slice::from_raw_parts(b.ptr, b.len) };
                return Ok(Some(Frame {
                    data,
                    width: w,
                    height: h,
                    stride,
                    format: map_format(fmt).unwrap(),
                    y_invert: self.state.frame.y_invert,
                    ready_us: now_us(),
                }));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            self.pump(deadline - now)?;
        }
    }

    fn pump(&mut self, timeout: Duration) -> Result<()> {
        self.conn.flush()?;
        if self.queue.dispatch_pending(&mut self.state)? > 0 {
            return Ok(());
        }
        if let Some(guard) = self.queue.prepare_read() {
            let mut pfd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let ms = timeout.as_millis().clamp(1, i32::MAX as u128) as i32;
            // SAFETY: one valid pollfd.
            let r = unsafe { libc::poll(&mut pfd, 1, ms) };
            if r > 0 {
                guard.read().context("reading from Wayland display")?;
            }
        }
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }
}

impl Drop for WlrCapture {
    fn drop(&mut self) {
        self.cancel();
        self.buffer = None;
        let _ = self.conn.flush();
    }
}
