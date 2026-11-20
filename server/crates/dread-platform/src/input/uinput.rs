//! Linux input injection through `/dev/uinput`.
//!
//! DREAD creates separate kernel input devices so that libinput and the
//! compositor see genuine hardware classes:
//!
//! * `DREAD Touchscreen` — multitouch protocol B (slots), direct device
//! * `DREAD Pen`         — screen tablet: pen + eraser, pressure, tilt, hover, two barrel buttons
//! * `DREAD Pointer`     — absolute pointer (mouse mode)
//! * `DREAD Mouse`       — relative pointer + high-resolution wheels
//! * `DREAD Keyboard`    — full key set
//!
//! Touch and pen devices are bound to the DREAD output by the display backend
//! (e.g. Hyprland `device { output = ... }`), so coordinates are normalized
//! to the virtual display, not to the whole desktop.

use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_ABS: u16 = 0x03;
pub const EV_MSC: u16 = 0x04;
pub const SYN_REPORT: u16 = 0;
pub const MSC_TIMESTAMP: u16 = 0x05;

pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;
pub const ABS_PRESSURE: u16 = 0x18;
pub const ABS_DISTANCE: u16 = 0x19;
pub const ABS_TILT_X: u16 = 0x1a;
pub const ABS_TILT_Y: u16 = 0x1b;
pub const ABS_MT_SLOT: u16 = 0x2f;
pub const ABS_MT_TOUCH_MAJOR: u16 = 0x30;
pub const ABS_MT_POSITION_X: u16 = 0x35;
pub const ABS_MT_POSITION_Y: u16 = 0x36;
pub const ABS_MT_TRACKING_ID: u16 = 0x39;
pub const ABS_MT_PRESSURE: u16 = 0x3a;

pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_HWHEEL: u16 = 0x06;
pub const REL_WHEEL: u16 = 0x08;
pub const REL_WHEEL_HI_RES: u16 = 0x0b;
pub const REL_HWHEEL_HI_RES: u16 = 0x0c;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
pub const BTN_TOOL_PEN: u16 = 0x140;
pub const BTN_TOOL_RUBBER: u16 = 0x141;
pub const BTN_TOOL_FINGER: u16 = 0x145;
pub const BTN_TOUCH: u16 = 0x14a;
pub const BTN_STYLUS: u16 = 0x14b;
pub const BTN_STYLUS2: u16 = 0x14c;
pub const BTN_TOOL_DOUBLETAP: u16 = 0x14d;
pub const BTN_TOOL_TRIPLETAP: u16 = 0x14e;
pub const BTN_TOOL_QUADTAP: u16 = 0x14f;

pub const INPUT_PROP_POINTER: u16 = 0x00;
pub const INPUT_PROP_DIRECT: u16 = 0x01;

const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_SET_RELBIT: libc::c_ulong = 0x4004_5566;
const UI_SET_ABSBIT: libc::c_ulong = 0x4004_5567;
const UI_SET_PROPBIT: libc::c_ulong = 0x4004_556e;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_DEV_SETUP: libc::c_ulong = 0x405c_5503;
const UI_ABS_SETUP: libc::c_ulong = 0x401c_5504;
const BUS_VIRTUAL: u16 = 0x06;
const DREAD_VENDOR: u16 = 0x1d5e; // arbitrary, identifies DREAD devices in logs

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
struct UinputAbsSetup {
    code: u16,
    _pad: u16,
    absinfo: AbsInfo,
}

#[repr(C)]
struct RawEvent {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
    ty: u16,
    code: u16,
    value: i32,
}

pub struct AbsAxis {
    pub code: u16,
    pub min: i32,
    pub max: i32,
    pub resolution: i32,
}

pub struct DeviceSpec<'a> {
    pub name: &'a str,
    pub product: u16,
    pub keys: Vec<u16>,
    pub rels: Vec<u16>,
    pub abs: Vec<AbsAxis>,
    pub props: Vec<u16>,
    pub msc: bool,
}

pub struct UinputDevice {
    file: File,
    buf: Vec<u8>,
    pub name: String,
}

fn ioctl_int(fd: i32, req: libc::c_ulong, v: u16) -> Result<()> {
    // SAFETY: plain integer-argument uinput ioctl on a valid fd.
    let r = unsafe { libc::ioctl(fd, req as _, v as libc::c_int) };
    if r < 0 {
        bail!("uinput ioctl {req:#x}({v:#x}) failed: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

impl UinputDevice {
    pub fn create(spec: &DeviceSpec) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open("/dev/uinput")
            .context(
                "cannot open /dev/uinput — add a udev rule granting your user access \
                 (see packaging/linux/70-dread-uinput.rules) or load the uinput module",
            )?;
        let fd = file.as_raw_fd();
        if !spec.keys.is_empty() {
            ioctl_int(fd, UI_SET_EVBIT, EV_KEY)?;
            for k in &spec.keys {
                ioctl_int(fd, UI_SET_KEYBIT, *k)?;
            }
        }
        if !spec.rels.is_empty() {
            ioctl_int(fd, UI_SET_EVBIT, EV_REL)?;
            for r in &spec.rels {
                ioctl_int(fd, UI_SET_RELBIT, *r)?;
            }
        }
        if !spec.abs.is_empty() {
            ioctl_int(fd, UI_SET_EVBIT, EV_ABS)?;
            for a in &spec.abs {
                ioctl_int(fd, UI_SET_ABSBIT, a.code)?;
                let setup = UinputAbsSetup {
                    code: a.code,
                    _pad: 0,
                    absinfo: AbsInfo { value: 0, minimum: a.min, maximum: a.max, fuzz: 0, flat: 0, resolution: a.resolution },
                };
                // SAFETY: struct layout matches `struct uinput_abs_setup`.
                if unsafe { libc::ioctl(fd, UI_ABS_SETUP as _, &setup) } < 0 {
                    bail!("UI_ABS_SETUP failed: {}", std::io::Error::last_os_error());
                }
            }
        }
        if spec.msc {
            ioctl_int(fd, UI_SET_EVBIT, EV_MSC)?;
            // MSC_TIMESTAMP bit
            // SAFETY: UI_SET_MSCBIT = _IOW('U', 104, int)
            unsafe { libc::ioctl(fd, 0x4004_5568u64 as _, MSC_TIMESTAMP as libc::c_int) };
        }
        for p in &spec.props {
            ioctl_int(fd, UI_SET_PROPBIT, *p)?;
        }
        let mut name = [0u8; 80];
        let n = spec.name.as_bytes();
        name[..n.len().min(79)].copy_from_slice(&n[..n.len().min(79)]);
        let setup = UinputSetup {
            id: InputId { bustype: BUS_VIRTUAL, vendor: DREAD_VENDOR, product: spec.product, version: 1 },
            name,
            ff_effects_max: 0,
        };
        // SAFETY: struct layout matches `struct uinput_setup`.
        if unsafe { libc::ioctl(fd, UI_DEV_SETUP as _, &setup) } < 0 {
            bail!("UI_DEV_SETUP failed: {}", std::io::Error::last_os_error());
        }
        // SAFETY: argument-less ioctl.
        if unsafe { libc::ioctl(fd, UI_DEV_CREATE as _) } < 0 {
            bail!("UI_DEV_CREATE failed: {}", std::io::Error::last_os_error());
        }
        Ok(Self { file, buf: Vec::with_capacity(512), name: spec.name.to_string() })
    }

    /// Queue an event; nothing reaches the kernel until [`sync`](Self::sync).
    pub fn emit(&mut self, ty: u16, code: u16, value: i32) {
        let ev = RawEvent { tv_sec: 0, tv_usec: 0, ty, code, value };
        // SAFETY: RawEvent is a plain repr(C) struct matching `struct input_event`.
        let bytes = unsafe {
            std::slice::from_raw_parts(&ev as *const RawEvent as *const u8, std::mem::size_of::<RawEvent>())
        };
        self.buf.extend_from_slice(bytes);
    }

    pub fn sync(&mut self) -> Result<()> {
        self.emit(EV_SYN, SYN_REPORT, 0);
        let r = self.file.write_all(&self.buf);
        self.buf.clear();
        r.with_context(|| format!("writing events to {}", self.name))
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        // SAFETY: argument-less ioctl on our own fd.
        unsafe { libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY as _) };
    }
}
