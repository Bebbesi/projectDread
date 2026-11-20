//! DREAD control panel.

mod pages;
pub mod theme;

use anyhow::Result;
use dread_core::config::Config;
use dread_core::Engine;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use theme::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Dashboard,
    Display,
    Streaming,
    Connection,
    Input,
    Advanced,
}

impl Page {
    const ALL: [Page; 6] = [Page::Dashboard, Page::Display, Page::Streaming, Page::Connection, Page::Input, Page::Advanced];
    fn label(self) -> &'static str {
        match self {
            Page::Dashboard => "Dashboard",
            Page::Display => "Display",
            Page::Streaming => "Streaming",
            Page::Connection => "Connection",
            Page::Input => "Input",
            Page::Advanced => "Advanced",
        }
    }
}

pub struct History {
    pub fps: VecDeque<f64>,
    pub bitrate: VecDeque<f64>,
    pub rtt: VecDeque<f64>,
    pub encode: VecDeque<f64>,
    last: Instant,
}

impl History {
    fn push(&mut self, st: &dread_core::engine::Stats) {
        if self.last.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last = Instant::now();
        for (q, v) in [(&mut self.fps, st.fps), (&mut self.bitrate, st.bitrate_kbps / 1000.0), (&mut self.rtt, st.rtt_ms), (&mut self.encode, st.encode_ms)] {
            q.push_back(v);
            while q.len() > 120 {
                q.pop_front();
            }
        }
    }
}

pub struct App {
    pub engine: Engine,
    pub page: Page,
    /// Working copy edited by the pages; applied with "Apply".
    pub draft: Config,
    pub qr_texture: Option<(String, egui::TextureHandle)>,
    pub history: History,
    pub log_filter: String,
    pub log_level: usize,
    pub notice: Option<(String, Instant)>,
    pub custom_res: (u32, u32),
    pub diag_cache: Option<(String, Instant)>,
    minimize_once: bool,
}

impl App {
    fn new(engine: Engine, minimized: bool, page: Page) -> Self {
        let draft = engine.config();
        let custom_res = (draft.display.width, draft.display.height);
        Self {
            engine,
            page,
            draft,
            qr_texture: None,
            history: History { fps: VecDeque::new(), bitrate: VecDeque::new(), rtt: VecDeque::new(), encode: VecDeque::new(), last: Instant::now() },
            log_filter: String::new(),
            log_level: 2,
            notice: None,
            custom_res,
            diag_cache: None,
            minimize_once: minimized,
        }
    }

    pub fn dirty(&self) -> bool {
        self.draft != self.engine.config()
    }

    pub fn apply(&mut self) {
        self.engine.set_config(self.draft.clone());
        self.notify("Settings applied");
    }

    pub fn notify(&mut self, s: impl Into<String>) {
        self.notice = Some((s.into(), Instant::now()));
    }

    fn banners(&mut self, ui: &mut egui::Ui) {
        // pairing requests always on top: they need a decision
        for p in self.engine.pending_pairs() {
            egui::Frame::new().fill(LIGHT).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(format!("Pairing request from {} ({})", p.device_name, p.addr)).color(BG).strong());
                        ui.label(egui::RichText::new("Approve only if the code below matches the one shown on the device.").color(Color32Dark::TEXT).size(12.0));
                    });
                    ui.add_space(16.0);
                    let code = format!("{} {}", &p.code[..3], &p.code[3..]);
                    ui.label(egui::RichText::new(code).color(BG).monospace().size(26.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("Decline").color(BG)).fill(LIGHT).stroke(egui::Stroke::new(1.0_f32, BG))).clicked() {
                            self.engine.decide_pair(p.id, false);
                        }
                        if ui.add(egui::Button::new(egui::RichText::new("Approve").color(TEXT).strong()).fill(BG)).clicked() {
                            self.engine.decide_pair(p.id, true);
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
        if let Some(err) = self.engine.last_error.lock().clone() {
            egui::Frame::new().fill(BG).stroke(egui::Stroke::new(1.0_f32, TEXT2)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("ERROR").monospace().strong().color(TEXT));
                    ui.label(egui::RichText::new(&err).color(LIGHT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            *self.engine.last_error.lock() = None;
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
    }
}

struct Color32Dark;
impl Color32Dark {
    const TEXT: egui::Color32 = egui::Color32::from_rgb(0x30, 0x30, 0x30);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.minimize_once {
            self.minimize_once = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        let stats = self.engine.stats.lock().clone();
        self.history.push(&stats);

        egui::SidePanel::left("nav").exact_width(196.0).resizable(false).frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(0, 16)).stroke(egui::Stroke::new(1.0_f32, BORDER))).show(ctx, |ui| {
            // status block first so it reserves the bottom of the sidebar
            egui::TopBottomPanel::bottom("navstatus")
                .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 18, right: 8, top: 10, bottom: 0 }))
                .show_separator_line(false)
                .show_inside(ui, |ui| {
                    let streaming = stats.streaming;
                    ui.horizontal(|ui| {
                        status_dot(ui, streaming);
                        ui.label(egui::RichText::new(if streaming { "Streaming" } else { "Idle" }).color(if streaming { TEXT } else { TEXT2 }));
                    });
                    let n = self.engine.client_views().len();
                    ui.label(caption(&format!("{n} device{} connected", if n == 1 { "" } else { "s" })));
                    ui.label(caption(&format!("v{}", env!("CARGO_PKG_VERSION"))));
                });
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("DREAD").size(22.0).strong().color(TEXT).extra_letter_spacing(4.0));
                    ui.label(caption("Your screen. Expanded."));
                });
            });
            ui.add_space(22.0);
            for p in Page::ALL {
                let sel = self.page == p;
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
                let painter = ui.painter();
                if sel {
                    painter.rect_filled(rect, 0.0, PANEL);
                    painter.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())), 0.0, TEXT);
                } else if resp.hovered() {
                    painter.rect_filled(rect, 0.0, BG2);
                }
                painter.text(rect.left_center() + egui::vec2(20.0, 0.0), egui::Align2::LEFT_CENTER, p.label(), egui::FontId::proportional(14.0), if sel { TEXT } else { TEXT2 });
                if resp.clicked() {
                    self.page = p;
                }
            }
        });

        if self.dirty() {
            egui::TopBottomPanel::bottom("apply").frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(18, 10)).stroke(egui::Stroke::new(1.0_f32, BORDER))).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Unsaved changes").color(LIGHT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(ui, "Apply").clicked() {
                            self.apply();
                        }
                        if ui.button("Revert").clicked() {
                            self.draft = self.engine.config();
                        }
                    });
                });
            });
        }

        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG2).inner_margin(egui::Margin::same(20))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new(self.page.label()).color(TEXT));
                if let Some((n, t)) = &self.notice {
                    if t.elapsed() < Duration::from_secs(3) {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(caption(n)));
                    }
                }
            });
            ui.add_space(10.0);
            self.banners(ui);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.page {
                Page::Dashboard => pages::dashboard(self, ui, &stats),
                Page::Display => pages::display(self, ui),
                Page::Streaming => pages::streaming(self, ui, &stats),
                Page::Connection => pages::connection(self, ui),
                Page::Input => pages::input(self, ui),
                Page::Advanced => pages::advanced(self, ui),
            });
        });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

pub fn page_from_name(n: &str) -> Page {
    Page::ALL.into_iter().find(|p| p.label().eq_ignore_ascii_case(n)).unwrap_or(Page::Dashboard)
}

pub fn run(engine: Engine, minimized: bool, page: Page) -> Result<()> {
    let icon = egui::IconData { rgba: icon_rgba(), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DREAD")
            .with_app_id("dread")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "DREAD",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(App::new(engine, minimized, page)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("control panel: {e}"))
}

/// The DREAD mark: a white frame with an offset second frame (two screens).
pub fn icon_rgba() -> Vec<u8> {
    let n = 64usize;
    let mut px = vec![0u8; n * n * 4];
    let mut set = |x: usize, y: usize, v: u8| {
        let i = (y * n + x) * 4;
        px[i] = v;
        px[i + 1] = v;
        px[i + 2] = v;
        px[i + 3] = 255;
    };
    for y in 0..n {
        for x in 0..n {
            set(x, y, 0x10);
        }
    }
    let rect = |set: &mut dyn FnMut(usize, usize, u8), x0: usize, y0: usize, x1: usize, y1: usize, t: usize, v: u8| {
        for y in y0..y1 {
            for x in x0..x1 {
                if x < x0 + t || x >= x1 - t || y < y0 + t || y >= y1 - t {
                    set(x, y, v);
                }
            }
        }
    };
    rect(&mut set, 8, 14, 42, 40, 4, 0xa0);
    rect(&mut set, 22, 24, 56, 50, 4, 0xff);
    px
}
