//! The zone list. Zones wired to the camera follow what it sees; any non-fire
//! zone can be drawn directly onto the Room Map or Live view with one click,
//! and unmapped zones stay available as virtual sensors you can open and close
//! by hand to test the panel.

use eframe::egui::{self, RichText, Sense, Vec2};

use crate::config::CameraCfg;
use crate::engine::{EngineHandle, EngineMsg, View};
use crate::theme;

use super::camera::{zone_color, zone_drawable};

/// Renders the Zones card. Returns `Some(zone_num)` when the user clicks
/// "Draw" to map a zone onto the camera/room map.
pub fn ui(ui: &mut egui::Ui, view: &View, cfg: &CameraCfg, engine: &EngineHandle) -> Option<u8> {
    ui.horizontal(|ui| {
        ui.label(theme::heading("Zones"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(theme::muted("Click Draw to place on map"));
        });
    });
    ui.add_space(4.0);
    let mut want_draw: Option<u8> = None;
    let total_w = ui.available_width();
    for z in &view.zones {
        let (state, color) = if !z.note.is_empty() && (z.open || z.tamper || z.fault) {
            // The camera itself is the problem, so say that instead of "Open".
            (z.note.as_str(), theme::AMBER)
        } else if z.tamper {
            ("Tamper", theme::AMBER)
        } else if z.fault {
            ("Fault", theme::AMBER)
        } else if z.bypassed && !z.auto_bypass {
            ("Bypassed", theme::BLUE)
        } else if z.auto_bypass {
            ("Stay off", theme::MUTED)
        } else if z.open {
            ("Open", if view.armed { theme::RED } else { theme::AMBER })
        } else {
            ("Closed", theme::GREEN)
        };
        let can_draw = cfg.enabled && zone_drawable(z);
        let mapped_box = cfg.zone_rect(z.num).is_some();
        let right_reserve = if !z.camera {
            (if z.alarm { 178.0 } else { 132.0 }) + if can_draw { 46.0 } else { 0.0 }
        } else if z.alarm {
            112.0 + if can_draw { 52.0 } else { 0.0 }
        } else {
            70.0 + if can_draw { 52.0 } else { 0.0 }
        };
        let name_max_w = (total_w - right_reserve - 28.0).max(82.0);
        ui.horizontal(|ui| {
            if mapped_box {
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.3, zone_color(z.num));
            }
            ui.label(RichText::new(format!("{:02}", z.num)).font(theme::semi(12.5)).color(theme::MUTED));
            ui.vertical(|ui| {
                ui.set_max_width(name_max_w);
                ui.add(egui::Label::new(RichText::new(&z.label).font(theme::semi(13.0))).truncate());
                let mut kind = z.kind.short().to_string();
                if mapped_box {
                    kind.push_str(", on map");
                } else if z.camera {
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
                if can_draw {
                    let dlbl = if mapped_box { "Redraw" } else { "Draw" };
                    let tip = if mapped_box {
                        "Redraw this zone on the Camera or Room Map"
                    } else {
                        "Draw a camera box for this zone on the Camera or Room Map"
                    };
                    if ui.small_button(dlbl).on_hover_text(tip).clicked() {
                        want_draw = Some(z.num);
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
    want_draw
}
