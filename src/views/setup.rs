//! First run setup: your name, the keypad, your master code, the camera,
//! phone alerts and a few last choices.

use eframe::egui::{self, Color32, Rect, RichText, Sense};

use crate::config::AppConfig;
use crate::engine::types::Model;
use crate::engine::{EngineHandle, EngineMsg};
use crate::notify::NtfyCfg;
use crate::theme;
use crate::vision::{CamCmd, CameraHandle, DEMO_DEVICE};

const STEPS: u8 = 7;

pub struct SetupView {
    step: u8,
    name: String,
    model: Model,
    code1: String,
    code2: String,
    passcode: String,
    error: String,
    tested: bool,
    focus_name: bool,
    face_tex: Option<egui::TextureHandle>,
    face_seq: u64,
}

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::semi(22.0)).color(theme::TEXT));
    ui.add_space(4.0);
}

impl SetupView {
    pub fn new(cfg: &AppConfig) -> SetupView {
        SetupView {
            step: 0,
            name: cfg.admin_name.clone(),
            model: cfg.model,
            code1: String::new(),
            code2: String::new(),
            passcode: String::new(),
            error: String::new(),
            tested: false,
            focus_name: true,
            face_tex: None,
            face_seq: 0,
        }
    }

    /// Returns true once setup is finished.
    pub fn ui(&mut self, ctx: &egui::Context, cfg: &mut AppConfig, changed: &mut bool, engine: &EngineHandle, cam: &CameraHandle) -> bool {
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("setup-dim")).fixed_pos(screen.min).order(egui::Order::Middle).show(ctx, |ui| {
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(175));
            ui.allocate_rect(screen, egui::Sense::click());
        });
        let mut finished = false;
        let max_w = if self.step == 1 { 820.0 } else { 620.0 };
        let win_w = (screen.width() - 64.0).clamp(480.0, max_w);
        egui::Window::new("setup")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([win_w, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(win_w - 16.0);
                ui.set_max_width(win_w - 16.0);
                ui.horizontal(|ui| {
                    for i in 0..STEPS {
                        let w = if i == self.step { 22.0 } else { 8.0 };
                        let (r, _) = ui.allocate_exact_size(egui::vec2(w, 8.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 4.0, if i <= self.step { theme::GREEN } else { theme::LINE });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::muted(format!("Step {} of {}", self.step + 1, STEPS)));
                    });
                });
                ui.add_space(16.0);
                let next_ok = match self.step {
                    0 => self.welcome(ui),
                    1 => self.keypad(ui),
                    2 => self.code(ui),
                    3 => self.camera(ui, cfg, changed, cam),
                    4 => self.face(ui, cfg, changed, cam),
                    5 => self.phone(ui, cfg, changed, engine),
                    _ => self.finish(ui, cfg, changed),
                };
                if !self.error.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(&self.error).color(theme::AMBER));
                }
                ui.add_space(18.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if self.step > 0 && ui.button("Back").clicked() {
                        self.step -= 1;
                        self.error.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.step + 1 == STEPS { "Start RoomGuard" } else { "Next" };
                        let button = egui::Button::new(RichText::new(label).font(theme::semi(14.0)).color(Color32::WHITE))
                            .fill(theme::GREEN.gamma_multiply(0.85));
                        if ui.add_enabled(next_ok, button).clicked() && self.advance(cfg, changed, engine, cam) {
                            finished = true;
                        }
                    });
                });
            });
        finished
    }

    fn advance(&mut self, cfg: &mut AppConfig, changed: &mut bool, engine: &EngineHandle, cam: &CameraHandle) -> bool {
        self.error.clear();
        match self.step {
            0 => {
                cfg.admin_name = self.name.trim().to_string();
                *changed = true;
            }
            1 => {
                cfg.model = self.model;
                *changed = true;
            }
            2 => {
                let digits = self.code1.len() == 4 && self.code1.chars().all(|c| c.is_ascii_digit());
                if !digits {
                    self.error = "The code needs to be exactly four digits.".to_string();
                    return false;
                }
                if self.code1 != self.code2 {
                    self.error = "Those two codes don't match.".to_string();
                    return false;
                }
                engine.send(EngineMsg::SetMasterCode(self.code1.clone()));
                self.code1.clear();
                self.code2.clear();
            }
            4 => {
                // Cancel any half-finished face enrollment if moving on.
                cam.send(CamCmd::CancelEnroll);
            }
            5 => {
                if !self.passcode.trim().is_empty() {
                    cfg.operator.set_passcode(&self.passcode);
                    self.passcode.clear();
                    *changed = true;
                }
            }
            6 => {
                cfg.setup_done = true;
                *changed = true;
                return true;
            }
            _ => {}
        }
        self.step += 1;
        false
    }

    fn welcome(&mut self, ui: &mut egui::Ui) -> bool {
        title(ui, "Welcome to RoomGuard");
        ui.label("RoomGuard turns your webcam into the sensors of a real alarm panel. You arm and disarm it at the keypad on screen, it watches the room, and a monitoring center lets you know when something happens.");
        ui.add_space(16.0);
        ui.label(RichText::new("What should the operator call you?").font(theme::semi(14.0)));
        ui.add_space(4.0);
        let r = ui.add(egui::TextEdit::singleline(&mut self.name).hint_text("Your first name").desired_width(260.0));
        if self.focus_name {
            r.request_focus();
            self.focus_name = false;
        }
        !self.name.trim().is_empty()
    }

    fn keypad(&mut self, ui: &mut egui::Ui) -> bool {
        title(ui, "Pick your keypad");
        ui.label(theme::muted("Each behaves like the real panel it's modelled on. You can switch any time in Settings."));
        ui.add_space(12.0);
        let options = [
            (
                Model::Pc1550,
                "LED keypad (RmG Classic)",
                "PC1550 style · 6 zones",
                [
                    "Eleven recessed status & zone LEDs",
                    "Round rubber keys with F, A, P bar",
                    "Side-hinged ribbed swing cover",
                ],
            ),
            (
                Model::Pc1832,
                "LCD keypad (GuardSeries)",
                "PC1832 style · 8 zones",
                [
                    "Green backlit 2\u{00d7}16 dot-matrix display",
                    "Stay, Away, Chime, Reset & Exit keys",
                    "Bottom-hinged flip cover with guide",
                ],
            ),
            (
                Model::Vista20p,
                "Voice LCD (VistaGuard 20P)",
                "6160V Voice style · 8 zones",
                [
                    "Speaks status & alarms with 6160V voice",
                    "[Code]+Action keys & 2-step alarm clear",
                    "Flip cover with exposed A–D quick keys",
                ],
            ),
        ];
        ui.columns(3, |cols| {
            for (idx, (model, name, subtitle, lines)) in options.iter().enumerate() {
                let col_ui = &mut cols[idx];
                let selected = self.model == *model;
                let fill = if selected { Color32::from_rgb(0x20, 0x38, 0x2D) } else { theme::CARD_HI };
                let stroke = if selected { theme::GREEN } else { theme::LINE };
                let resp = egui::Frame::none()
                    .fill(fill)
                    .stroke(egui::Stroke::new(if selected { 1.8f32 } else { 1.0f32 }, stroke))
                    .rounding(10.0)
                    .inner_margin(egui::Margin::same(14.0))
                    .show(col_ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.set_min_height(130.0);
                            ui.label(RichText::new(*name).font(theme::semi(15.5)).color(theme::TEXT));
                            ui.label(RichText::new(*subtitle).size(12.0).color(if selected { theme::GREEN } else { theme::MUTED }));
                            ui.add_space(8.0);
                            for l in lines {
                                ui.label(theme::muted(format!("\u{2022}  {}", l)));
                            }
                        });
                    })
                    .response;
                if resp.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.model = *model;
                }
            }
        });
        true
    }

    fn code(&mut self, ui: &mut egui::Ui) -> bool {
        title(ui, "Choose your master code");
        ui.label("You type it on the keypad to arm and disarm, just like a real alarm. Pick four digits that aren't easy to guess.");
        ui.add_space(12.0);
        egui::Grid::new("setup-code").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Code");
            ui.add_sized([140.0, 24.0], egui::TextEdit::singleline(&mut self.code1).password(true).char_limit(4));
            ui.end_row();
            ui.label("Once more");
            ui.add_sized([140.0, 24.0], egui::TextEdit::singleline(&mut self.code2).password(true).char_limit(4));
            ui.end_row();
        });
        if self.code1 == "1234" {
            ui.add_space(6.0);
            ui.label(RichText::new("1234 is the factory code everyone knows. Pick something else.").color(theme::AMBER));
        }
        ui.add_space(10.0);
        ui.label(theme::muted(
            "The installer code stays at each panel's factory default (1500 on the LED panel, 5555 on the LCD panel) so you can explore programming. You can change it there.",
        ));
        self.code1.len() == 4 && self.code2.len() == 4 && self.code1 != "1234"
    }

    fn camera(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, cam: &CameraHandle) -> bool {
        title(ui, "Your camera");
        ui.label("RoomGuard watches through your webcam. Point it so it can see the door into the room. No webcam handy? Pick the demo camera to try everything out.");
        ui.add_space(12.0);
        if ui.checkbox(&mut cfg.camera.enabled, "Use a camera").changed() {
            *changed = true;
        }
        ui.add_space(6.0);
        let devices = cam.shared.lock().map(|s| s.devices.clone()).unwrap_or_default();
        ui.horizontal(|ui| {
            let current = devices
                .iter()
                .find(|d| d.0 == cfg.camera.device)
                .map(|d| d.1.clone())
                .unwrap_or_else(|| "Choose a camera".to_string());
            egui::ComboBox::new("setup-camera", "").selected_text(current).width(300.0).show_ui(ui, |ui| {
                for (idx, name) in &devices {
                    if ui.selectable_value(&mut cfg.camera.device, *idx, name).changed() {
                        *changed = true;
                    }
                }
            });
            if ui.button("Look again").clicked() {
                cam.send(CamCmd::Rescan);
            }
        });
        ui.add_space(10.0);
        ui.label(theme::muted(
            "Once it's running you can draw a box around the doorway on the camera card, or let RoomGuard find the doorway by itself as people walk through.",
        ));
        true
    }

    fn face(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, cam: &CameraHandle) -> bool {
        title(ui, "Teach RoomGuard your face (optional)");
        ui.label("RoomGuard can recognize your face offline and disarm automatically during the entry delay. Nothing is ever uploaded.");
        ui.add_space(8.0);
        if ui.checkbox(&mut cfg.camera.faces, "Recognize faces").changed() {
            *changed = true;
        }
        if cfg.camera.faces && ui.checkbox(&mut cfg.camera.face_disarm, "Disarm automatically when it recognizes me during entry delay").changed() {
            *changed = true;
        }

        let (people, enroll, preview) = cam
            .shared
            .lock()
            .map(|s| (s.people.clone(), s.enroll.clone(), s.preview.clone()))
            .unwrap_or_default();

        if !cfg.camera.enabled || cfg.camera.device == DEMO_DEVICE {
            ui.add_space(8.0);
            ui.label(theme::muted(
                "Face training needs a real webcam. You can continue now and train your face later in Settings → Camera.",
            ));
            return true;
        }

        if cfg.camera.faces {
            if let Some(pv) = &preview {
                if pv.seq != self.face_seq && pv.rgb.len() == pv.w * pv.h * 3 {
                    self.face_seq = pv.seq;
                    let img = egui::ColorImage::from_rgb([pv.w, pv.h], &pv.rgb);
                    match &mut self.face_tex {
                        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                        None => self.face_tex = Some(ui.ctx().load_texture("setup-face-preview", img, egui::TextureOptions::LINEAR)),
                    }
                }
            }
            let enrolling_now = matches!(&enroll, Some(e) if !e.done && e.error.is_none());
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                if let (Some(tex), Some(pv)) = (&self.face_tex, &preview) {
                    let pw = 240.0;
                    let ph = (pw * pv.h as f32 / pv.w.max(1) as f32).clamp(135.0, 175.0);
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
                    }
                    let (st, col) = if !pv.ov.faces.is_empty() {
                        ("Face detected", theme::GREEN)
                    } else {
                        ("No face in view", theme::AMBER)
                    };
                    let pill = Rect::from_min_size(rect.left_bottom() + egui::vec2(6.0, -22.0), egui::vec2(104.0, 18.0));
                    p.rect_filled(pill, 4.0, Color32::from_black_alpha(175));
                    p.text(pill.center(), egui::Align2::CENTER_CENTER, st, theme::semi(10.5), col);
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    let who = if self.name.trim().is_empty() { "Owner".to_string() } else { self.name.trim().to_string() };
                    if !people.is_empty() {
                        for (n, samples) in &people {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("Learned: {} ({} samples)", n, samples)).color(theme::GREEN).size(12.5));
                                if ui.small_button("Forget").clicked() {
                                    cam.send(CamCmd::Forget(n.clone()));
                                }
                            });
                        }
                        ui.add_space(4.0);
                    }
                    match &enroll {
                        Some(e) if !e.done && e.error.is_none() => {
                            ui.label(RichText::new(&e.prompt).font(theme::semi(13.5)).color(theme::GREEN));
                            ui.add_space(4.0);
                            ui.add(egui::ProgressBar::new(e.progress).show_percentage().desired_width(220.0));
                            ui.add_space(4.0);
                            if ui.button("Stop").clicked() {
                                cam.send(CamCmd::CancelEnroll);
                            }
                        }
                        other => {
                            if let Some(e) = other {
                                if let Some(err) = &e.error {
                                    ui.label(RichText::new(err).color(theme::AMBER).size(12.0));
                                } else if e.done {
                                    ui.label(RichText::new(format!("Learned {}'s face!", e.name)).color(theme::GREEN).font(theme::semi(13.5)));
                                }
                            }
                            let label = if people.iter().any(|p| p.0 == who) {
                                format!("Retrain {}'s face", who)
                            } else {
                                format!("Learn {}'s face", who)
                            };
                            if super::accent_button(ui, &label, theme::GREEN).clicked() {
                                cam.send(CamCmd::Enroll(who));
                            }
                            ui.add_space(4.0);
                            ui.label(theme::muted("Sit about an arm's length away in good light (~10 seconds), or click Next to skip."));
                        }
                    }
                });
            });
        }
        true
    }

    fn phone(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, engine: &EngineHandle) -> bool {
        title(ui, "Alerts on your phone");
        ui.label("RoomGuard can push alarms, with a camera snapshot, to your phone through ntfy, a free notification app. There's no account to make.");
        ui.add_space(8.0);
        if ui.checkbox(&mut cfg.operator.phone_alerts, "Send alerts to my phone").changed() {
            *changed = true;
        }
        if cfg.operator.phone_alerts {
            let url = NtfyCfg::from(&cfg.operator).topic_url();
            ui.add_space(8.0);
            let avail = ui.available_width();
            ui.horizontal_top(|ui| {
                super::settings::qr(ui, &url, 144.0);
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.set_max_width((avail - 166.0).max(200.0));
                    ui.label(RichText::new("1. Install the free ntfy app").font(theme::semi(13.5)));
                    if ui.link("Get it from ntfy.sh, for Android and iPhone").clicked() {
                        let _ = open::that("https://ntfy.sh");
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("2. Tap + and subscribe to this topic").font(theme::semi(13.5)));
                    ui.label(RichText::new(&cfg.operator.ntfy_topic).font(theme::semi(13.5)).color(theme::GREEN));
                    ui.label(theme::muted("or scan the code with your phone's camera."));
                    ui.add_space(4.0);
                    ui.label(RichText::new("3. Send yourself a test").font(theme::semi(13.5)));
                    if ui.button("Send a test alert").clicked() {
                        engine.send(EngineMsg::TestNotify);
                        self.tested = true;
                    }
                    if self.tested {
                        ui.label(theme::muted("Sent. It should arrive within a few seconds."));
                    }
                });
            });
        }
        ui.add_space(10.0);
        ui.label(RichText::new("A passcode for the operator (optional)").font(theme::semi(13.5)));
        ui.add_space(2.0);
        ui.add(egui::TextEdit::singleline(&mut self.passcode).password(true).hint_text("A word only you know").desired_width(220.0));
        ui.label(theme::muted("When the operator calls about an alarm, they ask for it to make sure it's really you."));
        true
    }

    fn finish(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool) -> bool {
        title(ui, "You're all set");
        let tips = [
            ("To arm", "Type your code on the keypad, then leave through the doorway before the exit delay ends. On the LCD keypad you can also hold Away or Stay for two seconds."),
            ("To disarm", "Come back in and type your code before the entry delay runs out. If you've taught it your face, it can disarm for you."),
            ("If something happens", "The siren sounds, the camera saves a snapshot, and the operator messages you and calls you here."),
        ];
        for (head, text) in tips {
            ui.label(RichText::new(head).font(theme::semi(14.0)));
            ui.label(theme::muted(text));
            ui.add_space(4.0);
        }
        ui.add_space(8.0);
        if ui.checkbox(&mut cfg.autostart, "Open RoomGuard when I sign in to this computer").changed() {
            *changed = true;
        }
        if ui.checkbox(&mut cfg.keep_awake, "Keep the computer awake while armed").changed() {
            *changed = true;
        }
        true
    }
}
