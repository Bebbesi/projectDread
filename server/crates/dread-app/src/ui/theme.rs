//! DREAD visual language: flat, monochrome, thin borders, no ornament.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x10, 0x10, 0x10);
pub const BG2: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
pub const PANEL: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
pub const PANEL_HI: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const BORDER: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);
pub const MID: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x6a);
pub const TEXT2: Color32 = Color32::from_rgb(0xa0, 0xa0, 0xa0);
pub const TEXT: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const LIGHT: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    let r0 = CornerRadius::same(0);
    let r2 = CornerRadius::same(2);
    v.override_text_color = None;
    v.panel_fill = BG2;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = Color32::from_rgb(0x1c, 0x1c, 0x1c);
    v.code_bg_color = BG;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = r2;
    v.menu_corner_radius = r2;
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.hyperlink_color = LIGHT;
    v.warn_fg_color = LIGHT;
    v.error_fg_color = TEXT;
    v.selection.bg_fill = Color32::from_rgb(0x4a, 0x4a, 0x4a);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);
    v.slider_trailing_fill = true;
    v.striped = true;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    v.text_cursor.stroke = Stroke::new(1.5_f32, TEXT);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = BG2;
    w.noninteractive.weak_bg_fill = BG2;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT2);
    w.noninteractive.corner_radius = r0;

    w.inactive.bg_fill = PANEL;
    w.inactive.weak_bg_fill = PANEL;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, LIGHT);
    w.inactive.corner_radius = r2;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = PANEL_HI;
    w.hovered.weak_bg_fill = PANEL_HI;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, MID);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.corner_radius = r2;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = Color32::from_rgb(0x38, 0x38, 0x38);
    w.active.weak_bg_fill = Color32::from_rgb(0x38, 0x38, 0x38);
    w.active.bg_stroke = Stroke::new(1.0_f32, TEXT2);
    w.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.active.corner_radius = r2;
    w.active.expansion = 0.0;

    w.open.bg_fill = PANEL_HI;
    w.open.weak_bg_fill = PANEL_HI;
    w.open.bg_stroke = Stroke::new(1.0_f32, MID);
    w.open.corner_radius = r2;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
        s.spacing.combo_width = 180.0;
        s.spacing.slider_width = 200.0;
        s.spacing.window_margin = Margin::same(12);
        s.animation_time = 0.0;
        s.text_styles = [
            (TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
            (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        ]
        .into();
    });
}

/// Flat bordered section with a small uppercase caption.
pub fn section<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(0))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title.to_uppercase()).size(11.0).color(TEXT2).strong());
            ui.add_space(6.0);
            add(ui)
        })
        .inner
}

pub fn caption(text: &str) -> egui::RichText {
    egui::RichText::new(text).color(TEXT2).size(12.0)
}

pub fn value(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).color(TEXT).monospace()
}

/// Key/value row inside a grid.
pub fn kv(ui: &mut egui::Ui, k: &str, v: impl Into<String>) {
    let v: String = v.into();
    ui.label(caption(k));
    ui.add(egui::Label::new(value(v.clone())).truncate()).on_hover_text(v);
    ui.end_row();
}

/// Primary button: white on black, used for the single main action of a page.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(text).color(BG).strong()).fill(LIGHT).stroke(Stroke::NONE).min_size(egui::vec2(120.0, 28.0)))
}

/// Small square status indicator (filled = ok/active, hollow = inactive).
pub fn status_dot(ui: &mut egui::Ui, on: bool) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
    let p = ui.painter();
    if on {
        p.rect_filled(r, 0.0, TEXT);
    } else {
        p.rect_stroke(r, 0.0, Stroke::new(1.0_f32, MID), egui::StrokeKind::Inside);
    }
}

/// Minimal line chart for the statistics history.
pub fn sparkline(ui: &mut egui::Ui, values: &[f64], height: f32, label: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, BG);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, BORDER), egui::StrokeKind::Inside);
    let max = values.iter().cloned().fold(1e-9, f64::max);
    if values.len() >= 2 {
        let n = values.len() as f32 - 1.0;
        let pts: Vec<egui::Pos2> = values
            .iter()
            .enumerate()
            .map(|(i, v)| egui::pos2(rect.left() + rect.width() * i as f32 / n, rect.bottom() - 2.0 - (rect.height() - 4.0) * (*v / max) as f32))
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.2_f32, LIGHT)));
    }
    p.text(rect.left_top() + egui::vec2(6.0, 4.0), egui::Align2::LEFT_TOP, format!("{label}  max {max:.1}"), FontId::proportional(11.0), TEXT2);
}
