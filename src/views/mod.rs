//! The panels around the keypad.

pub mod camera;
pub mod events;
pub mod operator;
pub mod sensors;
pub mod settings;
pub mod setup;

use eframe::egui::{self, Color32, RichText};

use crate::engine::types::StatusKind;
use crate::theme;

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

/// A small coloured label, like a status chip.
pub fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.18))
        .rounding(egui::Rounding::same(9.0))
        .inner_margin(egui::Margin::symmetric(7.0, 1.5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(11.5));
        });
}

pub fn accent_button(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE).font(theme::semi(13.5)))
            .fill(color.gamma_multiply(0.85))
            .rounding(egui::Rounding::same(6.0)),
    )
}
