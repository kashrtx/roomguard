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

const TABS: [&str; 6] = ["General", "Camera", "Sound", "Monitoring", "Updates", "About"];

#[derive(Default)]
pub struct SettingsView {
    pub open: bool,
    pub restart: bool,
    pub tab: u8,
    new_pass: String,
    enroll_name: String,
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
    ui.add_space(2.0);
}

impl SettingsView {
    #[allow(clippy::too_many_arguments)]
    pub fn window(&mut self, ctx: &egui::Context, cfg: &mut AppConfig, changed: &mut bool, view: &View, engine: &EngineHandle, cam: &CameraHandle, audio: &AudioHandle, updater: &Updater) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([760.0, 560.0])
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(140.0);
                        for (i, name) in TABS.iter().enumerate() {
                            if ui.selectable_label(self.tab == i as u8, *name).clicked() {
                                self.tab = i as u8;
                            }
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical().id_salt("settings-body").auto_shrink([false, false]).show(ui, |ui| match self.tab {
                            0 => self.general(ui, cfg, changed, view),
                            1 => self.camera(ui, cfg, changed, cam),
                            2 => self.sound(ui, cfg, changed, audio),
                            3 => self.monitoring(ui, cfg, changed, view, engine),
                            4 => self.updates(ui, cfg, changed, updater),
                            _ => self.about(ui),
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
            if ui.text_edit_singleline(&mut cfg.admin_name).changed() {
                *changed = true;
            }
        });
        ui.label(theme::muted("The operator uses it when they message or call you."));

        section(ui, "Keypad");
        ui.add_enabled_ui(!view.armed, |ui| {
            if ui.radio_value(&mut cfg.model, Model::Pc1550, "LED keypad, works like a DSC PC1550 (six zones, lights only)").changed() {
                *changed = true;
            }
            if ui.radio_value(&mut cfg.model, Model::Pc1832, "LCD keypad, works like a DSC PC1832 (eight zones, messages on screen)").changed() {
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
        for (i, label) in labels.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Zone {}", i + 1)).color(theme::MUTED));
                if ui.add(egui::TextEdit::singleline(label).char_limit(24).desired_width(220.0)).changed() {
                    *changed = true;
                }
                if i == 0 {
                    ui.label(theme::muted("the doorway (or the whole camera view)"));
                } else if i == 1 {
                    ui.label(theme::muted("the rest of the room, once a doorway is marked"));
                }
            });
        }
        ui.label(theme::muted("The LCD keypad shows the first 14 letters."));

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
        if ui.button("Open the data folder").clicked() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = open::that(&dir);
        }
    }

    fn camera(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, cam: &CameraHandle) {
        let c = &mut cfg.camera;
        let (devices, people, enroll) = cam
            .shared
            .lock()
            .map(|s| (s.devices.clone(), s.people.clone(), s.enroll.clone()))
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
        ui.horizontal(|ui| {
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
        ui.horizontal(|ui| {
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
                ui.horizontal(|ui| {
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
            ui.horizontal(|ui| {
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
            match &enroll {
                Some(e) if !e.done && e.error.is_none() => {
                    ui.label(RichText::new(&e.prompt).font(theme::semi(14.0)).color(theme::GREEN));
                    ui.add(egui::ProgressBar::new(e.progress).desired_width(260.0));
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
        for (label, v) in [("Everything", &mut s.master), ("Siren", &mut s.siren), ("Keypad", &mut s.keypad)] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(theme::MUTED));
                if ui.add(egui::Slider::new(v, 0.0..=1.0).show_value(false)).changed() {
                    *changed = true;
                }
            });
        }
        section(ui, "Siren");
        for m in SirenModel::ALL {
            ui.horizontal(|ui| {
                if ui.radio_value(&mut s.siren_model, m, m.label()).changed() {
                    *changed = true;
                }
                ui.label(theme::muted(m.blurb()));
                if ui.small_button("Test").clicked() {
                    audio.send(AudioCmd::Preview { model: m, warble: s.wave2_warble, ms: 3000 });
                }
            });
        }
        if s.siren_model == SirenModel::Wave2F && ui.checkbox(&mut s.wave2_warble, "Wire the WAVE-2F to its warble terminal instead of steady").changed() {
            *changed = true;
        }
        section(ui, "Communicator");
        if ui.checkbox(&mut s.line_audio, "Play the phone line sounds while the panel reports an alarm").changed() {
            *changed = true;
        }
        ui.label(theme::muted(
            "Every sound is synthesized from how the real device makes it. DSC doesn't publish its keypad buzzer frequencies, so those two are close estimates.",
        ));
    }

    fn monitoring(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, view: &View, engine: &EngineHandle) {
        let o = &mut cfg.operator;
        section(ui, "Operator");
        if ui.checkbox(&mut o.enabled, "Simulate a monitoring center that messages and calls you about alarms").changed() {
            *changed = true;
        }
        ui.horizontal(|ui| {
            ui.label("Operator's name");
            if ui.add(egui::TextEdit::singleline(&mut o.operator_name).desired_width(140.0)).changed() {
                *changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Passcode");
            ui.add(egui::TextEdit::singleline(&mut self.new_pass).password(true).hint_text(if o.has_passcode() { "set, type to change" } else { "not set" }).desired_width(160.0));
            if ui.add_enabled(self.new_pass.trim().len() >= 3, egui::Button::new("Save")).clicked() {
                o.set_passcode(&self.new_pass);
                self.new_pass.clear();
                *changed = true;
            }
        });
        ui.label(theme::muted("The operator asks for it on the call, like a real monitoring company, to make sure it's really you."));

        section(ui, "Phone alerts");
        if ui.checkbox(&mut o.phone_alerts, "Send alerts to my phone").changed() {
            *changed = true;
        }
        ui.label(theme::muted(
            "Alerts go through ntfy, a free push notification service. Install the ntfy app on your phone and subscribe to your private topic below.",
        ));
        ui.horizontal(|ui| {
            ui.label("Topic");
            if ui.add(egui::TextEdit::singleline(&mut o.ntfy_topic).desired_width(240.0)).changed() {
                *changed = true;
            }
            if ui.button("New topic").clicked() {
                o.ntfy_topic = format!("roomguard-{}", util::random_token(14));
                *changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Server");
            if ui.add(egui::TextEdit::singleline(&mut o.ntfy_server).desired_width(240.0)).changed() {
                *changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Access token");
            if ui.add(egui::TextEdit::singleline(&mut o.ntfy_token).password(true).hint_text("only for a private server").desired_width(200.0)).changed() {
                *changed = true;
            }
        });
        let url = NtfyCfg::from(o).topic_url();
        ui.horizontal(|ui| {
            qr(ui, &url, 132.0);
            ui.vertical(|ui| {
                ui.label("Scan this with your phone, or subscribe in the ntfy app to:");
                ui.label(RichText::new(&o.ntfy_topic).font(theme::semi(13.5)));
                ui.label(theme::muted("Anyone who knows the topic name can read your alerts, so keep it private."));
            });
        });
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
        ui.horizontal(|ui| {
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
            "RoomGuard emulates how the DSC PC1550 and PC1832 alarm panels behave. It isn't made by, endorsed by or affiliated with DSC. The keypads drawn here are original designs.",
        ));
        section(ui, "Built with");
        for line in [
            "Inter typeface, SIL Open Font License 1.1",
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
