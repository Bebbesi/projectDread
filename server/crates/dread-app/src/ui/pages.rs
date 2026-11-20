//! Control panel pages.

use super::theme::*;
use super::App;
use dread_core::config::Config;
use dread_core::engine::Stats;
use dread_platform::display::{DisplayMode, Orientation, Placement, STANDARD_MODES};
use dread_platform::encode::{Backend, Codec, Quality};
use egui::RichText;

const BITRATES: &[u32] = &[2_000, 4_000, 8_000, 12_000, 16_000, 25_000, 40_000];
const FPS: &[u32] = &[15, 24, 30, 45, 60, 90, 120];
const SCALES: &[f32] = &[0.0, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

fn two_cols<T>(ui: &mut egui::Ui, st: &mut T, left: impl FnOnce(&mut egui::Ui, &mut T), right: impl FnOnce(&mut egui::Ui, &mut T)) {
    ui.columns(2, |c| {
        let (a, b) = c.split_at_mut(1);
        left(&mut a[0], st);
        right(&mut b[0], st);
    });
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    // keep the grid inside its column: long values are truncated, not widened
    let w = ui.available_width();
    egui::Grid::new(id).num_columns(2).spacing([16.0, 8.0]).min_col_width(140.0).max_col_width((w - 156.0).max(140.0)).show(ui, add);
}

fn mbps(kbps: f64) -> String {
    format!("{:.1} Mbps", kbps / 1000.0)
}

// ------------------------------------------------------------------ dashboard

pub fn dashboard(app: &mut App, ui: &mut egui::Ui, st: &Stats) {
    let e = app.engine.clone();
    let clients = e.client_views();
    let display = e.active_display();
    let enabled = e.is_streaming_enabled();

    ui.horizontal(|ui| {
        let label = if enabled { "Stop streaming" } else { "Start streaming" };
        if primary_button(ui, label).clicked() {
            e.set_streaming(!enabled);
        }
        ui.add_space(12.0);
        let status = if st.streaming {
            format!("Streaming to {} device{}", clients.iter().filter(|c| c.streaming).count(), if clients.len() == 1 { "" } else { "s" })
        } else if !enabled {
            "Streaming stopped — clients may connect but no display is created".into()
        } else if clients.is_empty() {
            "Waiting for a device. Open DREAD on Android and connect, or scan the QR code on the Connection page.".into()
        } else {
            "Starting…".into()
        };
        ui.label(RichText::new(status).color(TEXT2));
    });
    ui.add_space(14.0);

    two_cols(
        ui,
        app,
        |ui, _app| {
            section(ui, "Display", |ui| {
                grid(ui, "dash_disp", |ui| match &display {
                    Some(d) => {
                        kv(ui, "Mode", if d.owned { "Extended (virtual output)" } else { "Duplicate" });
                        kv(ui, "Output", d.output.name.clone());
                        kv(ui, "Resolution", format!("{} × {} @ {:.0} Hz", d.output.width, d.output.height, d.output.refresh));
                        kv(ui, "Position", format!("{}, {}", d.output.x, d.output.y));
                        kv(ui, "Scale", format!("{:.2}", d.output.scale));
                    }
                    None => {
                        kv(ui, "Mode", match e.config.read().display.mode {
                            DisplayMode::Extend => "Extended (created when a device streams)",
                            DisplayMode::Duplicate => "Duplicate",
                        });
                        kv(ui, "Output", "—");
                    }
                });
                ui.add_space(4.0);
                let ds = e.driver_status();
                ui.horizontal(|ui| {
                    status_dot(ui, ds.ok);
                    ui.label(caption(&ds.backend));
                });
            });
            ui.add_space(10.0);
            section(ui, "Network", |ui| {
                let cfg = e.config.read().clone();
                grid(ui, "dash_net", |ui| {
                    let addrs: Vec<String> = dread_core::net::lan_addresses().iter().map(|a| a.to_string()).collect();
                    kv(ui, "Addresses", if addrs.is_empty() { "no LAN address".into() } else { addrs.join(", ") });
                    kv(ui, "Port", cfg.connection.port.to_string());
                    kv(ui, "Wi-Fi / LAN", if cfg.connection.wifi_enabled { "enabled" } else { "disabled" });
                    kv(ui, "USB", e.usb_status.lock().clone());
                    kv(ui, "Latency (RTT)", if st.streaming { format!("{:.1} ms", st.rtt_ms) } else { "—".into() });
                });
            });
        },
        |ui, app| {
            section(ui, "Stream", |ui| {
                grid(ui, "dash_stream", |ui| {
                    if st.streaming {
                        kv(ui, "Resolution", format!("{} × {}", st.width, st.height));
                        kv(ui, "Frame rate", format!("{:.1} fps (max {})", st.fps, st.target_fps));
                        kv(ui, "Bitrate", format!("{} (target {})", mbps(st.bitrate_kbps), mbps(st.target_bitrate_kbps as f64)));
                        kv(ui, "Codec", st.codec.clone());
                        kv(ui, "Encoder", st.encoder.clone());
                        kv(ui, "Encode latency", format!("{:.1} ms", st.encode_ms));
                    } else {
                        kv(ui, "State", "idle");
                    }
                });
            });
            ui.add_space(10.0);
            section(ui, "Activity", |ui| {
                sparkline(ui, &app.history.fps.iter().cloned().collect::<Vec<_>>(), 48.0, "fps");
                ui.add_space(4.0);
                sparkline(ui, &app.history.bitrate.iter().cloned().collect::<Vec<_>>(), 48.0, "Mbps");
            });
        },
    );
    ui.add_space(10.0);
    section(ui, "Connected devices", |ui| {
        if clients.is_empty() {
            ui.label(caption("No device connected."));
            return;
        }
        egui::Grid::new("clients").num_columns(6).spacing([22.0, 8.0]).striped(true).show(ui, |ui| {
            for h in ["Device", "Address", "Link", "Screen", "Latency", ""] {
                ui.label(caption(h));
            }
            ui.end_row();
            for c in &clients {
                ui.horizontal(|ui| {
                    status_dot(ui, c.streaming);
                    ui.label(value(c.name.clone()));
                });
                ui.label(value(c.addr.clone()));
                ui.label(value(c.transport.label()));
                ui.label(value(format!("{} × {}", c.width, c.height)));
                ui.label(value(format!("{:.1} ms", c.rtt_ms)));
                if ui.small_button("Disconnect").clicked() {
                    e.disconnect(c.id, "disconnected from the computer");
                }
                ui.end_row();
            }
        });
    });
}

// ------------------------------------------------------------------ display

fn layout_preview(ui: &mut egui::Ui, outputs: &[dread_platform::display::OutputInfo]) {
    let h = 170.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, BG);
    p.rect_stroke(rect, 0.0, egui::Stroke::new(1.0_f32, BORDER), egui::StrokeKind::Inside);
    if outputs.is_empty() {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "No outputs reported", egui::FontId::proportional(12.0), TEXT2);
        return;
    }
    let (bx, by, bw, bh) = dread_platform::display::layout_box(outputs);
    let k = ((rect.width() - 40.0) as f64 / bw).min((rect.height() - 30.0) as f64 / bh);
    let off = rect.center() - egui::vec2((bw * k) as f32 / 2.0, (bh * k) as f32 / 2.0);
    for o in outputs {
        let (w, hh) = o.logical_size();
        let r = egui::Rect::from_min_size(off + egui::vec2(((o.x as f64 - bx) * k) as f32, ((o.y as f64 - by) * k) as f32), egui::vec2((w * k) as f32, (hh * k) as f32)).shrink(2.0);
        if o.is_dread {
            p.rect_filled(r, 0.0, LIGHT);
            p.text(r.center(), egui::Align2::CENTER_CENTER, format!("{}\n{}×{}", o.name, o.width, o.height), egui::FontId::proportional(11.0), BG);
        } else {
            p.rect_filled(r, 0.0, PANEL);
            p.rect_stroke(r, 0.0, egui::Stroke::new(1.0_f32, MID), egui::StrokeKind::Inside);
            p.text(r.center(), egui::Align2::CENTER_CENTER, format!("{}\n{}×{}", o.name, o.width, o.height), egui::FontId::proportional(11.0), TEXT2);
        }
    }
}

pub fn display(app: &mut App, ui: &mut egui::Ui) {
    let e = app.engine.clone();
    let outputs = e.outputs();
    let (modes, rates, max) = e.virtual_modes();
    let encoder_max = e
        .caps
        .read()
        .as_ref()
        .and_then(|c| c.iter().map(|x| (x.max_width, x.max_height)).max())
        .unwrap_or((1920, 1080));

    section(ui, "Layout", |ui| {
        layout_preview(ui, &outputs);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Identify").on_hover_text("Show the display name on the Android device").clicked() {
                if let Some(a) = e.active_display() {
                    e.send_to_all(&serde_json::json!({"t":"identify","name":a.output.name,"width":a.output.width,"height":a.output.height}));
                }
            }
            ui.label(caption("Shows the output name and resolution on the connected device for three seconds."));
        });
    });
    ui.add_space(10.0);
    two_cols(
        ui,
        app,
        |ui, app| {
            let d = &mut app.draft.display;
            section(ui, "Mode", |ui| {
                ui.radio_value(&mut d.mode, DisplayMode::Extend, "Extend — DREAD becomes an additional monitor");
                ui.radio_value(&mut d.mode, DisplayMode::Duplicate, "Duplicate — mirror an existing monitor");
                if d.mode == DisplayMode::Duplicate {
                    ui.add_space(6.0);
                    grid(ui, "dup", |ui| {
                        ui.label(caption("Monitor to duplicate"));
                        egui::ComboBox::from_id_salt("dup_src")
                            .selected_text(if d.duplicate_source.is_empty() { "Primary".to_string() } else { d.duplicate_source.clone() })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut d.duplicate_source, String::new(), "Primary");
                                for o in outputs.iter().filter(|o| !o.is_dread) {
                                    ui.selectable_value(&mut d.duplicate_source, o.name.clone(), format!("{} — {}×{}", o.name, o.width, o.height));
                                }
                            });
                        ui.end_row();
                    });
                }
            });
            ui.add_space(10.0);
            section(ui, "Position", |ui| {
                let ext = d.mode == DisplayMode::Extend;
                ui.add_enabled_ui(ext, |ui| {
                    grid(ui, "pos", |ui| {
                        ui.label(caption("Placement"));
                        egui::ComboBox::from_id_salt("placement").selected_text(format!("{:?}", d.placement)).show_ui(ui, |ui| {
                            for p in [Placement::Right, Placement::Left, Placement::Above, Placement::Below, Placement::Custom] {
                                ui.selectable_value(&mut d.placement, p, format!("{p:?}"));
                            }
                        });
                        ui.end_row();
                        ui.label(caption("Relative to"));
                        egui::ComboBox::from_id_salt("relto")
                            .selected_text(if d.relative_to.is_empty() { "Primary".to_string() } else { d.relative_to.clone() })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut d.relative_to, String::new(), "Primary");
                                for o in outputs.iter().filter(|o| !o.is_dread) {
                                    ui.selectable_value(&mut d.relative_to, o.name.clone(), o.name.clone());
                                }
                            });
                        ui.end_row();
                        if d.placement == Placement::Custom {
                            ui.label(caption("X / Y (logical px)"));
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut d.custom_x).range(-16384..=16384));
                                ui.add(egui::DragValue::new(&mut d.custom_y).range(-16384..=16384));
                            });
                            ui.end_row();
                        }
                        ui.label(caption("Primary display"));
                        let can_primary = cfg!(windows);
                        ui.add_enabled(can_primary, egui::Checkbox::new(&mut d.primary, "Make DREAD the primary display"))
                            .on_disabled_hover_text("Wayland compositors such as Hyprland and Sway have no primary display; windows open on the focused output.");
                        ui.end_row();
                    });
                });
                if !ext {
                    ui.label(caption("Position applies to extended mode only."));
                }
            });
        },
        |ui, app| {
            let d = &mut app.draft.display;
            section(ui, "Resolution", |ui| {
                let ext = d.mode == DisplayMode::Extend;
                ui.add_enabled_ui(ext, |ui| {
                    ui.checkbox(&mut d.auto_resolution, "Match the device automatically (resolution, orientation, density)");
                    ui.add_enabled_ui(!d.auto_resolution, |ui| {
                        grid(ui, "res", |ui| {
                            ui.label(caption("Resolution"));
                            let cur = (d.width.max(d.height), d.width.min(d.height));
                            let is_std = modes.contains(&cur);
                            egui::ComboBox::from_id_salt("res").selected_text(if is_std { format!("{} × {}", cur.0, cur.1) } else { "Custom".into() }).show_ui(ui, |ui| {
                                for &(w, h) in STANDARD_MODES.iter().filter(|m| modes.contains(m) && m.0 <= max.0 && m.1 <= max.1) {
                                    let note = if w > encoder_max.0.max(encoder_max.1) { "  (encoded downscaled)" } else { "" };
                                    if ui.selectable_label(cur == (w, h), format!("{w} × {h}{note}")).clicked() {
                                        d.width = w;
                                        d.height = h;
                                    }
                                }
                                if ui.selectable_label(!is_std, "Custom…").clicked() {
                                    d.width = app.custom_res.0;
                                    d.height = app.custom_res.1 + 2;
                                }
                            });
                            ui.end_row();
                            if !is_std {
                                ui.label(caption("Custom size"));
                                ui.horizontal(|ui| {
                                    ui.add(egui::DragValue::new(&mut d.width).range(320..=max.0.max(320)).suffix(" w"));
                                    ui.add(egui::DragValue::new(&mut d.height).range(240..=max.1.max(240)).suffix(" h"));
                                });
                                d.width &= !1;
                                d.height &= !1;
                                ui.end_row();
                            }
                            ui.label(caption("Orientation"));
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut d.orientation, Orientation::Landscape, "Landscape");
                                ui.radio_value(&mut d.orientation, Orientation::Portrait, "Portrait");
                            });
                            ui.end_row();
                        });
                    });
                    grid(ui, "res2", |ui| {
                        ui.label(caption("Refresh rate"));
                        egui::ComboBox::from_id_salt("hz").selected_text(format!("{} Hz", d.refresh)).show_ui(ui, |ui| {
                            for &r in &rates {
                                ui.selectable_value(&mut d.refresh, r, format!("{r} Hz"));
                            }
                        });
                        ui.end_row();
                        ui.label(caption("Scaling"));
                        egui::ComboBox::from_id_salt("scale")
                            .selected_text(if d.scale <= 0.0 { "Automatic (from device density)".into() } else { format!("{:.0} %", d.scale * 100.0) })
                            .show_ui(ui, |ui| {
                                for &s in SCALES {
                                    let label = if s <= 0.0 { "Automatic (from device density)".into() } else { format!("{:.0} %", s * 100.0) };
                                    ui.selectable_value(&mut d.scale, s, label);
                                }
                            });
                        ui.end_row();
                    });
                });
                ui.add_space(4.0);
                ui.label(caption(&format!(
                    "Virtual outputs up to {}×{}. Encoder verified up to {}×{}; larger displays are scaled for transmission.",
                    max.0, max.1, encoder_max.0, encoder_max.1
                )));
            });
        },
    );
}

// ------------------------------------------------------------------ streaming

pub fn streaming(app: &mut App, ui: &mut egui::Ui, st: &Stats) {
    let caps = app.engine.caps.read().clone();
    two_cols(
        ui,
        app,
        |ui, app| {
            let s = &mut app.draft.stream;
            section(ui, "Encoder", |ui| {
                let Some(caps) = caps.as_ref() else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(caption("Testing which encoders work on this machine…"));
                    });
                    return;
                };
                if caps.is_empty() {
                    ui.label(RichText::new("No working encoder. Check that FFmpeg is installed (Advanced, Diagnostics).").color(TEXT));
                    return;
                }
                grid(ui, "enc", |ui| {
                    ui.label(caption("Codec"));
                    egui::ComboBox::from_id_salt("codec").selected_text(s.codec.label()).show_ui(ui, |ui| {
                        for c in Codec::ALL {
                            if caps.iter().any(|x| x.codec == c) {
                                ui.selectable_value(&mut s.codec, c, c.label());
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(caption("Encoder"));
                    egui::ComboBox::from_id_salt("backend").selected_text(s.backend.label()).show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.backend, Backend::Auto, "Automatic");
                        for x in caps.iter().filter(|x| x.codec == s.codec) {
                            ui.selectable_value(&mut s.backend, x.backend, format!("{} ({})", x.backend.label(), x.encoder));
                        }
                    });
                    ui.end_row();
                    ui.label(caption("Hardware acceleration"));
                    ui.checkbox(&mut s.hardware_acceleration, "Use the GPU when available");
                    ui.end_row();
                    ui.label(caption("Compression"));
                    egui::ComboBox::from_id_salt("quality").selected_text(format!("{:?}", s.quality)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.quality, Quality::Speed, "Speed — lowest latency");
                        ui.selectable_value(&mut s.quality, Quality::Balanced, "Balanced");
                        ui.selectable_value(&mut s.quality, Quality::Quality, "Quality — sharper text, more GPU time");
                    });
                    ui.end_row();
                    ui.label(caption("Key frame interval"));
                    ui.add(egui::Slider::new(&mut s.keyframe_interval_s, 0.0..=20.0).suffix(" s").custom_formatter(|v, _| if v <= 0.0 { "on demand".into() } else { format!("{v:.1}") }));
                    ui.end_row();
                });
                ui.label(caption("The device reports which codecs it can decode; DREAD falls back to H.264 when needed."));
            });
            ui.add_space(10.0);
            section(ui, "Rate", |ui| {
                grid(ui, "rate", |ui| {
                    ui.label(caption("Bitrate"));
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("bitrate")
                            .selected_text(if BITRATES.contains(&s.bitrate_kbps) { format!("{} Mbps", s.bitrate_kbps / 1000) } else { "Custom".into() })
                            .show_ui(ui, |ui| {
                                for &b in BITRATES {
                                    ui.selectable_value(&mut s.bitrate_kbps, b, format!("{} Mbps", b / 1000));
                                }
                            });
                        let mut m = s.bitrate_kbps as f32 / 1000.0;
                        if ui.add(egui::DragValue::new(&mut m).range(0.5..=200.0).speed(0.1).suffix(" Mbps")).changed() {
                            s.bitrate_kbps = (m * 1000.0) as u32;
                        }
                    });
                    ui.end_row();
                    ui.label(caption("Frame rate"));
                    egui::ComboBox::from_id_salt("fps").selected_text(format!("{} fps", s.fps)).show_ui(ui, |ui| {
                        for &f in FPS {
                            ui.selectable_value(&mut s.fps, f, format!("{f} fps"));
                        }
                    });
                    ui.end_row();
                    ui.label(caption("Bandwidth limit"));
                    let mut lim = s.bandwidth_limit_kbps as f32 / 1000.0;
                    if ui.add(egui::DragValue::new(&mut lim).range(0.0..=500.0).speed(0.2).custom_formatter(|v, _| if v <= 0.0 { "unlimited".into() } else { format!("{v:.1} Mbps") })).changed() {
                        s.bandwidth_limit_kbps = (lim * 1000.0) as u32;
                    }
                    ui.end_row();
                    ui.label(caption("Frames in flight"));
                    ui.add(egui::Slider::new(&mut s.max_frames_in_flight, 1..=6)).on_hover_text("Frames sent but not yet shown on the device. Lower = less latency, higher = smoother on jittery Wi-Fi.");
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.checkbox(&mut s.adaptive_bitrate, "Adaptive bitrate — lower the bitrate when the link congests");
                ui.checkbox(&mut s.auto_quality, "Automatic quality — also lower resolution at minimum bitrate");
                ui.checkbox(&mut s.low_latency, "Low-latency mode — small encoder buffer (2 frames)");
                ui.checkbox(&mut s.show_cursor, "Draw the mouse cursor into the stream");
            });
        },
        |ui, app| {
            section(ui, "Live statistics", |ui| {
                grid(ui, "live", |ui| {
                    if !st.streaming {
                        kv(ui, "State", "not streaming");
                        return;
                    }
                    kv(ui, "Resolution", format!("{} × {}", st.width, st.height));
                    kv(ui, "Bitrate", format!("{} / target {}", mbps(st.bitrate_kbps), mbps(st.target_bitrate_kbps as f64)));
                    kv(ui, "Frame rate", format!("{:.1} fps", st.fps));
                    kv(ui, "Network latency (RTT)", format!("{:.1} ms", st.rtt_ms));
                    kv(ui, "Frame delivery", format!("{:.1} ms", st.delivery_ms));
                    kv(ui, "Packet loss (TCP retransmits)", format!("{:.2} %", st.packet_loss_pct));
                    kv(ui, "Encoding latency", format!("{:.1} ms", st.encode_ms));
                    kv(ui, "Decoding latency", format!("{:.1} ms", st.decode_ms));
                    kv(ui, "Connection", st.connection.clone());
                    kv(ui, "Codec", format!("{} — {}", st.codec, st.encoder));
                    kv(ui, "Frames dropped", st.frames_dropped.to_string());
                    kv(ui, "Waiting on link", format!("{} %", st.frames_skipped));
                });
            });
            ui.add_space(10.0);
            section(ui, "History (2 min)", |ui| {
                sparkline(ui, &app.history.rtt.iter().cloned().collect::<Vec<_>>(), 44.0, "RTT ms");
                ui.add_space(4.0);
                sparkline(ui, &app.history.encode.iter().cloned().collect::<Vec<_>>(), 44.0, "encode ms");
            });
        },
    );
}

// ------------------------------------------------------------------ connection

fn qr_texture(app: &mut App, ctx: &egui::Context, uri: &str) -> Option<egui::TextureHandle> {
    if let Some((u, t)) = &app.qr_texture {
        if u == uri {
            return Some(t.clone());
        }
    }
    let code = qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M).ok()?;
    let w = code.width();
    let quiet = 4;
    let n = w + 2 * quiet;
    let mut px = vec![255u8; n * n * 4];
    for y in 0..w {
        for x in 0..w {
            if code[(x, y)] == qrcode::Color::Dark {
                let i = ((y + quiet) * n + x + quiet) * 4;
                px[i] = 0;
                px[i + 1] = 0;
                px[i + 2] = 0;
            }
        }
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([n, n], &px);
    let t = ctx.load_texture("qr", img, egui::TextureOptions::NEAREST);
    app.qr_texture = Some((uri.to_string(), t.clone()));
    Some(t)
}

pub fn connection(app: &mut App, ui: &mut egui::Ui) {
    let e = app.engine.clone();
    let uri = e.pairing_uri();
    let tex = qr_texture(app, ui.ctx(), &uri);
    two_cols(
        ui,
        app,
        |ui, app| {
            let c = &mut app.draft.connection;
            section(ui, "Pair a device", |ui| {
                ui.horizontal(|ui| {
                    if let Some(t) = &tex {
                        ui.add(egui::Image::new(t).fit_to_exact_size(egui::vec2(220.0, 220.0)));
                    }
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Scan with DREAD on Android").color(TEXT));
                        ui.label(caption("The code contains this computer's addresses, port, certificate fingerprint and a one-time secret valid for 5 minutes."));
                        ui.add_space(6.0);
                        let left = e.qr.lock().created.elapsed().as_secs();
                        ui.label(caption(&format!("Expires in {}:{:02}", (300u64.saturating_sub(left)) / 60, (300u64.saturating_sub(left)) % 60)));
                        if ui.button("New code").clicked() {
                            e.rotate_qr();
                        }
                        ui.add_space(6.0);
                        ui.label(caption("Without the camera: enter the address in the app. Both screens then show a 6-digit code to compare and approve here."));
                    });
                });
            });
            ui.add_space(10.0);
            section(ui, "Security", |ui| {
                grid(ui, "sec", |ui| {
                    kv(ui, "Encryption", "TLS 1.3 / 1.2, pinned certificate");
                    kv(ui, "Fingerprint", e.identity.fingerprint_short());
                    ui.label(caption("Public addresses"));
                    ui.checkbox(&mut c.allow_public_addresses, "Accept connections from internet-routable addresses");
                    ui.end_row();
                    ui.label(caption("Heartbeat timeout"));
                    ui.add(egui::Slider::new(&mut c.timeout_s, 3..=30).suffix(" s"));
                    ui.end_row();
                    ui.label(caption("Keep display after disconnect"));
                    ui.add(egui::Slider::new(&mut c.display_linger_s, 0..=300).suffix(" s")).on_hover_text("A device that reconnects within this time finds its windows where it left them.");
                    ui.end_row();
                });
                if c.allow_public_addresses {
                    ui.label(RichText::new("Warning: only enable this behind a VPN you control. Pairing still applies.").color(TEXT));
                }
            });
            ui.add_space(10.0);
            section(ui, "Paired devices", |ui| {
                let devs = e.devices();
                if devs.is_empty() {
                    ui.label(caption("No paired devices."));
                }
                egui::Grid::new("paired").num_columns(3).spacing([20.0, 6.0]).show(ui, |ui| {
                    for d in devs {
                        ui.label(value(d.name.clone()));
                        ui.label(caption(&format!("id {}…", &d.device_id[..d.device_id.len().min(8)])));
                        if ui.small_button("Unpair").clicked() {
                            e.revoke(&d.device_id);
                        }
                        ui.end_row();
                    }
                });
            });
        },
        |ui, app| {
            let c = &mut app.draft.connection;
            section(ui, "Wi-Fi / LAN", |ui| {
                grid(ui, "wifi", |ui| {
                    ui.label(caption("Network connections"));
                    ui.checkbox(&mut c.wifi_enabled, "Accept connections over the network");
                    ui.end_row();
                    ui.label(caption("Port"));
                    ui.add(egui::DragValue::new(&mut c.port).range(1024..=65535));
                    ui.end_row();
                    ui.label(caption("Discovery"));
                    ui.checkbox(&mut c.discovery_enabled, "Announce on the LAN (mDNS + UDP 47801)");
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.label(caption("Interfaces"));
                egui::Grid::new("ifaces").num_columns(2).spacing([20.0, 4.0]).show(ui, |ui| {
                    for (name, ip) in dread_core::net::interfaces() {
                        ui.label(value(name));
                        ui.label(value(ip.to_string()));
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                ui.label(caption(&format!("Firewall: allow TCP {} and UDP 47801 on private networks.", c.port)));
            });
            ui.add_space(10.0);
            section(ui, "USB", |ui| {
                ui.checkbox(&mut c.usb_enabled, "Use USB (Android accessory mode, no USB debugging needed)");
                ui.add_space(4.0);
                ui.label(value(e.usb_status.lock().clone()));
                ui.add_space(6.0);
                ui.label(caption(
                    "1. Plug the device in with a data cable.\n2. Android asks to open DREAD for \"DREAD Display\" — allow it (tick \"always\").\n3. The stream starts over USB.\n\nAlternative: enable USB tethering on the device; it then connects over the tethered network like Wi-Fi.",
                ));
                #[cfg(target_os = "linux")]
                ui.label(caption("Linux needs permission to open USB devices: install packaging/linux/70-dread-usb.rules (see docs/USB.md)."));
            });
        },
    );
}

// ------------------------------------------------------------------ input

fn pressure_curve(ui: &mut egui::Ui, gamma: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 120.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, BG);
    p.rect_stroke(rect, 0.0, egui::Stroke::new(1.0_f32, BORDER), egui::StrokeKind::Inside);
    let pts: Vec<egui::Pos2> = (0..=40).map(|i| {
        let x = i as f32 / 40.0;
        egui::pos2(rect.left() + x * rect.width(), rect.bottom() - x.powf(gamma) * rect.height())
    }).collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.5_f32, LIGHT)));
    p.text(rect.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, "output", egui::FontId::proportional(10.0), TEXT2);
    p.text(rect.right_bottom() - egui::vec2(4.0, 2.0), egui::Align2::RIGHT_BOTTOM, "pressure", egui::FontId::proportional(10.0), TEXT2);
}

pub fn input(app: &mut App, ui: &mut egui::Ui) {
    let e = app.engine.clone();
    two_cols(
        ui,
        app,
        |ui, app| {
            let i = &mut app.draft.input;
            section(ui, "Touch", |ui| {
                ui.checkbox(&mut i.touch_enabled, "Accept multitouch from the device");
                ui.label(caption("Delivered as a real multitouch screen (up to 10 contacts) bound to the DREAD display. Gestures — scroll, pinch, long press — are interpreted by your desktop exactly as on a hardware touchscreen."));
            });
            ui.add_space(10.0);
            section(ui, "Stylus", |ui| {
                ui.checkbox(&mut i.pen_enabled, "Accept stylus input");
                ui.checkbox(&mut i.pen_tilt, "Forward tilt");
                ui.horizontal(|ui| {
                    pressure_curve(ui, i.pen_pressure_gamma);
                    ui.vertical(|ui| {
                        ui.label(caption("Pressure curve"));
                        ui.add(egui::Slider::new(&mut i.pen_pressure_gamma, 0.3..=3.0).logarithmic(true).text("γ"));
                        ui.horizontal(|ui| {
                            if ui.small_button("Soft").clicked() {
                                i.pen_pressure_gamma = 0.6;
                            }
                            if ui.small_button("Linear").clicked() {
                                i.pen_pressure_gamma = 1.0;
                            }
                            if ui.small_button("Firm").clicked() {
                                i.pen_pressure_gamma = 1.7;
                            }
                        });
                        ui.label(caption("Pen, eraser, hover, pressure, tilt and both barrel buttons are passed through as a tablet tool — not as mouse events."));
                    });
                });
            });
        },
        |ui, app| {
            let i = &mut app.draft.input;
            section(ui, "Mouse", |ui| {
                ui.checkbox(&mut i.mouse_enabled, "Accept mouse / trackpad mode from the device");
                grid(ui, "mouse", |ui| {
                    ui.label(caption("Trackpad speed"));
                    ui.add(egui::Slider::new(&mut i.mouse_speed, 0.2..=5.0));
                    ui.end_row();
                    ui.label(caption("Scroll speed"));
                    ui.add(egui::Slider::new(&mut i.scroll_speed, 0.2..=5.0));
                    ui.end_row();
                    ui.label(caption("Scroll direction"));
                    ui.checkbox(&mut i.invert_scroll, "Invert");
                    ui.end_row();
                });
            });
            ui.add_space(10.0);
            section(ui, "Keyboard", |ui| {
                ui.checkbox(&mut i.keyboard_enabled, "Accept keys and text from the device");
                ui.checkbox(&mut i.auto_keyboard, "Open the Android keyboard when a text field on the DREAD display is focused");
                ui.label(caption("Hardware keys (Ctrl, Alt, Super, Tab, Esc, F-keys, shortcuts) go through a virtual keyboard; typed text is inserted as Unicode, independent of the keyboard layout."));
            });
            ui.add_space(10.0);
            section(ui, "Mapping", |ui| {
                grid(ui, "map", |ui| {
                    let out = e.active_display().map(|d| d.output.name).unwrap_or_else(|| "DREAD display (when active)".into());
                    kv(ui, "Touchscreen", out.clone());
                    kv(ui, "Pen", out.clone());
                    kv(ui, "Pointer", format!("{out}, mapped into the desktop layout"));
                });
                ui.label(caption("Coordinates are normalised to the streamed picture, so rotation, letterboxing and scaling on the device never shift the pointer."));
            });
        },
    );
}

// ------------------------------------------------------------------ advanced

pub fn advanced(app: &mut App, ui: &mut egui::Ui) {
    let e = app.engine.clone();
    two_cols(
        ui,
        app,
        |ui, app| {
            section(ui, "System", |ui| {
                let g = &mut app.draft.general;
                grid(ui, "sys", |ui| {
                    ui.label(caption("Server name"));
                    ui.text_edit_singleline(&mut g.server_name);
                    ui.end_row();
                    ui.label(caption("Automatic startup"));
                    ui.checkbox(&mut g.autostart, "Start DREAD when I log in");
                    ui.end_row();
                    ui.label(caption("Start minimized"));
                    ui.checkbox(&mut g.start_minimized, "");
                    ui.end_row();
                    ui.label(caption("Auto stream"));
                    ui.checkbox(&mut g.auto_stream, "Stream as soon as a paired device connects");
                    ui.end_row();
                    ui.label(caption("FFmpeg"));
                    ui.add(egui::TextEdit::singleline(&mut g.ffmpeg_path).hint_text(e.ffmpeg.read().clone().unwrap_or_else(|| "not found".into())));
                    ui.end_row();
                    #[cfg(target_os = "linux")]
                    {
                        ui.label(caption("VA-API device"));
                        ui.text_edit_singleline(&mut g.vaapi_device);
                        ui.end_row();
                    }
                    ui.label(caption("Log level"));
                    egui::ComboBox::from_id_salt("loglevel").selected_text(g.log_level.clone()).show_ui(ui, |ui| {
                        for l in ["error", "warn", "info", "debug"] {
                            ui.selectable_value(&mut g.log_level, l.to_string(), l);
                        }
                    });
                    ui.end_row();
                });
                ui.label(caption("Log level changes apply after restart."));
            });
            ui.add_space(10.0);
            section(ui, "Configuration", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Export…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().set_file_name("dread-config.json").save_file() {
                            match e.config().save_to(&p) {
                                Ok(()) => app.notify(format!("Exported to {}", p.display())),
                                Err(err) => app.notify(format!("Export failed: {err}")),
                            }
                        }
                    }
                    if ui.button("Import…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                            match Config::load_from(&p) {
                                Ok(mut c) => {
                                    // keep this machine's identity
                                    c.general.server_id = e.config().general.server_id;
                                    app.draft = c;
                                    app.notify("Imported — review and Apply");
                                }
                                Err(err) => app.notify(format!("Import failed: {err}")),
                            }
                        }
                    }
                    if ui.button("Reset to defaults").clicked() {
                        let id = app.draft.general.server_id.clone();
                        app.draft = Config::default();
                        app.draft.general.server_id = id;
                    }
                });
                let p = Config::path().display().to_string();
                ui.add(egui::Label::new(caption(&format!("Stored in {p}"))).truncate()).on_hover_text(p);
            });
            ui.add_space(10.0);
            section(ui, "Drivers and encoders", |ui| {
                let ds = e.driver_status();
                ui.horizontal(|ui| {
                    status_dot(ui, ds.ok);
                    ui.label(value(ds.backend.clone()));
                });
                ui.label(caption(&ds.detail));
                ui.add_space(6.0);
                match e.caps.read().as_ref() {
                    None => {
                        ui.label(caption("Probing encoders…"));
                    }
                    Some(c) => {
                        for x in c {
                            ui.label(value(format!("{:<12} {:<11} up to {}×{}", x.encoder, x.backend.label(), x.max_width, x.max_height)));
                        }
                        if c.is_empty() {
                            ui.label(value("no working encoder"));
                        }
                    }
                }
                if ui.button("Probe again").clicked() {
                    *e.caps.write() = None;
                    e.start_probe();
                }
            });
            ui.add_space(10.0);
            section(ui, "Updates", |ui| {
                ui.label(value(format!("DREAD Server {}", env!("CARGO_PKG_VERSION"))));
                ui.label(caption("DREAD never contacts the internet on its own. Install a newer AppImage / installer to update; settings and pairings are kept."));
            });
        },
        |ui, app| {
            section(ui, "Diagnostics", |ui| {
                let stale = app.diag_cache.as_ref().map(|(_, t)| t.elapsed().as_secs() > 2).unwrap_or(true);
                if stale {
                    app.diag_cache = Some((dread_core::diag::report(&e), std::time::Instant::now()));
                }
                let report = app.diag_cache.as_ref().unwrap().0.clone();
                ui.horizontal(|ui| {
                    if ui.button("Copy report").clicked() {
                        ui.ctx().copy_text(report.clone());
                        app.notify("Diagnostics copied");
                    }
                    if ui.button("Clear log").clicked() {
                        e.logs.clear();
                    }
                    ui.add(egui::TextEdit::singleline(&mut app.log_filter).hint_text("filter").desired_width(140.0));
                    egui::ComboBox::from_id_salt("lvl").selected_text(["ERROR", "WARN", "INFO", "DEBUG"][app.log_level]).width(80.0).show_ui(ui, |ui| {
                        for (n, l) in ["ERROR", "WARN", "INFO", "DEBUG"].iter().enumerate() {
                            ui.selectable_value(&mut app.log_level, n, *l);
                        }
                    });
                });
                ui.add_space(6.0);
                let rank = |l: &str| match l {
                    "ERROR" => 0,
                    "WARN" => 1,
                    "INFO" => 2,
                    _ => 3,
                };
                let filter = app.log_filter.to_lowercase();
                egui::Frame::new().fill(BG).stroke(egui::Stroke::new(1.0_f32, BORDER)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(420.0).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                        for l in e.logs.snapshot().iter().filter(|l| rank(&l.level) <= app.log_level && (filter.is_empty() || l.message.to_lowercase().contains(&filter))) {
                            let c = if rank(&l.level) <= 1 { TEXT } else { TEXT2 };
                            let line = format!("{} {:5} {}", l.time, l.level, l.message);
                            ui.add(egui::Label::new(RichText::new(&line).monospace().size(11.5).color(c)).truncate()).on_hover_text(line);
                        }
                    });
                });
            });
        },
    );
}
