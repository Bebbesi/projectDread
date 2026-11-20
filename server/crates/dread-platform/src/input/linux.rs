//! Translates DREAD input events into uinput device events.

use super::uinput::*;
use super::{AbsMap, InputSettings};
use anyhow::Result;
use dread_proto::{Contact, InputEvent, PenTool, TouchAction, COORD_MAX, MOUSE_LEFT, MOUSE_MIDDLE, MOUSE_RIGHT, PEN_BUTTON1, PEN_BUTTON2, PEN_CONTACT, PEN_IN_RANGE};
use tracing::warn;

pub const TOUCH_NAME: &str = "DREAD Touchscreen";
pub const PEN_NAME: &str = "DREAD Pen";
pub const POINTER_NAME: &str = "DREAD Pointer";
pub const MOUSE_NAME: &str = "DREAD Mouse";
pub const KEYBOARD_NAME: &str = "DREAD Keyboard";

const SLOTS: usize = 10;
const PRESSURE_MAX: i32 = 4095;
/// 0..65535 over roughly 0.5 m: libinput only needs a plausible physical size.
const COORD_RES: i32 = 128;

pub struct LinuxInjector {
    touch: Option<UinputDevice>,
    pen: Option<UinputDevice>,
    pointer: Option<UinputDevice>,
    mouse: Option<UinputDevice>,
    keyboard: Option<UinputDevice>,
    slots: [Option<u8>; SLOTS],
    next_tracking: i32,
    pen_state: Option<(PenTool, u8)>,
    abs_buttons: u8,
    last_abs: (u16, u16),
    rel_buttons: u8,
    pressed_keys: Vec<u16>,
    scroll_rem: (i32, i32),
}

fn btn_list(mask: u8) -> [(u8, u16); 3] {
    let _ = mask;
    [(MOUSE_LEFT, BTN_LEFT), (MOUSE_RIGHT, BTN_RIGHT), (MOUSE_MIDDLE, BTN_MIDDLE)]
}

impl LinuxInjector {
    pub fn new() -> Result<Self> {
        let xy = |code, res| AbsAxis { code, min: 0, max: COORD_MAX as i32, resolution: res };
        let touch = UinputDevice::create(&DeviceSpec {
            name: TOUCH_NAME,
            product: 1,
            keys: vec![BTN_TOUCH, BTN_TOOL_FINGER, BTN_TOOL_DOUBLETAP, BTN_TOOL_TRIPLETAP, BTN_TOOL_QUADTAP],
            rels: vec![],
            abs: vec![
                xy(ABS_X, COORD_RES),
                xy(ABS_Y, COORD_RES),
                AbsAxis { code: ABS_MT_SLOT, min: 0, max: SLOTS as i32 - 1, resolution: 0 },
                AbsAxis { code: ABS_MT_TRACKING_ID, min: 0, max: 65535, resolution: 0 },
                xy(ABS_MT_POSITION_X, COORD_RES),
                xy(ABS_MT_POSITION_Y, COORD_RES),
                AbsAxis { code: ABS_MT_PRESSURE, min: 0, max: PRESSURE_MAX, resolution: 0 },
                AbsAxis { code: ABS_MT_TOUCH_MAJOR, min: 0, max: COORD_MAX as i32, resolution: COORD_RES },
            ],
            props: vec![INPUT_PROP_DIRECT],
            msc: false,
        })?;
        let pen = UinputDevice::create(&DeviceSpec {
            name: PEN_NAME,
            product: 2,
            keys: vec![BTN_TOOL_PEN, BTN_TOOL_RUBBER, BTN_TOUCH, BTN_STYLUS, BTN_STYLUS2],
            rels: vec![],
            abs: vec![
                xy(ABS_X, COORD_RES),
                xy(ABS_Y, COORD_RES),
                AbsAxis { code: ABS_PRESSURE, min: 0, max: PRESSURE_MAX, resolution: 0 },
                // hundredths of a degree; resolution is units per radian
                AbsAxis { code: ABS_TILT_X, min: -9000, max: 9000, resolution: 5730 },
                AbsAxis { code: ABS_TILT_Y, min: -9000, max: 9000, resolution: 5730 },
            ],
            props: vec![INPUT_PROP_DIRECT],
            msc: false,
        })?;
        let pointer = UinputDevice::create(&DeviceSpec {
            name: POINTER_NAME,
            product: 3,
            keys: vec![BTN_LEFT, BTN_RIGHT, BTN_MIDDLE],
            rels: vec![],
            abs: vec![xy(ABS_X, 0), xy(ABS_Y, 0)],
            props: vec![],
            msc: false,
        })?;
        let mouse = UinputDevice::create(&DeviceSpec {
            name: MOUSE_NAME,
            product: 4,
            keys: vec![BTN_LEFT, BTN_RIGHT, BTN_MIDDLE],
            rels: vec![REL_X, REL_Y, REL_WHEEL, REL_HWHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES],
            abs: vec![],
            props: vec![],
            msc: false,
        })?;
        let keyboard = UinputDevice::create(&DeviceSpec {
            name: KEYBOARD_NAME,
            product: 5,
            keys: (1..=248).collect(),
            rels: vec![],
            abs: vec![],
            props: vec![],
            msc: false,
        })?;
        Ok(Self {
            touch: Some(touch),
            pen: Some(pen),
            pointer: Some(pointer),
            mouse: Some(mouse),
            keyboard: Some(keyboard),
            slots: [None; SLOTS],
            next_tracking: 1,
            pen_state: None,
            abs_buttons: 0,
            last_abs: (0, 0),
            rel_buttons: 0,
            pressed_keys: Vec::new(),
            scroll_rem: (0, 0),
        })
    }

    /// Device names as the compositor sees them (used for output binding).
    pub fn device_names() -> [&'static str; 2] {
        [TOUCH_NAME, PEN_NAME]
    }

    pub fn handle(&mut self, ev: &InputEvent, s: &InputSettings, abs: &AbsMap) -> Result<()> {
        match ev {
            InputEvent::Touch { action, changed, contacts } if s.touch_enabled => self.touch(*action, *changed, contacts),
            InputEvent::Pen { tool, flags, x, y, pressure, tilt_x, tilt_y } if s.pen_enabled => {
                self.pen(*tool, *flags, *x, *y, *pressure, *tilt_x, *tilt_y, s)
            }
            InputEvent::MouseAbs { x, y, buttons } if s.mouse_enabled => self.mouse_abs(*x, *y, *buttons, abs),
            InputEvent::MouseRel { dx, dy, buttons } if s.mouse_enabled => self.mouse_rel(*dx, *dy, *buttons, s),
            InputEvent::Scroll { vertical, horizontal } if s.mouse_enabled => self.scroll(*vertical, *horizontal, s),
            InputEvent::Key { code, down } if s.keyboard_enabled => self.key(*code, *down),
            _ => Ok(()),
        }
    }

    fn touch(&mut self, action: TouchAction, changed: u8, contacts: &[Contact]) -> Result<()> {
        let Some(dev) = self.touch.as_mut() else { return Ok(()) };
        let lift_changed = matches!(action, TouchAction::Up | TouchAction::Cancel);
        let lift_all = action == TouchAction::Cancel;
        // release slots whose contact disappeared or was lifted
        for (slot, cur) in self.slots.iter_mut().enumerate() {
            if let Some(id) = *cur {
                let present = contacts.iter().any(|c| c.id == id);
                if !present || lift_all || (lift_changed && id == changed) {
                    dev.emit(EV_ABS, ABS_MT_SLOT, slot as i32);
                    dev.emit(EV_ABS, ABS_MT_TRACKING_ID, -1);
                    *cur = None;
                }
            }
        }
        if !lift_all {
            for c in contacts {
                if lift_changed && c.id == changed {
                    continue;
                }
                let slot = match self.slots.iter().position(|s| *s == Some(c.id)) {
                    Some(s) => s,
                    None => match self.slots.iter().position(|s| s.is_none()) {
                        Some(s) => {
                            self.slots[s] = Some(c.id);
                            dev.emit(EV_ABS, ABS_MT_SLOT, s as i32);
                            dev.emit(EV_ABS, ABS_MT_TRACKING_ID, self.next_tracking);
                            self.next_tracking = (self.next_tracking + 1) % 65535;
                            s
                        }
                        None => {
                            warn!("more than {SLOTS} touch contacts, ignoring extra");
                            continue;
                        }
                    },
                };
                dev.emit(EV_ABS, ABS_MT_SLOT, slot as i32);
                dev.emit(EV_ABS, ABS_MT_POSITION_X, c.x as i32);
                dev.emit(EV_ABS, ABS_MT_POSITION_Y, c.y as i32);
                dev.emit(EV_ABS, ABS_MT_PRESSURE, (c.pressure as i32 * PRESSURE_MAX) / 65535);
                dev.emit(EV_ABS, ABS_MT_TOUCH_MAJOR, c.major as i32);
            }
        }
        let active: Vec<&Contact> = contacts
            .iter()
            .filter(|c| self.slots.contains(&Some(c.id)))
            .collect();
        let n = active.len();
        dev.emit(EV_KEY, BTN_TOUCH, (n > 0) as i32);
        dev.emit(EV_KEY, BTN_TOOL_FINGER, (n == 1) as i32);
        dev.emit(EV_KEY, BTN_TOOL_DOUBLETAP, (n == 2) as i32);
        dev.emit(EV_KEY, BTN_TOOL_TRIPLETAP, (n == 3) as i32);
        dev.emit(EV_KEY, BTN_TOOL_QUADTAP, (n >= 4) as i32);
        if let Some(first) = active.first() {
            dev.emit(EV_ABS, ABS_X, first.x as i32);
            dev.emit(EV_ABS, ABS_Y, first.y as i32);
        }
        dev.sync()
    }

    #[allow(clippy::too_many_arguments)]
    fn pen(&mut self, tool: PenTool, flags: u8, x: u16, y: u16, pressure: u16, tx: i16, ty: i16, s: &InputSettings) -> Result<()> {
        let Some(dev) = self.pen.as_mut() else { return Ok(()) };
        let in_range = flags & (PEN_IN_RANGE | PEN_CONTACT) != 0;
        let tool_btn = |t: PenTool| if t == PenTool::Eraser { BTN_TOOL_RUBBER } else { BTN_TOOL_PEN };
        // tool switch or leaving proximity: close the previous tool first
        if let Some((prev, _)) = self.pen_state {
            if !in_range || prev != tool {
                dev.emit(EV_KEY, BTN_TOUCH, 0);
                dev.emit(EV_ABS, ABS_PRESSURE, 0);
                dev.emit(EV_KEY, BTN_STYLUS, 0);
                dev.emit(EV_KEY, BTN_STYLUS2, 0);
                dev.emit(EV_KEY, tool_btn(prev), 0);
                dev.sync()?;
                self.pen_state = None;
            }
        }
        if !in_range {
            return Ok(());
        }
        let contact = flags & PEN_CONTACT != 0;
        let p = if contact {
            let n = pressure as f32 / 65535.0;
            let curved = n.max(0.0).powf(s.pen_pressure_gamma.clamp(0.2, 5.0));
            ((curved * PRESSURE_MAX as f32) as i32).max(1)
        } else {
            0
        };
        dev.emit(EV_ABS, ABS_X, x as i32);
        dev.emit(EV_ABS, ABS_Y, y as i32);
        dev.emit(EV_ABS, ABS_PRESSURE, p);
        let (tx, ty) = if s.pen_tilt { (tx as i32, ty as i32) } else { (0, 0) };
        dev.emit(EV_ABS, ABS_TILT_X, tx.clamp(-9000, 9000));
        dev.emit(EV_ABS, ABS_TILT_Y, ty.clamp(-9000, 9000));
        if self.pen_state.is_none() {
            dev.emit(EV_KEY, tool_btn(tool), 1);
        }
        dev.emit(EV_KEY, BTN_TOUCH, contact as i32);
        dev.emit(EV_KEY, BTN_STYLUS, (flags & PEN_BUTTON1 != 0) as i32);
        dev.emit(EV_KEY, BTN_STYLUS2, (flags & PEN_BUTTON2 != 0) as i32);
        self.pen_state = Some((tool, flags));
        dev.sync()
    }

    fn mouse_abs(&mut self, x: u16, y: u16, buttons: u8, abs: &AbsMap) -> Result<()> {
        let Some(dev) = self.pointer.as_mut() else { return Ok(()) };
        self.last_abs = (x, y);
        let (lx, ly) = abs.map(x, y);
        dev.emit(EV_ABS, ABS_X, lx as i32);
        dev.emit(EV_ABS, ABS_Y, ly as i32);
        for (mask, btn) in btn_list(buttons) {
            if (buttons ^ self.abs_buttons) & mask != 0 {
                dev.emit(EV_KEY, btn, (buttons & mask != 0) as i32);
            }
        }
        self.abs_buttons = buttons;
        dev.sync()
    }

    fn mouse_rel(&mut self, dx: i16, dy: i16, buttons: u8, s: &InputSettings) -> Result<()> {
        let Some(dev) = self.mouse.as_mut() else { return Ok(()) };
        let k = s.mouse_speed.clamp(0.1, 10.0);
        if dx != 0 {
            dev.emit(EV_REL, REL_X, (dx as f32 * k).round() as i32);
        }
        if dy != 0 {
            dev.emit(EV_REL, REL_Y, (dy as f32 * k).round() as i32);
        }
        for (mask, btn) in btn_list(buttons) {
            if (buttons ^ self.rel_buttons) & mask != 0 {
                dev.emit(EV_KEY, btn, (buttons & mask != 0) as i32);
            }
        }
        self.rel_buttons = buttons;
        dev.sync()
    }

    fn scroll(&mut self, v: i16, h: i16, s: &InputSettings) -> Result<()> {
        let Some(dev) = self.mouse.as_mut() else { return Ok(()) };
        let k = s.scroll_speed.clamp(0.1, 10.0) * if s.invert_scroll { -1.0 } else { 1.0 };
        // protocol: positive = down/right; evdev wheel: positive = up
        let vv = -((v as f32) * k).round() as i32;
        let hh = ((h as f32) * k).round() as i32;
        if vv != 0 {
            dev.emit(EV_REL, REL_WHEEL_HI_RES, vv);
            self.scroll_rem.0 += vv;
            let notches = self.scroll_rem.0 / 120;
            if notches != 0 {
                dev.emit(EV_REL, REL_WHEEL, notches);
                self.scroll_rem.0 -= notches * 120;
            }
        }
        if hh != 0 {
            dev.emit(EV_REL, REL_HWHEEL_HI_RES, hh);
            self.scroll_rem.1 += hh;
            let notches = self.scroll_rem.1 / 120;
            if notches != 0 {
                dev.emit(EV_REL, REL_HWHEEL, notches);
                self.scroll_rem.1 -= notches * 120;
            }
        }
        dev.sync()
    }

    fn key(&mut self, code: u16, down: bool) -> Result<()> {
        let Some(dev) = self.keyboard.as_mut() else { return Ok(()) };
        if code == 0 || code > 248 {
            return Ok(());
        }
        dev.emit(EV_KEY, code, down as i32);
        if down {
            if !self.pressed_keys.contains(&code) {
                self.pressed_keys.push(code);
            }
        } else {
            self.pressed_keys.retain(|k| *k != code);
        }
        dev.sync()
    }

    /// Lift every finger, pen and key — called when a client disconnects so
    /// nothing stays stuck "down" on the desktop.
    pub fn release_all(&mut self) {
        let empty: [Contact; 0] = [];
        let _ = self.touch(TouchAction::Cancel, 0, &empty);
        if let Some((tool, _)) = self.pen_state {
            let _ = self.pen(tool, 0, 0, 0, 0, 0, 0, &InputSettings::default());
        }
        if self.abs_buttons != 0 {
            if let Some(dev) = self.pointer.as_mut() {
                for (_, btn) in btn_list(0) {
                    dev.emit(EV_KEY, btn, 0);
                }
                let _ = dev.sync();
            }
            self.abs_buttons = 0;
        }
        if self.rel_buttons != 0 {
            let _ = self.mouse_rel(0, 0, 0, &InputSettings::default());
        }
        for k in std::mem::take(&mut self.pressed_keys) {
            let _ = self.key(k, false);
        }
    }
}
