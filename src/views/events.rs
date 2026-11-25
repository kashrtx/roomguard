//! The event history, newest first, like the panel's event buffer.

use eframe::egui::{self, Color32, RichText};

use crate::engine::types::EvKind;
use crate::engine::{EngineHandle, EngineMsg, View};
use crate::theme;
use crate::util;

#[derive(Default)]
pub struct EventsView {
    filter: u8,
    confirm_clear: bool,
}

fn kind_color(k: EvKind) -> Color32 {
    match k {
        EvKind::Alarm => theme::RED,
        EvKind::Arm => theme::BLUE,
        EvKind::Disarm | EvKind::Restore => theme::GREEN,
        EvKind::Trouble => theme::AMBER,
        EvKind::Program => Color32::from_rgb(0xB0, 0x8C, 0xD8),
        EvKind::Camera => Color32::from_rgb(0x6F, 0xB7, 0xC9),
        EvKind::Info => theme::FAINT,
    }
}

fn passes(filter: u8, k: EvKind) -> bool {
    match filter {
        1 => k == EvKind::Alarm,
        2 => matches!(k, EvKind::Arm | EvKind::Disarm),
        3 => matches!(k, EvKind::Trouble | EvKind::Restore),
        4 => k == EvKind::Camera,
        _ => true,
    }
}

impl EventsView {
    pub fn ui(&mut self, ui: &mut egui::Ui, view: &View, engine: &EngineHandle) {
        ui.label(theme::heading("History"));
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            for (i, name) in ["All", "Alarms", "Arming", "Troubles", "Camera"].iter().enumerate() {
                if ui.selectable_label(self.filter == i as u8, *name).clicked() {
                    self.filter = i as u8;
                }
            }
        });
        ui.add_space(4.0);
        let h = (ui.available_height() - 32.0).max(120.0);
        egui::ScrollArea::vertical()
            .id_salt("event-history")
            .max_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let row_w = ui.available_width();
                let mut shown = 0;
                for ev in view.events.iter().rev().filter(|e| passes(self.filter, e.kind)) {
                    shown += 1;
                    let col = kind_color(ev.kind);
                    let resp = ui.horizontal_top(|ui| {
                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            ui.set_max_width((row_w - 14.0).max(100.0));
                            ui.label(RichText::new(&ev.text).color(theme::TEXT).size(13.0));
                            ui.label(RichText::new(util::fmt_stamp(ev.ts)).size(11.0).color(theme::FAINT));
                        });
                    });
                    let r = resp.response.rect;
                    let bar = egui::Rect::from_min_max(
                        egui::pos2(r.left() + 1.0, r.top() + 2.0),
                        egui::pos2(r.left() + 4.0, r.bottom() - 2.0),
                    );
                    ui.painter().rect_filled(bar, 1.5, col);
                    ui.add_space(4.0);
                    if shown >= 400 {
                        break;
                    }
                }
                if shown == 0 {
                    ui.label(theme::muted("Nothing here yet."));
                }
            });
        ui.horizontal(|ui| {
            if self.confirm_clear {
                ui.label("Clear history?");
                if ui.small_button("Yes, clear it").clicked() {
                    engine.send(EngineMsg::ClearEvents);
                    self.confirm_clear = false;
                }
                if ui.small_button("Keep it").clicked() {
                    self.confirm_clear = false;
                }
            } else if !view.events.is_empty() && ui.small_button("Clear history").clicked() {
                self.confirm_clear = true;
            }
        });
    }
}
