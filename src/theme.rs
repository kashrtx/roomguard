//! Colours, fonts and widget styling. The look is deliberately plain and
//! utilitarian, like the software a monitoring company runs: dark neutral
//! panels, one green for "all good", red only for alarms, amber for trouble.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke, TextStyle, Visuals};

pub const BG: Color32 = Color32::from_rgb(0x15, 0x17, 0x19);
pub const PANEL: Color32 = Color32::from_rgb(0x1C, 0x1F, 0x22);
pub const CARD: Color32 = Color32::from_rgb(0x23, 0x27, 0x2B);
pub const CARD_HI: Color32 = Color32::from_rgb(0x2B, 0x30, 0x35);
pub const LINE: Color32 = Color32::from_rgb(0x34, 0x39, 0x3F);
pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xE6, 0xE1);
pub const MUTED: Color32 = Color32::from_rgb(0x9A, 0x9F, 0xA5);
pub const FAINT: Color32 = Color32::from_rgb(0x69, 0x6F, 0x76);
pub const GREEN: Color32 = Color32::from_rgb(0x3D, 0xB2, 0x72);
pub const RED: Color32 = Color32::from_rgb(0xE0, 0x50, 0x46);
pub const AMBER: Color32 = Color32::from_rgb(0xE6, 0xA2, 0x3C);
pub const BLUE: Color32 = Color32::from_rgb(0x5C, 0x9C, 0xD4);

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".to_owned(), FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.otf")));
    fonts
        .font_data
        .insert("inter-semi".to_owned(), FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.otf")));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".to_owned());
    let mut semi = vec!["inter-semi".to_owned(), "inter".to_owned()];
    if let Some(fallback) = fonts.families.get(&FontFamily::Proportional) {
        semi.extend(fallback.iter().skip(1).cloned());
    }
    fonts.families.insert(FontFamily::Name("semi".into()), semi);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.extreme_bg_color = BG;
    v.faint_bg_color = CARD_HI;
    v.window_rounding = Rounding::same(10.0);
    v.window_stroke = Stroke::new(1.0f32, LINE);
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = BLUE;
    v.selection.bg_fill = Color32::from_rgb(0x2C, 0x5A, 0x44);
    v.selection.stroke = Stroke::new(1.0f32, GREEN);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.weak_bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0f32, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0f32, TEXT);
    v.widgets.noninteractive.rounding = Rounding::same(6.0);
    for (w, fill) in [
        (&mut v.widgets.inactive, Color32::from_rgb(0x2E, 0x33, 0x39)),
        (&mut v.widgets.hovered, Color32::from_rgb(0x37, 0x3D, 0x44)),
        (&mut v.widgets.active, Color32::from_rgb(0x40, 0x47, 0x4F)),
        (&mut v.widgets.open, Color32::from_rgb(0x37, 0x3D, 0x44)),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0f32, LINE);
        w.fg_stroke = Stroke::new(1.0f32, TEXT);
        w.rounding = Rounding::same(6.0);
    }
    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.slider_width = 180.0;
    style.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
    style.text_styles.insert(TextStyle::Heading, FontId::new(19.0, FontFamily::Name("semi".into())));
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
    ctx.set_style(style);
}

/// Font sizes are rounded to the nearest half point. The keypads scale their
/// lettering with the window, and every distinct size gets its own entry in
/// the font cache that's never freed, so unrounded sizes slowly grow memory
/// each time the window is resized. Half a point is invisible on screen.
fn snap_size(size: f32) -> f32 {
    ((size * 2.0).round() / 2.0).max(1.0)
}

pub fn semi(size: f32) -> FontId {
    FontId::new(snap_size(size), FontFamily::Name("semi".into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(snap_size(size))
}

pub fn heading(text: &str) -> egui::RichText {
    egui::RichText::new(text).font(semi(15.0)).color(TEXT)
}

pub fn muted(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).color(MUTED).size(12.5)
}

/// A rounded card behind a block of controls.
pub fn card() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD)
        .rounding(Rounding::same(10.0))
        .stroke(Stroke::new(1.0f32, LINE))
        .inner_margin(egui::Margin::same(12.0))
}
