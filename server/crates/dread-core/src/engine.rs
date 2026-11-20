//! The DREAD engine: shared state between network sessions, the streaming
//! pipeline, input injection and the control panel.

use crate::config::Config;
use crate::diag::LogBuffer;
use crate::security::{DeviceStore, Identity, PairedDevice, QrSecret};
use anyhow::{Context, Result};
use dread_platform::display::{self, ActiveDisplay, DisplayBackend, DisplayMode, DriverStatus, OutputInfo};
use dread_platform::encode::{self, EncoderCaps};
use dread_platform::input::{AbsMap, InputSettings};
use dread_proto::InputEvent;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TransportKind {
    Wifi,
    Usb,
}

impl TransportKind {
    pub fn label(self) -> &'static str {
        match self {
            TransportKind::Wifi => "Wi-Fi / LAN",
            TransportKind::Usb => "USB",
        }
    }
}

/// What the client told us about itself in `hello`.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ClientCaps {
    pub width: u32,
    pub height: u32,
    pub density_dpi: u32,
    pub max_fps: u32,
    pub codecs: Vec<String>,
    pub touch_points: u32,
    pub stylus: bool,
    /// Resolution the user picked in the Android settings (0 = native).
    pub preferred_width: u32,
    pub preferred_height: u32,
}

/// Per-client flow-control and latency bookkeeping.
#[derive(Debug, Default)]
pub struct FlowState {
    pub generation: u64,
    pub last_sent: u32,
    pub last_acked: u32,
    pub sent_at: VecDeque<(u32, Instant)>,
    pub delivery_ms: f64,
    pub decode_ms: f64,
    pub rtt_ms: f64,
    pub needs_key: bool,
    pub dropped: u64,
}

pub struct Client {
    pub id: u64,
    pub device_id: String,
    pub name: String,
    pub addr: String,
    pub transport: TransportKind,
    pub caps: ClientCaps,
    pub session: String,
    pub video_key: String,
    pub connected_at: Instant,
    pub ctl_tx: mpsc::UnboundedSender<Vec<u8>>,
    pub video_tx: Option<mpsc::Sender<Arc<Vec<u8>>>>,
    pub flow: FlowState,
    /// Viewport reported by the client (after rotation).
    pub viewport: Option<(u32, u32)>,
    pub tcp_retrans: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientView {
    pub id: u64,
    pub device_id: String,
    pub name: String,
    pub addr: String,
    pub transport: TransportKind,
    pub width: u32,
    pub height: u32,
    pub streaming: bool,
    pub rtt_ms: f64,
    pub decode_ms: f64,
    pub connected_s: u64,
}

pub struct PendingPair {
    pub id: u64,
    pub device_id: String,
    pub device_name: String,
    pub addr: String,
    pub code: String,
    pub created: Instant,
    pub decision: Option<oneshot::Sender<bool>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingPairView {
    pub id: u64,
    pub device_name: String,
    pub addr: String,
    pub code: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub streaming: bool,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub target_fps: u32,
    pub bitrate_kbps: f64,
    pub target_bitrate_kbps: u32,
    pub encode_ms: f64,
    pub decode_ms: f64,
    pub rtt_ms: f64,
    pub delivery_ms: f64,
    pub packet_loss_pct: f64,
    pub frames_dropped: u64,
    pub frames_skipped: u64,
    pub codec: String,
    pub encoder: String,
    pub connection: String,
    pub generation: u64,
}

pub enum InputCmd {
    Event(InputEvent),
    AbsMap(AbsMap),
    Settings(InputSettings),
    ReleaseAll,
}

pub struct DisplayState {
    pub backend: Option<Box<dyn DisplayBackend>>,
    pub backend_error: Option<String>,
    pub active: Option<ActiveDisplay>,
    pub linger_until: Option<Instant>,
}

pub struct Inner {
    pub config: RwLock<Config>,
    pub identity: Identity,
    pub tls: Arc<rustls::ServerConfig>,
    pub devices: Mutex<DeviceStore>,
    pub qr: Mutex<QrSecret>,
    pub clients: Mutex<Vec<Client>>,
    pub pending: Mutex<Vec<PendingPair>>,
    pub display: Mutex<DisplayState>,
    pub stats: Mutex<Stats>,
    pub caps: RwLock<Option<Vec<EncoderCaps>>>,
    pub ffmpeg: RwLock<Option<String>>,
    pub input_tx: std::sync::mpsc::Sender<InputCmd>,
    pub text_tx: Mutex<Option<std::sync::mpsc::Sender<String>>>,
    pub next_id: AtomicU64,
    pub logs: LogBuffer,
    pub streaming_enabled: AtomicBool,
    pub keyframe_request: AtomicBool,
    pub reconfigure_request: AtomicBool,
    pub pipeline: Mutex<Option<crate::pipeline::PipelineHandle>>,
    pub last_error: Mutex<Option<String>>,
    pub usb_status: Mutex<String>,
    pub rt: tokio::runtime::Handle,
}

#[derive(Clone)]
pub struct Engine(pub Arc<Inner>);

impl std::ops::Deref for Engine {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl Engine {
    pub fn new(config: Config, logs: LogBuffer, rt: tokio::runtime::Handle) -> Result<Self> {
        let identity = Identity::load_or_create().context("loading server identity")?;
        let tls = identity.tls_config()?;
        let (input_tx, input_rx) = std::sync::mpsc::channel();
        let (backend, backend_error) = match display::detect_backend() {
            Ok(mut b) => {
                // a previous run that crashed or was killed may have left its output behind
                if let Ok(outs) = b.outputs() {
                    for o in outs.into_iter().filter(|o| o.is_dread) {
                        info!(output = %o.name, "removing virtual display left by a previous run");
                        let _ = b.deactivate(&ActiveDisplay { output: o, owned: true, capture_index: None });
                    }
                }
                (Some(b), None)
            }
            Err(e) => {
                error!("display backend unavailable: {e:#}");
                (None, Some(format!("{e:#}")))
            }
        };
        let ffmpeg = if config.general.ffmpeg_path.is_empty() { encode::find_ffmpeg() } else { Some(config.general.ffmpeg_path.clone()) };
        let streaming_enabled = config.general.auto_stream;
        let input_settings = config.input.clone();
        let engine = Engine(Arc::new(Inner {
            config: RwLock::new(config),
            identity,
            tls,
            devices: Mutex::new(DeviceStore::load()),
            qr: Mutex::new(QrSecret::new()),
            clients: Mutex::new(vec![]),
            pending: Mutex::new(vec![]),
            display: Mutex::new(DisplayState { backend, backend_error, active: None, linger_until: None }),
            stats: Mutex::new(Stats::default()),
            caps: RwLock::new(None),
            ffmpeg: RwLock::new(ffmpeg),
            input_tx,
            text_tx: Mutex::new(None),
            next_id: AtomicU64::new(1),
            logs,
            streaming_enabled: AtomicBool::new(streaming_enabled),
            keyframe_request: AtomicBool::new(false),
            reconfigure_request: AtomicBool::new(false),
            pipeline: Mutex::new(None),
            last_error: Mutex::new(None),
            usb_status: Mutex::new("USB accessory transport idle".into()),
            rt,
        }));
        crate::input_thread::spawn(engine.clone(), input_rx, input_settings);
        engine.start_probe();
        engine.start_housekeeping();
        Ok(engine)
    }

    pub fn start_probe(&self) {
        let Some(ff) = self.ffmpeg.read().clone() else {
            error!("FFmpeg not found: install it or set its path under Advanced");
            *self.caps.write() = Some(vec![]);
            return;
        };
        let dev = self.config.read().general.vaapi_device.clone();
        let e = self.clone();
        std::thread::spawn(move || {
            info!("probing video encoders");
            let caps = encode::probe(&ff, &dev);
            if caps.is_empty() {
                error!("no working video encoder found");
            }
            *e.caps.write() = Some(caps);
            e.update_pipeline();
        });
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    // ------------------------------------------------------------ config

    pub fn config(&self) -> Config {
        self.config.read().clone()
    }

    /// Applies a new configuration; restarts what has to be restarted.
    pub fn set_config(&self, new: Config) {
        let old = self.config();
        if new == old {
            return;
        }
        *self.config.write() = new.clone();
        if let Err(e) = new.save() {
            warn!("could not save config: {e:#}");
        }
        if new.input != old.input {
            let _ = self.input_tx.send(InputCmd::Settings(new.input.clone()));
        }
        if new.display != old.display {
            self.reconfigure_request.store(true, Ordering::SeqCst);
        } else if new.stream != old.stream {
            // codec/bitrate/fps changes need a fresh encoder
            self.keyframe_request.store(true, Ordering::SeqCst);
            self.reconfigure_request.store(true, Ordering::SeqCst);
        }
        if new.general.ffmpeg_path != old.general.ffmpeg_path || new.general.vaapi_device != old.general.vaapi_device {
            let ff = if new.general.ffmpeg_path.is_empty() { encode::find_ffmpeg() } else { Some(new.general.ffmpeg_path.clone()) };
            *self.ffmpeg.write() = ff;
            *self.caps.write() = None;
            self.start_probe();
        }
        crate::autostart::apply(new.general.autostart);
    }

    // ------------------------------------------------------------ clients

    pub fn client_views(&self) -> Vec<ClientView> {
        self.clients
            .lock()
            .iter()
            .map(|c| ClientView {
                id: c.id,
                device_id: c.device_id.clone(),
                name: c.name.clone(),
                addr: c.addr.clone(),
                transport: c.transport,
                width: c.caps.width,
                height: c.caps.height,
                streaming: c.video_tx.is_some(),
                rtt_ms: c.flow.rtt_ms,
                decode_ms: c.flow.decode_ms,
                connected_s: c.connected_at.elapsed().as_secs(),
            })
            .collect()
    }

    pub fn send_to_all(&self, v: &Value) {
        let wire = dread_proto::encode_json(v);
        for c in self.clients.lock().iter() {
            let _ = c.ctl_tx.send(wire.clone());
        }
    }

    pub fn disconnect(&self, id: u64, reason: &str) {
        if let Some(c) = self.clients.lock().iter().find(|c| c.id == id) {
            let _ = c.ctl_tx.send(dread_proto::encode_json(&json!({"t":"bye","reason":reason})));
        }
    }

    pub fn register_client(&self, c: Client) {
        info!(name = %c.name, addr = %c.addr, transport = c.transport.label(), "client connected");
        self.clients.lock().push(c);
        self.update_pipeline();
    }

    pub fn unregister_client(&self, id: u64) {
        let removed = {
            let mut cl = self.clients.lock();
            let before = cl.len();
            cl.retain(|c| c.id != id);
            before != cl.len()
        };
        if removed {
            info!(id, "client disconnected");
            let _ = self.input_tx.send(InputCmd::ReleaseAll);
            self.update_pipeline();
        }
    }

    pub fn attach_video(&self, session: &str, key: &str, tx: mpsc::Sender<Arc<Vec<u8>>>) -> Option<u64> {
        let id = {
            let mut cl = self.clients.lock();
            let c = cl.iter_mut().find(|c| c.session == session && c.video_key == key)?;
            c.video_tx = Some(tx);
            c.flow = FlowState { needs_key: true, ..Default::default() };
            c.id
        };
        self.keyframe_request.store(true, Ordering::SeqCst);
        self.update_pipeline();
        Some(id)
    }

    pub fn detach_video(&self, id: u64) {
        if let Some(c) = self.clients.lock().iter_mut().find(|c| c.id == id) {
            c.video_tx = None;
        }
        self.update_pipeline();
    }

    pub fn client_viewport(&self, id: u64, w: u32, h: u32) {
        let changed = {
            let mut cl = self.clients.lock();
            let Some(c) = cl.iter_mut().find(|c| c.id == id) else { return };
            let changed = c.viewport != Some((w, h));
            c.viewport = Some((w, h));
            changed && cl.first().map(|f| f.id) == Some(id)
        };
        if changed && self.config.read().display.auto_resolution {
            info!(w, h, "client viewport changed; reconfiguring display");
            self.reconfigure_request.store(true, Ordering::SeqCst);
        }
    }

    pub fn on_ack(&self, id: u64, gen: u64, seq: u32, decode_us: u64) {
        let mut cl = self.clients.lock();
        let Some(c) = cl.iter_mut().find(|c| c.id == id) else { return };
        if gen != c.flow.generation {
            return;
        }
        if seq > c.flow.last_acked || c.flow.last_acked == 0 {
            c.flow.last_acked = seq;
        }
        while let Some(&(s, t)) = c.flow.sent_at.front() {
            if s > seq {
                break;
            }
            c.flow.sent_at.pop_front();
            if s == seq {
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                c.flow.delivery_ms = if c.flow.delivery_ms == 0.0 { ms } else { c.flow.delivery_ms * 0.9 + ms * 0.1 };
            }
        }
        let d = decode_us as f64 / 1000.0;
        c.flow.decode_ms = if c.flow.decode_ms == 0.0 { d } else { c.flow.decode_ms * 0.9 + d * 0.1 };
    }

    pub fn on_rtt(&self, id: u64, rtt_ms: f64) {
        if let Some(c) = self.clients.lock().iter_mut().find(|c| c.id == id) {
            c.flow.rtt_ms = if c.flow.rtt_ms == 0.0 { rtt_ms } else { c.flow.rtt_ms * 0.7 + rtt_ms * 0.3 };
        }
    }

    pub fn request_keyframe(&self) {
        self.keyframe_request.store(true, Ordering::SeqCst);
    }

    // ------------------------------------------------------------ pairing

    pub fn devices(&self) -> Vec<PairedDevice> {
        self.devices.lock().devices.clone()
    }

    pub fn revoke(&self, device_id: &str) {
        if let Err(e) = self.devices.lock().revoke(device_id) {
            warn!("revoke failed: {e:#}");
        }
        let ids: Vec<u64> = self.clients.lock().iter().filter(|c| c.device_id == device_id).map(|c| c.id).collect();
        for id in ids {
            self.disconnect(id, "device was unpaired on the server");
        }
    }

    pub fn pending_pairs(&self) -> Vec<PendingPairView> {
        let mut p = self.pending.lock();
        p.retain(|x| x.created.elapsed() < Duration::from_secs(120) && x.decision.is_some());
        p.iter().map(|x| PendingPairView { id: x.id, device_name: x.device_name.clone(), addr: x.addr.clone(), code: x.code.clone() }).collect()
    }

    pub fn decide_pair(&self, id: u64, accept: bool) {
        let mut p = self.pending.lock();
        if let Some(x) = p.iter_mut().find(|x| x.id == id) {
            if let Some(tx) = x.decision.take() {
                let _ = tx.send(accept);
            }
        }
        p.retain(|x| x.id != id);
    }

    pub fn qr_secret(&self) -> String {
        let mut q = self.qr.lock();
        if q.expired() {
            *q = QrSecret::new();
        }
        q.b64()
    }

    pub fn rotate_qr(&self) {
        *self.qr.lock() = QrSecret::new();
    }

    /// `dread://pair?...` URI shown as a QR code.
    pub fn pairing_uri(&self) -> String {
        let cfg = self.config.read();
        let hosts: Vec<String> = crate::net::lan_addresses().iter().map(|a| a.to_string()).collect();
        let name: String = cfg.general.server_name.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
        format!(
            "dread://pair?v=1&h={}&p={}&fp={}&s={}&n={}",
            hosts.join(","),
            cfg.connection.port,
            self.identity.fingerprint_hex(),
            self.qr_secret(),
            name
        )
    }

    // ------------------------------------------------------------ display

    pub fn outputs(&self) -> Vec<OutputInfo> {
        let mut d = self.display.lock();
        d.backend.as_mut().and_then(|b| b.outputs().ok()).unwrap_or_default()
    }

    pub fn driver_status(&self) -> DriverStatus {
        let mut d = self.display.lock();
        if let Some(b) = d.backend.as_mut() {
            return b.driver_status();
        }
        DriverStatus { backend: "none".into(), ok: false, detail: d.backend_error.clone().unwrap_or_default() }
    }

    pub fn virtual_modes(&self) -> (Vec<(u32, u32)>, Vec<u32>, (u32, u32)) {
        let d = self.display.lock();
        match d.backend.as_ref() {
            Some(b) => (b.virtual_modes(), b.virtual_refresh_rates(), b.max_virtual_size()),
            None => (vec![], vec![], (0, 0)),
        }
    }

    pub fn active_display(&self) -> Option<ActiveDisplay> {
        self.display.lock().active.clone()
    }

    pub fn set_streaming(&self, on: bool) {
        self.streaming_enabled.store(on, Ordering::SeqCst);
        self.update_pipeline();
    }

    pub fn is_streaming_enabled(&self) -> bool {
        self.streaming_enabled.load(Ordering::SeqCst)
    }

    /// Starts or stops the pipeline to match clients and the Start/Stop switch.
    pub fn update_pipeline(&self) {
        let want = self.is_streaming_enabled() && self.clients.lock().iter().any(|c| c.video_tx.is_some()) && self.caps.read().is_some();
        let mut p = self.pipeline.lock();
        let running = p.as_ref().map(|h| h.is_running()).unwrap_or(false);
        if want && !running {
            *p = Some(crate::pipeline::start(self.clone()));
        } else if !want && running {
            if let Some(h) = p.take() {
                h.stop();
            }
        }
    }

    /// Removes the virtual display once nobody has used it for the linger time.
    fn start_housekeeping(&self) {
        let e = self.clone();
        std::thread::Builder::new()
            .name("housekeeping".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(500));
                let running = e.pipeline.lock().as_ref().map(|h| h.is_running()).unwrap_or(false);
                let mut d = e.display.lock();
                if running {
                    d.linger_until = None;
                    continue;
                }
                let linger = Duration::from_secs(e.config.read().connection.display_linger_s as u64);
                if d.active.is_some() {
                    let until = *d.linger_until.get_or_insert_with(|| Instant::now() + linger);
                    if Instant::now() >= until || !e.is_streaming_enabled() {
                        let act = d.active.take().unwrap();
                        d.linger_until = None;
                        if let Some(b) = d.backend.as_mut() {
                            if let Err(err) = b.deactivate(&act) {
                                warn!("removing display failed: {err:#}");
                            }
                        }
                        e.stats.lock().streaming = false;
                    }
                }
            })
            .expect("spawn housekeeping");
    }

    /// Tears everything down (called on exit).
    pub fn shutdown(&self) {
        info!("shutting down");
        self.send_to_all(&json!({"t":"bye","reason":"server shutting down"}));
        if let Some(h) = self.pipeline.lock().take() {
            h.stop();
        }
        let _ = self.input_tx.send(InputCmd::ReleaseAll);
        let mut d = self.display.lock();
        if let Some(act) = d.active.take() {
            if let Some(b) = d.backend.as_mut() {
                let _ = b.deactivate(&act);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    pub fn set_error(&self, e: Option<String>) {
        if let Some(m) = &e {
            error!("{m}");
        }
        *self.last_error.lock() = e;
    }

    pub fn display_mode(&self) -> DisplayMode {
        self.config.read().display.mode
    }

    pub fn type_text(&self, text: String) {
        match self.text_tx.lock().as_ref() {
            Some(tx) => {
                let _ = tx.send(text);
            }
            None => warn!("text input is not available on this desktop session"),
        }
    }
}
