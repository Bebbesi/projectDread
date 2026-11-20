//! Hardware smoke test: virtual display -> screencopy -> encoder -> file.
//! cargo run -p dread-platform --example smoke -- <seconds> <out.h264> [vaapi|software]
use dread_platform::{capture::wlr::WlrCapture, display, encode::*};
use std::io::Write;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let secs: u64 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let out_path = a.get(2).cloned().unwrap_or("/tmp/dread-smoke.h264".into());
    let backend = if a.get(3).map(String::as_str) == Some("software") { Backend::Software } else { Backend::Vaapi };
    let mut be = display::detect_backend()?;
    let s = display::DisplaySettings { width: 1280, height: 800, ..Default::default() };
    let d = be.activate(&s)?;
    println!("display {:?}", d.output);
    let res = (|| -> anyhow::Result<()> {
        let mut cap = WlrCapture::new(&d.output.name, true)?;
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        let mut enc: Option<Encoder> = None;
        let mut file = std::fs::File::create(&out_path)?;
        let t0 = Instant::now();
        let (mut nin, mut nout, mut bytes, mut lat) = (0u32, 0u32, 0usize, 0u64);
        let mut force = true;
        while t0.elapsed() < Duration::from_secs(secs) {
            if let Some(f) = cap.next_frame(force, Duration::from_millis(50))? {
                force = false;
                if enc.is_none() {
                    enc = Some(Encoder::start(EncoderConfig { ffmpeg: "ffmpeg".into(), codec: Codec::H264, backend, width: f.width, height: f.height, fps: 60, bitrate_kbps: 8000, keyframe_interval_s: 2.0, quality: Quality::Speed, low_latency: true, input: EncoderInput::Raw { format: f.format }, vaapi_device: "/dev/dri/renderD128".into(), scale_to: None }, tx.clone())?);
                }
                enc.as_mut().unwrap().encode(&f)?;
                nin += 1;
            }
            while let Ok(e) = rx.try_recv() { nout += 1; bytes += e.data.len(); lat += e.encode_us as u64; if nout > 5 { print!("{} ", e.encode_us/1000); } if nout <= 2 || e.key { println!("AU {} bytes key={} cfg={} enc={}us", e.data.len(), e.key, e.config, e.encode_us); } file.write_all(&e.data)?; }
        }
        std::thread::sleep(Duration::from_millis(300));
        while let Ok(e) = rx.try_recv() { nout += 1; bytes += e.data.len(); file.write_all(&e.data)?; }
        println!("frames in={nin} out={nout} bytes={bytes} avg_encode={}us", if nout>0 { lat / nout as u64 } else { 0 });
        Ok(())
    })();
    be.deactivate(&d)?;
    res
}
