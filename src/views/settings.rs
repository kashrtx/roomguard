//! The settings window.

use eframe::egui::{self, Color32, Rect, RichText, Sense};

use crate::audio::{AudioCmd, AudioHandle};
use crate::config::{AppConfig, Lens, PtzKind};
use crate::engine::types::{Model, SirenModel};
use crate::engine::{EngineHandle, EngineMsg, View};
use crate::notify::NtfyCfg;
use crate::theme;
use crate::update::{UpdateState, Updater};
use crate::util;
use crate::vision::{CamCmd, CameraHandle};

const TABS: [&str; 7] = ["General", "Panel", "Camera", "Sound", "Monitoring", "Updates", "About"];

#[derive(Default)]
pub struct SettingsView {
    pub open: bool,
    pub restart: bool,
    pub rerun_setup: bool,
    pub tab: u8,
    new_pass: String,
    enroll_name: String,
    face_tex: Option<egui::TextureHandle>,
    face_seq: u64,
    confirm_wipe: bool,
    /// Set once you confirm erasing everything; the app does the erasing.
    pub wipe: bool,
    panel_ui: super::installer::InstallerState,
}

/// Draws a QR code for `data`.
pub fn qr(ui: &mut egui::Ui, data: &str, size: f32) {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        return;
    };
    let n = code.width();
    let colors = code.to_colors();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 6.0, Color32::WHITE);
    let quiet = 2.0;
    let cell = size / (n as f32 + quiet * 2.0);
    for y in 0..n {
        for x in 0..n {
            if colors[y * n + x] == qrcode::Color::Dark {
                let min = rect.min + egui::vec2((x as f32 + quiet) * cell, (y as f32 + quiet) * cell);
                p.rect_filled(Rect::from_min_size(min, egui::vec2(cell + 0.35, cell + 0.35)), 0.0, Color32::BLACK);
            }
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(theme::heading(title));
    ui.add_space(3.0);
}

impl SettingsView {
    #[allow(clippy::too_many_arguments)]
    pub fn window(&mut self, ctx: &egui::Context, cfg: &mut AppConfig, changed: &mut bool, view: &View, engine: &EngineHandle, cam: &CameraHandle, audio: &AudioHandle, updater: &Updater) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        if self.tab as usize >= TABS.len() {
            self.tab = 0;
        }
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([800.0, 580.0])
            .min_size([680.0, 460.0])
            .show(ctx, |ui| {
                let avail_w = ui.available_width();
                let body_w = (avail_w - 166.0).max(360.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(136.0);
                        for (i, name) in TABS.iter().enumerate() {
                            let selected = self.tab == i as u8;
                            let btn = egui::Button::new(
                                RichText::new(*name)
                                    .font(if selected { theme::semi(13.5) } else { theme::regular(13.5) })
                                    .color(if selected { Color32::WHITE } else { theme::MUTED }),
                            )
                            .fill(if selected { Color32::from_rgb(0x2C, 0x5A, 0x44) } else { Color32::TRANSPARENT })
                            .stroke(if selected { egui::Stroke::new(1.0f32, theme::GREEN) } else { egui::Stroke::NONE })
                            .min_size(egui::vec2(132.0, 28.0));
                            if ui.add(btn).clicked() {
                                self.tab = i as u8;
                            }
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(body_w);
                        ui.set_max_width(body_w);
                        egui::ScrollArea::vertical().id_salt("settings-body").auto_shrink([false, false]).show(ui, |ui| {
                            ui.set_max_width((body_w - 14.0).max(320.0));
                            match self.tab {
                                0 => self.general(ui, cfg, changed, view),
                                1 => super::installer::ui(ui, &view.prog, view.armed, engine, &mut self.panel_ui),
                                2 => self.camera(ui, cfg, changed, cam),
                                3 => self.sound(ui, cfg, changed, audio),
                                4 => self.monitoring(ui, cfg, changed, view, engine),
                                5 => self.updates(ui, cfg, changed, updater),
                                _ => self.about(ui),
                            }
                        });
                    });
                });
            });
        self.open = open;
    }

    fn general(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, view: &View) {
        section(ui, "You");
        ui.horizontal(|ui| {
            ui.label("Your name");
            if ui.add(egui::TextEdit::singleline(&mut cfg.admin_name).desired_width(220.0)).changed() {
                *changed = true;
            }
        });
        ui.label(theme::muted("The operator uses it when they message or call you."));

        section(ui, "Keypad");
        ui.add_enabled_ui(!view.armed, |ui| {
            if ui.radio_value(&mut cfg.model, Model::Pc1550, "LED keypad (RmG Classic, PC1550 compatible, 6 zones)").changed() {
                *changed = true;
            }
            if ui.radio_value(&mut cfg.model, Model::Pc1832, "LCD keypad (GuardSeries, PC1832 compatible, 8 zones)").changed() {
                *changed = true;
            }
        });
        if view.armed {
            ui.label(theme::muted("Disarm first to switch panels."));
        }
        ui.label(theme::muted("Each panel keeps its own codes, programming and history, so you can switch back and forth."));

        section(ui, "Zone names");
        let model = cfg.model;
        let labels = cfg.labels_mut(model);
        egui::Grid::new("settings-zones").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
            for (i, label) in labels.iter_mut().enumerate() {
                ui.label(RichText::new(format!("Zone {}", i + 1)).color(theme::MUTED));
                if ui.add_sized([210.0, 24.0], egui::TextEdit::singleline(label).char_limit(24)).changed() {
                    *changed = true;
                }
                if i == 0 {
                    ui.label(theme::muted("Doorway / whole view"));
                } else if i == 1 {
                    ui.label(theme::muted("Rest of room"));
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
        ui.label(theme::muted("The LCD keypad displays the first 14 characters of each zone name."));

        section(ui, "Graphics");
        if ui.checkbox(&mut cfg.ui.opengl, "Use OpenGL instead of Direct3D, Vulkan or Metal").changed() {
            *changed = true;
        }
        ui.label(theme::muted(
            "RoomGuard draws on your graphics card. Only turn this on if the window looks wrong or won't draw. It takes effect the next time RoomGuard starts.",
        ));

        section(ui, "Camera view");
        if ui.checkbox(&mut cfg.ui.show_motion, "Show motion").changed() {
            *changed = true;
        }
        if ui.checkbox(&mut cfg.ui.show_tracks, "Show tracked people and where they're heading").changed() {
            *changed = true;
        }

        section(ui, "Staying on guard");
        if ui.checkbox(&mut cfg.close_needs_code, "Ask for my code before RoomGuard can be closed while armed").changed() {
            *changed = true;
        }
        if ui.checkbox(&mut cfg.keep_awake, "Keep the computer from sleeping while armed").changed() {
            *changed = true;
        }
        if ui.checkbox(&mut cfg.autostart, "Open RoomGuard when I sign in to this computer").changed() {
            *changed = true;
        }
        ui.label(theme::muted(
            "If RoomGuard is shut down some other way while armed, it comes back armed the next time it starts and lets you know on your phone.",
        ));

        section(ui, "Saved data");
        let dir = util::data_dir();
        ui.label(theme::muted(format!(
            "Codes, faces, the room map and history are saved in {}. Deleting or updating the app never touches this folder.",
            dir.display()
        )));
        ui.horizontal(|ui| {
            if ui.button("Open the data folder").clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = open::that(&dir);
            }
            if ui.button("Run setup wizard again").clicked() {
                self.rerun_setup = true;
                self.open = false;
            }
        });
        ui.label(theme::muted("Running the setup again keeps your codes, faces and history and lets you pick everything else over."));

        section(ui, "Start over");
        ui.label(theme::muted(
            "Erases everything RoomGuard saved on this computer: codes, panel programming, faces, room maps, history and settings. Then it restarts with the setup wizard, like the day you installed it.",
        ));
        if view.armed {
            ui.label(theme::muted("Disarm first to start over."));
        } else if !self.confirm_wipe {
            if ui.button("Erase everything and start over").clicked() {
                self.confirm_wipe = true;
            }
        } else {
            ui.label(RichText::new("This can't be undone. Your codes, faces, room maps and history will be gone.").color(theme::AMBER));
            ui.horizontal(|ui| {
                if super::accent_button(ui, "Erase and start over", theme::RED).clicked() {
                    self.confirm_wipe = false;
                    self.wipe = true;
                    self.open = false;
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_wipe = false;
                }
            });
        }
    }

    fn camera(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, cam: &CameraHandle) {
        let c = &mut cfg.camera;
        let (devices, people, enroll, preview) = cam
            .shared
            .lock()
            .map(|s| (s.devices.clone(), s.people.clone(), s.enroll.clone(), s.preview.clone()))
            .unwrap_or_default();
        section(ui, "Camera");
        if ui.checkbox(&mut c.enabled, "Use the camera as the room's sensors").changed() {
            *changed = true;
        }
        ui.horizontal(|ui| {
            let current = devices
                .iter()
                .find(|d| d.0 == c.device)
                .map(|d| d.1.clone())
                .unwrap_or_else(|| format!("Camera {}", c.device));
            egui::ComboBox::new("camera-device", "").selected_text(current).width(260.0).show_ui(ui, |ui| {
                for (idx, name) in &devices {
                    if ui.selectable_value(&mut c.device, *idx, name).changed() {
                        c.device_name = name.clone();
                        *changed = true;
                    }
                }
            });
            if ui.button("Look again").clicked() {
                cam.send(CamCmd::Rescan);
            }
        });
        if ui.checkbox(&mut c.mirror, "Mirror the picture").changed() {
            *changed = true;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Lens");
            for lens in [Lens::Normal, Lens::Wide, Lens::Fisheye] {
                if ui.radio_value(&mut c.lens, lens, lens.label()).changed() {
                    *changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Motion sensitivity");
            if ui.add(egui::Slider::new(&mut c.sensitivity, 1..=5)).changed() {
                *changed = true;
            }
        });

        section(ui, "Pan and tilt");
        ui.horizontal_wrapped(|ui| {
            for kind in [PtzKind::Fixed, PtzKind::Uvc, PtzKind::Obsbot] {
                if ui.radio_value(&mut c.ptz.kind, kind, kind.label()).changed() {
                    *changed = true;
                }
            }
        });
        if c.ptz.kind != PtzKind::Fixed {
            if ui.checkbox(&mut c.ptz.follow, "Follow people, and search where they went when they get away").changed() {
                *changed = true;
            }
            ui.horizontal(|ui| {
                if ui.checkbox(&mut c.ptz.invert_pan, "Reverse pan").changed() {
                    *changed = true;
                }
                if ui.checkbox(&mut c.ptz.invert_tilt, "Reverse tilt").changed() {
                    *changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Give up searching after");
                if ui.add(egui::Slider::new(&mut c.ptz.search_secs, 5.0..=60.0).suffix(" s")).changed() {
                    *changed = true;
                }
            });
            if c.ptz.kind == PtzKind::Obsbot {
                ui.horizontal_wrapped(|ui| {
                    ui.label("OBSBOT Center address");
                    if ui.add(egui::TextEdit::singleline(&mut c.ptz.obsbot_host).desired_width(130.0)).changed() {
                        *changed = true;
                    }
                    ui.label("port");
                    if ui.add(egui::DragValue::new(&mut c.ptz.obsbot_port).range(1..=65535)).changed() {
                        *changed = true;
                    }
                });
                if ui.checkbox(&mut c.ptz.obsbot_four_args, "Send the four value gimbal command (newer OBSBOT Center versions)").changed() {
                    *changed = true;
                }
                ui.label(theme::muted("In OBSBOT Center, turn on OSC remote control with receive port 16284 and turn off its own AI tracking."));
            }
            if c.ptz.kind == PtzKind::Uvc && cfg!(target_os = "macos") {
                ui.label(RichText::new("macOS doesn't let apps steer USB pan and tilt cameras. Use an OBSBOT through OBSBOT Center instead.").color(theme::AMBER));
            }
            ui.label(theme::muted("After choosing, open the camera's Room map and press Map the room once so it can measure how it turns."));
        }

        section(ui, "Faces");
        if ui.checkbox(&mut c.faces, "Recognize faces").changed() {
            *changed = true;
        }
        ui.add_enabled_ui(c.faces, |ui| {
            if ui.checkbox(&mut c.face_disarm, "Disarm when it recognizes you during the entry delay").changed() {
                *changed = true;
            }
            if ui.checkbox(&mut c.liveness, "Only accept a face that moves (blocks a photo held still)").changed() {
                *changed = true;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Matching");
                for (v, name) in [(1u8, "Relaxed"), (2, "Balanced"), (3, "Strict")] {
                    if ui.radio_value(&mut c.strictness, v, name).changed() {
                        *changed = true;
                    }
                }
            });
            for (name, samples) in &people {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).font(theme::semi(13.5)));
                    ui.label(theme::muted(format!("{} samples", samples)));
                    if ui.small_button("Forget").clicked() {
                        cam.send(CamCmd::Forget(name.clone()));
                    }
                });
            }
            if let Some(pv) = &preview {
                if pv.seq != self.face_seq && pv.rgb.len() == pv.w * pv.h * 3 {
                    self.face_seq = pv.seq;
                    let img = egui::ColorImage::from_rgb([pv.w, pv.h], &pv.rgb);
                    match &mut self.face_tex {
                        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                        None => self.face_tex = Some(ui.ctx().load_texture("settings-face-preview", img, egui::TextureOptions::LINEAR)),
                    }
                }
            }
            let enrolling_now = matches!(&enroll, Some(e) if !e.done && e.error.is_none());
            if c.enabled && (enrolling_now || self.face_tex.is_some()) {
                if let (Some(tex), Some(pv)) = (&self.face_tex, &preview) {
                    ui.add_space(4.0);
                    let pw = 240.0;
                    let ph = (pw * pv.h as f32 / pv.w.max(1) as f32).clamp(120.0, 180.0);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(pw, ph), Sense::hover());
                    let p = ui.painter_at(rect);
                    let uv = Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0));
                    p.image(tex.id(), rect, uv, Color32::WHITE);
                    for f in &pv.ov.faces {
                        let rr = Rect::from_two_pos(
                            egui::Pos2::new(rect.left() + f.x * rect.width(), rect.top() + f.y * rect.height()),
                            egui::Pos2::new(rect.left() + (f.x + f.w) * rect.width(), rect.top() + (f.y + f.h) * rect.height()),
                        );
                        let col = if f.known || enrolling_now { theme::GREEN } else { theme::AMBER };
                        p.rect_stroke(rr, 3.0, egui::Stroke::new(2.0f32, col));
                        if !f.label.is_empty() {
                            p.text(rr.left_top() - egui::vec2(0.0, 2.0), egui::Align2::LEFT_BOTTOM, &f.label, theme::semi(11.0), col);
                        }
                    }
                    let status_txt = if !pv.ov.faces.is_empty() {
                        ("Face detected", theme::GREEN)
                    } else {
                        ("No face in view", theme::AMBER)
                    };
                    let pill = Rect::from_min_size(rect.left_bottom() + egui::vec2(6.0, -22.0), egui::vec2(104.0, 18.0));
                    p.rect_filled(pill, 4.0, Color32::from_black_alpha(175));
                    p.text(pill.center(), egui::Align2::CENTER_CENTER, status_txt.0, theme::semi(10.5), status_txt.1);
                    ui.add_space(4.0);
                }
            }
            match &enroll {
                Some(e) if !e.done && e.error.is_none() => {
                    ui.label(RichText::new(&e.prompt).font(theme::semi(14.0)).color(theme::GREEN));
                    ui.add(egui::ProgressBar::new(e.progress).show_percentage().desired_width(260.0));
                    if ui.button("Stop").clicked() {
                        cam.send(CamCmd::CancelEnroll);
                    }
                }
                other => {
                    if let Some(e) = other {
                        if let Some(err) = &e.error {
                            ui.label(RichText::new(err).color(theme::AMBER));
                        } else if e.done {
                            ui.label(RichText::new(format!("Learned {}'s face.", e.name)).color(theme::GREEN));
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.enroll_name).hint_text("Name").desired_width(140.0));
                        let name = self.enroll_name.trim().to_string();
                        if ui.add_enabled(!name.is_empty(), egui::Button::new("Learn this face")).clicked() {
                            cam.send(CamCmd::Enroll(name));
                        }
                    });
                    ui.label(theme::muted("Sit about an arm's length from the camera in good light. It takes about ten seconds."));
                }
            }
        });

        section(ui, "Room smarts");
        if ui.checkbox(&mut c.learn_map, "Learn the room: doorways, walking paths and things that move on their own").changed() {
            *changed = true;
        }
        if ui.checkbox(&mut c.door_contact, "Treat the doorway like a door contact (an open door trips it even with nobody in view)").changed() {
            *changed = true;
        }
        if ui.checkbox(&mut c.auto_frame, "Zoom the picture in on whoever is moving (fixed cameras)").changed() {
            *changed = true;
        }
    }

    fn sound(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, audio: &AudioHandle) {
        let s = &mut cfg.sound;
        section(ui, "Volume");
        egui::Grid::new("settings-volume").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            for (label, v) in [("Everything", &mut s.master), ("Siren", &mut s.siren), ("Keypad", &mut s.keypad)] {
                ui.label(RichText::new(label).color(theme::MUTED));
                if ui.add(egui::Slider::new(v, 0.0..=1.0).show_value(false)).changed() {
                    *changed = true;
                }
                ui.label(theme::muted(format!("{}%", (*v * 100.0).round() as i32)));
                ui.end_row();
            }
        });
        section(ui, "Siren");
        for m in SirenModel::ALL {
            ui.horizontal(|ui| {
                if ui.radio_value(&mut s.siren_model, m, m.label()).changed() {
                    *changed = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Test").clicked() {
                        audio.send(AudioCmd::Preview { model: m, warble: s.wave2_warble, ms: 3000 });
                    }
                });
            });
            ui.indent(egui::Id::new(("siren-blurb", m)), |ui| {
                ui.label(theme::muted(m.blurb()));
            });
            ui.add_space(2.0);
        }
        if s.siren_model == SirenModel::Wave2F && ui.checkbox(&mut s.wave2_warble, "Wire the WAVE-2F to its warble (yelp) terminal instead of steady").changed() {
            *changed = true;
        }
        section(ui, "Communicator");
        if ui.checkbox(&mut s.line_audio, "Play the phone line sounds while the panel reports an alarm").changed() {
            *changed = true;
        }
        ui.label(theme::muted(
            "Every sound is synthesized live from physical models of the real acoustic hardware (re-entrant horn, motor impeller, ceramic piezo, and 10-inch steel gong).",
        ));
    }

    fn monitoring(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, view: &View, engine: &EngineHandle) {
        let o = &mut cfg.operator;
        section(ui, "Operator");
        if ui.checkbox(&mut o.enabled, "Simulate a monitoring center that messages and calls you about alarms").changed() {
            *changed = true;
        }
        egui::Grid::new("settings-operator").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Operator's name");
            if ui.add_sized([180.0, 24.0], egui::TextEdit::singleline(&mut o.operator_name)).changed() {
                *changed = true;
            }
            ui.end_row();
            ui.label("Passcode");
            ui.horizontal(|ui| {
                ui.add_sized([180.0, 24.0], egui::TextEdit::singleline(&mut self.new_pass).password(true).hint_text(if o.has_passcode() { "set, type to change" } else { "not set" }));
                if ui.add_enabled(self.new_pass.trim().len() >= 3, egui::Button::new("Save")).clicked() {
                    o.set_passcode(&self.new_pass);
                    self.new_pass.clear();
                    *changed = true;
                }
            });
            ui.end_row();
        });
        ui.label(theme::muted("The operator asks for it on the call, like a real monitoring company, to make sure it's really you."));

        section(ui, "Phone alerts");
        if ui.checkbox(&mut o.phone_alerts, "Send alerts to my phone").changed() {
            *changed = true;
        }
        ui.label(theme::muted(
            "Alerts go through ntfy, a free push notification service. Install the ntfy app on your phone and subscribe to your private topic below.",
        ));
        ui.add_space(4.0);
        egui::Grid::new("settings-ntfy").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Topic");
            ui.horizontal(|ui| {
                if ui.add_sized([220.0, 24.0], egui::TextEdit::singleline(&mut o.ntfy_topic)).changed() {
                    *changed = true;
                }
                if ui.button("New topic").clicked() {
                    o.ntfy_topic = format!("roomguard-{}", util::random_token(14));
                    *changed = true;
                }
            });
            ui.end_row();
            ui.label("Server");
            if ui.add_sized([220.0, 24.0], egui::TextEdit::singleline(&mut o.ntfy_server)).changed() {
                *changed = true;
            }
            ui.end_row();
            ui.label("Access token");
            if ui.add_sized([220.0, 24.0], egui::TextEdit::singleline(&mut o.ntfy_token).password(true).hint_text("only for a private server")).changed() {
                *changed = true;
            }
            ui.end_row();
        });
        ui.add_space(6.0);
        let url = NtfyCfg::from(o).topic_url();
        let avail = ui.available_width();
        ui.horizontal_top(|ui| {
            qr(ui, &url, 132.0);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_max_width((avail - 150.0).max(180.0));
                ui.label("Scan this with your phone, or subscribe in the ntfy app to:");
                ui.label(RichText::new(&o.ntfy_topic).font(theme::semi(13.5)).color(theme::GREEN));
                ui.label(theme::muted("Anyone who knows the topic name can read your alerts, so keep it private."));
            });
        });
        ui.add_space(6.0);
        for (v, label) in [
            (&mut o.send_photos, "Attach the camera snapshot"),
            (&mut o.heads_up_on_entry, "Heads up when an entry delay starts"),
            (&mut o.open_close_reports, "Tell me every time it's armed or disarmed"),
            (&mut o.trouble_reports, "Tell me about troubles (power, camera, internet)"),
        ] {
            if ui.checkbox(v, label).changed() {
                *changed = true;
            }
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Send a test alert").clicked() {
                engine.send(EngineMsg::TestNotify);
            }
            if !view.notify_status.is_empty() {
                ui.label(theme::muted(view.notify_status.clone()));
            }
        });
    }

    fn updates(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, updater: &Updater) {
        section(ui, "Updates");
        ui.label(format!("You have version {}.", env!("CARGO_PKG_VERSION")));
        if ui.checkbox(&mut cfg.check_updates, "Check for updates when RoomGuard starts").changed() {
            *changed = true;
        }
        let state = updater.state();
        let busy = matches!(state, UpdateState::Checking | UpdateState::Downloading { .. });
        if ui.add_enabled(!busy, egui::Button::new("Check now")).clicked() {
            updater.check(ui.ctx());
        }
        match state {
            UpdateState::Idle => {}
            UpdateState::Checking => {
                ui.label(theme::muted("Checking..."));
            }
            UpdateState::UpToDate => {
                ui.label(RichText::new("You're up to date.").color(theme::GREEN));
            }
            UpdateState::NoRepo => {
                ui.label(theme::muted("This copy wasn't built by the release workflow, so it doesn't know where updates come from."));
            }
            UpdateState::Failed(e) => {
                ui.label(RichText::new(format!("Couldn't check: {}", e)).color(theme::AMBER));
            }
            UpdateState::Available(rel) => {
                ui.label(RichText::new(format!("Version {} is available.", rel.version)).font(theme::semi(14.0)));
                egui::ScrollArea::vertical().id_salt("release-notes").max_height(180.0).show(ui, |ui| {
                    ui.label(if rel.notes.trim().is_empty() { "No release notes." } else { rel.notes.as_str() });
                });
                ui.horizontal(|ui| {
                    if super::accent_button(ui, "Download and install", theme::GREEN).clicked() {
                        updater.install(rel.clone(), ui.ctx());
                    }
                    if ui.button("Open the release page").clicked() {
                        let _ = open::that(&rel.page);
                    }
                });
            }
            UpdateState::Downloading { got, total } => {
                let frac = if total > 0 { got as f32 / total as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(frac).show_percentage().desired_width(260.0));
            }
            UpdateState::Ready(msg) => {
                ui.label(RichText::new(msg).color(theme::GREEN));
                if !cfg!(target_os = "macos") && ui.button("Restart now").clicked() {
                    self.restart = true;
                }
            }
        }
    }

    fn about(&mut self, ui: &mut egui::Ui) {
        section(ui, "RoomGuard");
        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
        ui.label("Copyright 2026 Kaushal Bhingaradia. Licensed under the Apache License 2.0.");
        ui.add_space(6.0);
        ui.label(theme::muted(
            "RoomGuard emulates how the classic PC1550 (RmG Classic) and PC1832 (GuardSeries) alarm panels behave. It isn't made by, endorsed by or affiliated with any hardware manufacturer. All keypad graphics are rendered procedurally in code.",
        ));
        section(ui, "Built with");
        for line in [
            "Inter typeface, SIL Open Font License 1.1",
            "OpenMMLab RTMDet-Nano and RTMPose-Tiny (Apache 2.0) via tract-onnx",
            "OpenCV's LBP frontal face cascade, see NOTICE",
            "egui and eframe, MIT or Apache 2.0",
            "nokhwa, cpal, ureq and other Rust crates, see Cargo.toml",
        ] {
            ui.label(theme::muted(line));
        }
        ui.add_space(6.0);
        if ui.button("Open the log file").clicked() {
            let _ = open::that(util::data_dir().join("roomguard.log"));
        }
    }
}
