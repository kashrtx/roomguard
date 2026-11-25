//! The zone list. Zones wired to the camera follow what it sees; the others
//! are virtual sensors you can open and close by hand to try the panel out.

use eframe::egui::{self, RichText};

use crate::engine::{EngineHandle, EngineMsg, View};
use crate::theme;

pub fn ui(ui: &mut egui::Ui, view: &View, engine: &EngineHandle) {
    ui.label(theme::heading("Zones"));
    ui.add_space(4.0);
    let total_w = ui.available_width();
    for z in &view.zones {
        let (state, color) = if !z.note.is_empty() && (z.open || z.tamper || z.fault) {
            // The camera itself is the problem, so say that instead of "Open".
            (z.note.as_str(), theme::AMBER)
        } else if z.tamper {
            ("Tamper", theme::AMBER)
        } else if z.fault {
            ("Fault", theme::AMBER)
        } else if z.bypassed {
            ("Bypassed", theme::BLUE)
        } else if z.open {
            ("Open", if view.armed { theme::RED } else { theme::AMBER })
        } else {
            ("Closed", theme::GREEN)
        };
        let right_reserve = if !z.camera {
            if z.alarm { 178.0 } else { 132.0 }
        } else if z.alarm {
            112.0
        } else {
            70.0
        };
        let name_max_w = (total_w - right_reserve - 28.0).max(90.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{:02}", z.num)).font(theme::semi(12.5)).color(theme::MUTED));
            ui.vertical(|ui| {
                ui.set_max_width(name_max_w);
                ui.add(egui::Label::new(RichText::new(&z.label).font(theme::semi(13.0))).truncate());
                let mut kind = z.kind.short().to_string();
                if z.camera {
                    kind.push_str(", camera");
                }
                ui.add(egui::Label::new(theme::muted(kind)).truncate());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !z.camera {
                    let open = view.sim.get(z.num as usize - 1).copied().unwrap_or(false);
                    let label = if open { "Close" } else { "Open" };
                    if ui.small_button(label).on_hover_text("Simulate this sensor").clicked() {
                        engine.send(EngineMsg::SimZone { zone: z.num, open: !open });
                    }
                }
                super::chip(ui, state, color);
                if z.alarm {
                    super::chip(ui, "Alarm", theme::RED);
                }
            });
        });
        ui.add_space(3.0);
    }
}
