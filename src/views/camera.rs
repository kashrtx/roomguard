//! The camera card. "Live" shows the picture with what the camera is
//! thinking drawn on top: motion, tracked people with their predicted paths,
//! faces, every drawn zone and ignored areas. "Room map" shows what RoomGuard
//! has learned about the room and lets you draw, redraw, or remove any camera
//! zone directly on the map.

use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, Painter, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::config::{CameraCfg, NRect, PtzKind};
use crate::engine::types::{ZoneKind, ZoneView};
use crate::engine::View;
use crate::paint;
use crate::theme;
use crate::util;
use crate::vision::{self, map, CamCmd, CamStatus, CameraHandle, Overlay};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Draw {
    Off,
    Zone(u8),
    Ignore,
}

pub struct CameraView {
    pub tab: u8,
    tex: Option<egui::TextureHandle>,
    seq: u64,
    size: [usize; 2],
    pano: Option<egui::TextureHandle>,
    pano_seq: u64,
    pub draw: Draw,
    drag_from: Option<Pos2>,
}

impl Default for CameraView {
    fn default() -> Self {
        CameraView { tab: 0, tex: None, seq: 0, size: [640, 480], pano: None, pano_seq: 0, draw: Draw::Off, drag_from: None }
    }
}

struct Snap {
    preview: Option<Arc<vision::Preview>>,
    status: CamStatus,
    fps: f32,
    name: String,
    xf: Option<vision::ptz::Xf>,
    map: vision::MapView,
    pano: Option<Arc<vision::frame::Rgb>>,
    pano_seq: u64,
    mapping: Option<f32>,
    ptz_status: String,
    door_learned: bool,
    calib: Option<vision::CalibView>,
    room_calibrated: bool,
    calibrated_at_ms: i64,
}

/// Distinct, high-contrast color for each panel zone (1..=8).
pub fn zone_color(zone: u8) -> Color32 {
    match zone {
        1 => theme::GREEN,
        2 => Color32::from_rgb(0x4F, 0xC3, 0xF7),
        3 => Color32::from_rgb(0xB3, 0x88, 0xFF),
        4 => Color32::from_rgb(0x26, 0xA6, 0x9A),
        5 => Color32::from_rgb(0xFF, 0xB7, 0x4D),
        6 => Color32::from_rgb(0xF0, 0x62, 0x92),
        7 => Color32::from_rgb(0xAE, 0xD5, 0x81),
        _ => Color32::from_rgb(0x79, 0x86, 0xCB),
    }
}

/// Whether a panel zone makes sense as a camera-drawn zone (skip unused & 24h fire/life-safety).
pub fn zone_drawable(z: &ZoneView) -> bool {
    z.kind != ZoneKind::Null && !z.kind.is_24h()
}

/// Default visual contact setting for a newly drawn zone: ON for doors/windows/perimeter,
/// OFF for interior motion zones.
pub fn default_contact_for(kind: ZoneKind) -> bool {
    matches!(kind, ZoneKind::Delay1 | ZoneKind::Delay2 | ZoneKind::Instant | ZoneKind::DayZone)
}

fn zone_name(zones: &[ZoneView], num: u8) -> String {
    zones
        .iter()
        .find(|z| z.num == num)
        .map(|z| z.label.clone())
        .unwrap_or_else(|| format!("Zone {}", num))
}

fn zone_kind(zones: &[ZoneView], num: u8) -> ZoneKind {
    zones.iter().find(|z| z.num == num).map(|z| z.kind).unwrap_or(ZoneKind::Instant)
}

fn full_uv() -> Rect {
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
}

fn draw_centered_wrapped(p: &Painter, rect: Rect, text: String, size: f32, color: Color32) {
    let wrap_w = (rect.width() - 28.0).max(80.0);
    let galley = p.fonts(|f| f.layout(text, theme::regular(size), color, wrap_w));
    let pos = rect.center() - galley.size() * 0.5;
    p.galley(pos, galley, color);
}

fn draw_zone_box(p: &Painter, rr: Rect, col: Color32, tag: &str, active: bool) {
    if active {
        p.rect_filled(rr, 2.0, col.gamma_multiply(0.16));
    }
    paint::dashed_rect(p, rr, Stroke::new(2.0f32, col), 7.0);
    let g = p.fonts(|f| f.layout_no_wrap(tag.to_string(), theme::semi(11.0), col));
    let pill_sz = g.size() + Vec2::new(8.0, 4.0);
    let pill_y = if rr.bottom() - pill_sz.y - 2.0 >= rr.top() {
        rr.bottom() - pill_sz.y - 2.0
    } else {
        rr.top() + 2.0
    };
    let pill = Rect::from_min_size(Pos2::new(rr.left() + 3.0, pill_y), pill_sz);
    p.rect_filled(pill, 4.0, Color32::from_black_alpha(185));
    p.galley(pill.min + Vec2::new(4.0, 2.0), g, col);
}

impl CameraView {
    pub fn start_draw_zone(&mut self, zone: u8) {
        self.draw = Draw::Zone(zone);
        self.drag_from = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        cam: &CameraHandle,
        cfg: &mut CameraCfg,
        view: &View,
        changed: &mut bool,
        show_motion: bool,
        show_tracks: bool,
    ) {
        let snap = match cam.shared.lock() {
            Ok(mut s) => {
                if let Some((pan, tilt)) = s.home_pose.take() {
                    cfg.ptz.home_pan = pan;
                    cfg.ptz.home_tilt = tilt;
                    *changed = true;
                }
                Snap {
                    preview: s.preview.clone(),
                    status: s.status.clone(),
                    fps: s.fps,
                    name: s.active_name.clone(),
                    xf: s.xf,
                    map: s.map.clone(),
                    pano: s.pano.clone(),
                    pano_seq: s.pano_seq,
                    mapping: s.mapping_progress,
                    ptz_status: s.ptz_status.clone(),
                    door_learned: s.door_learned,
                    calib: s.calib.clone(),
                    room_calibrated: s.room_calibrated,
                    calibrated_at_ms: s.calibrated_at_ms,
                }
            }
            Err(_) => return,
        };
        if let Some(pv) = &snap.preview {
            if pv.seq != self.seq && pv.rgb.len() == pv.w * pv.h * 3 {
                self.seq = pv.seq;
                self.size = [pv.w, pv.h];
                let img = egui::ColorImage::from_rgb([pv.w, pv.h], &pv.rgb);
                match &mut self.tex {
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => self.tex = Some(ui.ctx().load_texture("camera-preview", img, egui::TextureOptions::LINEAR)),
                }
            }
        }
        if snap.pano_seq != self.pano_seq {
            self.pano_seq = snap.pano_seq;
            self.pano = snap.pano.as_ref().filter(|r| r.data.len() == r.w * r.h * 3).map(|r| {
                let img = egui::ColorImage::from_rgb([r.w, r.h], &r.data);
                ui.ctx().load_texture("room-panorama", img, egui::TextureOptions::LINEAR)
            });
        }
        // If the active panel switched to a 6-zone panel while drawing zone 7/8, cancel cleanly.
        if let Draw::Zone(z) = self.draw {
            if z as usize > view.zones.len() {
                self.draw = Draw::Off;
                self.drag_from = None;
            }
        }

        ui.horizontal(|ui| {
            ui.label(theme::heading("Camera"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.selectable_label(self.tab == 1, "Room map").clicked() {
                    self.tab = 1;
                }
                if ui.selectable_label(self.tab == 0, "Live").clicked() {
                    self.tab = 0;
                }
            });
        });
        ui.add_space(4.0);
        if self.tab == 0 {
            self.live(ui, &snap, cam, cfg, view, changed, show_motion, show_tracks);
        } else {
            self.map_ui(ui, &snap, cam, cfg, view, changed);
        }
    }

    /// Handles drag-to-draw on either the Live canvas or the Room Map canvas.
    #[allow(clippy::too_many_arguments)]
    fn handle_draw_drag(
        &mut self,
        ui: &egui::Ui,
        p: &Painter,
        rect: Rect,
        resp: &egui::Response,
        from_screen: &dyn Fn(Pos2) -> (f32, f32),
        to_map: Option<vision::ptz::Xf>,
        cfg: &mut CameraCfg,
        zones: &[ZoneView],
        changed: &mut bool,
    ) {
        if self.draw == Draw::Off {
            return;
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let (hint, col) = match self.draw {
            Draw::Zone(z) => {
                let name = zone_name(zones, z);
                let kind = zone_kind(zones, z).short();
                (format!("Drag a box for Z{:02}: {} ({})", z, name, kind), zone_color(z))
            }
            Draw::Ignore => ("Drag a box over your desk/chair or any area to ignore".to_string(), theme::BLUE),
            Draw::Off => return,
        };
        let wrap_w = (rect.width() - 28.0).max(120.0);
        let g = p.fonts(|f| f.layout(hint, theme::semi(11.5), col, wrap_w));
        let pill = Rect::from_min_size(rect.left_top() + Vec2::new(8.0, 8.0), g.size() + Vec2::new(14.0, 6.0));
        p.rect_filled(pill, 5.0, Color32::from_black_alpha(190));
        p.rect_stroke(pill, 5.0, Stroke::new(1.0f32, col.gamma_multiply(0.7)));
        p.galley(pill.min + Vec2::new(7.0, 3.0), g, col);

        if resp.drag_started() {
            self.drag_from = resp.interact_pointer_pos();
        }
        if let (Some(a), Some(b)) = (self.drag_from, resp.interact_pointer_pos()) {
            if resp.dragged() {
                let drag_rect = Rect::from_two_pos(a, b).intersect(rect);
                p.rect_filled(drag_rect, 2.0, col.gamma_multiply(0.18));
                p.rect_stroke(drag_rect, 2.0, Stroke::new(2.0f32, col));
            }
        }
        if resp.drag_stopped() {
            if let (Some(a), Some(b)) = (self.drag_from.take(), resp.interact_pointer_pos()) {
                let (ax, ay) = from_screen(a);
                let (bx, by) = from_screen(b);
                let r = NRect::from_points(ax, ay, bx, by);
                if r.w > 0.03 && r.h > 0.03 {
                    let r = match to_map {
                        Some(x) if x.active => x.rect_view_to_map(r),
                        _ => r,
                    };
                    match self.draw {
                        Draw::Zone(z) => {
                            let def_contact = default_contact_for(zone_kind(zones, z));
                            cfg.set_zone_rect(z, r, def_contact);
                        }
                        Draw::Ignore => cfg.ignore.push(r),
                        Draw::Off => {}
                    }
                    *changed = true;
                }
            }
            self.draw = Draw::Off;
        }
    }

    /// Prominent fresh-calibration banner so the user always clearly sees when
    /// the app needs a fresh empty-room calibration for new objects & accuracy.
    fn calibration_banner(&mut self, ui: &mut egui::Ui, snap: &Snap, cam: &CameraHandle, cfg: &CameraCfg, running: bool) {
        if !cfg.enabled || !running {
            return;
        }
        if let Some(cal) = &snap.calib {
            let (accent, title, detail) = match cal.phase {
                vision::CalibPhase::Countdown => (
                    theme::AMBER,
                    "Fresh startup calibration",
                    "Step out of view and close doors/windows so RoomGuard learns today's room layout and new objects.",
                ),
                vision::CalibPhase::WaitingClear => (
                    theme::RED,
                    "Calibration waiting for an empty room",
                    "RoomGuard calibrates fresh each launch so moved or new objects don't cause false alarms. Step completely out of view.",
                ),
                vision::CalibPhase::Capturing => (
                    theme::GREEN,
                    "Scanning empty room...",
                    "Hold clear of the camera while RoomGuard locks the room background and closed contacts.",
                ),
            };
            ui.add_space(6.0);
            egui::Frame::none()
                .fill(accent.gamma_multiply(0.11))
                .stroke(Stroke::new(1.0f32, accent.gamma_multiply(0.45)))
                .rounding(7.0)
                .inner_margin(egui::Margin::symmetric(9.0, 7.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(title).font(theme::semi(12.5)).color(accent));
                    ui.label(RichText::new(&cal.prompt).font(theme::semi(11.5)).color(theme::TEXT));
                    ui.label(RichText::new(detail).size(11.0).color(theme::MUTED));
                    if cal.phase == vision::CalibPhase::Capturing {
                        ui.add_space(4.0);
                        ui.add(egui::ProgressBar::new(cal.progress.clamp(0.0, 1.0)).desired_height(6.0));
                    }
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        if ui.small_button("Restart 5s").clicked() {
                            cam.send(CamCmd::CalibrateRoom);
                        }
                        if ui.small_button("Skip for now").clicked() {
                            cam.send(CamCmd::CancelCalibrate);
                        }
                    });
                });
        } else if !snap.room_calibrated {
            ui.add_space(6.0);
            egui::Frame::none()
                .fill(theme::AMBER.gamma_multiply(0.12))
                .stroke(Stroke::new(1.2f32, theme::AMBER.gamma_multiply(0.55)))
                .rounding(7.0)
                .inner_margin(egui::Margin::symmetric(9.0, 7.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Fresh calibration needed for accuracy").font(theme::semi(12.5)).color(theme::AMBER));
                    ui.label(
                        RichText::new(
                            "RoomGuard recalibrates every time it starts so new or moved objects and lighting changes are detected accurately. Clear the room and close doors/windows, then calibrate.",
                        )
                        .size(11.5)
                        .color(theme::TEXT),
                    );
                    ui.add_space(5.0);
                    if super::accent_button(ui, "Calibrate empty room", theme::GREEN).clicked() {
                        cam.send(CamCmd::CalibrateRoom);
                    }
                });
        } else if snap.calibrated_at_ms > 0 && util::now_ms() - snap.calibrated_at_ms < 8_000 {
            ui.add_space(6.0);
            egui::Frame::none()
                .fill(theme::GREEN.gamma_multiply(0.11))
                .stroke(Stroke::new(1.0f32, theme::GREEN.gamma_multiply(0.4)))
                .rounding(7.0)
                .inner_margin(egui::Margin::symmetric(9.0, 5.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("Room calibrated for this session — new objects & closed contacts locked.")
                            .size(11.5)
                            .color(theme::GREEN),
                    );
                });
        }
    }

    /// Intuitive multi-zone drawing toolbar and removable drawn-zone chips,
    /// shared by both the Live view and the Room Map tab.
    fn zone_controls(&mut self, ui: &mut egui::Ui, snap: &Snap, cam: &CameraHandle, cfg: &mut CameraCfg, view: &View, changed: &mut bool, running: bool, id_prefix: &str) {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let draw_text = match self.draw {
                Draw::Zone(z) => format!("Drawing Z{:02}...", z),
                _ => "Draw zone...".to_string(),
            };
            egui::ComboBox::new((id_prefix, "draw-zone-picker"), "")
                .selected_text(draw_text)
                .width(142.0)
                .show_ui(ui, |ui| {
                    for z in &view.zones {
                        if !zone_drawable(z) {
                            continue;
                        }
                        let mapped = cfg.zone_rect(z.num).is_some();
                        let tag = if mapped { " (mapped)" } else { "" };
                        let label = format!("Z{:02} {} · {}{}", z.num, z.label, z.kind.short(), tag);
                        let selected = self.draw == Draw::Zone(z.num);
                        ui.horizontal(|ui| {
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 4.0, zone_color(z.num));
                            if ui.selectable_label(selected, label).clicked() {
                                self.draw = if selected { Draw::Off } else { Draw::Zone(z.num) };
                                self.drag_from = None;
                            }
                        });
                    }
                });

            if self.draw != Draw::Off {
                if ui.button(RichText::new("Cancel draw").color(theme::AMBER)).clicked() {
                    self.draw = Draw::Off;
                    self.drag_from = None;
                }
            } else {
                let door_label = if cfg.doorway.is_some() { "Redraw Z01" } else { "Mark doorway" };
                if ui.button(door_label).on_hover_text("Draw Zone 01 (Doorway)").clicked() {
                    self.draw = Draw::Zone(1);
                    self.drag_from = None;
                }
            }

            if ui.selectable_label(self.draw == Draw::Ignore, "Ignore area").clicked() {
                self.draw = if self.draw == Draw::Ignore { Draw::Off } else { Draw::Ignore };
                self.drag_from = None;
            }

            if snap.calib.is_none() && snap.room_calibrated {
                if ui.add_enabled(running, egui::Button::new("Recalibrate")).clicked() {
                    cam.send(CamCmd::CalibrateRoom);
                }
            }
        });

        // Compact interactive chips for every currently drawn zone & ignore box.
        let max_z = view.zones.len() as u8;
        let mut drawn_zones: Vec<u8> = Vec::new();
        if cfg.doorway.is_some() {
            drawn_zones.push(1);
        }
        for cz in &cfg.zones {
            if cz.zone >= 2 && cz.zone <= max_z {
                drawn_zones.push(cz.zone);
            }
        }
        if !drawn_zones.is_empty() || !cfg.ignore.is_empty() {
            ui.add_space(4.0);
            let mut to_remove: Option<u8> = None;
            let mut to_toggle_contact: Option<(u8, bool)> = None;
            let mut clear_ignore = false;
            ui.horizontal_wrapped(|ui| {
                for &z in &drawn_zones {
                    let col = zone_color(z);
                    let name = zone_name(&view.zones, z);
                    let contact = cfg.zone_contact(z);
                    egui::Frame::none()
                        .fill(col.gamma_multiply(0.13))
                        .stroke(Stroke::new(1.0f32, col.gamma_multiply(0.45)))
                        .rounding(6.0)
                        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let (dot, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                                ui.painter().circle_filled(dot.center(), 3.2, col);
                                let lbl = RichText::new(format!("Z{:02} {}", z, name)).size(11.0).color(theme::TEXT);
                                if ui.add(egui::Label::new(lbl).sense(Sense::click())).on_hover_text("Click to redraw this zone").clicked() {
                                    self.draw = Draw::Zone(z);
                                    self.drag_from = None;
                                }
                                let c_lbl = if contact { "Contact" } else { "Motion" };
                                let c_col = if contact { col } else { theme::MUTED };
                                let c_tip = if contact {
                                    "Watches both people and open/closed door or window state (click for motion only)"
                                } else {
                                    "Watches people/motion only (click to also watch open/closed contact state)"
                                };
                                if ui
                                    .add(egui::Button::new(RichText::new(c_lbl).size(10.0).color(c_col)).frame(false))
                                    .on_hover_text(c_tip)
                                    .clicked()
                                {
                                    to_toggle_contact = Some((z, !contact));
                                }
                                if ui
                                    .add(egui::Button::new(RichText::new("\u{00d7}").size(12.0).color(theme::MUTED)).frame(false))
                                    .on_hover_text("Remove this drawn zone")
                                    .clicked()
                                {
                                    to_remove = Some(z);
                                }
                            });
                        });
                }
                if !cfg.ignore.is_empty() {
                    egui::Frame::none()
                        .fill(theme::CARD_HI)
                        .stroke(Stroke::new(1.0f32, theme::LINE))
                        .rounding(6.0)
                        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.label(RichText::new(format!("Ignored ({})", cfg.ignore.len())).size(11.0).color(theme::MUTED));
                                if ui
                                    .add(egui::Button::new(RichText::new("\u{00d7}").size(12.0).color(theme::MUTED)).frame(false))
                                    .on_hover_text("Clear all ignored areas")
                                    .clicked()
                                {
                                    clear_ignore = true;
                                }
                            });
                        });
                }
            });
            if let Some(z) = to_remove {
                cfg.remove_zone(z);
                if self.draw == Draw::Zone(z) {
                    self.draw = Draw::Off;
                }
                *changed = true;
            }
            if let Some((z, on)) = to_toggle_contact {
                cfg.set_zone_contact(z, on);
                *changed = true;
            }
            if clear_ignore {
                cfg.ignore.clear();
                *changed = true;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn live(
        &mut self,
        ui: &mut egui::Ui,
        snap: &Snap,
        cam: &CameraHandle,
        cfg: &mut CameraCfg,
        view: &View,
        changed: &mut bool,
        show_motion: bool,
        show_tracks: bool,
    ) {
        let w = ui.available_width();
        let aspect = self.size[1] as f32 / self.size[0].max(1) as f32;
        let h = (w * aspect).clamp(140.0, 420.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 8.0, Color32::from_rgb(0x0D, 0x0F, 0x10));
        let running = matches!(snap.status, CamStatus::Running) && snap.preview.is_some();
        let focus = snap.preview.as_ref().and_then(|pv| pv.ov.focus).filter(|_| cfg.auto_frame);
        let uv = focus
            .map(|f| Rect::from_min_size(Pos2::new(f.x, f.y), Vec2::new(f.w.max(0.05), f.h.max(0.05))))
            .unwrap_or_else(full_uv);
        let to_screen = |x: f32, y: f32| {
            Pos2::new(
                rect.left() + (x - uv.left()) / uv.width() * rect.width(),
                rect.top() + (y - uv.top()) / uv.height() * rect.height(),
            )
        };
        let from_screen = |q: Pos2| {
            (
                uv.left() + (q.x - rect.left()) / rect.width() * uv.width(),
                uv.top() + (q.y - rect.top()) / rect.height() * uv.height(),
            )
        };
        if let (Some(tex), true) = (&self.tex, running) {
            p.image(tex.id(), rect, uv, Color32::WHITE);
            if let Some(pv) = &snap.preview {
                draw_overlay(&p, rect, &pv.ov, &view.zones, &to_screen, show_motion, show_tracks);
            }
        } else {
            let msg = match &snap.status {
                CamStatus::Off => "The camera is turned off in Settings".to_string(),
                CamStatus::Starting => "Starting the camera...".to_string(),
                CamStatus::Running => "Waiting for the first picture...".to_string(),
                CamStatus::Waiting(m) => m.clone(),
                CamStatus::Failed(e) => format!("The camera couldn't start: {}", e),
                CamStatus::NoPermission => "RoomGuard needs camera access. On a Mac, allow it in System Settings, Privacy and Security, Camera, then reopen RoomGuard.".to_string(),
            };
            draw_centered_wrapped(&p, rect, msg, 12.5, theme::MUTED);
        }

        self.handle_draw_drag(ui, &p, rect, &resp, &from_screen, snap.xf, cfg, &view.zones, changed);

        if let Some(cal) = &snap.calib {
            let col = match cal.phase {
                vision::CalibPhase::Countdown => theme::AMBER,
                vision::CalibPhase::WaitingClear => theme::RED,
                vision::CalibPhase::Capturing => theme::GREEN,
            };
            let wrap_w = (rect.width() - 36.0).max(120.0);
            let g = p.fonts(|f| f.layout(cal.prompt.clone(), theme::semi(12.0), col, wrap_w));
            let pill_w = (g.size().x + 20.0).min(rect.width() - 16.0);
            let pill_h = g.size().y + if cal.phase == vision::CalibPhase::Capturing { 16.0 } else { 10.0 };
            let pill = Rect::from_min_size(Pos2::new(rect.center().x - pill_w * 0.5, rect.top() + 8.0), Vec2::new(pill_w, pill_h));
            p.rect_filled(pill, 6.0, Color32::from_black_alpha(215));
            p.rect_stroke(pill, 6.0, Stroke::new(1.2f32, col.gamma_multiply(0.8)));
            p.galley(Pos2::new(pill.center().x - g.size().x * 0.5, pill.top() + 5.0), g, col);
            if cal.phase == vision::CalibPhase::Capturing {
                let bar_bg = Rect::from_min_size(Pos2::new(pill.left() + 8.0, pill.bottom() - 7.0), Vec2::new(pill.width() - 16.0, 3.5));
                p.rect_filled(bar_bg, 2.0, Color32::from_white_alpha(35));
                let fill = Rect::from_min_size(bar_bg.min, Vec2::new(bar_bg.width() * cal.progress.clamp(0.0, 1.0), bar_bg.height()));
                p.rect_filled(fill, 2.0, theme::GREEN);
            }
        } else if running && !snap.room_calibrated && self.draw == Draw::Off {
            let g = p.fonts(|f| f.layout_no_wrap("Needs calibration".to_string(), theme::semi(11.0), theme::AMBER));
            let pill = Rect::from_min_size(rect.left_top() + Vec2::new(8.0, 8.0), g.size() + Vec2::new(14.0, 6.0));
            p.rect_filled(pill, 5.0, Color32::from_black_alpha(190));
            p.rect_stroke(pill, 5.0, Stroke::new(1.0f32, theme::AMBER.gamma_multiply(0.7)));
            p.galley(pill.min + Vec2::new(7.0, 3.0), g, theme::AMBER);
        }

        self.calibration_banner(ui, snap, cam, cfg, running);
        self.zone_controls(ui, snap, cam, cfg, view, changed, running, "live");

        // Friendly, zero-jargon arming tip when no ignore area is set yet.
        if running && cfg.ignore.is_empty() && snap.calib.is_none() && snap.room_calibrated {
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(theme::BLUE.gamma_multiply(0.10))
                .stroke(Stroke::new(1.0f32, theme::BLUE.gamma_multiply(0.32)))
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Tip: Put an Ignore box over your desk/chair so you can arm easily while seated.")
                                .size(11.5)
                                .color(theme::TEXT),
                        );
                        if ui.small_button("Ignore desk").clicked() {
                            self.draw = Draw::Ignore;
                            self.drag_from = None;
                        }
                    });
                });
        }
        let mut info = if snap.name.is_empty() { "No camera".to_string() } else { snap.name.clone() };
        if running {
            info.push_str(&format!("  \u{00b7}  {:.0} fps", snap.fps));
            if snap.room_calibrated {
                info.push_str("  \u{00b7}  Calibrated");
            }
            if let Some(pv) = &snap.preview {
                let body = match pv.ov.pose_state {
                    vision::pose::POSE_READY if pv.ov.pose_ms > 0.5 => format!("Body tracking on, {:.0} ms", pv.ov.pose_ms),
                    vision::pose::POSE_READY => "Body tracking on".to_string(),
                    vision::pose::POSE_LOADING => "Starting body tracking".to_string(),
                    _ => "Body tracking unavailable, using motion only".to_string(),
                };
                info.push_str(&format!("  \u{00b7}  {}", body));
            }
        }
        if cfg.doorway.is_none() && cfg.zones.is_empty() {
            info.push_str("  \u{00b7}  Without drawn zones the whole view is zone 1");
        }
        ui.label(theme::muted(info));
        if let Some(pv) = &snap.preview {
            if !pv.ov.ptz_line.is_empty() {
                ui.label(theme::muted(pv.ov.ptz_line.clone()));
            }
        }
        if !snap.ptz_status.is_empty() {
            ui.label(RichText::new(&snap.ptz_status).color(theme::AMBER).size(12.0));
        }
    }

    fn map_ui(&mut self, ui: &mut egui::Ui, snap: &Snap, cam: &CameraHandle, cfg: &mut CameraCfg, view: &View, changed: &mut bool) {
        let ptz = cfg.ptz.kind != PtzKind::Fixed;
        let running = matches!(snap.status, CamStatus::Running) && snap.preview.is_some();
        let w = ui.available_width();
        let aspect = if ptz { 0.5 } else { self.size[1] as f32 / self.size[0].max(1) as f32 };
        let h = (w * aspect).clamp(140.0, 380.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 8.0, Color32::from_rgb(0x0D, 0x0F, 0x10));
        if ptz {
            if let Some(t) = &self.pano {
                let band = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.25), Pos2::new(rect.right(), rect.top() + rect.height() * 0.75));
                p.image(t.id(), band, full_uv(), Color32::from_gray(150));
            } else {
                draw_centered_wrapped(&p, rect, "Press \"Map the room\" and the camera will sweep the room".to_string(), 12.5, theme::MUTED);
            }
        } else if let Some(t) = &self.tex {
            p.image(t.id(), rect, full_uv(), Color32::from_gray(115));
        }
        let to = |x: f32, y: f32| Pos2::new(rect.left() + x * rect.width(), rect.top() + y * rect.height());
        let from_map = |q: Pos2| {
            (
                ((q.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((q.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
            )
        };
        let m = &snap.map;
        let n = map::GW * map::GH;
        if m.heat.len() == n {
            let max = m.heat.iter().cloned().fold(0.0f32, f32::max);
            if max > 0.0 {
                for gy in 0..map::GH {
                    for gx in 0..map::GW {
                        let v = (m.heat[gy * map::GW + gx] / max).sqrt();
                        if v < 0.06 {
                            continue;
                        }
                        let a = to(gx as f32 / map::GW as f32, gy as f32 / map::GH as f32);
                        let b = to((gx + 1) as f32 / map::GW as f32, (gy + 1) as f32 / map::GH as f32);
                        let c = paint::mix(Color32::from_rgb(0xF2, 0xB1, 0x3C), Color32::from_rgb(0xE5, 0x4B, 0x3C), v);
                        p.rect_filled(Rect::from_two_pos(a, b), 0.0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (v * 140.0) as u8));
                    }
                }
            }
        }
        if m.floor.len() == n {
            let max = m.floor.iter().cloned().fold(0.0f32, f32::max);
            if max > 0.0 {
                for k in 0..n {
                    let v = m.floor[k] / max;
                    if v > 0.05 {
                        let (cx, cy) = (((k % map::GW) as f32 + 0.5) / map::GW as f32, ((k / map::GW) as f32 + 0.5) / map::GH as f32);
                        p.circle_filled(to(cx, cy), 1.5 + 2.5 * v, Color32::from_rgba_unmultiplied(0x7F, 0xC8, 0xFF, (60.0 + 140.0 * v) as u8));
                    }
                }
            }
        }
        for (i, portal) in m.portals.iter().enumerate() {
            if portal.dismissed {
                continue;
            }
            let rr = Rect::from_two_pos(to(portal.x, portal.y), to(portal.x + portal.w, portal.y + portal.h));
            paint::dashed_rect(&p, rr, Stroke::new(1.5f32, theme::BLUE), 5.0);
            p.text(rr.left_top() + Vec2::new(4.0, 3.0), Align2::LEFT_TOP, format!("{}", i + 1), theme::semi(12.0), theme::BLUE);
        }
        for r in &cfg.ignore {
            let rr = Rect::from_two_pos(to(r.x, r.y), to(r.x + r.w, r.y + r.h));
            p.rect_filled(rr, 2.0, Color32::from_rgba_unmultiplied(10, 10, 10, 130));
            paint::dashed_rect(&p, rr, Stroke::new(1.0f32, theme::FAINT), 5.0);
            p.text(rr.left_top() + Vec2::new(4.0, 3.0), Align2::LEFT_TOP, "Ignored", theme::regular(10.5), theme::MUTED);
        }
        let max_z = view.zones.len() as u8;
        let active_flags = snap.preview.as_ref().map(|pv| pv.ov.zone_active).unwrap_or([false; 8]);
        if let Some(d) = cfg.doorway {
            let rr = Rect::from_two_pos(to(d.x, d.y), to(d.x + d.w, d.y + d.h));
            let active = active_flags[0];
            let col = if active { theme::RED } else { zone_color(1) };
            let tag = format!("Z01 {}", zone_name(&view.zones, 1));
            draw_zone_box(&p, rr, col, &tag, active);
        }
        for cz in &cfg.zones {
            if cz.zone < 2 || cz.zone > max_z {
                continue;
            }
            let rr = Rect::from_two_pos(to(cz.rect.x, cz.rect.y), to(cz.rect.x + cz.rect.w, cz.rect.y + cz.rect.h));
            let active = active_flags[cz.zone as usize - 1];
            let col = if active { theme::RED } else { zone_color(cz.zone) };
            let tag = format!("Z{:02} {}", cz.zone, zone_name(&view.zones, cz.zone));
            draw_zone_box(&p, rr, col, &tag, active);
        }
        if let Some(prog) = snap.mapping {
            let bar = Rect::from_min_size(rect.left_bottom() + Vec2::new(10.0, -16.0), Vec2::new((rect.width() - 20.0) * prog, 5.0));
            p.rect_filled(bar, 2.0, theme::BLUE);
        }

        // Room Map coordinates are already in map space, so pass `to_map: None`.
        self.handle_draw_drag(ui, &p, rect, &resp, &from_map, None, cfg, &view.zones, changed);

        self.calibration_banner(ui, snap, cam, cfg, running);
        self.zone_controls(ui, snap, cam, cfg, view, changed, running, "map");

        ui.add_space(6.0);
        if m.tracks_learned == 0 {
            ui.label(theme::muted(
                "Pick any zone above and drag on the map to place it. Walk in and out a few times so RoomGuard also learns traffic paths and doorways.",
            ));
        } else {
            ui.label(theme::muted(format!(
                "Learned from {} walk{} through the room. Pick any zone above to draw directly on the map. Orange is motion, blue dots are walking paths.",
                m.tracks_learned,
                if m.tracks_learned == 1 { "" } else { "s" }
            )));
        }
        if cfg.doorway.is_none() {
            if let Some(hint) = m.doorway_hint {
                ui.add_space(4.0);
                egui::Frame::none()
                    .fill(theme::GREEN.gamma_multiply(0.10))
                    .stroke(Stroke::new(1.0f32, theme::GREEN.gamma_multiply(0.35)))
                    .rounding(6.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("This looks like your doorway.").color(theme::TEXT).size(12.5));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if super::accent_button(ui, "Use as Z01", theme::GREEN).clicked() {
                                    cfg.doorway = Some(hint);
                                    *changed = true;
                                }
                            });
                        });
                    });
            }
        }
        for (i, portal) in m.portals.iter().enumerate() {
            if portal.dismissed {
                continue;
            }
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(theme::CARD_HI)
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(format!("{}.  {} ({} trips)", i + 1, portal.describe(), portal.uses))
                            .size(12.5)
                            .color(theme::TEXT),
                    );
                    ui.add_space(3.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.small_button("Use as Z01 Doorway").clicked() {
                            cfg.doorway = Some(portal.rect());
                            *changed = true;
                        }
                        ui.menu_button("Assign to zone...", |ui| {
                            for z in &view.zones {
                                if !zone_drawable(z) {
                                    continue;
                                }
                                if ui.button(format!("Z{:02} {}", z.num, z.label)).clicked() {
                                    cfg.set_zone_rect(z.num, portal.rect(), default_contact_for(z.kind));
                                    *changed = true;
                                    ui.close_menu();
                                }
                            }
                        });
                        if ui.small_button("Dismiss").clicked() {
                            cam.send(CamCmd::DismissPortal(i));
                        }
                    });
                });
        }
        for r in &m.ignore_hints {
            let already = cfg.ignore.iter().any(|q| q.overlap(r) > 0.5);
            if already {
                continue;
            }
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(theme::CARD_HI)
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Something keeps moving here with nobody near it.").size(12.5).color(theme::MUTED));
                    ui.add_space(3.0);
                    if ui.small_button("Ignore that spot").clicked() {
                        cfg.ignore.push(*r);
                        *changed = true;
                    }
                });
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ptz {
                let mapping = snap.mapping.is_some();
                if ui.add_enabled(!mapping, egui::Button::new("Map the room")).clicked() {
                    cam.send(CamCmd::MapRoom);
                }
                if ui.button("Make this the home view").clicked() {
                    cam.send(CamCmd::SetHome);
                }
            }
            let any_contact = (cfg.door_contact && cfg.doorway.is_some()) || cfg.zones.iter().any(|z| z.contact);
            if any_contact {
                let label = if snap.door_learned { "Relearn closed contacts" } else { "Learning closed contacts..." };
                if ui.add_enabled(snap.door_learned, egui::Button::new(label)).clicked() {
                    cam.send(CamCmd::RelearnDoor);
                }
            }
            if ui.button("Forget what it learned").clicked() {
                cam.send(CamCmd::ResetMap);
            }
        });
    }
}

/// Kinect style skeleton: glowing bones and joints over the tracked body.
fn draw_skeleton(p: &Painter, t: &vision::TrackView, to: &dyn Fn(f32, f32) -> Pos2) {
    use vision::pose::{BONES, JOINTS, KP_THR};
    if t.keypoints.len() < JOINTS {
        return;
    }
    let col = if t.known { theme::GREEN } else { Color32::from_rgb(0x5C, 0xD6, 0xFF) };
    let col = if t.lost { col.gamma_multiply(0.45) } else { col };
    let k = &t.keypoints;
    let seen = |i: usize| k[i].2 >= KP_THR;
    let pt = |i: usize| to(k[i].0, k[i].1);
    let glow = Stroke::new(5.0f32, col.gamma_multiply(0.22));
    let bone = Stroke::new(2.0f32, col);
    let mut lines: Vec<[Pos2; 2]> = BONES.iter().filter(|(a, b)| seen(*a) && seen(*b)).map(|(a, b)| [pt(*a), pt(*b)]).collect();
    // Neck: shoulder midpoint to the nose.
    if seen(5) && seen(6) && seen(0) {
        let mid = pt(5) + (pt(6) - pt(5)) * 0.5;
        lines.push([mid, pt(0)]);
    }
    for l in &lines {
        p.line_segment(*l, glow);
    }
    for l in &lines {
        p.line_segment(*l, bone);
    }
    for i in 0..JOINTS {
        if seen(i) {
            let c = pt(i);
            let r = if i < 5 { 2.2 } else { 3.0 };
            p.circle_filled(c, r + 2.0, col.gamma_multiply(0.25));
            p.circle_filled(c, r, Color32::WHITE);
            p.circle_stroke(c, r, Stroke::new(1.2f32, col));
        }
    }
}

fn draw_overlay(
    p: &Painter,
    rect: Rect,
    o: &Overlay,
    zones: &[ZoneView],
    to: &dyn Fn(f32, f32) -> Pos2,
    show_motion: bool,
    show_tracks: bool,
) {
    if show_motion && o.mgw > 0 && o.motion.len() == o.mgw * o.mgh {
        for gy in 0..o.mgh {
            for gx in 0..o.mgw {
                let v = o.motion[gy * o.mgw + gx];
                if v > 0.03 {
                    let a = to(gx as f32 / o.mgw as f32, gy as f32 / o.mgh as f32);
                    let b = to((gx + 1) as f32 / o.mgw as f32, (gy + 1) as f32 / o.mgh as f32);
                    p.rect_filled(Rect::from_two_pos(a, b), 0.0, Color32::from_rgba_unmultiplied(255, 90, 60, (v.min(1.0) * 120.0) as u8));
                }
            }
        }
    }
    for r in &o.ignore {
        let rr = Rect::from_two_pos(to(r.x, r.y), to(r.x + r.w, r.y + r.h));
        p.rect_filled(rr, 2.0, Color32::from_rgba_unmultiplied(15, 15, 15, 120));
        paint::dashed_rect(p, rr, Stroke::new(1.0f32, theme::FAINT), 5.0);
        p.text(rr.left_top() + Vec2::new(4.0, 3.0), Align2::LEFT_TOP, "Ignored", theme::regular(11.0), theme::MUTED);
    }
    let max_z = zones.len() as u8;
    if let Some(d) = o.doorway {
        let rr = Rect::from_two_pos(to(d.x, d.y), to(d.x + d.w, d.y + d.h));
        let active = o.zone_active[0];
        let col = if active {
            theme::RED
        } else if o.door_open {
            theme::AMBER
        } else {
            zone_color(1)
        };
        let base = zone_name(zones, 1);
        let label = if o.door_open { format!("Z01 {} (open)", base) } else { format!("Z01 {}", base) };
        draw_zone_box(p, rr, col, &label, active);
    }
    for &(z, r, contact_open) in &o.zones {
        if z < 2 || z > max_z {
            continue;
        }
        let rr = Rect::from_two_pos(to(r.x, r.y), to(r.x + r.w, r.y + r.h));
        let active = o.zone_active.get(z as usize - 1).copied().unwrap_or(false);
        let col = if active {
            theme::RED
        } else if contact_open {
            theme::AMBER
        } else {
            zone_color(z)
        };
        let base = zone_name(zones, z);
        let label = if contact_open {
            format!("Z{:02} {} (open)", z, base)
        } else {
            format!("Z{:02} {}", z, base)
        };
        draw_zone_box(p, rr, col, &label, active);
    }
    if show_tracks {
        for f in &o.faces {
            let rr = Rect::from_two_pos(to(f.x, f.y), to(f.x + f.w, f.y + f.h));
            let col = if f.known { theme::GREEN } else { theme::AMBER };
            p.rect_stroke(rr, 3.0, Stroke::new(1.6f32, col));
            if !f.label.is_empty() {
                p.text(rr.left_top() - Vec2::new(0.0, 3.0), Align2::LEFT_BOTTOM, &f.label, theme::semi(12.0), col);
            }
        }
        for t in &o.tracks {
            let a = to(t.x - t.w * 0.5, t.y - t.h * 0.5);
            let b = to(t.x + t.w * 0.5, t.y + t.h * 0.5);
            let rr = Rect::from_two_pos(a, b);
            let col = if t.known { theme::GREEN } else { Color32::from_rgb(0xF4, 0xF1, 0xEA) };
            let col = if t.lost { col.gamma_multiply(0.45) } else { col };
            let c = rr.width().min(rr.height()) * 0.22;
            for (corner, dx, dy) in [(rr.left_top(), 1.0, 1.0), (rr.right_top(), -1.0, 1.0), (rr.left_bottom(), 1.0, -1.0), (rr.right_bottom(), -1.0, -1.0)] {
                p.line_segment([corner, corner + Vec2::new(c * dx, 0.0)], Stroke::new(2.0f32, col));
                p.line_segment([corner, corner + Vec2::new(0.0, c * dy)], Stroke::new(2.0f32, col));
            }
            if t.path.len() > 1 {
                let pts: Vec<Pos2> = t.path.iter().map(|q| to(q.0, q.1)).collect();
                p.add(egui::Shape::line(pts, Stroke::new(1.5f32, col.gamma_multiply(0.5))));
            }
            draw_skeleton(p, t, to);
            if !t.predicted.is_empty() {
                let mut pts = vec![to(t.x, t.y)];
                pts.extend(t.predicted.iter().map(|q| to(q.0, q.1)));
                p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.8f32, theme::BLUE), 6.0, 4.0));
                if let Some(end) = pts.last() {
                    p.circle_stroke(*end, 5.0, Stroke::new(1.5f32, theme::BLUE));
                }
            }
            let name = if t.label.is_empty() { format!("Person {}", t.id) } else { t.label.clone() };
            let tag = if t.lost { format!("{} (predicted)", name) } else { name };
            p.text(rr.left_top() - Vec2::new(0.0, 3.0), Align2::LEFT_BOTTOM, tag, theme::semi(11.5), col);
        }
    }
    let mut y = rect.top() + 8.0;
    for (on, text, col) in [
        (o.covered, "Camera view blocked", theme::RED),
        (o.light_change, "Lighting changed, relearning the room", theme::AMBER),
    ] {
        if on {
            let g = p.fonts(|f| f.layout_no_wrap(text.to_string(), theme::semi(11.5), col));
            let pill_w = g.size().x + 14.0;
            let pill_h = g.size().y + 6.0;
            let pill = Rect::from_min_size(Pos2::new(rect.right() - 8.0 - pill_w, y), Vec2::new(pill_w, pill_h));
            p.rect_filled(pill, 5.0, Color32::from_black_alpha(165));
            p.galley(pill.min + Vec2::new(7.0, 3.0), g, col);
            y += pill_h + 5.0;
        }
    }
}
