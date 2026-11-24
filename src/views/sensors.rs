//! The zone list. Zones wired to the camera follow what it sees; the others
//! are virtual sensors you can open and close by hand to try the panel out.

use eframe::egui::{self, RichText};

use crate::engine::{EngineHandle, EngineMsg, View};
use crate::theme;

pub fn ui(ui: &mut egui::Ui, view: &View, engine: &EngineHandle) {
    ui.label(theme::heading("Zones"));
    ui.add_space(4.0);
    for z in &view.zones {
        let (state, color) = if z.tamper {
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
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{:02}", z.num)).font(theme::semi(13.0)).color(theme::MUTED));
            ui.vertical(|ui| {
                ui.label(RichText::new(&z.label).font(theme::semi(13.5)));
                let mut kind = z.kind.short().to_string();
                if z.camera {
                    kind.push_str(", camera");
                }
                ui.label(theme::muted(kind));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !z.camera {
                    let open = view.sim.get(z.num as usize - 1).copied().unwrap_or(false);
                    let label = if open { "Close it" } else { "Open it" };
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
        ui.add_space(2.0);
    }
}
