//! The event history, newest first, like the panel's event buffer.

use eframe::egui::{self, Color32};

use crate::engine::types::{EvKind, EventRow};
use crate::engine::{EngineHandle, EngineMsg, View};
use crate::theme;
use crate::util;

/// Height of one history row: the event, then when it happened.
const ROW_H: f32 = 38.0;

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
        // Only the rows on screen are drawn, so a full history costs no more
        // each frame than a short one. Before, every row was laid out and its
        // time converted to the local time zone on every frame, which slowed
        // the whole window down a little more with every arm and disarm.
        let rows: Vec<&EventRow> = view.events.iter().rev().filter(|e| passes(self.filter, e.kind)).collect();
        egui::ScrollArea::vertical()
            .id_salt("event-history")
            .max_height(h)
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                let row_w = ui.available_width();
                for ev in &rows[range] {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(row_w, ROW_H), egui::Sense::hover());
                    let p = ui.painter_at(rect);
                    let bar = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 1.0, rect.top() + 4.0),
                        egui::pos2(rect.left() + 4.0, rect.bottom() - 6.0),
                    );
                    p.rect_filled(bar, 1.5, kind_color(ev.kind));
                    p.text(egui::pos2(rect.left() + 12.0, rect.top() + 3.0), egui::Align2::LEFT_TOP, &ev.text, theme::regular(13.0), theme::TEXT);
                    p.text(
                        egui::pos2(rect.left() + 12.0, rect.top() + 21.0),
                        egui::Align2::LEFT_TOP,
                        util::fmt_stamp(ev.ts),
                        theme::regular(11.0),
                        theme::FAINT,
                    );
                    resp.on_hover_text(&ev.text);
                }
            });
        if rows.is_empty() {
            ui.label(theme::muted("Nothing here yet."));
        }
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
