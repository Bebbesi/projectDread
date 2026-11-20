//! Video encoding through an FFmpeg child process.
//!
//! FFmpeg gives DREAD one interface to VAAPI (Intel/AMD on Linux), NVENC
//! (NVIDIA), AMF (AMD on Windows), Quick Sync and the x264/x265/SVT-AV1
//! software encoders, on both operating systems. Every backend is configured
//! for real-time use: no B-frames, no lookahead, a VBV buffer of a few frames.
//!
//! Output is a raw elementary stream with access-unit delimiters (or AV1
//! temporal delimiters); `-avioflags direct` makes FFmpeg write each packet
//! with a single `write()`, so an access unit can be forwarded the moment it
//! is produced — no frame of latency is added waiting for the next delimiter.

use crate::capture::{now_us, Frame, PixelFormat};
use anyhow::{bail, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    #[default]
    H264,
    Hevc,
    Av1,
}

impl Codec {
    pub fn wire_name(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
            Codec::Av1 => "av1",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Codec::H264 => "H.264",
            Codec::Hevc => "H.265 / HEVC",
            Codec::Av1 => "AV1",
        }
    }
    pub const ALL: [Codec; 3] = [Codec::H264, Codec::Hevc, Codec::Av1];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    /// Best available hardware encoder, software as last resort.
    #[default]
    Auto,
    Vaapi,
    Nvenc,
    Qsv,
    Amf,
    Software,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Auto => "Automatic",
            Backend::Vaapi => "VA-API",
            Backend::Nvenc => "NVENC",
            Backend::Qsv => "Quick Sync",
            Backend::Amf => "AMF",
            Backend::Software => "Software",
        }
    }
    pub fn is_hardware(self) -> bool {
        !matches!(self, Backend::Software | Backend::Auto)
    }
    fn platform_order() -> &'static [Backend] {
        if cfg!(windows) {
            &[Backend::Nvenc, Backend::Amf, Backend::Qsv, Backend::Software]
        } else {
            &[Backend::Nvenc, Backend::Vaapi, Backend::Software]
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Fastest preset, lowest latency.
    Speed,
    #[default]
    Balanced,
    Quality,
}

/// Where frames come from.
#[derive(Debug, Clone, PartialEq)]
pub enum EncoderInput {
    /// Raw frames written to stdin by DREAD's own capture.
    Raw { format: PixelFormat },
    /// FFmpeg captures the given output itself (Windows Desktop Duplication).
    Ddagrab { output_idx: u32, draw_mouse: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncoderConfig {
    pub ffmpeg: String,
    pub codec: Codec,
    pub backend: Backend,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub keyframe_interval_s: f32,
    pub quality: Quality,
    pub low_latency: bool,
    pub input: EncoderInput,
    pub vaapi_device: String,
    /// Encode at this size instead of the capture size (adaptive quality,
    /// or a display larger than the encoder supports).
    pub scale_to: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub key: bool,
    pub config: bool,
    pub capture_us: u64,
    pub encode_us: u32,
}

fn encoder_name(codec: Codec, backend: Backend) -> Option<&'static str> {
    use Backend::*;
    use Codec::*;
    Some(match (codec, backend) {
        (H264, Vaapi) => "h264_vaapi",
        (Hevc, Vaapi) => "hevc_vaapi",
        (Av1, Vaapi) => "av1_vaapi",
        (H264, Nvenc) => "h264_nvenc",
        (Hevc, Nvenc) => "hevc_nvenc",
        (Av1, Nvenc) => "av1_nvenc",
        (H264, Qsv) => "h264_qsv",
        (Hevc, Qsv) => "hevc_qsv",
        (Av1, Qsv) => "av1_qsv",
        (H264, Amf) => "h264_amf",
        (Hevc, Amf) => "hevc_amf",
        (Av1, Amf) => "av1_amf",
        (H264, Software) => "libx264",
        (Hevc, Software) => "libx265",
        (Av1, Software) => "libsvtav1",
        (_, Auto) => return None,
    })
}

impl EncoderConfig {
    pub fn encoder_name(&self) -> Option<&'static str> {
        encoder_name(self.codec, self.backend)
    }

    /// Size of the encoded picture.
    pub fn output_size(&self) -> (u32, u32) {
        self.scale_to.map(|(w, h)| (w & !1, h & !1)).unwrap_or((self.width, self.height))
    }

    /// Full FFmpeg argument list.
    pub fn args(&self) -> Result<Vec<String>> {
        let enc = self.encoder_name().context("encoder backend must be resolved before starting")?;
        let fps = self.fps.max(1);
        let b = format!("{}k", self.bitrate_kbps.max(100));
        // VBV: ~2 frames in low-latency mode, ~1 s otherwise
        let buf_kbits = if self.low_latency { (self.bitrate_kbps * 2 / fps).max(64) } else { self.bitrate_kbps };
        let bufsize = format!("{buf_kbits}k");
        let gop = if self.keyframe_interval_s <= 0.0 { 100_000 } else { ((self.keyframe_interval_s * fps as f32) as u32).max(1) };
        let mut a: Vec<String> = ["-hide_banner", "-nostats", "-loglevel", "warning"].iter().map(|s| s.to_string()).collect();
        let push = |a: &mut Vec<String>, xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));

        // ---- device init
        if self.backend == Backend::Vaapi {
            push(&mut a, &["-init_hw_device", &format!("vaapi=va:{}", self.vaapi_device), "-filter_hw_device", "va"]);
        }
        if self.backend == Backend::Qsv && matches!(self.input, EncoderInput::Raw { .. }) {
            push(&mut a, &["-init_hw_device", "qsv=qs:hw", "-filter_hw_device", "qs"]);
        }

        // ---- input
        let mut filters: Vec<String> = vec![];
        match &self.input {
            EncoderInput::Raw { format } => {
                push(&mut a, &["-f", "rawvideo", "-pix_fmt", format.ffmpeg_name(), "-s", &format!("{}x{}", self.width, self.height)]);
                push(&mut a, &["-framerate", &fps.to_string(), "-i", "pipe:0"]);
                let sz = self.scale_to.map(|(w, h)| (w & !1, h & !1));
                match (self.backend, sz) {
                    (Backend::Vaapi, None) => filters.push("hwupload,scale_vaapi=format=nv12".into()),
                    (Backend::Vaapi, Some((w, h))) => filters.push(format!("hwupload,scale_vaapi=w={w}:h={h}:format=nv12")),
                    (Backend::Qsv, None) => filters.push("format=bgra,hwupload=extra_hw_frames=16,vpp_qsv=format=nv12".into()),
                    (Backend::Qsv, Some((w, h))) => filters.push(format!("format=bgra,hwupload=extra_hw_frames=16,vpp_qsv=w={w}:h={h}:format=nv12")),
                    // NVENC/AMF accept RGB input directly
                    (Backend::Nvenc | Backend::Amf, None) => {}
                    (Backend::Nvenc | Backend::Amf, Some((w, h))) => filters.push(format!("scale={w}:{h}")),
                    (Backend::Software, None) => filters.push("format=yuv420p".into()),
                    (Backend::Software, Some((w, h))) => filters.push(format!("scale={w}:{h}:flags=bilinear,format=yuv420p")),
                    (Backend::Auto, _) => unreachable!(),
                }
            }
            EncoderInput::Ddagrab { output_idx, draw_mouse } => {
                push(&mut a, &["-f", "lavfi", "-i", &format!("ddagrab=output_idx={output_idx}:framerate={fps}:draw_mouse={}", *draw_mouse as u8)]);
                match self.backend {
                    Backend::Nvenc | Backend::Amf => {}
                    Backend::Qsv => filters.push("hwmap=derive_device=qsv,format=qsv".into()),
                    Backend::Software => filters.push("hwdownload,format=bgra,format=yuv420p".into()),
                    Backend::Vaapi | Backend::Auto => bail!("{:?} cannot encode Desktop Duplication frames", self.backend),
                }
            }
        }
        if !filters.is_empty() {
            push(&mut a, &["-vf", &filters.join(",")]);
        }

        // ---- encoder
        push(&mut a, &["-c:v", enc, "-g", &gop.to_string(), "-bf", "0", "-b:v", &b, "-maxrate", &b, "-bufsize", &bufsize]);
        let q = self.quality;
        match self.backend {
            Backend::Vaapi => {
                let level = match q {
                    Quality::Speed => "7",
                    Quality::Balanced => "4",
                    Quality::Quality => "1",
                };
                push(&mut a, &["-rc_mode", "CBR", "-async_depth", "1", "-compression_level", level]);
            }
            Backend::Nvenc => {
                let preset = match q {
                    Quality::Speed => "p1",
                    Quality::Balanced => "p3",
                    Quality::Quality => "p5",
                };
                push(&mut a, &["-preset", preset, "-tune", "ull", "-rc", "cbr", "-zerolatency", "1", "-delay", "0", "-rc-lookahead", "0"]);
            }
            Backend::Qsv => {
                let preset = match q {
                    Quality::Speed => "veryfast",
                    Quality::Balanced => "faster",
                    Quality::Quality => "medium",
                };
                push(&mut a, &["-preset", preset, "-async_depth", "1", "-look_ahead", "0"]);
            }
            Backend::Amf => {
                let quality = match q {
                    Quality::Speed => "speed",
                    Quality::Balanced => "balanced",
                    Quality::Quality => "quality",
                };
                push(&mut a, &["-usage", "ultralowlatency", "-rc", "cbr", "-quality", quality]);
            }
            Backend::Software => match self.codec {
                Codec::H264 | Codec::Hevc => {
                    let preset = match q {
                        Quality::Speed => "ultrafast",
                        Quality::Balanced => "superfast",
                        Quality::Quality => "veryfast",
                    };
                    push(&mut a, &["-preset", preset, "-tune", "zerolatency"]);
                    if self.codec == Codec::Hevc {
                        push(&mut a, &["-x265-params", "repeat-headers=1:bframes=0:rc-lookahead=0"]);
                    }
                }
                Codec::Av1 => {
                    let preset = match q {
                        Quality::Speed => "12",
                        Quality::Balanced => "10",
                        Quality::Quality => "8",
                    };
                    push(&mut a, &["-preset", preset, "-svtav1-params", "pred-struct=1:lookahead=0"]);
                }
            },
            Backend::Auto => unreachable!(),
        }

        // ---- output: elementary stream, one write per packet
        push(&mut a, &["-fps_mode", "passthrough", "-flush_packets", "1", "-avioflags", "direct"]);
        match self.codec {
            Codec::H264 => push(&mut a, &["-bsf:v", "h264_metadata=aud=insert", "-f", "h264"]),
            Codec::Hevc => push(&mut a, &["-bsf:v", "hevc_metadata=aud=insert", "-f", "hevc"]),
            Codec::Av1 => push(&mut a, &["-f", "obu"]),
        }
        push(&mut a, &["pipe:1"]);
        Ok(a)
    }
}

// ------------------------------------------------------------ AU splitting

/// Finds Annex-B start codes; returns (start-code offset, NAL header offset).
fn start_codes(b: &[u8]) -> Vec<(usize, usize)> {
    let mut v = vec![];
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 {
            if b[i + 2] == 1 {
                let s = if i > 0 && b[i - 1] == 0 { i - 1 } else { i };
                v.push((s, i + 3));
                i += 3;
                continue;
            } else if b[i + 2] != 0 {
                i += 3;
                continue;
            }
        }
        i += 1;
    }
    v
}

fn nal_type(codec: Codec, hdr: u8) -> u8 {
    match codec {
        Codec::Hevc => (hdr >> 1) & 0x3f,
        _ => hdr & 0x1f,
    }
}

/// Splits an Annex-B buffer at access-unit delimiters.
pub fn split_annexb(codec: Codec, buf: &[u8]) -> Vec<(usize, usize)> {
    let aud = if codec == Codec::Hevc { 35 } else { 9 };
    let mut cuts: Vec<usize> = start_codes(buf)
        .into_iter()
        .filter(|(_, h)| *h < buf.len() && nal_type(codec, buf[*h]) == aud)
        .map(|(s, _)| s)
        .collect();
    if cuts.first() != Some(&0) {
        cuts.insert(0, 0);
    }
    cuts.push(buf.len());
    cuts.windows(2).filter(|w| w[1] > w[0]).map(|w| (w[0], w[1])).collect()
}

/// Classifies an access unit: (key frame, carries parameter sets).
pub fn classify_annexb(codec: Codec, au: &[u8]) -> (bool, bool) {
    let mut key = false;
    let mut cfg = false;
    for (_, h) in start_codes(au) {
        if h >= au.len() {
            continue;
        }
        let t = nal_type(codec, au[h]);
        match codec {
            Codec::H264 => {
                key |= t == 5;
                cfg |= t == 7;
            }
            Codec::Hevc => {
                key |= (16..=21).contains(&t);
                cfg |= t == 32 || t == 33;
            }
            Codec::Av1 => {}
        }
    }
    (key, cfg)
}

fn leb128(b: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, byte) in b.iter().take(8).enumerate() {
        v |= ((byte & 0x7f) as u64) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

/// Splits a low-overhead AV1 OBU stream into temporal units.
/// Returns (ranges, consumed bytes) — a trailing incomplete OBU is left over.
pub fn split_av1(buf: &[u8]) -> (Vec<(usize, usize, bool)>, usize) {
    let mut tus = vec![];
    let mut start: Option<usize> = None;
    let mut has_seq = false;
    let mut p = 0;
    while p < buf.len() {
        let hdr = buf[p];
        let ty = (hdr >> 3) & 0x0f;
        let ext = (hdr >> 2) & 1 == 1;
        let has_size = (hdr >> 1) & 1 == 1;
        let hlen = 1 + ext as usize;
        if !has_size || p + hlen > buf.len() {
            break;
        }
        let Some((size, n)) = leb128(&buf[p + hlen..]) else { break };
        let end = p + hlen + n + size as usize;
        if end > buf.len() {
            break;
        }
        if ty == 2 {
            // temporal delimiter starts a new TU
            if let Some(s) = start {
                tus.push((s, p, has_seq));
            }
            start = Some(p);
            has_seq = false;
        } else if ty == 1 {
            has_seq = true;
        }
        p = end;
    }
    let consumed = p;
    if let Some(s) = start {
        if consumed > s {
            tus.push((s, consumed, has_seq));
        }
    }
    (tus, consumed)
}

// ------------------------------------------------------------ process

pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    pending_ts: Arc<Mutex<VecDeque<u64>>>,
    pub config: EncoderConfig,
    pub stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

#[cfg(target_os = "linux")]
fn enlarge_pipe<F: std::os::fd::AsRawFd>(f: &F) {
    // 1 MiB lets a whole access unit land in the pipe in one write
    // SAFETY: F_SETPIPE_SZ on a pipe fd we own.
    unsafe { libc::fcntl(f.as_raw_fd(), libc::F_SETPIPE_SZ, 1 << 20) };
}

#[cfg(target_os = "linux")]
fn data_pending<F: std::os::fd::AsRawFd>(f: &F, wait_ms: i32) -> bool {
    let mut pfd = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: single valid pollfd
    unsafe { libc::poll(&mut pfd, 1, wait_ms) > 0 }
}

#[cfg(windows)]
fn data_pending<F: std::os::windows::io::AsRawHandle>(f: &F, wait_ms: i32) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Pipes::PeekNamedPipe;
    let h = HANDLE(f.as_raw_handle());
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms as u64);
    loop {
        let mut avail = 0u32;
        // SAFETY: valid pipe handle owned by the child stdout
        let ok = unsafe { PeekNamedPipe(h, None, 0, None, Some(&mut avail), None) }.is_ok();
        if !ok || avail > 0 {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
}

impl Encoder {
    /// Starts FFmpeg; encoded frames are delivered to `out`.
    pub fn start(config: EncoderConfig, out: SyncSender<EncodedFrame>) -> Result<Self> {
        let args = config.args()?;
        info!(encoder = config.encoder_name(), w = config.width, h = config.height, fps = config.fps, kbps = config.bitrate_kbps, "starting encoder");
        debug!("ffmpeg {}", args.join(" "));
        let mut cmd = Command::new(&config.ffmpeg);
        cmd.args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().with_context(|| format!("starting {}", config.ffmpeg))?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().context("ffmpeg stdout")?;
        let stderr = child.stderr.take().context("ffmpeg stderr")?;
        #[cfg(target_os = "linux")]
        {
            enlarge_pipe(&stdout);
            if let Some(s) = &stdin {
                enlarge_pipe(s);
            }
        }
        let pending_ts: Arc<Mutex<VecDeque<u64>>> = Arc::default();
        let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
        {
            let tail = stderr_tail.clone();
            std::thread::Builder::new().name("ffmpeg-stderr".into()).spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                    warn!(target: "ffmpeg", "{line}");
                    let mut t = tail.lock();
                    t.push_back(line);
                    while t.len() > 50 {
                        t.pop_front();
                    }
                }
            })?;
        }
        {
            let ts = pending_ts.clone();
            let codec = config.codec;
            let raw_input = matches!(config.input, EncoderInput::Raw { .. });
            std::thread::Builder::new().name("ffmpeg-reader".into()).spawn(move || {
                let mut acc: Vec<u8> = Vec::with_capacity(1 << 20);
                let mut chunk = vec![0u8; 1 << 20];
                loop {
                    let n = match stdout.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    acc.extend_from_slice(&chunk[..n]);
                    // more bytes of the same packet still in flight?
                    if data_pending(&stdout, 1) {
                        continue;
                    }
                    let emit = |data: Vec<u8>, key: bool, cfg: bool| {
                        let now = now_us();
                        let cap = if raw_input { ts.lock().pop_front().unwrap_or(now) } else { now };
                        let f = EncodedFrame { data, key, config: cfg, capture_us: cap, encode_us: now.saturating_sub(cap) as u32 };
                        out.send(f).is_ok()
                    };
                    let mut alive = true;
                    match codec {
                        Codec::Av1 => {
                            let (tus, consumed) = split_av1(&acc);
                            for (s, e, seq) in tus {
                                alive &= emit(acc[s..e].to_vec(), seq, seq);
                            }
                            acc.drain(..consumed);
                        }
                        _ => {
                            for (s, e) in split_annexb(codec, &acc) {
                                let au = &acc[s..e];
                                let (key, cfg) = classify_annexb(codec, au);
                                alive &= emit(au.to_vec(), key, cfg);
                            }
                            acc.clear();
                        }
                    }
                    if !alive {
                        break;
                    }
                }
                debug!("encoder output closed");
            })?;
        }
        Ok(Self { child, stdin, pending_ts, config, stderr_tail })
    }

    /// Sends one captured frame to the encoder.
    pub fn encode(&mut self, frame: &Frame) -> Result<()> {
        let stdin = self.stdin.as_mut().context("encoder has no raw input")?;
        if frame.width != self.config.width || frame.height != self.config.height {
            bail!("frame {}x{} does not match encoder {}x{}", frame.width, frame.height, self.config.width, self.config.height);
        }
        self.pending_ts.lock().push_back(frame.ready_us);
        frame.write_packed(stdin).context("encoder input closed")?;
        stdin.flush()?;
        Ok(())
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn last_errors(&self) -> String {
        self.stderr_tail.lock().iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ------------------------------------------------------------ probing

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncoderCaps {
    pub codec: Codec,
    pub backend: Backend,
    pub encoder: String,
    /// Largest frame size verified to encode.
    pub max_width: u32,
    pub max_height: u32,
}

fn try_encode(ffmpeg: &str, codec: Codec, backend: Backend, w: u32, h: u32, vaapi_device: &str) -> bool {
    let cfg = EncoderConfig {
        ffmpeg: ffmpeg.into(),
        codec,
        backend,
        width: w,
        height: h,
        fps: 30,
        bitrate_kbps: 2000,
        keyframe_interval_s: 1.0,
        quality: Quality::Speed,
        low_latency: true,
        input: EncoderInput::Raw { format: PixelFormat::Bgr0 },
        vaapi_device: vaapi_device.into(),
        scale_to: None,
    };
    let Ok(args) = cfg.args() else { return false };
    let mut cmd = Command::new(ffmpeg);
    cmd.args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(mut child) = cmd.spawn() else { return false };
    let frame = vec![0x40u8; (w * h * 4) as usize];
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(&frame);
    }
    let mut out = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_end(&mut out);
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    ok && out.len() > 16
}

/// Encoders that actually work on this machine (each is test-run), largest
/// supported size first verified at 1080p, then 4K.
pub fn probe(ffmpeg: &str, vaapi_device: &str) -> Vec<EncoderCaps> {
    let mut caps = vec![];
    for &backend in Backend::platform_order() {
        for codec in Codec::ALL {
            let Some(name) = encoder_name(codec, backend) else { continue };
            if !try_encode(ffmpeg, codec, backend, 1280, 720, vaapi_device) {
                debug!(name, "encoder unavailable");
                continue;
            }
            let (mw, mh) = if try_encode(ffmpeg, codec, backend, 3840, 2160, vaapi_device) {
                (3840, 2160)
            } else if try_encode(ffmpeg, codec, backend, 2560, 1440, vaapi_device) {
                (2560, 1440)
            } else {
                (1920, 1080)
            };
            info!(name, max = format!("{mw}x{mh}"), "encoder available");
            caps.push(EncoderCaps { codec, backend, encoder: name.into(), max_width: mw, max_height: mh });
        }
    }
    caps
}

/// Resolves `Auto` to the best probed backend for `codec`.
pub fn resolve(caps: &[EncoderCaps], codec: Codec, wanted: Backend, hw_allowed: bool) -> Option<EncoderCaps> {
    let ok = |c: &&EncoderCaps| c.codec == codec && (hw_allowed || c.backend == Backend::Software);
    if wanted != Backend::Auto {
        if let Some(c) = caps.iter().filter(ok).find(|c| c.backend == wanted) {
            return Some(c.clone());
        }
    }
    caps.iter().filter(ok).find(|c| c.backend.is_hardware()).or_else(|| caps.iter().filter(ok).next()).cloned()
}

/// Locates the FFmpeg binary: next to the executable first (bundled), then PATH.
pub fn find_ffmpeg() -> Option<String> {
    let exe_name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for cand in [dir.join(exe_name), dir.join("ffmpeg").join(exe_name), dir.join("../lib/dread").join(exe_name)] {
                if cand.is_file() {
                    return Some(cand.to_string_lossy().into_owned());
                }
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|p| p.join(exe_name)).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

/// Spawns a probe in the background.
pub fn probe_async(ffmpeg: String, vaapi_device: String) -> Receiver<Vec<EncoderCaps>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(probe(&ffmpeg, &vaapi_device));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splits_h264_on_aud() {
        let mut b = vec![];
        b.extend_from_slice(&[0, 0, 0, 1, 0x09, 0x10, 0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x65, 9, 9]);
        b.extend_from_slice(&[0, 0, 0, 1, 0x09, 0x30, 0, 0, 1, 0x41, 7, 7]);
        let parts = split_annexb(Codec::H264, &b);
        assert_eq!(parts.len(), 2);
        assert_eq!(classify_annexb(Codec::H264, &b[parts[0].0..parts[0].1]), (true, true));
        assert_eq!(classify_annexb(Codec::H264, &b[parts[1].0..parts[1].1]), (false, false));
    }
    #[test]
    fn splits_av1_on_td() {
        // TD, seq hdr(2 bytes), frame(3 bytes), TD, frame(1 byte), partial
        let b = [0x12, 0, 0x0a, 2, 1, 2, 0x32, 3, 9, 9, 9, 0x12, 0, 0x32, 1, 5, 0x32, 9];
        let (tus, consumed) = split_av1(&b);
        assert_eq!(tus, vec![(0, 11, true), (11, 16, false)]);
        assert_eq!(consumed, 16);
    }
}
