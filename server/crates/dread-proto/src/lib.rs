//! DREAD wire protocol (see `docs/PROTOCOL.md`).
//!
//! Every message is framed as `u32 length | u8 kind | payload` (big-endian).

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTO_VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 47800;
pub const DISCOVERY_PORT: u16 = 47801;
pub const MDNS_SERVICE: &str = "_dread._tcp.local.";
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

pub const KIND_JSON: u8 = 0x01;
pub const KIND_INPUT: u8 = 0x02;
pub const KIND_VIDEO: u8 = 0x03;

/// Normalized coordinate range used on the wire.
pub const COORD_MAX: u16 = 65535;

#[derive(Debug, Clone)]
pub enum Message {
    Json(Value),
    Input(InputEvent),
    Video(VideoFrame),
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<(u8, Vec<u8>)> {
    let len = r.read_u32().await? as usize;
    if len == 0 || len > MAX_FRAME {
        bail!("invalid frame length {len}");
    }
    let kind = r.read_u8().await?;
    let mut buf = vec![0u8; len - 1];
    r.read_exact(&mut buf).await?;
    Ok((kind, buf))
}

pub async fn read_message<R: AsyncRead + Unpin>(r: &mut R) -> Result<Message> {
    let (kind, buf) = read_frame(r).await?;
    decode_message(kind, &buf)
}

pub fn decode_message(kind: u8, buf: &[u8]) -> Result<Message> {
    Ok(match kind {
        KIND_JSON => Message::Json(serde_json::from_slice(buf).context("bad json message")?),
        KIND_INPUT => Message::Input(InputEvent::decode(buf)?),
        KIND_VIDEO => Message::Video(VideoFrame::decode(buf)?),
        k => bail!("unknown frame kind {k:#x}"),
    })
}

pub fn encode_frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.extend_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
    out.push(kind);
    out.extend_from_slice(payload);
    out
}

pub fn encode_json(v: &Value) -> Vec<u8> {
    encode_frame(KIND_JSON, &serde_json::to_vec(v).expect("json"))
}

pub async fn write_json<W: AsyncWrite + Unpin>(w: &mut W, v: &Value) -> Result<()> {
    w.write_all(&encode_json(v)).await?;
    w.flush().await?;
    Ok(())
}

/// Reads one JSON message, failing on any other kind.
pub async fn read_json<R: AsyncRead + Unpin>(r: &mut R) -> Result<Value> {
    match read_message(r).await? {
        Message::Json(v) => Ok(v),
        _ => bail!("expected json message"),
    }
}

pub fn msg_type(v: &Value) -> &str {
    v.get("t").and_then(Value::as_str).unwrap_or("")
}

// ---------------------------------------------------------------- video

pub const VIDEO_FLAG_KEY: u8 = 1;
pub const VIDEO_FLAG_CONFIG: u8 = 2;

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub flags: u8,
    pub seq: u32,
    pub capture_us: u64,
    pub encode_us: u32,
    pub data: Vec<u8>,
}

impl VideoFrame {
    pub const HEADER: usize = 1 + 4 + 8 + 4;

    pub fn encode_header(&self) -> [u8; Self::HEADER] {
        let mut h = [0u8; Self::HEADER];
        h[0] = self.flags;
        h[1..5].copy_from_slice(&self.seq.to_be_bytes());
        h[5..13].copy_from_slice(&self.capture_us.to_be_bytes());
        h[13..17].copy_from_slice(&self.encode_us.to_be_bytes());
        h
    }

    /// Full wire frame (length + kind + header + data).
    pub fn to_wire(&self) -> Vec<u8> {
        let len = 1 + Self::HEADER + self.data.len();
        let mut out = Vec::with_capacity(4 + len);
        out.extend_from_slice(&(len as u32).to_be_bytes());
        out.push(KIND_VIDEO);
        out.extend_from_slice(&self.encode_header());
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < Self::HEADER {
            bail!("short video frame");
        }
        Ok(Self {
            flags: b[0],
            seq: u32::from_be_bytes(b[1..5].try_into()?),
            capture_us: u64::from_be_bytes(b[5..13].try_into()?),
            encode_us: u32::from_be_bytes(b[13..17].try_into()?),
            data: b[Self::HEADER..].to_vec(),
        })
    }
}

// ---------------------------------------------------------------- input

pub const IN_TOUCH: u8 = 0x01;
pub const IN_PEN: u8 = 0x02;
pub const IN_MOUSE_ABS: u8 = 0x03;
pub const IN_MOUSE_REL: u8 = 0x04;
pub const IN_SCROLL: u8 = 0x05;
pub const IN_KEY: u8 = 0x06;
pub const IN_TEXT: u8 = 0x07;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchAction {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    pub id: u8,
    pub x: u16,
    pub y: u16,
    pub pressure: u16,
    pub major: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PenTool {
    Pen,
    Eraser,
}

pub const PEN_IN_RANGE: u8 = 1;
pub const PEN_CONTACT: u8 = 2;
pub const PEN_BUTTON1: u8 = 4;
pub const PEN_BUTTON2: u8 = 8;

pub const MOUSE_LEFT: u8 = 1;
pub const MOUSE_RIGHT: u8 = 2;
pub const MOUSE_MIDDLE: u8 = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Touch { action: TouchAction, changed: u8, contacts: Vec<Contact> },
    Pen { tool: PenTool, flags: u8, x: u16, y: u16, pressure: u16, tilt_x: i16, tilt_y: i16 },
    MouseAbs { x: u16, y: u16, buttons: u8 },
    MouseRel { dx: i16, dy: i16, buttons: u8 },
    Scroll { vertical: i16, horizontal: i16 },
    Key { code: u16, down: bool },
    Text(String),
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.b.get(self.p).context("truncated input event")?;
        self.p += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
}

impl InputEvent {
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Rd { b, p: 0 };
        let ty = r.u8()?;
        Ok(match ty {
            IN_TOUCH => {
                let action = match r.u8()? {
                    0 => TouchAction::Down,
                    1 => TouchAction::Move,
                    2 => TouchAction::Up,
                    3 => TouchAction::Cancel,
                    a => bail!("bad touch action {a}"),
                };
                let changed = r.u8()?;
                let n = r.u8()? as usize;
                if n > 32 {
                    bail!("too many contacts");
                }
                let mut contacts = Vec::with_capacity(n);
                for _ in 0..n {
                    contacts.push(Contact { id: r.u8()?, x: r.u16()?, y: r.u16()?, pressure: r.u16()?, major: r.u16()? });
                }
                InputEvent::Touch { action, changed, contacts }
            }
            IN_PEN => InputEvent::Pen {
                tool: if r.u8()? == 1 { PenTool::Eraser } else { PenTool::Pen },
                flags: r.u8()?,
                x: r.u16()?,
                y: r.u16()?,
                pressure: r.u16()?,
                tilt_x: r.i16()?,
                tilt_y: r.i16()?,
            },
            IN_MOUSE_ABS => InputEvent::MouseAbs { x: r.u16()?, y: r.u16()?, buttons: r.u8()? },
            IN_MOUSE_REL => InputEvent::MouseRel { dx: r.i16()?, dy: r.i16()?, buttons: r.u8()? },
            IN_SCROLL => InputEvent::Scroll { vertical: r.i16()?, horizontal: r.i16()? },
            IN_KEY => InputEvent::Key { code: r.u16()?, down: r.u8()? != 0 },
            IN_TEXT => InputEvent::Text(String::from_utf8(b[1..].to_vec()).context("text not utf-8")?),
            t => bail!("unknown input type {t:#x}"),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(32);
        match self {
            InputEvent::Touch { action, changed, contacts } => {
                o.push(IN_TOUCH);
                o.push(match action {
                    TouchAction::Down => 0,
                    TouchAction::Move => 1,
                    TouchAction::Up => 2,
                    TouchAction::Cancel => 3,
                });
                o.push(*changed);
                o.push(contacts.len() as u8);
                for c in contacts {
                    o.push(c.id);
                    for v in [c.x, c.y, c.pressure, c.major] {
                        o.extend_from_slice(&v.to_be_bytes());
                    }
                }
            }
            InputEvent::Pen { tool, flags, x, y, pressure, tilt_x, tilt_y } => {
                o.push(IN_PEN);
                o.push(if *tool == PenTool::Eraser { 1 } else { 0 });
                o.push(*flags);
                for v in [*x, *y, *pressure, *tilt_x as u16, *tilt_y as u16] {
                    o.extend_from_slice(&v.to_be_bytes());
                }
            }
            InputEvent::MouseAbs { x, y, buttons } => {
                o.push(IN_MOUSE_ABS);
                o.extend_from_slice(&x.to_be_bytes());
                o.extend_from_slice(&y.to_be_bytes());
                o.push(*buttons);
            }
            InputEvent::MouseRel { dx, dy, buttons } => {
                o.push(IN_MOUSE_REL);
                o.extend_from_slice(&dx.to_be_bytes());
                o.extend_from_slice(&dy.to_be_bytes());
                o.push(*buttons);
            }
            InputEvent::Scroll { vertical, horizontal } => {
                o.push(IN_SCROLL);
                o.extend_from_slice(&vertical.to_be_bytes());
                o.extend_from_slice(&horizontal.to_be_bytes());
            }
            InputEvent::Key { code, down } => {
                o.push(IN_KEY);
                o.extend_from_slice(&code.to_be_bytes());
                o.push(*down as u8);
            }
            InputEvent::Text(s) => {
                o.push(IN_TEXT);
                o.extend_from_slice(s.as_bytes());
            }
        }
        o
    }

    pub fn to_wire(&self) -> Vec<u8> {
        encode_frame(KIND_INPUT, &self.encode())
    }
}

// ---------------------------------------------------------------- pairing

/// Short authentication string shown on both screens during code-comparison pairing.
pub fn pairing_code(cert_fp: &[u8], nc: &[u8], ns: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"DREAD-SAS-v1");
    h.update(cert_fp);
    h.update(nc);
    h.update(ns);
    let d = h.finalize();
    let v = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000;
    format!("{v:06}")
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_roundtrip() {
        let evs = vec![
            InputEvent::Touch {
                action: TouchAction::Move,
                changed: 1,
                contacts: vec![
                    Contact { id: 0, x: 1, y: 2, pressure: 3, major: 4 },
                    Contact { id: 1, x: 65535, y: 0, pressure: 9, major: 0 },
                ],
            },
            InputEvent::Pen { tool: PenTool::Eraser, flags: 3, x: 10, y: 20, pressure: 30000, tilt_x: -4500, tilt_y: 1200 },
            InputEvent::MouseAbs { x: 5, y: 6, buttons: 1 },
            InputEvent::MouseRel { dx: -5, dy: 6, buttons: 2 },
            InputEvent::Scroll { vertical: -120, horizontal: 60 },
            InputEvent::Key { code: 28, down: true },
            InputEvent::Text("ciao è €".into()),
        ];
        for e in evs {
            assert_eq!(InputEvent::decode(&e.encode()).unwrap(), e);
        }
    }

    #[test]
    fn video_roundtrip() {
        let f = VideoFrame { flags: 3, seq: 7, capture_us: 99, encode_us: 5, data: vec![0, 0, 0, 1, 9] };
        let w = f.to_wire();
        assert_eq!(w[4], KIND_VIDEO);
        let d = VideoFrame::decode(&w[5..]).unwrap();
        assert_eq!(d.seq, 7);
        assert_eq!(d.data, f.data);
    }

    #[test]
    fn sas_is_six_digits_and_binds_fingerprint() {
        let a = pairing_code(&[1; 32], &[2; 32], &[3; 32]);
        let b = pairing_code(&[9; 32], &[2; 32], &[3; 32]);
        assert_eq!(a.len(), 6);
        assert_ne!(a, b);
    }
}
