//! Per-connection protocol handling, identical for TLS/TCP and USB links.

use crate::engine::{Client, ClientCaps, Engine, InputCmd, PendingPair, TransportKind};
use crate::security::random_hex;
use anyhow::{bail, Context, Result};
use dread_proto::{msg_type, pairing_code, read_json, read_message, sha256, write_json, Message, PROTO_VERSION};
use rand::RngCore;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tracing::{debug, info, warn};

pub struct ConnInfo {
    pub peer: String,
    pub transport: TransportKind,
    /// Raw socket for TCP statistics (Linux `TCP_INFO`).
    pub socket_fd: Option<i32>,
}

pub async fn handle<S>(e: Engine, stream: S, info: ConnInfo)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let peer = info.peer.clone();
    if let Err(err) = handle_inner(e, stream, info).await {
        debug!(%peer, "connection ended: {err:#}");
    }
}

async fn handle_inner<S>(e: Engine, stream: S, info: ConnInfo) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut rd, mut wr) = tokio::io::split(stream);
    // over USB the channel exists before the client is ready to speak on it
    let wait = if info.transport == TransportKind::Usb { Duration::from_secs(24 * 3600) } else { Duration::from_secs(10) };
    let hello = timeout(wait, read_json(&mut rd)).await.context("hello timeout")??;
    if msg_type(&hello) != "hello" {
        bail!("expected hello");
    }
    let proto = hello["proto"].as_u64().unwrap_or(0) as u32;
    if proto != PROTO_VERSION {
        write_json(&mut wr, &json!({"t":"bye","reason":format!("protocol version {proto} not supported (server speaks {PROTO_VERSION}); update DREAD")})).await?;
        bail!("protocol mismatch");
    }
    match hello["role"].as_str() {
        Some("control") => control(e, rd, wr, hello, info).await,
        Some("video") => video(e, rd, wr, hello, info).await,
        _ => bail!("unknown role"),
    }
}

fn caps_from(v: &Value) -> ClientCaps {
    let u = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
    ClientCaps {
        width: u("width"),
        height: u("height"),
        density_dpi: u("density_dpi"),
        max_fps: u("max_fps"),
        codecs: v.get("codecs").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_else(|| vec!["h264".into()]),
        touch_points: u("touch_points"),
        stylus: v.get("stylus").and_then(Value::as_bool).unwrap_or(false),
        preferred_width: u("preferred_width"),
        preferred_height: u("preferred_height"),
    }
}

/// Runs a pairing flow. Returns the new device token on success.
async fn pair<R, W>(e: &Engine, rd: &mut R, wr: &mut W, device_id: &str, device_name: &str, info: &ConnInfo) -> Result<Option<String>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    write_json(
        wr,
        &json!({
            "t": "pair_required",
            "methods": ["qr", "sas"],
            "server_name": e.config.read().general.server_name,
            "fp": e.identity.fingerprint_hex(),
        }),
    )
    .await?;
    let first = timeout(Duration::from_secs(120), read_json(rd)).await.context("pairing timeout")??;
    match msg_type(&first) {
        "pair_qr" => {
            let secret = first["secret"].as_str().unwrap_or("");
            let ok = e.qr.lock().matches(secret);
            if !ok {
                write_json(wr, &json!({"t":"pair_rejected","reason":"QR code expired or invalid — refresh it on the computer"})).await?;
                return Ok(None);
            }
            e.rotate_qr();
            info!(device = device_name, "paired via QR code");
            Ok(Some(e.devices.lock().pair(device_id, device_name)?))
        }
        "pair_commit" => {
            let commit = hex::decode(first["commit"].as_str().unwrap_or("")).unwrap_or_default();
            let mut ns = [0u8; 32];
            rand::rng().fill_bytes(&mut ns);
            write_json(wr, &json!({"t":"pair_nonce","ns":hex::encode(ns)})).await?;
            let reveal = timeout(Duration::from_secs(30), read_json(rd)).await.context("pairing timeout")??;
            let nc = hex::decode(reveal["nc"].as_str().unwrap_or("")).unwrap_or_default();
            if msg_type(&reveal) != "pair_reveal" || nc.len() != 32 || sha256(&nc).as_slice() != commit.as_slice() {
                write_json(wr, &json!({"t":"pair_rejected","reason":"pairing protocol violation"})).await?;
                bail!("bad pairing reveal");
            }
            let code = pairing_code(&e.identity.fingerprint, &nc, &ns);
            let (tx, rx) = oneshot::channel();
            let id = e.next_id();
            e.pending.lock().push(PendingPair {
                id,
                device_id: device_id.into(),
                device_name: device_name.into(),
                addr: info.peer.clone(),
                code: code.clone(),
                created: Instant::now(),
                decision: Some(tx),
            });
            info!(device = device_name, %code, "pairing request waiting for approval");
            // wait for the user's decision, or the client giving up
            let decision = tokio::select! {
                d = timeout(Duration::from_secs(120), rx) => matches!(d, Ok(Ok(true))),
                _ = read_json(rd) => false,
            };
            e.pending.lock().retain(|p| p.id != id);
            if !decision {
                let _ = write_json(wr, &json!({"t":"pair_rejected","reason":"pairing was declined on the computer"})).await;
                return Ok(None);
            }
            info!(device = device_name, "paired via code comparison");
            Ok(Some(e.devices.lock().pair(device_id, device_name)?))
        }
        _ => bail!("unexpected pairing message"),
    }
}

async fn control<S>(e: Engine, mut rd: ReadHalf<S>, mut wr: WriteHalf<S>, hello: Value, info: ConnInfo) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let device_id: String = hello["device_id"].as_str().unwrap_or("").chars().take(64).collect();
    let device_name: String = hello["device_name"].as_str().unwrap_or("Android device").chars().take(64).collect();
    if device_id.is_empty() {
        bail!("hello without device_id");
    }
    let token = hello["token"].as_str().unwrap_or("");
    let mut authed = !token.is_empty() && e.devices.lock().verify(&device_id, token);
    if !authed {
        match pair(&e, &mut rd, &mut wr, &device_id, &device_name, &info).await? {
            Some(tok) => {
                write_json(&mut wr, &json!({"t":"paired","token":tok})).await?;
                authed = true;
            }
            None => return Ok(()),
        }
    }
    debug_assert!(authed);
    let session = random_hex(16);
    let video_key = random_hex(32);
    let cfg = e.config();
    write_json(
        &mut wr,
        &json!({
            "t": "welcome",
            "session": session,
            "video_key": video_key,
            "server_name": cfg.general.server_name,
            "server_id": cfg.general.server_id,
            "fp": e.identity.fingerprint_hex(),
            "transport": info.transport.label(),
        }),
    )
    .await?;

    let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let id = e.next_id();
    e.register_client(Client {
        id,
        device_id: device_id.clone(),
        name: device_name.clone(),
        addr: info.peer.clone(),
        transport: info.transport,
        caps: caps_from(&hello["client"]),
        session,
        video_key,
        connected_at: Instant::now(),
        ctl_tx: ctl_tx.clone(),
        video_tx: None,
        flow: Default::default(),
        viewport: None,
        tcp_retrans: None,
    });

    // writer: control messages + periodic stats for the client overlay
    let e_w = e.clone();
    let writer = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                m = ctl_rx.recv() => {
                    let Some(m) = m else { break };
                    let bye = m.len() > 5 && serde_json::from_slice::<Value>(&m[5..]).map(|v| msg_type(&v) == "bye").unwrap_or(false);
                    if timeout(Duration::from_secs(5), wr.write_all(&m)).await.map(|r| r.is_err()).unwrap_or(true) { break; }
                    let _ = wr.flush().await;
                    if bye { break; }
                }
                _ = tick.tick() => {
                    let st = e_w.stats.lock().clone();
                    let v = json!({"t":"stats","fps":st.fps,"bitrate_kbps":st.bitrate_kbps,"encode_ms":st.encode_ms,
                        "encoder":st.encoder,"codec":st.codec,"width":st.width,"height":st.height,"loss_pct":st.packet_loss_pct,
                        "target_kbps":st.target_bitrate_kbps});
                    if wr.write_all(&dread_proto::encode_json(&v)).await.is_err() { break; }
                    let _ = wr.flush().await;
                }
            }
        }
        let _ = wr.shutdown().await;
    });

    let idle = Duration::from_secs(cfg.connection.timeout_s.max(2) as u64);
    let result: Result<()> = async {
        loop {
            let msg = match timeout(idle, read_message(&mut rd)).await {
                Err(_) => bail!("heartbeat timeout"),
                Ok(r) => r?,
            };
            match msg {
                Message::Input(ev) => {
                    let _ = e.input_tx.send(InputCmd::Event(ev));
                }
                Message::Json(v) => match msg_type(&v) {
                    "ping" => {
                        let server_us = dread_platform::capture::now_us();
                        let _ = ctl_tx.send(dread_proto::encode_json(&json!({"t":"pong","ts":v["ts"],"server_us":server_us})));
                        if let Some(r) = v["rtt_ms"].as_f64() {
                            e.on_rtt(id, r);
                        }
                    }
                    "ack" => e.on_ack(id, v["gen"].as_u64().unwrap_or(0), v["seq"].as_u64().unwrap_or(0) as u32, v["decode_us"].as_u64().unwrap_or(0)),
                    "request_keyframe" => e.request_keyframe(),
                    "viewport" => e.client_viewport(id, v["width"].as_u64().unwrap_or(0) as u32, v["height"].as_u64().unwrap_or(0) as u32),
                    "bye" => return Ok(()),
                    other => debug!("ignoring control message {other}"),
                },
                Message::Video(_) => bail!("client sent video on control channel"),
            }
        }
    }
    .await;
    e.unregister_client(id);
    writer.abort();
    result
}

async fn video<S>(e: Engine, mut rd: ReadHalf<S>, mut wr: WriteHalf<S>, hello: Value, info: ConnInfo) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let session = hello["session"].as_str().unwrap_or("");
    let key = hello["video_key"].as_str().unwrap_or("");
    let (tx, mut rx) = mpsc::channel::<Arc<Vec<u8>>>(6);
    let Some(id) = e.attach_video(session, key, tx) else {
        write_json(&mut wr, &json!({"t":"bye","reason":"unknown video session"})).await?;
        bail!("video channel with unknown session");
    };
    write_json(&mut wr, &json!({"t":"video_ok"})).await?;
    info!(peer = %info.peer, "video channel attached");
    // the client never sends on this channel: a read completing means it closed
    let mut closed = tokio::spawn(async move {
        let mut b = [0u8; 64];
        loop {
            match tokio::io::AsyncReadExt::read(&mut rd, &mut b).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });
    let mut last_info = Instant::now();
    let result: Result<()> = async {
        loop {
            tokio::select! {
                f = rx.recv() => {
                    let Some(f) = f else { return Ok(()) };
                    match timeout(Duration::from_secs(5), wr.write_all(&f)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => return Err(err.into()),
                        Err(_) => bail!("video write stalled"),
                    }
                    wr.flush().await?;
                    if last_info.elapsed() > Duration::from_secs(1) {
                        last_info = Instant::now();
                        if let Some(fd) = info.socket_fd {
                            if let Some(r) = crate::net::tcp_retrans(fd) {
                                if let Some(c) = e.clients.lock().iter_mut().find(|c| c.id == id) { c.tcp_retrans = Some(r); }
                            }
                        }
                    }
                }
                _ = &mut closed => return Ok(()),
            }
        }
    }
    .await;
    e.detach_video(id);
    if let Err(err) = &result {
        warn!(peer = %info.peer, "video channel closed: {err:#}");
    }
    result
}
