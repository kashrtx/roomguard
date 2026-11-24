//! The camera card. "Live" shows the picture with what the camera is
//! thinking drawn on top: motion, tracked people with their predicted paths,
//! faces, the doorway and ignored areas. "Room map" shows what RoomGuard has
//! learned about the room and lets you accept or dismiss its suggestions.

use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, Painter, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::config::{CameraCfg, NRect, PtzKind};
use crate::paint;
use crate::theme;
use crate::vision::{self, map, CamCmd, CamStatus, CameraHandle, Overlay};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Draw {
    Off,
    Doorway,
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
}

fn full_uv() -> Rect {
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
}

impl CameraView {
    pub fn ui(&mut self, ui: &mut egui::Ui, cam: &CameraHandle, cfg: &mut CameraCfg, changed: &mut bool, show_motion: bool, show_tracks: bool) {
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
            self.live(ui, &snap, cfg, changed, show_motion, show_tracks);
        } else {
            self.map_ui(ui, &snap, cam, cfg, changed);
        }
    }

    fn live(&mut self, ui: &mut egui::Ui, snap: &Snap, cfg: &mut CameraCfg, changed: &mut bool, show_motion: bool, show_tracks: bool) {
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
                draw_overlay(&p, rect, &pv.ov, &to_screen, show_motion, show_tracks);
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
            p.text(rect.center(), Align2::CENTER_CENTER, msg, theme::regular(13.0), theme::MUTED);
        }

        if self.draw != Draw::Off {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            let hint = if self.draw == Draw::Doorway { "Drag a box around the doorway" } else { "Drag a box around the area to ignore" };
            p.text(rect.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, hint, theme::semi(12.5), theme::BLUE);
            if resp.drag_started() {
                self.drag_from = resp.interact_pointer_pos();
            }
            if let (Some(a), Some(b)) = (self.drag_from, resp.interact_pointer_pos()) {
                if resp.dragged() {
                    p.rect_stroke(Rect::from_two_pos(a, b), 0.0, Stroke::new(2.0, theme::BLUE));
                }
            }
            if resp.drag_stopped() {
                if let (Some(a), Some(b)) = (self.drag_from.take(), resp.interact_pointer_pos()) {
                    let (ax, ay) = from_screen(a);
                    let (bx, by) = from_screen(b);
                    let r = NRect::from_points(ax, ay, bx, by);
                    if r.w > 0.03 && r.h > 0.03 {
                        let r = match snap.xf {
                            Some(x) if x.active => x.rect_view_to_map(r),
                            _ => r,
                        };
                        match self.draw {
                            Draw::Doorway => cfg.doorway = Some(r),
                            Draw::Ignore => cfg.ignore.push(r),
                            Draw::Off => {}
                        }
                        *changed = true;
                    }
                }
                self.draw = Draw::Off;
            }
        }

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let door_label = if cfg.doorway.is_some() { "Redraw doorway" } else { "Mark the doorway" };
            if ui.selectable_label(self.draw == Draw::Doorway, door_label).clicked() {
                self.draw = if self.draw == Draw::Doorway { Draw::Off } else { Draw::Doorway };
            }
            if ui.selectable_label(self.draw == Draw::Ignore, "Ignore an area").clicked() {
                self.draw = if self.draw == Draw::Ignore { Draw::Off } else { Draw::Ignore };
            }
            if cfg.doorway.is_some() && ui.button("Remove doorway").clicked() {
                cfg.doorway = None;
                *changed = true;
            }
            if !cfg.ignore.is_empty() && ui.button("Clear ignored areas").clicked() {
                cfg.ignore.clear();
                *changed = true;
            }
        });
        let mut info = if snap.name.is_empty() { "No camera".to_string() } else { snap.name.clone() };
        if running {
            info.push_str(&format!("  \u{00b7}  {:.0} fps", snap.fps));
        }
        if cfg.doorway.is_none() {
            info.push_str("  \u{00b7}  Without a doorway the whole view is zone 1");
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

    fn map_ui(&mut self, ui: &mut egui::Ui, snap: &Snap, cam: &CameraHandle, cfg: &mut CameraCfg, changed: &mut bool) {
        let ptz = cfg.ptz.kind != PtzKind::Fixed;
        let w = ui.available_width();
        let aspect = if ptz { 0.5 } else { self.size[1] as f32 / self.size[0].max(1) as f32 };
        let h = (w * aspect).clamp(140.0, 380.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 8.0, Color32::from_rgb(0x0D, 0x0F, 0x10));
        if ptz {
            if let Some(t) = &self.pano {
                let band = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.25), Pos2::new(rect.right(), rect.top() + rect.height() * 0.75));
                p.image(t.id(), band, full_uv(), Color32::from_gray(150));
            } else {
                p.text(rect.center(), Align2::CENTER_CENTER, "Press \"Map the room\" and the camera will sweep the room", theme::regular(12.5), theme::MUTED);
            }
        } else if let Some(t) = &self.tex {
            p.image(t.id(), rect, full_uv(), Color32::from_gray(105));
        }
        let to = |x: f32, y: f32| Pos2::new(rect.left() + x * rect.width(), rect.top() + y * rect.height());
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
            paint::dashed_rect(&p, rr, Stroke::new(1.5, theme::BLUE), 5.0);
            p.text(rr.left_top() + Vec2::new(4.0, 3.0), Align2::LEFT_TOP, format!("{}", i + 1), theme::semi(12.0), theme::BLUE);
        }
        if let Some(d) = cfg.doorway {
            let rr = Rect::from_two_pos(to(d.x, d.y), to(d.x + d.w, d.y + d.h));
            paint::dashed_rect(&p, rr, Stroke::new(2.0, theme::GREEN), 7.0);
            p.text(rr.left_bottom() + Vec2::new(4.0, -3.0), Align2::LEFT_BOTTOM, "Doorway", theme::semi(12.0), theme::GREEN);
        }
        for r in &cfg.ignore {
            let rr = Rect::from_two_pos(to(r.x, r.y), to(r.x + r.w, r.y + r.h));
            p.rect_filled(rr, 2.0, Color32::from_rgba_unmultiplied(10, 10, 10, 130));
        }
        if let Some(prog) = snap.mapping {
            let bar = Rect::from_min_size(rect.left_bottom() + Vec2::new(10.0, -16.0), Vec2::new((rect.width() - 20.0) * prog, 5.0));
            p.rect_filled(bar, 2.0, theme::BLUE);
        }

        ui.add_space(6.0);
        if m.tracks_learned == 0 {
            ui.label(theme::muted("Walk in and out of the room a few times. RoomGuard learns where the doors are and how people move by watching, and draws it here."));
        } else {
            ui.label(theme::muted(format!(
                "Learned from {} walk{} through the room. Orange is where things move, blue dots are where people walk.",
                m.tracks_learned,
                if m.tracks_learned == 1 { "" } else { "s" }
            )));
        }
        if cfg.doorway.is_none() {
            if let Some(hint) = m.doorway_hint {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("This looks like your doorway.").color(theme::TEXT));
                    if super::accent_button(ui, "Use it", theme::GREEN).clicked() {
                        cfg.doorway = Some(hint);
                        *changed = true;
                    }
                });
            }
        }
        for (i, portal) in m.portals.iter().enumerate() {
            if portal.dismissed {
                continue;
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}.", i + 1)).color(theme::BLUE).strong());
                ui.label(format!("{} ({} trips)", portal.describe(), portal.uses));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Dismiss").clicked() {
                        cam.send(CamCmd::DismissPortal(i));
                    }
                    if ui.small_button("Use as doorway").clicked() {
                        cfg.doorway = Some(portal.rect());
                        *changed = true;
                    }
                });
            });
        }
        for r in &m.ignore_hints {
            let already = cfg.ignore.iter().any(|q| q.overlap(r) > 0.5);
            if already {
                continue;
            }
            ui.horizontal(|ui| {
                ui.label("Something keeps moving here with nobody near it.");
                if ui.small_button("Ignore that spot").clicked() {
                    cfg.ignore.push(*r);
                    *changed = true;
                }
            });
        }
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
            if cfg.door_contact && cfg.doorway.is_some() {
                let label = if snap.door_learned { "Relearn the closed door" } else { "Learning the closed door..." };
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

fn draw_overlay(p: &Painter, rect: Rect, o: &Overlay, to: &dyn Fn(f32, f32) -> Pos2, show_motion: bool, show_tracks: bool) {
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
        paint::dashed_rect(p, rr, Stroke::new(1.0, theme::FAINT), 5.0);
        p.text(rr.left_top() + Vec2::new(4.0, 3.0), Align2::LEFT_TOP, "Ignored", theme::regular(11.0), theme::MUTED);
    }
    if let Some(d) = o.doorway {
        let rr = Rect::from_two_pos(to(d.x, d.y), to(d.x + d.w, d.y + d.h));
        let col = if o.zone_active[0] {
            theme::RED
        } else if o.door_open {
            theme::AMBER
        } else {
            theme::GREEN
        };
        paint::dashed_rect(p, rr, Stroke::new(2.0, col), 7.0);
        let label = if o.door_open { "Doorway (door open)" } else { "Doorway" };
        p.text(rr.left_bottom() + Vec2::new(4.0, -3.0), Align2::LEFT_BOTTOM, label, theme::semi(12.0), col);
    }
    if show_tracks {
        for f in &o.faces {
            let rr = Rect::from_two_pos(to(f.x, f.y), to(f.x + f.w, f.y + f.h));
            let col = if f.known { theme::GREEN } else { theme::AMBER };
            p.rect_stroke(rr, 3.0, Stroke::new(1.6, col));
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
                p.line_segment([corner, corner + Vec2::new(c * dx, 0.0)], Stroke::new(2.0, col));
                p.line_segment([corner, corner + Vec2::new(0.0, c * dy)], Stroke::new(2.0, col));
            }
            if t.path.len() > 1 {
                let pts: Vec<Pos2> = t.path.iter().map(|q| to(q.0, q.1)).collect();
                p.add(egui::Shape::line(pts, Stroke::new(1.5, col.gamma_multiply(0.5))));
            }
            if !t.predicted.is_empty() {
                let mut pts = vec![to(t.x, t.y)];
                pts.extend(t.predicted.iter().map(|q| to(q.0, q.1)));
                p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.8, theme::BLUE), 6.0, 4.0));
                if let Some(end) = pts.last() {
                    p.circle_stroke(*end, 5.0, Stroke::new(1.5, theme::BLUE));
                }
            }
            let name = if t.label.is_empty() { format!("Person {}", t.id) } else { t.label.clone() };
            let tag = if t.lost { format!("{} (predicted)", name) } else { name };
            p.text(rr.left_top() - Vec2::new(0.0, 3.0), Align2::LEFT_BOTTOM, tag, theme::semi(11.5), col);
        }
    }
    let mut y = rect.top() + 10.0;
    for (on, text, col) in [
        (o.covered, "Camera view blocked", theme::RED),
        (o.light_change, "Lighting changed, relearning the room", theme::AMBER),
    ] {
        if on {
            let pos = Pos2::new(rect.right() - 10.0, y);
            let galley_rect = Rect::from_min_size(Pos2::new(rect.right() - 250.0, y - 2.0), Vec2::new(240.0, 20.0));
            p.rect_filled(galley_rect, 5.0, Color32::from_black_alpha(150));
            p.text(pos, Align2::RIGHT_TOP, text, theme::semi(12.0), col);
            y += 24.0;
        }
    }
}
