//! Windows input injection.
//!
//! STATUS: written against the Win32 documentation; NOT yet compiled or run
//! on Windows (see docs/PLATFORMS.md).
//!
//! * Touch and pen use synthetic pointer devices
//!   (`CreateSyntheticPointerDevice` / `InjectSyntheticPointerInput`, Windows
//!   10 1809+), so applications receive real `WM_POINTER` touch and pen input
//!   with pressure, tilt, eraser, barrel button and hover — Windows Ink works.
//! * Mouse, wheel, keys and Unicode text use `SendInput`.
//! * Coordinates are mapped onto the DREAD monitor's desktop rectangle.

use super::InputSettings;
use anyhow::{bail, Result};
use dread_proto::{Contact, InputEvent, PenTool, TouchAction, MOUSE_LEFT, MOUSE_MIDDLE, MOUSE_RIGHT, PEN_BUTTON1, PEN_BUTTON2, PEN_CONTACT, PEN_IN_RANGE};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Controls::{HSYNTHETICPOINTERDEVICE, POINTER_FEEDBACK_DEFAULT};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::Pointer::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const MAX_TOUCH: usize = 10;

pub struct WindowsInjector {
    touch_dev: HSYNTHETICPOINTERDEVICE,
    pen_dev: HSYNTHETICPOINTERDEVICE,
    touch_down: Vec<(u8, POINT)>,
    pen_down: bool,
    pen_in_range: bool,
    last_pen: Option<(PenTool, POINT)>,
    buttons: u8,
    pressed_keys: Vec<u16>,
    last_rect: Option<(i32, i32, u32, u32)>,
}

// handles are only used from the input thread
unsafe impl Send for WindowsInjector {}

/// evdev key code → (PS/2 set-1 scan code, extended)
fn scancode(code: u16) -> Option<(u16, bool)> {
    Some(match code {
        1..=83 => (code, false), // evdev and set-1 agree for the main block
        87 => (0x57, false),     // F11
        88 => (0x58, false),     // F12
        96 => (0x1c, true),      // keypad enter
        97 => (0x1d, true),      // right ctrl
        98 => (0x35, true),      // keypad /
        99 => (0x37, true),      // sysrq
        100 => (0x38, true),     // right alt
        102 => (0x47, true),     // home
        103 => (0x48, true),     // up
        104 => (0x49, true),     // page up
        105 => (0x4b, true),     // left
        106 => (0x4d, true),     // right
        107 => (0x4f, true),     // end
        108 => (0x50, true),     // down
        109 => (0x51, true),     // page down
        110 => (0x52, true),     // insert
        111 => (0x53, true),     // delete
        119 => (0x45, false),    // pause
        125 => (0x5b, true),     // left meta / Windows
        126 => (0x5c, true),     // right meta
        127 => (0x5d, true),     // menu
        _ => return None,
    })
}

fn send(inputs: &[INPUT]) {
    // SAFETY: slice of fully initialised INPUT structures
    unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn key_input(sc: u16, ext: bool, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if ext {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: sc, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn mouse_input(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

impl WindowsInjector {
    pub fn new() -> Result<Self> {
        // SAFETY: plain API calls; failure is reported via Result
        let touch_dev = unsafe { CreateSyntheticPointerDevice(PT_TOUCH, MAX_TOUCH as u32, POINTER_FEEDBACK_DEFAULT) }?;
        let pen_dev = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT) }?;
        Ok(Self { touch_dev, pen_dev, touch_down: vec![], pen_down: false, pen_in_range: false, last_pen: None, buttons: 0, pressed_keys: vec![], last_rect: None })
    }

    fn map(rect: (i32, i32, u32, u32), x: u16, y: u16) -> POINT {
        let (rx, ry, rw, rh) = rect;
        POINT { x: rx + ((x as f64 / 65535.0) * (rw.saturating_sub(1)) as f64).round() as i32, y: ry + ((y as f64 / 65535.0) * (rh.saturating_sub(1)) as f64).round() as i32 }
    }

    pub fn handle(&mut self, ev: &InputEvent, s: &InputSettings, rect: Option<(i32, i32, u32, u32)>) -> Result<()> {
        let Some(rect) = rect else { return Ok(()) };
        self.last_rect = Some(rect);
        match ev {
            InputEvent::Touch { action, changed, contacts } if s.touch_enabled => self.touch(*action, *changed, contacts, rect),
            InputEvent::Pen { tool, flags, x, y, pressure, tilt_x, tilt_y } if s.pen_enabled => {
                self.pen(*tool, *flags, Self::map(rect, *x, *y), *pressure, *tilt_x, *tilt_y, s)
            }
            InputEvent::MouseAbs { x, y, buttons } if s.mouse_enabled => {
                self.mouse_abs(Self::map(rect, *x, *y), *buttons);
                Ok(())
            }
            InputEvent::MouseRel { dx, dy, buttons } if s.mouse_enabled => {
                let k = s.mouse_speed.clamp(0.1, 10.0);
                send(&[mouse_input((*dx as f32 * k) as i32, (*dy as f32 * k) as i32, 0, MOUSEEVENTF_MOVE)]);
                self.set_buttons(*buttons);
                Ok(())
            }
            InputEvent::Scroll { vertical, horizontal } if s.mouse_enabled => {
                let k = s.scroll_speed * if s.invert_scroll { -1.0 } else { 1.0 };
                if *vertical != 0 {
                    // protocol: positive = down; WHEEL_DELTA positive = up
                    send(&[mouse_input(0, 0, -((*vertical as f32 * k) as i32), MOUSEEVENTF_WHEEL)]);
                }
                if *horizontal != 0 {
                    send(&[mouse_input(0, 0, (*horizontal as f32 * k) as i32, MOUSEEVENTF_HWHEEL)]);
                }
                Ok(())
            }
            InputEvent::Key { code, down } if s.keyboard_enabled => {
                if let Some((sc, ext)) = scancode(*code) {
                    send(&[key_input(sc, ext, !*down)]);
                    if *down {
                        self.pressed_keys.push(*code);
                    } else {
                        self.pressed_keys.retain(|k| k != code);
                    }
                }
                Ok(())
            }
            InputEvent::Text(t) if s.keyboard_enabled => {
                let mut v = vec![];
                for u in t.encode_utf16() {
                    for up in [false, true] {
                        let mut flags = KEYEVENTF_UNICODE;
                        if up {
                            flags |= KEYEVENTF_KEYUP;
                        }
                        v.push(INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: u, dwFlags: flags, time: 0, dwExtraInfo: 0 } } });
                    }
                }
                send(&v);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn mouse_abs(&mut self, p: POINT, buttons: u8) {
        // SAFETY: simple metric queries
        let (vx, vy, vw, vh) = unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
        let nx = (((p.x - vx) as f64 * 65535.0) / (vw.max(2) - 1) as f64).round() as i32;
        let ny = (((p.y - vy) as f64 * 65535.0) / (vh.max(2) - 1) as f64).round() as i32;
        send(&[mouse_input(nx, ny, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)]);
        self.set_buttons(buttons);
    }

    fn set_buttons(&mut self, b: u8) {
        let mut v = vec![];
        for (mask, down, up) in [(MOUSE_LEFT, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP), (MOUSE_RIGHT, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP), (MOUSE_MIDDLE, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP)] {
            if (b ^ self.buttons) & mask != 0 {
                v.push(mouse_input(0, 0, 0, if b & mask != 0 { down } else { up }));
            }
        }
        self.buttons = b;
        if !v.is_empty() {
            send(&v);
        }
    }

    fn pointer_info(id: u32, p: POINT, flags: POINTER_FLAGS, ty: POINTER_INPUT_TYPE) -> POINTER_INFO {
        POINTER_INFO { pointerType: ty, pointerId: id, pointerFlags: flags, ptPixelLocation: p, ptPixelLocationRaw: p, ..Default::default() }
    }

    fn touch(&mut self, action: TouchAction, changed: u8, contacts: &[Contact], rect: (i32, i32, u32, u32)) -> Result<()> {
        let mut frames: Vec<POINTER_TYPE_INFO> = vec![];
        let lifting = matches!(action, TouchAction::Up | TouchAction::Cancel);
        for c in contacts.iter().take(MAX_TOUCH) {
            let p = Self::map(rect, c.x, c.y);
            let was_down = self.touch_down.iter().any(|(id, _)| *id == c.id);
            let up = action == TouchAction::Cancel || (lifting && c.id == changed);
            let flags = if up {
                POINTER_FLAG_UP
            } else if !was_down {
                POINTER_FLAG_DOWN | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT
            } else {
                POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT
            };
            let mut ti = POINTER_TOUCH_INFO {
                pointerInfo: Self::pointer_info(c.id as u32, p, flags, PT_TOUCH),
                touchFlags: TOUCH_FLAG_NONE,
                touchMask: TOUCH_MASK_CONTACTAREA | TOUCH_MASK_PRESSURE,
                pressure: ((c.pressure as u32 * 1024) / 65535).max(1),
                ..Default::default()
            };
            let r = ((c.major as f64 / 65535.0) * rect.2 as f64 / 2.0).max(2.0) as i32;
            ti.rcContact = windows::Win32::Foundation::RECT { left: p.x - r, top: p.y - r, right: p.x + r, bottom: p.y + r };
            frames.push(POINTER_TYPE_INFO { r#type: PT_TOUCH, Anonymous: POINTER_TYPE_INFO_0 { touchInfo: ti } });
            if up {
                self.touch_down.retain(|(id, _)| *id != c.id);
            } else if !was_down {
                self.touch_down.push((c.id, p));
            }
        }
        if frames.is_empty() {
            return Ok(());
        }
        // SAFETY: frames are fully initialised and belong to our touch device
        unsafe { InjectSyntheticPointerInput(self.touch_dev, &frames) }.map_err(|e| anyhow::anyhow!("inject touch: {e}"))
    }

    #[allow(clippy::too_many_arguments)]
    fn pen(&mut self, tool: PenTool, flags: u8, p: POINT, pressure: u16, tx: i16, ty: i16, s: &InputSettings) -> Result<()> {
        let in_range = flags & (PEN_IN_RANGE | PEN_CONTACT) != 0;
        let contact = flags & PEN_CONTACT != 0;
        if !in_range && !self.pen_in_range {
            return Ok(());
        }
        let mut pf = POINTER_FLAG_NONE;
        if in_range {
            pf |= POINTER_FLAG_INRANGE;
        }
        if contact {
            pf |= POINTER_FLAG_INCONTACT;
        }
        pf |= match (self.pen_down, contact) {
            (false, true) => POINTER_FLAG_DOWN,
            (true, false) => POINTER_FLAG_UP,
            _ => POINTER_FLAG_UPDATE,
        };
        let mut pen_flags = PEN_FLAG_NONE;
        if flags & PEN_BUTTON1 != 0 {
            pen_flags |= PEN_FLAG_BARREL;
        }
        if tool == PenTool::Eraser {
            pen_flags |= PEN_FLAG_ERASER | PEN_FLAG_INVERTED;
        }
        let _ = flags & PEN_BUTTON2;
        let curved = (pressure as f32 / 65535.0).powf(s.pen_pressure_gamma.clamp(0.2, 5.0));
        let pt = if in_range { p } else { self.last_pen.map(|l| l.1).unwrap_or(p) };
        let info = POINTER_PEN_INFO {
            pointerInfo: Self::pointer_info(0, pt, pf, PT_PEN),
            penFlags: pen_flags,
            penMask: PEN_MASK_PRESSURE | if s.pen_tilt { PEN_MASK_TILT_X | PEN_MASK_TILT_Y } else { PEN_MASK_NONE },
            pressure: if contact { ((curved * 1024.0) as u32).max(1) } else { 0 },
            rotation: 0,
            tiltX: (tx / 100) as i32,
            tiltY: (ty / 100) as i32,
        };
        let frame = [POINTER_TYPE_INFO { r#type: PT_PEN, Anonymous: POINTER_TYPE_INFO_0 { penInfo: info } }];
        // SAFETY: frame is fully initialised and belongs to our pen device
        let r = unsafe { InjectSyntheticPointerInput(self.pen_dev, &frame) };
        self.pen_down = contact;
        self.pen_in_range = in_range;
        self.last_pen = Some((tool, pt));
        if let Err(e) = r {
            bail!("inject pen: {e}");
        }
        Ok(())
    }

    pub fn release_all(&mut self) {
        let downs: Vec<Contact> = self.touch_down.iter().map(|(id, _)| Contact { id: *id, x: 0, y: 0, pressure: 0, major: 0 }).collect();
        if let (false, Some(rect)) = (downs.is_empty(), self.last_rect) {
            let _ = self.touch(TouchAction::Cancel, 0, &downs, rect);
        }
        self.touch_down.clear();
        if self.pen_in_range {
            if let Some((tool, p)) = self.last_pen {
                let _ = self.pen(tool, 0, p, 0, 0, 0, &InputSettings::default());
            }
        }
        self.set_buttons(0);
        for k in std::mem::take(&mut self.pressed_keys) {
            if let Some((sc, ext)) = scancode(k) {
                send(&[key_input(sc, ext, true)]);
            }
        }
    }
}

impl Drop for WindowsInjector {
    fn drop(&mut self) {
        // SAFETY: devices were created by us
        unsafe {
            DestroySyntheticPointerDevice(self.touch_dev);
            DestroySyntheticPointerDevice(self.pen_dev);
        }
    }
}
