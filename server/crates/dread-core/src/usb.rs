//! USB transport using the Android Open Accessory (AOA) protocol.
//!
//! The computer acts as USB host and asks the phone to switch into accessory
//! mode. This works without USB debugging / developer options: Android shows
//! "Open DREAD to handle DREAD Display?" and the app talks over two bulk
//! endpoints. Both logical channels (control, video) are multiplexed over the
//! pipe as `[u8 channel][u32 length][bytes]`, then handed to the very same
//! session code used for Wi-Fi.

use crate::engine::{Engine, TransportKind};
use crate::session::{self, ConnInfo};
use anyhow::{bail, Context, Result};
use rusb::{Device, DeviceHandle, Direction, GlobalContext, TransferType, UsbContext};
use std::collections::HashSet;
use std::sync::mpsc as smpsc;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info, warn};

const AOA_VID: u16 = 0x18d1;
const AOA_PIDS: std::ops::RangeInclusive<u16> = 0x2d00..=0x2d05;
const AOA_GET_PROTOCOL: u8 = 51;
const AOA_SEND_STRING: u8 = 52;
const AOA_START: u8 = 53;

pub const ACC_MANUFACTURER: &str = "DREAD";
pub const ACC_MODEL: &str = "DREAD Display";
const ACC_DESCRIPTION: &str = "DREAD virtual display link";
const ACC_VERSION: &str = "1";
const ACC_URI: &str = "https://dread.app/android";
const ACC_SERIAL: &str = "DREAD-1";

const CH_CONTROL: u8 = 0;
const CH_VIDEO: u8 = 1;
const READ_BUF: usize = 16384;

fn set_status(e: &Engine, s: impl Into<String>) {
    let s = s.into();
    let mut st = e.usb_status.lock();
    if *st != s {
        info!("USB: {s}");
        *st = s;
    }
}

/// Does this look like a phone/tablet worth asking to switch to accessory mode?
fn looks_like_android(dev: &Device<GlobalContext>) -> bool {
    let Ok(desc) = dev.device_descriptor() else { return false };
    if matches!(desc.class_code(), 0x09 | 0x03 | 0x01 | 0x0e | 0xe0) {
        return false; // hub, HID, audio, video, wireless
    }
    let Ok(cfg) = dev.active_config_descriptor() else { return false };
    cfg.interfaces().flat_map(|i| i.descriptors()).any(|d| {
        matches!((d.class_code(), d.sub_class_code()), (0x06, 0x01) | (0xff, 0xff) | (0xff, 0x42) | (0x02, _) | (0x0a, _) | (0xef, _) | (0xe0, 0x01))
    })
}

fn switch_to_accessory(dev: &Device<GlobalContext>) -> Result<u16> {
    let h = dev.open().context("opening USB device (permission? see docs/USB.md)")?;
    let t = Duration::from_millis(500);
    let mut ver = [0u8; 2];
    let n = h.read_control(0xc0, AOA_GET_PROTOCOL, 0, 0, &mut ver, t).context("AOA get protocol")?;
    let version = if n == 2 { u16::from_le_bytes(ver) } else { 0 };
    if version == 0 {
        bail!("device does not support Android accessory mode");
    }
    for (i, s) in [ACC_MANUFACTURER, ACC_MODEL, ACC_DESCRIPTION, ACC_VERSION, ACC_URI, ACC_SERIAL].iter().enumerate() {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        h.write_control(0x40, AOA_SEND_STRING, 0, i as u16, &b, t).context("AOA send string")?;
    }
    h.write_control(0x40, AOA_START, 0, 0, &[], t).context("AOA start")?;
    Ok(version)
}

fn bulk_endpoints(dev: &Device<GlobalContext>) -> Result<(u8, u8, u8, u16)> {
    let cfg = dev.active_config_descriptor()?;
    for iface in cfg.interfaces() {
        for d in iface.descriptors() {
            let mut inp = None;
            let mut out = None;
            let mut mps = 512;
            for ep in d.endpoint_descriptors() {
                if ep.transfer_type() != TransferType::Bulk {
                    continue;
                }
                mps = ep.max_packet_size();
                match ep.direction() {
                    Direction::In => inp = Some(ep.address()),
                    Direction::Out => out = Some(ep.address()),
                }
            }
            if let (Some(i), Some(o)) = (inp, out) {
                return Ok((d.interface_number(), i, o, mps));
            }
        }
    }
    bail!("accessory interface has no bulk endpoints")
}

/// Polls for Android devices; runs a session per accessory.
pub fn spawn(e: Engine) {
    std::thread::Builder::new().name("usb".into()).spawn(move || run(e)).expect("spawn usb");
}

fn run(e: Engine) {
    if !rusb::has_hotplug() {
        debug!("libusb without hotplug; polling");
    }
    let mut tried: HashSet<(u8, u8)> = HashSet::new();
    let mut active: HashSet<(u8, u8)> = HashSet::new();
    let (done_tx, done_rx) = smpsc::channel::<(u8, u8)>();
    loop {
        std::thread::sleep(Duration::from_millis(800));
        while let Ok(k) = done_rx.try_recv() {
            active.remove(&k);
        }
        if !e.config.read().connection.usb_enabled {
            set_status(&e, "USB disabled in settings");
            continue;
        }
        let Ok(list) = GlobalContext::default().devices() else {
            set_status(&e, "libusb unavailable");
            continue;
        };
        let present: HashSet<(u8, u8)> = list.iter().map(|d| (d.bus_number(), d.address())).collect();
        tried.retain(|k| present.contains(k));
        for dev in list.iter() {
            let key = (dev.bus_number(), dev.address());
            if active.contains(&key) {
                continue;
            }
            let Ok(desc) = dev.device_descriptor() else { continue };
            if desc.vendor_id() == AOA_VID && AOA_PIDS.contains(&desc.product_id()) {
                active.insert(key);
                let (e2, done) = (e.clone(), done_tx.clone());
                std::thread::spawn(move || {
                    if let Err(err) = accessory_session(&e2, &dev) {
                        warn!("USB accessory session ended: {err:#}");
                        set_status(&e2, format!("USB session ended: {err:#}"));
                    }
                    let _ = done.send(key);
                });
                continue;
            }
            if tried.contains(&key) || !looks_like_android(&dev) {
                continue;
            }
            tried.insert(key);
            match switch_to_accessory(&dev) {
                Ok(v) => set_status(&e, format!("Android device {:04x}:{:04x} switching to accessory mode (AOA v{v}) — accept the prompt on the device", desc.vendor_id(), desc.product_id())),
                Err(err) => {
                    let msg = format!("{err:#}");
                    if msg.contains("Access") || msg.contains("permission") {
                        set_status(&e, format!("No permission to open USB device {:04x}:{:04x}. Install packaging/linux/70-dread-usb.rules (docs/USB.md).", desc.vendor_id(), desc.product_id()));
                    } else {
                        debug!("{:04x}:{:04x} not an accessory-capable device: {msg}", desc.vendor_id(), desc.product_id());
                    }
                }
            }
        }
        if active.is_empty() && e.usb_status.lock().starts_with("USB session") {
            set_status(&e, "Waiting for a USB device");
        }
    }
}

fn write_all_bulk(h: &DeviceHandle<GlobalContext>, ep: u8, mps: u16, data: &[u8]) -> Result<()> {
    let mut off = 0;
    while off < data.len() {
        let n = h.write_bulk(ep, &data[off..], Duration::from_secs(3)).context("USB write")?;
        off += n;
    }
    // terminate transfers that end on a packet boundary
    if !data.is_empty() && data.len() % mps as usize == 0 {
        let _ = h.write_bulk(ep, &[], Duration::from_millis(500));
    }
    Ok(())
}

/// Bridges one logical USB channel into the session code. Each time a
/// session on this channel ends, a fresh one is started so the device can
/// reconnect without re-plugging. A zero-length frame closes the channel
/// (in both directions).
fn spawn_channel(e: &Engine, ch: u8, peer: String, ctl: smpsc::Sender<Vec<u8>>, vid: smpsc::SyncSender<Vec<u8>>, inbox: Arc<parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>>, alive: Arc<std::sync::atomic::AtomicBool>) {
    let e2 = e.clone();
    e.rt.spawn(async move {
        use std::sync::atomic::Ordering;
        while alive.load(Ordering::SeqCst) {
            let (session_end, bridge_end) = tokio::io::duplex(1 << 20);
            let (mut br, mut bw) = tokio::io::split(bridge_end);
            let (in_tx, mut in_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            *inbox.lock() = Some(in_tx);
            let send = {
                let (ctl, vid) = (ctl.clone(), vid.clone());
                move |chunk: Vec<u8>| -> bool {
                    if ch == CH_CONTROL {
                        ctl.send(chunk).is_ok()
                    } else {
                        tokio::task::block_in_place(|| vid.send(chunk).is_ok())
                    }
                }
            };
            let send2 = send.clone();
            // session → USB
            let out_task = tokio::spawn(async move {
                let mut buf = vec![0u8; 256 * 1024];
                loop {
                    let n = match br.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if !send2(buf[..n].to_vec()) {
                        break;
                    }
                }
            });
            // USB → session
            let in_task = tokio::spawn(async move {
                while let Some(d) = in_rx.recv().await {
                    if d.is_empty() || bw.write_all(&d).await.is_err() {
                        break;
                    }
                }
                let _ = bw.shutdown().await;
            });
            session::handle(e2.clone(), session_end, ConnInfo { peer: format!("{peer}/{ch}"), transport: TransportKind::Usb, socket_fd: None }).await;
            *inbox.lock() = None;
            in_task.abort();
            let _ = out_task.await;
            // tell the device this channel closed
            send(Vec::new());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}

fn accessory_session(e: &Engine, dev: &Device<GlobalContext>) -> Result<()> {
    let (iface, ep_in, ep_out, mps) = bulk_endpoints(dev)?;
    let h = dev.open().context("opening accessory")?;
    h.claim_interface(iface).context("claiming accessory interface")?;
    let h = Arc::new(h);
    set_status(e, format!("USB accessory connected (bus {} device {})", dev.bus_number(), dev.address()));
    let peer = format!("usb:{}-{}", dev.bus_number(), dev.address());

    // writer: control has priority over video; empty payload = channel close
    let (ctl_tx, ctl_rx) = smpsc::channel::<Vec<u8>>();
    let (vid_tx, vid_rx) = smpsc::sync_channel::<Vec<u8>>(8);
    let hw = h.clone();
    let writer = std::thread::spawn(move || -> Result<()> {
        loop {
            let (ch, data) = match ctl_rx.try_recv() {
                Ok(d) => (CH_CONTROL, d),
                Err(smpsc::TryRecvError::Disconnected) => return Ok(()),
                Err(smpsc::TryRecvError::Empty) => match vid_rx.recv_timeout(Duration::from_millis(2)) {
                    Ok(d) => (CH_VIDEO, d),
                    Err(smpsc::RecvTimeoutError::Timeout) => continue,
                    Err(smpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                },
            };
            let mut pkt = Vec::with_capacity(5 + data.len());
            pkt.push(ch);
            pkt.extend_from_slice(&(data.len() as u32).to_be_bytes());
            pkt.extend_from_slice(&data);
            write_all_bulk(&hw, ep_out, mps, &pkt)?;
        }
    });

    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let inboxes: Vec<Arc<parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>>> = (0..2).map(|_| Arc::default()).collect();
    for ch in [CH_CONTROL, CH_VIDEO] {
        spawn_channel(e, ch, peer.clone(), ctl_tx.clone(), vid_tx.clone(), inboxes[ch as usize].clone(), alive.clone());
    }
    drop(ctl_tx);
    drop(vid_tx);

    // reader: demultiplex
    let mut acc: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; READ_BUF];
    let res = loop {
        match h.read_bulk(ep_in, &mut buf, Duration::from_millis(200)) {
            Ok(n) => acc.extend_from_slice(&buf[..n]),
            Err(rusb::Error::Timeout) => {}
            Err(err) => break Err(anyhow::anyhow!("USB read: {err}")),
        }
        while acc.len() >= 5 {
            let ch = acc[0];
            let len = u32::from_be_bytes(acc[1..5].try_into().unwrap()) as usize;
            if len > dread_proto::MAX_FRAME + 64 {
                acc.clear();
                break;
            }
            if acc.len() < 5 + len {
                break;
            }
            let data = acc[5..5 + len].to_vec();
            acc.drain(..5 + len);
            if let Some(inbox) = inboxes.get(ch as usize) {
                if let Some(tx) = inbox.lock().as_ref() {
                    let _ = tx.send(data);
                }
            }
        }
        if writer.is_finished() {
            break Err(anyhow::anyhow!("USB writer stopped"));
        }
    };
    alive.store(false, std::sync::atomic::Ordering::SeqCst);
    for inbox in &inboxes {
        if let Some(tx) = inbox.lock().take() {
            let _ = tx.send(Vec::new());
        }
    }
    let _ = h.release_interface(iface);
    res
}
