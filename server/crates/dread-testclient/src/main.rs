//! Headless DREAD client for end-to-end verification.
//!
//! ```text
//! dread-testclient --host 127.0.0.1 [--port 47800] [--qr 'dread://pair?...'] [--sas]
//!                  [--seconds 5] [--out video.h264] [--input-test] [--state client.json]
//! ```
//! Pairs (QR secret or code comparison), opens control + video channels,
//! acknowledges frames, measures RTT and optionally injects a touch, pen and
//! text sequence. Prints a JSON summary at the end.

use anyhow::{bail, Context, Result};
use dread_proto::*;
use rand::RngCore;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

#[derive(Debug)]
struct Pinning {
    expected: Option<[u8; 32]>,
    seen: Mutex<Option<[u8; 32]>>,
}

impl ServerCertVerifier for Pinning {
    fn verify_server_cert(&self, end: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let fp = sha256(end);
        *self.seen.lock().unwrap() = Some(fp);
        match self.expected {
            Some(e) if e != fp => Err(rustls::Error::General("server certificate fingerprint mismatch".into())),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

type Rd = ReadHalf<TlsStream<TcpStream>>;
type Wr = WriteHalf<TlsStream<TcpStream>>;

async fn connect(host: &str, port: u16, pin: Option<[u8; 32]>) -> Result<(Rd, Wr, [u8; 32])> {
    let verifier = Arc::new(Pinning { expected: pin, seen: Mutex::new(None) });
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let tcp = TcpStream::connect((host, port)).await.context("tcp connect")?;
    tcp.set_nodelay(true)?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(ServerName::try_from("dread.local")?, tcp).await.context("tls")?;
    let fp = verifier.seen.lock().unwrap().context("no cert")?;
    let (r, w) = tokio::io::split(tls);
    Ok((r, w, fp))
}

fn arg(a: &[String], k: &str) -> Option<String> {
    a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned()
}

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let host = arg(&a, "--host").unwrap_or("127.0.0.1".into());
    let mut port: u16 = arg(&a, "--port").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    let secs: u64 = arg(&a, "--seconds").and_then(|p| p.parse().ok()).unwrap_or(5);
    let state_path = arg(&a, "--state").unwrap_or("/tmp/dread-testclient.json".into());
    let out_path = arg(&a, "--out");
    let input_test = a.iter().any(|x| x == "--input-test");
    // --probe pen|touch|mouse : place the tool at x,y points (normalized) given by --points "x,y;x,y"
    let probe = arg(&a, "--probe");
    let points: Vec<(u16, u16)> = arg(&a, "--points")
        .unwrap_or_default()
        .split(';')
        .filter_map(|p| p.split_once(',').and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?))))
        .collect();
    let viewport = arg(&a, "--viewport");
    let qr = arg(&a, "--qr");

    let mut state: Value = std::fs::read(&state_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    if state["device_id"].is_null() {
        let mut b = [0u8; 16];
        rand::rng().fill_bytes(&mut b);
        state["device_id"] = json!(hex::encode(b));
    }
    let mut pin: Option<[u8; 32]> = state["fp"].as_str().and_then(|s| hex::decode(s).ok()).and_then(|v| v.try_into().ok());
    let mut qr_secret = None;
    if let Some(uri) = &qr {
        let q = uri.split_once('?').map(|x| x.1).unwrap_or("");
        for kv in q.split('&') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            match k {
                "fp" => pin = hex::decode(v).ok().and_then(|v| v.try_into().ok()),
                "s" => qr_secret = Some(v.to_string()),
                "p" => port = v.parse().unwrap_or(port),
                _ => {}
            }
        }
    }

    let (mut rd, mut wr, fp) = connect(&host, port, pin).await?;
    write_json(
        &mut wr,
        &json!({"t":"hello","proto":PROTO_VERSION,"role":"control","device_id":state["device_id"],"device_name":"dread-testclient",
            "token":state["token"],"client":{"width":1280,"height":800,"density_dpi":160,"max_fps":60,"codecs":["h264","hevc","av1"],"touch_points":10,"stylus":true}}),
    )
    .await?;
    let mut welcome = read_json(&mut rd).await?;
    if msg_type(&welcome) == "pair_required" {
        if let Some(s) = qr_secret {
            write_json(&mut wr, &json!({"t":"pair_qr","secret":s})).await?;
        } else {
            let mut nc = [0u8; 32];
            rand::rng().fill_bytes(&mut nc);
            write_json(&mut wr, &json!({"t":"pair_commit","commit":hex::encode(sha256(&nc))})).await?;
            let n = read_json(&mut rd).await?;
            let ns = hex::decode(n["ns"].as_str().unwrap_or(""))?;
            write_json(&mut wr, &json!({"t":"pair_reveal","nc":hex::encode(nc)})).await?;
            println!("PAIRING CODE {} — approve it on the computer", pairing_code(&fp, &nc, &ns));
        }
        let r = read_json(&mut rd).await?;
        if msg_type(&r) != "paired" {
            bail!("pairing failed: {r}");
        }
        state["token"] = r["token"].clone();
        state["fp"] = json!(hex::encode(fp));
        std::fs::write(&state_path, serde_json::to_vec_pretty(&state)?)?;
        welcome = read_json(&mut rd).await?;
    }
    if msg_type(&welcome) != "welcome" {
        bail!("not welcomed: {welcome}");
    }
    println!("welcome from {}", welcome["server_name"]);

    // video channel
    let (mut vrd, mut vwr, _) = connect(&host, port, Some(fp)).await?;
    write_json(&mut vwr, &json!({"t":"hello","proto":PROTO_VERSION,"role":"video","session":welcome["session"],"video_key":welcome["video_key"]})).await?;
    let ok = read_json(&mut vrd).await?;
    if msg_type(&ok) != "video_ok" {
        bail!("video rejected: {ok}");
    }

    let (ctl_tx, mut ctl_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(m) = ctl_rx.recv().await {
            if wr.write_all(&m).await.is_err() {
                break;
            }
            let _ = wr.flush().await;
        }
    });
    if let Some(v) = viewport {
        let (w, h) = v.split_once('x').context("viewport WxH")?;
        let _ = ctl_tx.send(encode_json(&json!({"t":"viewport","width":w.parse::<u32>()?,"height":h.parse::<u32>()?})));
    }
    let rtt = Arc::new(Mutex::new(Vec::<f64>::new()));
    let ime_events = Arc::new(Mutex::new(Vec::<bool>::new()));
    let start = Instant::now();
    {
        let rtt = rtt.clone();
        let ime = ime_events.clone();
        tokio::spawn(async move {
            while let Ok(m) = read_message(&mut rd).await {
                if let Message::Json(v) = m {
                    match msg_type(&v) {
                        "pong" => {
                            let sent = v["ts"].as_f64().unwrap_or(0.0);
                            rtt.lock().unwrap().push(start.elapsed().as_secs_f64() * 1000.0 - sent);
                        }
                        "ime" => ime.lock().unwrap().push(v["show"].as_bool().unwrap_or(false)),
                        "bye" => {
                            println!("server said bye: {}", v["reason"]);
                            break;
                        }
                        _ => {}
                    }
                }
            }
        });
    }
    {
        let tx = ctl_tx.clone();
        let rtt = rtt.clone();
        tokio::spawn(async move {
            loop {
                let last = rtt.lock().unwrap().last().copied().unwrap_or(0.0);
                let ts = start.elapsed().as_secs_f64() * 1000.0;
                if tx.send(encode_json(&json!({"t":"ping","ts":ts,"rtt_ms":last}))).is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }
    if let Some(kind) = probe.clone() {
        let tx = ctl_tx.clone();
        let pts = points.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1200)).await;
            for (x, y) in pts {
                let ev = match kind.as_str() {
                    "touch" => InputEvent::Touch { action: TouchAction::Down, changed: 0, contacts: vec![Contact { id: 0, x, y, pressure: 30000, major: 500 }] },
                    "mouse" => InputEvent::MouseAbs { x, y, buttons: 0 },
                    _ => InputEvent::Pen { tool: PenTool::Pen, flags: PEN_IN_RANGE, x, y, pressure: 0, tilt_x: 0, tilt_y: 0 },
                };
                let _ = tx.send(ev.to_wire());
                println!("PROBE {x},{y}");
                tokio::time::sleep(Duration::from_millis(700)).await;
                if kind == "touch" {
                    let _ = tx.send(InputEvent::Touch { action: TouchAction::Up, changed: 0, contacts: vec![Contact { id: 0, x, y, pressure: 0, major: 0 }] }.to_wire());
                }
            }
            let _ = tx.send(InputEvent::Pen { tool: PenTool::Pen, flags: 0, x: 0, y: 0, pressure: 0, tilt_x: 0, tilt_y: 0 }.to_wire());
        });
    }
    if input_test {
        let tx = ctl_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            // pen stroke with rising pressure and tilt
            for i in 0..=40u16 {
                let flags = PEN_IN_RANGE | if i > 2 && i < 38 { PEN_CONTACT } else { 0 };
                let ev = InputEvent::Pen { tool: PenTool::Pen, flags, x: 20000 + i * 500, y: 30000, pressure: i * 1500, tilt_x: 1500, tilt_y: -800 };
                let _ = tx.send(ev.to_wire());
                tokio::time::sleep(Duration::from_millis(8)).await;
            }
            let _ = tx.send(InputEvent::Pen { tool: PenTool::Pen, flags: 0, x: 0, y: 0, pressure: 0, tilt_x: 0, tilt_y: 0 }.to_wire());
            // two-finger touch
            let c = |id, x, y| Contact { id, x, y, pressure: 30000, major: 600 };
            let _ = tx.send(InputEvent::Touch { action: TouchAction::Down, changed: 0, contacts: vec![c(0, 30000, 30000)] }.to_wire());
            let _ = tx.send(InputEvent::Touch { action: TouchAction::Down, changed: 1, contacts: vec![c(0, 30000, 30000), c(1, 40000, 30000)] }.to_wire());
            for k in 0..10u16 {
                let _ = tx.send(InputEvent::Touch { action: TouchAction::Move, changed: 0, contacts: vec![c(0, 30000 - k * 300, 30000), c(1, 40000 + k * 300, 30000)] }.to_wire());
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let _ = tx.send(InputEvent::Touch { action: TouchAction::Up, changed: 1, contacts: vec![c(0, 27000, 30000), c(1, 43000, 30000)] }.to_wire());
            let _ = tx.send(InputEvent::Touch { action: TouchAction::Up, changed: 0, contacts: vec![c(0, 27000, 30000)] }.to_wire());
        });
    }

    let mut out = match &out_path {
        Some(p) => Some(std::fs::File::create(p)?),
        None => None,
    };
    let (mut frames, mut bytes, mut keys, mut configs) = (0u64, 0u64, 0u64, Vec::<Value>::new());
    let mut gen = 0u64;
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut first_frame_ms = None;
    while Instant::now() < deadline {
        let m = match tokio::time::timeout(deadline - Instant::now(), read_message(&mut vrd)).await {
            Ok(r) => r?,
            Err(_) => break,
        };
        match m {
            Message::Json(v) if msg_type(&v) == "stream_config" => {
                gen = v["generation"].as_u64().unwrap_or(0);
                configs.push(v);
            }
            Message::Video(f) => {
                frames += 1;
                bytes += f.data.len() as u64;
                if f.flags & VIDEO_FLAG_KEY != 0 {
                    keys += 1;
                }
                first_frame_ms.get_or_insert(start.elapsed().as_millis());
                if let Some(o) = out.as_mut() {
                    std::io::Write::write_all(o, &f.data)?;
                }
                let _ = ctl_tx.send(encode_json(&json!({"t":"ack","seq":f.seq,"gen":gen,"decode_us":1000})));
            }
            _ => {}
        }
    }
    let _ = ctl_tx.send(encode_json(&json!({"t":"bye"})));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let r = rtt.lock().unwrap().clone();
    let avg_rtt = if r.is_empty() { 0.0 } else { r.iter().sum::<f64>() / r.len() as f64 };
    println!(
        "{}",
        json!({"frames":frames,"bytes":bytes,"key_frames":keys,"stream_configs":configs,"first_frame_ms":first_frame_ms,
               "avg_rtt_ms":avg_rtt,"ime_events":*ime_events.lock().unwrap()})
    );
    Ok(())
}
