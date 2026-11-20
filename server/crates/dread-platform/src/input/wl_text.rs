//! Unicode text injection and text-field focus detection on wlroots-style
//! compositors (Hyprland, Sway, river, ...).
//!
//! * `zwp_input_method_v2` — DREAD registers as the seat's input method. The
//!   compositor tells it when a text field gains/loses focus (used to pop the
//!   Android soft keyboard) and DREAD commits text straight into the field.
//!   Only one input method may exist per seat; when another one (fcitx5, ibus)
//!   is running the compositor answers `unavailable` and DREAD falls back.
//! * `zwp_virtual_keyboard_v1` — fallback: a private keymap is generated for the
//!   characters of each string, uploaded, and the keys are pressed. This types
//!   any Unicode character independent of the user's keyboard layout.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::{zwp_input_method_manager_v2, zwp_input_method_v2};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1};

pub enum TextCmd {
    Type(String),
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEvent {
    /// A text field gained (true) or lost (false) focus.
    FieldFocus(bool),
}

struct State {
    seat: Option<wl_seat::WlSeat>,
    vk_mgr: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    im_mgr: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    im: Option<zwp_input_method_v2::ZwpInputMethodV2>,
    im_available: bool,
    pending_active: bool,
    active: bool,
    done_serial: u32,
    events: Sender<TextEvent>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(st: &mut Self, reg: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_seat" if st.seat.is_none() => st.seat = Some(reg.bind(name, version.min(7), qh, ())),
                "zwp_virtual_keyboard_manager_v1" => st.vk_mgr = Some(reg.bind(name, 1, qh, ())),
                "zwp_input_method_manager_v2" => st.im_mgr = Some(reg.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for State {
    fn event(st: &mut Self, _: &zwp_input_method_v2::ZwpInputMethodV2, ev: zwp_input_method_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwp_input_method_v2::Event as E;
        match ev {
            E::Activate => st.pending_active = true,
            E::Deactivate => st.pending_active = false,
            E::Done => {
                st.done_serial = st.done_serial.wrapping_add(1);
                if st.pending_active != st.active {
                    st.active = st.pending_active;
                    let _ = st.events.send(TextEvent::FieldFocus(st.active));
                }
            }
            E::Unavailable => {
                warn!("input-method-v2 unavailable (another input method is running); text uses virtual-keyboard and the Android keyboard will not open automatically");
                st.im_available = false;
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(State: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(State: zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(State: zwp_input_method_manager_v2::ZwpInputMethodManagerV2);

fn keysym_for(c: char) -> String {
    match c {
        '\n' | '\r' => "Return".into(),
        '\t' => "Tab".into(),
        ' ' => "space".into(),
        c => format!("U{:04X}", c as u32),
    }
}

fn memfd_with(data: &[u8]) -> Result<OwnedFd> {
    // SAFETY: memfd_create with a static NUL-terminated name.
    let fd = unsafe { libc::memfd_create(c"dread-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        bail!("memfd_create: {}", std::io::Error::last_os_error());
    }
    // SAFETY: fd was just created and is owned here.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut f = std::fs::File::from(owned.try_clone()?);
    f.write_all(data)?;
    Ok(owned)
}

/// Builds an XKB keymap that maps keycodes 9.. to the given characters.
fn build_keymap(chars: &[char]) -> String {
    let mut codes = String::new();
    let mut syms = String::new();
    for (i, c) in chars.iter().enumerate() {
        codes.push_str(&format!("  <K{i}> = {};\n", i + 9));
        syms.push_str(&format!("  key <K{i}> {{ [ {} ] }};\n", keysym_for(*c)));
    }
    format!(
        "xkb_keymap {{\nxkb_keycodes \"dread\" {{\n  minimum = 8;\n  maximum = 255;\n{codes}}};\n\
         xkb_types \"dread\" {{ include \"complete\" }};\n\
         xkb_compatibility \"dread\" {{ include \"complete\" }};\n\
         xkb_symbols \"dread\" {{\n{syms}}};\n}};\n\0"
    )
}

pub struct WlText {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    vk: Option<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1>,
    start: Instant,
}

impl WlText {
    pub fn connect(events: Sender<TextEvent>, use_input_method: bool) -> Result<Self> {
        let conn = Connection::connect_to_env().context("no Wayland display")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let mut state = State {
            seat: None,
            vk_mgr: None,
            im_mgr: None,
            im: None,
            im_available: false,
            pending_active: false,
            active: false,
            done_serial: 0,
            events,
        };
        queue.roundtrip(&mut state)?;
        let seat = state.seat.clone().context("compositor exposes no wl_seat")?;
        let vk = state.vk_mgr.as_ref().map(|m| m.create_virtual_keyboard(&seat, &qh, ()));
        if use_input_method {
            if let Some(m) = &state.im_mgr {
                state.im = Some(m.get_input_method(&seat, &qh, ()));
                state.im_available = true;
            }
        }
        queue.roundtrip(&mut state)?;
        if vk.is_none() && !state.im_available {
            bail!("compositor supports neither zwp_virtual_keyboard_v1 nor zwp_input_method_v2");
        }
        info!(virtual_keyboard = vk.is_some(), input_method = state.im_available, "wayland text input ready");
        Ok(Self { conn, queue, state, vk, start: Instant::now() })
    }

    pub fn type_text(&mut self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        if self.state.im_available && self.state.active {
            if let Some(im) = &self.state.im {
                im.commit_string(text.to_string());
                im.commit(self.state.done_serial);
                self.conn.flush()?;
                return Ok(());
            }
        }
        let Some(vk) = &self.vk else { bail!("no virtual keyboard available for text injection") };
        // chunk so each keymap stays small
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(200) {
            let mut uniq: Vec<char> = Vec::new();
            for c in chunk {
                if !uniq.contains(c) {
                    uniq.push(*c);
                }
            }
            let km = build_keymap(&uniq);
            let fd = memfd_with(km.as_bytes())?;
            vk.keymap(1, fd.as_fd(), km.len() as u32);
            for c in chunk {
                let idx = uniq.iter().position(|u| u == c).unwrap_or(0) as u32;
                let t = self.start.elapsed().as_millis() as u32;
                vk.key(t, idx + 1, 1); // evdev code = xkb code - 8
                vk.key(t, idx + 1, 0);
            }
            self.conn.flush()?;
            self.queue.roundtrip(&mut self.state)?;
        }
        debug!(len = text.len(), "typed text via virtual keyboard");
        Ok(())
    }

    /// Runs the text thread: executes commands and reports focus changes.
    pub fn run(mut self, cmds: Receiver<TextCmd>) {
        loop {
            loop {
                match cmds.try_recv() {
                    Ok(TextCmd::Type(t)) => {
                        if let Err(e) = self.type_text(&t) {
                            warn!("text injection failed: {e:#}");
                        }
                    }
                    Ok(TextCmd::Stop) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                }
            }
            if let Err(e) = self.pump(Duration::from_millis(8)) {
                warn!("wayland text connection lost: {e:#}");
                return;
            }
        }
    }

    fn pump(&mut self, timeout: Duration) -> Result<()> {
        self.queue.dispatch_pending(&mut self.state)?;
        self.conn.flush()?;
        if let Some(guard) = self.queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut pfd = libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&fd), events: libc::POLLIN, revents: 0 };
            // SAFETY: single valid pollfd.
            let r = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as i32) };
            if r > 0 {
                guard.read()?;
            }
        }
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }
}
