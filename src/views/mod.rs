//! The panels around the keypad.

pub mod camera;
pub mod events;
mod home;
pub mod installer;
pub mod operator;
pub mod sensors;
pub mod settings;
pub mod setup;

use eframe::egui::{self, Color32, RichText};

use crate::engine::types::StatusKind;
use crate::theme;

/// Keeps the first eight camera-capable zones easy to reach on larger panels.
pub(super) fn zone_page(ui: &mut egui::Ui, id: egui::Id, count: usize) -> std::ops::RangeInclusive<usize> {
    let pages = count.div_ceil(8);
    let mut page = ui.data(|data| data.get_temp::<usize>(id)).unwrap_or(0).min(pages.saturating_sub(1));
    if pages > 1 {
        ui.horizontal_wrapped(|ui| {
            for index in 0..pages {
                let first = index * 8 + 1;
                let last = (first + 7).min(count);
                ui.selectable_value(&mut page, index, format!("{:02}-{:02}", first, last));
            }
        });
        ui.data_mut(|data| data.insert_temp(id, page));
    }
    let first = page * 8 + 1;
    first..=(first + 7).min(count)
}

pub fn status_color(s: StatusKind) -> Color32 {
    match s {
        StatusKind::Ready => theme::GREEN,
        StatusKind::NotReady => theme::AMBER,
        StatusKind::Exit | StatusKind::Entry => theme::AMBER,
        StatusKind::Armed => theme::RED,
        StatusKind::Alarm => theme::RED,
        StatusKind::Program => theme::BLUE,
        StatusKind::Lockout => theme::RED,
    }
}

/// A small coloured label, like a status chip with consistent compact height.
pub fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    let font = theme::semi(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = galley.size() + egui::vec2(14.0, 5.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::Rounding::same(9.0), color.gamma_multiply(0.18));
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
}

pub fn accent_button(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE).font(theme::semi(13.5)))
            .fill(color.gamma_multiply(0.85))
            .rounding(egui::Rounding::same(6.0)),
    )
}
