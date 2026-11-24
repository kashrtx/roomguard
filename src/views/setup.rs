//! First run setup: your name, the keypad, your master code, the camera,
//! phone alerts and a few last choices.

use eframe::egui::{self, Color32, RichText};

use crate::config::AppConfig;
use crate::engine::types::Model;
use crate::engine::{EngineHandle, EngineMsg};
use crate::notify::NtfyCfg;
use crate::theme;
use crate::vision::{CamCmd, CameraHandle};

const STEPS: u8 = 6;

pub struct SetupView {
    step: u8,
    name: String,
    model: Model,
    code1: String,
    code2: String,
    passcode: String,
    error: String,
    tested: bool,
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
        egui::Window::new("setup")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(580.0)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(580.0);
                ui.set_min_height(440.0);
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
                ui.add_space(14.0);
                let next_ok = match self.step {
                    0 => self.welcome(ui),
                    1 => self.keypad(ui),
                    2 => self.code(ui),
                    3 => self.camera(ui, cfg, changed, cam),
                    4 => self.phone(ui, cfg, changed, engine),
                    _ => self.finish(ui, cfg, changed),
                };
                if !self.error.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(&self.error).color(theme::AMBER));
                }
                ui.add_space(14.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if self.step > 0 && ui.button("Back").clicked() {
                        self.step -= 1;
                        self.error.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.step + 1 == STEPS { "Start RoomGuard" } else { "Next" };
                        let button = egui::Button::new(RichText::new(label).font(theme::semi(14.0)).color(Color32::WHITE)).fill(theme::GREEN.gamma_multiply(0.85));
                        if ui.add_enabled(next_ok, button).clicked() && self.advance(cfg, changed, engine) {
                            finished = true;
                        }
                    });
                });
            });
        finished
    }

    fn advance(&mut self, cfg: &mut AppConfig, changed: &mut bool, engine: &EngineHandle) -> bool {
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
                if !self.passcode.trim().is_empty() {
                    cfg.operator.set_passcode(&self.passcode);
                    self.passcode.clear();
                    *changed = true;
                }
            }
            5 => {
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
        let r = ui.add(egui::TextEdit::singleline(&mut self.name).hint_text("Your first name").desired_width(260.0));
        if self.name.is_empty() && !r.has_focus() {
            r.request_focus();
        }
        !self.name.trim().is_empty()
    }

    fn keypad(&mut self, ui: &mut egui::Ui) -> bool {
        title(ui, "Pick your keypad");
        ui.label(theme::muted("Both behave like the real panels they're modelled on. You can switch any time in Settings."));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let options = [
                (Model::Pc1550, "LED keypad", ["Works like a DSC PC1550 from the 1990s", "Lights only, six zones", "Chunky keys with an amber backlight"]),
                (Model::Pc1832, "LCD keypad", ["Works like a DSC PowerSeries PC1832", "Messages on a green screen, eight zones", "Stay, Away and other function keys"]),
            ];
            for (model, name, lines) in options {
                let selected = self.model == model;
                let fill = if selected { Color32::from_rgb(0x22, 0x3A, 0x2F) } else { theme::CARD_HI };
                let stroke = if selected { theme::GREEN } else { theme::LINE };
                let resp = egui::Frame::none()
                    .fill(fill)
                    .stroke(egui::Stroke::new(1.5, stroke))
                    .rounding(10.0)
                    .inner_margin(egui::Margin::same(14.0))
                    .show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.label(RichText::new(name).font(theme::semi(16.0)));
                        ui.add_space(4.0);
                        for l in lines {
                            ui.label(theme::muted(l));
                        }
                    })
                    .response;
                if resp.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.model = model;
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
            ui.add(egui::TextEdit::singleline(&mut self.code1).password(true).char_limit(4).desired_width(120.0));
            ui.end_row();
            ui.label("Once more");
            ui.add(egui::TextEdit::singleline(&mut self.code2).password(true).char_limit(4).desired_width(120.0));
            ui.end_row();
        });
        if self.code1 == "1234" {
            ui.label(RichText::new("1234 is the factory code everyone knows. Pick something else.").color(theme::AMBER));
        }
        ui.add_space(8.0);
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

    fn phone(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, engine: &EngineHandle) -> bool {
        title(ui, "Alerts on your phone");
        ui.label("RoomGuard can push alarms, with a camera snapshot, to your phone through ntfy, a free notification app. There's no account to make.");
        ui.add_space(8.0);
        if ui.checkbox(&mut cfg.operator.phone_alerts, "Send alerts to my phone").changed() {
            *changed = true;
        }
        if cfg.operator.phone_alerts {
            let url = NtfyCfg::from(&cfg.operator).topic_url();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                super::settings::qr(ui, &url, 150.0);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("1. Install the free ntfy app").font(theme::semi(13.5)));
                    if ui.link("Get it from ntfy.sh, for Android and iPhone").clicked() {
                        let _ = open::that("https://ntfy.sh");
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("2. Tap + and subscribe to this topic").font(theme::semi(13.5)));
                    ui.label(RichText::new(&cfg.operator.ntfy_topic).font(theme::semi(14.0)).color(theme::GREEN));
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
