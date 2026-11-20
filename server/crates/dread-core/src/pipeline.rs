//! The streaming pipeline: display → capture → encode → clients.
//!
//! One pipeline thread owns capture and the encoder; a distributor thread per
//! encoder instance fans encoded frames out to the clients' video channels.
//!
//! Flow control: the pipeline only captures a new frame when every streaming
//! client has fewer than `max_frames_in_flight` frames sent but not yet
//! acknowledged. A slow link therefore lowers the frame rate instead of
//! building a queue — what is on the tablet is never more than a couple of
//! frames old.

use crate::config::Config;
use crate::engine::{Engine, InputCmd};
use anyhow::{bail, Context, Result};
use dread_platform::display::{self, ActiveDisplay, DisplayMode, DisplaySettings, Orientation};
use dread_platform::encode::{self, Codec, EncodedFrame, Encoder, EncoderCaps, EncoderConfig, EncoderInput};
use dread_proto::{VideoFrame, VIDEO_FLAG_CONFIG, VIDEO_FLAG_KEY};
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub struct PipelineHandle {
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PipelineHandle {
    pub fn is_running(&self) -> bool {
        !self.done.load(Ordering::SeqCst)
    }
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn start(e: Engine) -> PipelineHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let (s, d) = (stop.clone(), done.clone());
    let thread = std::thread::Builder::new()
        .name("pipeline".into())
        .spawn(move || {
            run(&e, &s);
            {
                let mut st = e.stats.lock();
                st.streaming = false;
                st.fps = 0.0;
                st.bitrate_kbps = 0.0;
            }
            d.store(true, Ordering::SeqCst);
        })
        .expect("spawn pipeline");
    PipelineHandle { stop, done, thread: Some(thread) }
}

#[derive(Default)]
struct Counters {
    frames: AtomicU64,
    bytes: AtomicU64,
    encode_us: AtomicU64,
    dropped: AtomicU64,
}

/// Adaptive bitrate / resolution state, kept across encoder restarts.
struct Abr {
    target_kbps: u32,
    scale: f32,
    congested_since: Option<Instant>,
    healthy_since: Instant,
    last_change: Instant,
}

impl Abr {
    fn new(kbps: u32) -> Self {
        Self { target_kbps: kbps, scale: 1.0, congested_since: None, healthy_since: Instant::now(), last_change: Instant::now() }
    }
}

const MIN_KBPS: u32 = 1500;

/// Display settings after automatic resolution / orientation / scale.
pub fn effective_display(e: &Engine, cfg: &Config) -> DisplaySettings {
    let mut s = cfg.display.clone();
    if s.mode != DisplayMode::Extend || !s.auto_resolution {
        if s.scale <= 0.0 {
            s.scale = 1.0;
        }
        return s;
    }
    let clients = e.clients.lock();
    let Some(c) = clients.iter().find(|c| c.video_tx.is_some()).or(clients.first()) else { return s };
    let (mut w, mut h) = if c.caps.preferred_width > 0 && c.caps.preferred_height > 0 {
        (c.caps.preferred_width, c.caps.preferred_height)
    } else {
        c.viewport.unwrap_or((c.caps.width, c.caps.height))
    };
    if let Some((vw, vh)) = c.viewport {
        // keep the preferred size but follow the current orientation
        if (vw > vh) != (w > h) {
            std::mem::swap(&mut w, &mut h);
        }
    }
    if w == 0 || h == 0 {
        return s;
    }
    let max_long = 3840.0f64;
    let long = w.max(h) as f64;
    if long > max_long {
        let k = max_long / long;
        w = (w as f64 * k) as u32;
        h = (h as f64 * k) as u32;
    }
    s.width = w & !1;
    s.height = h & !1;
    s.orientation = if h > w { Orientation::Portrait } else { Orientation::Landscape };
    if s.scale <= 0.0 {
        let dpi = if c.caps.density_dpi > 0 { c.caps.density_dpi } else { 160 } as f32;
        s.scale = ((dpi / 160.0 * 0.6 * 4.0).round() / 4.0).clamp(1.0, 3.0);
    }
    s.refresh = cfg.stream.fps.min(if c.caps.max_fps > 0 { c.caps.max_fps } else { 240 });
    s
}

fn activate_display(e: &Engine, s: &DisplaySettings) -> Result<ActiveDisplay> {
    let mut guard = e.display.lock();
    let d = &mut *guard;
    let backend = d.backend.as_mut().context("no display backend for this desktop session")?;
    let active = match d.active.take() {
        Some(a) if a.owned == (s.mode == DisplayMode::Extend) => {
            let same = a.output.width == s.effective_size().0 && a.output.height == s.effective_size().1;
            if same && s.mode == DisplayMode::Extend && !e.reconfigure_request.load(Ordering::SeqCst) {
                Ok(a)
            } else {
                backend.reconfigure(&a, s)
            }
        }
        Some(a) => {
            backend.deactivate(&a)?;
            backend.activate(s)
        }
        None => backend.activate(s),
    }?;
    let devices = dread_platform_input_devices();
    if let Err(err) = backend.bind_input(&active, &devices) {
        warn!("binding touch/pen to {} failed: {err:#}", active.output.name);
    }
    let outputs = backend.outputs().unwrap_or_default();
    let _ = e.input_tx.send(InputCmd::AbsMap(display::abs_map(&active.output, &outputs)));
    d.active = Some(active.clone());
    d.linger_until = None;
    Ok(active)
}

fn dread_platform_input_devices() -> Vec<&'static str> {
    #[cfg(target_os = "linux")]
    {
        dread_platform::input::linux::LinuxInjector::device_names().to_vec()
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec![]
    }
}

fn run(e: &Engine, stop: &AtomicBool) {
    let mut abr = Abr::new(e.config.read().stream.bitrate_kbps);
    let mut failures = 0u32;
    while !stop.load(Ordering::SeqCst) {
        let cfg = e.config();
        if abr.target_kbps > cfg.stream.bitrate_kbps || !cfg.stream.adaptive_bitrate {
            abr.target_kbps = cfg.stream.bitrate_kbps;
        }
        if !cfg.stream.auto_quality {
            abr.scale = 1.0;
        }
        let ds = effective_display(e, &cfg);
        let active = match activate_display(e, &ds) {
            Ok(a) => a,
            Err(err) => {
                e.set_error(Some(format!("Display: {err:#}")));
                sleep_unless(stop, Duration::from_secs(2));
                continue;
            }
        };
        e.reconfigure_request.store(false, Ordering::SeqCst);
        e.send_to_all(&json!({
            "t": "display_info",
            "mode": if active.owned { "extend" } else { "duplicate" },
            "name": active.output.name,
            "width": active.output.width,
            "height": active.output.height,
        }));
        match stream(e, stop, &active, &cfg, &mut abr) {
            Ok(()) => failures = 0,
            Err(err) => {
                failures += 1;
                e.set_error(Some(format!("Streaming: {err:#}")));
                sleep_unless(stop, Duration::from_millis(500 * failures.min(8) as u64));
            }
        }
    }
    info!("pipeline stopped");
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Codec all streaming clients can decode.
fn negotiate_codec(e: &Engine, wanted: Codec) -> Codec {
    let clients = e.clients.lock();
    let ok = |c: Codec| clients.iter().filter(|x| x.video_tx.is_some()).all(|x| x.caps.codecs.iter().any(|n| n == c.wire_name()));
    if ok(wanted) {
        wanted
    } else {
        warn!("a client cannot decode {}; using H.264", wanted.label());
        Codec::H264
    }
}

fn client_fps_cap(e: &Engine) -> u32 {
    e.clients.lock().iter().filter(|c| c.video_tx.is_some() && c.caps.max_fps > 0).map(|c| c.caps.max_fps).min().unwrap_or(240)
}

fn fit(w: u32, h: u32, max_w: u32, max_h: u32, scale: f32) -> Option<(u32, u32)> {
    let (lw, lh) = if w >= h { (max_w, max_h) } else { (max_h, max_w) };
    let k = (lw as f64 / w as f64).min(lh as f64 / h as f64).min(1.0) * scale as f64;
    if k >= 0.999 {
        None
    } else {
        Some((((w as f64 * k) as u32).max(320) & !1, ((h as f64 * k) as u32).max(240) & !1))
    }
}

struct EncoderPlan {
    caps: EncoderCaps,
    fps: u32,
}

fn plan_encoder(e: &Engine, cfg: &Config) -> Result<EncoderPlan> {
    let caps_all = e.caps.read().clone().unwrap_or_default();
    let codec = negotiate_codec(e, cfg.stream.codec);
    let caps = encode::resolve(&caps_all, codec, cfg.stream.backend, cfg.stream.hardware_acceleration)
        .or_else(|| encode::resolve(&caps_all, Codec::H264, cfg.stream.backend, cfg.stream.hardware_acceleration))
        .context("no working encoder (see Advanced → Diagnostics)")?;
    let fps = cfg.stream.fps.min(client_fps_cap(e)).max(1);
    Ok(EncoderPlan { caps, fps })
}

fn effective_kbps(cfg: &Config, abr: &Abr) -> u32 {
    let mut k = if cfg.stream.adaptive_bitrate { abr.target_kbps } else { cfg.stream.bitrate_kbps };
    if cfg.stream.bandwidth_limit_kbps > 0 {
        k = k.min(cfg.stream.bandwidth_limit_kbps);
    }
    k.max(300)
}

#[allow(clippy::too_many_arguments)]
fn start_encoder(
    e: &Engine,
    cfg: &Config,
    plan: &EncoderPlan,
    abr: &Abr,
    w: u32,
    h: u32,
    input: EncoderInput,
    counters: &Arc<Counters>,
    generation: u64,
) -> Result<Encoder> {
    let ffmpeg = e.ffmpeg.read().clone().context("FFmpeg not found")?;
    let scale = if cfg.stream.auto_quality { abr.scale } else { 1.0 };
    let ecfg = EncoderConfig {
        ffmpeg,
        codec: plan.caps.codec,
        backend: plan.caps.backend,
        width: w,
        height: h,
        fps: plan.fps,
        bitrate_kbps: effective_kbps(cfg, abr),
        keyframe_interval_s: cfg.stream.keyframe_interval_s,
        quality: cfg.stream.quality,
        low_latency: cfg.stream.low_latency,
        input,
        vaapi_device: cfg.general.vaapi_device.clone(),
        scale_to: fit(w, h, plan.caps.max_width, plan.caps.max_height, scale),
    };
    let (ow, oh) = ecfg.output_size();
    let (tx, rx) = std::sync::mpsc::sync_channel::<EncodedFrame>(32);
    let enc = Encoder::start(ecfg.clone(), tx)?;
    {
        let mut st = e.stats.lock();
        st.streaming = true;
        st.width = ow;
        st.height = oh;
        st.target_fps = plan.fps;
        st.target_bitrate_kbps = ecfg.bitrate_kbps;
        st.codec = plan.caps.codec.label().into();
        st.encoder = format!("{} ({})", plan.caps.encoder, plan.caps.backend.label());
        st.generation = generation;
    }
    let config_msg = json!({
        "t": "stream_config",
        "codec": plan.caps.codec.wire_name(),
        "width": ow,
        "height": oh,
        "fps": plan.fps,
        "bitrate": ecfg.bitrate_kbps * 1000,
        "generation": generation,
    });
    spawn_distributor(e.clone(), rx, generation, dread_proto::encode_json(&config_msg), counters.clone());
    e.set_error(None);
    Ok(enc)
}

fn spawn_distributor(e: Engine, rx: std::sync::mpsc::Receiver<EncodedFrame>, generation: u64, config_wire: Vec<u8>, counters: Arc<Counters>) {
    let config_wire = Arc::new(config_wire);
    std::thread::Builder::new()
        .name("distributor".into())
        .spawn(move || {
            let mut seq: u32 = 0;
            while let Ok(f) = rx.recv() {
                seq = seq.wrapping_add(1);
                let mut flags = 0;
                if f.key {
                    flags |= VIDEO_FLAG_KEY;
                }
                if f.config {
                    flags |= VIDEO_FLAG_CONFIG;
                }
                let len = f.data.len();
                let wire = Arc::new(VideoFrame { flags, seq, capture_us: f.capture_us, encode_us: f.encode_us, data: f.data }.to_wire());
                counters.frames.fetch_add(1, Ordering::Relaxed);
                counters.bytes.fetch_add(len as u64, Ordering::Relaxed);
                counters.encode_us.fetch_add(f.encode_us as u64, Ordering::Relaxed);
                let mut want_key = false;
                for c in e.clients.lock().iter_mut() {
                    let Some(tx) = c.video_tx.as_ref() else { continue };
                    if c.flow.generation != generation {
                        c.flow.generation = generation;
                        c.flow.last_sent = 0;
                        c.flow.last_acked = 0;
                        c.flow.sent_at.clear();
                        c.flow.needs_key = true;
                        if tx.try_send(config_wire.clone()).is_err() {
                            c.flow.generation = 0;
                            continue;
                        }
                    }
                    if c.flow.needs_key && !f.key {
                        continue;
                    }
                    match tx.try_send(wire.clone()) {
                        Ok(()) => {
                            c.flow.needs_key = false;
                            c.flow.last_sent = seq;
                            c.flow.sent_at.push_back((seq, Instant::now()));
                            while c.flow.sent_at.len() > 120 {
                                c.flow.sent_at.pop_front();
                            }
                        }
                        Err(_) => {
                            c.flow.needs_key = true;
                            c.flow.dropped += 1;
                            counters.dropped.fetch_add(1, Ordering::Relaxed);
                            want_key = true;
                        }
                    }
                }
                if want_key {
                    e.request_keyframe();
                }
            }
            debug!(generation, "distributor finished");
        })
        .expect("spawn distributor");
}

/// Largest number of unacknowledged frames among streaming clients.
fn frames_in_flight(e: &Engine, generation: u64) -> u32 {
    e.clients
        .lock()
        .iter()
        .filter(|c| c.video_tx.is_some() && c.flow.generation == generation && !c.flow.needs_key)
        .map(|c| c.flow.last_sent.saturating_sub(c.flow.last_acked))
        .max()
        .unwrap_or(0)
}

/// Updates stats once a second and runs the adaptive bitrate controller.
/// Returns true when the encoder must be restarted with new targets.
fn tick_stats(e: &Engine, cfg: &Config, counters: &Counters, abr: &mut Abr, waited: &mut Duration, period: Duration, kbps_now: u32) -> bool {
    let frames = counters.frames.swap(0, Ordering::Relaxed);
    let bytes = counters.bytes.swap(0, Ordering::Relaxed);
    let enc_us = counters.encode_us.swap(0, Ordering::Relaxed);
    let dropped = counters.dropped.swap(0, Ordering::Relaxed);
    let secs = period.as_secs_f64().max(0.001);
    let (rtt, decode, delivery, loss, conn) = {
        let mut cl = e.clients.lock();
        let streaming: Vec<_> = cl.iter_mut().filter(|c| c.video_tx.is_some()).collect();
        let n = streaming.len().max(1) as f64;
        let rtt = streaming.iter().map(|c| c.flow.rtt_ms).sum::<f64>() / n;
        let dec = streaming.iter().map(|c| c.flow.decode_ms).sum::<f64>() / n;
        let del = streaming.iter().map(|c| c.flow.delivery_ms).fold(0.0, f64::max);
        let loss = streaming
            .iter()
            .filter_map(|c| c.tcp_retrans)
            .map(|(r, s)| if s > 0 { r as f64 * 100.0 / s as f64 } else { 0.0 })
            .fold(0.0, f64::max);
        let conn = streaming.first().map(|c| c.transport.label().to_string()).unwrap_or_default();
        (rtt, dec, del, loss, conn)
    };
    let wait_frac = waited.as_secs_f64() / secs;
    *waited = Duration::ZERO;
    {
        let mut st = e.stats.lock();
        st.fps = frames as f64 / secs;
        st.bitrate_kbps = bytes as f64 * 8.0 / 1000.0 / secs;
        st.encode_ms = if frames > 0 { enc_us as f64 / frames as f64 / 1000.0 } else { st.encode_ms };
        st.rtt_ms = rtt;
        st.decode_ms = decode;
        st.delivery_ms = delivery;
        st.packet_loss_pct = loss;
        st.frames_dropped += dropped;
        st.frames_skipped = (wait_frac * 100.0) as u64;
        st.connection = conn;
    }
    if !cfg.stream.adaptive_bitrate {
        return false;
    }
    // Congestion: frames take much longer to arrive than the network RTT
    // explains, or the pipeline spent most of the second waiting for acks.
    let frame_ms = 1000.0 / cfg.stream.fps.max(1) as f64;
    let congested = dropped > 0 || (frames > 3 && delivery > rtt + decode + 3.0 * frame_ms + 40.0) || wait_frac > 0.5;
    let now = Instant::now();
    if congested {
        abr.healthy_since = now;
        let since = *abr.congested_since.get_or_insert(now);
        if now.duration_since(since) >= Duration::from_millis(1500) && now.duration_since(abr.last_change) >= Duration::from_secs(3) {
            abr.congested_since = None;
            abr.last_change = now;
            if abr.target_kbps > MIN_KBPS {
                abr.target_kbps = ((abr.target_kbps as f64 * 0.7) as u32).max(MIN_KBPS);
                info!(kbps = abr.target_kbps, delivery, rtt, "link congested: lowering bitrate");
                return true;
            } else if cfg.stream.auto_quality && abr.scale > 0.5 {
                abr.scale = (abr.scale * 0.75).max(0.5);
                info!(scale = abr.scale, "link congested at minimum bitrate: lowering resolution");
                return true;
            }
        }
    } else {
        abr.congested_since = None;
        if now.duration_since(abr.healthy_since) >= Duration::from_secs(8) && now.duration_since(abr.last_change) >= Duration::from_secs(8) {
            abr.healthy_since = now;
            if abr.scale < 1.0 {
                abr.scale = (abr.scale / 0.75).min(1.0);
                abr.last_change = now;
                info!(scale = abr.scale, "link healthy: raising resolution");
                return true;
            }
            if abr.target_kbps < cfg.stream.bitrate_kbps && frames > 0 {
                abr.target_kbps = ((abr.target_kbps as f64 * 1.25) as u32).min(cfg.stream.bitrate_kbps);
                abr.last_change = now;
                info!(kbps = abr.target_kbps, "link healthy: raising bitrate");
                return true;
            }
        }
    }
    let _ = kbps_now;
    false
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

#[cfg(target_os = "linux")]
fn stream(e: &Engine, stop: &AtomicBool, active: &ActiveDisplay, cfg: &Config, abr: &mut Abr) -> Result<()> {
    use dread_platform::capture::wlr::WlrCapture;
    let mut cap = WlrCapture::new(&active.output.name, cfg.stream.show_cursor)?;
    let counters = Arc::new(Counters::default());
    let plan = plan_encoder(e, cfg)?;
    let interval = Duration::from_secs_f64(1.0 / plan.fps as f64);
    let mut enc: Option<Encoder> = None;
    let mut generation = 0u64;
    let mut force = true;
    let mut last_frame = Instant::now() - interval;
    let mut last_restart = Instant::now() - Duration::from_secs(10);
    let mut stats_at = Instant::now();
    let mut waited = Duration::ZERO;
    let max_inflight = cfg.stream.max_frames_in_flight.clamp(1, 8);
    info!(output = %active.output.name, encoder = %plan.caps.encoder, fps = plan.fps, "streaming");
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        if e.reconfigure_request.load(Ordering::SeqCst) {
            return Ok(());
        }
        if e.keyframe_request.load(Ordering::SeqCst) && last_restart.elapsed() >= Duration::from_millis(700) {
            e.keyframe_request.store(false, Ordering::SeqCst);
            enc = None;
            force = true;
        }
        if let Some(en) = enc.as_mut() {
            if !en.is_running() {
                let msg = en.last_errors();
                drop(enc.take());
                bail!("encoder exited: {msg}");
            }
        }
        let now = Instant::now();
        if now.duration_since(stats_at) >= Duration::from_secs(1) {
            let kbps = enc.as_ref().map(|x| x.config.bitrate_kbps).unwrap_or(0);
            if tick_stats(e, cfg, &counters, abr, &mut waited, now.duration_since(stats_at), kbps) {
                enc = None;
                force = true;
            }
            stats_at = now;
        }
        // flow control
        if enc.is_some() && frames_in_flight(e, generation) >= max_inflight {
            let t = Instant::now();
            std::thread::sleep(Duration::from_millis(1));
            waited += t.elapsed();
            if waited > Duration::from_millis(1500) && frames_in_flight(e, generation) >= max_inflight {
                // a client stopped acknowledging: resynchronise with a key frame
                warn!("client stopped acknowledging frames; forcing key frame");
                e.request_keyframe();
                waited = Duration::ZERO;
                for c in e.clients.lock().iter_mut() {
                    c.flow.last_acked = c.flow.last_sent;
                }
            }
            continue;
        }
        // pacing
        let due = last_frame + interval;
        if now < due {
            std::thread::sleep((due - now).min(Duration::from_millis(5)));
            continue;
        }
        let Some(frame) = cap.next_frame(force, Duration::from_millis(50))? else { continue };
        last_frame = Instant::now();
        let restart = match &enc {
            None => true,
            Some(x) => x.config.width != frame.width || x.config.height != frame.height,
        };
        if restart {
            drop(enc.take());
            generation = GENERATION.fetch_add(1, Ordering::SeqCst);
            enc = Some(start_encoder(e, cfg, &plan, abr, frame.width, frame.height, EncoderInput::Raw { format: frame.format }, &counters, generation)?);
            last_restart = Instant::now();
        }
        force = false;
        enc.as_mut().unwrap().encode(&frame)?;
    }
}

#[cfg(windows)]
fn stream(e: &Engine, stop: &AtomicBool, active: &ActiveDisplay, cfg: &Config, abr: &mut Abr) -> Result<()> {
    let counters = Arc::new(Counters::default());
    let plan = plan_encoder(e, cfg)?;
    let idx = active.capture_index.context("display has no Desktop Duplication index")?;
    let (w, h) = (active.output.width, active.output.height);
    let mut stats_at = Instant::now();
    let mut waited = Duration::ZERO;
    loop {
        let generation = GENERATION.fetch_add(1, Ordering::SeqCst);
        let mut enc = start_encoder(e, cfg, &plan, abr, w, h, EncoderInput::Ddagrab { output_idx: idx, draw_mouse: cfg.stream.show_cursor }, &counters, generation)?;
        e.keyframe_request.store(false, Ordering::SeqCst);
        let started = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(10));
            if stop.load(Ordering::SeqCst) || e.reconfigure_request.load(Ordering::SeqCst) {
                return Ok(());
            }
            if !enc.is_running() {
                bail!("encoder exited: {}", enc.last_errors());
            }
            let now = Instant::now();
            if now.duration_since(stats_at) >= Duration::from_secs(1) {
                let restart = tick_stats(e, cfg, &counters, abr, &mut waited, now.duration_since(stats_at), enc.config.bitrate_kbps);
                stats_at = now;
                if restart {
                    break;
                }
            }
            if e.keyframe_request.load(Ordering::SeqCst) && started.elapsed() > Duration::from_millis(700) {
                break;
            }
        }
    }
}
