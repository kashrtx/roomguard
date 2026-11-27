//! The main window: status bar on top, camera and zones on the left, the
//! keypad on its wall in the middle, the monitoring center on the right.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::audio::{self, AudioCmd, AudioHandle};
use crate::autostart;
use crate::config::{AppConfig, NRect};
use crate::engine::panel::PanelSave;
use crate::engine::types::{ChatFrom, ChatMsg, EvKind, EventRow, Key, Model, StatusKind};
use crate::engine::{self, EngineHandle, EngineMsg, View};
use crate::keypads::{self, KeyInput};
use crate::notify;
use crate::paint;
use crate::png;
use crate::theme;
use crate::update::{self, UpdateState, Updater};
use crate::util;
use crate::views;
use crate::views::camera::CameraView;
use crate::views::events::EventsView;
use crate::views::operator::OperatorView;
use crate::views::settings::SettingsView;
use crate::views::setup::SetupView;
use crate::vision::{self, CameraHandle};

pub fn window_icon() -> egui::IconData {
    egui::IconData { rgba: include_bytes!("../assets/icon/icon-128.rgba").to_vec(), width: 128, height: 128 }
}

/// Screenshot mode runs in a throwaway data folder with the demo camera and
/// a little history, so the README pictures look like a lived in install.
pub fn prepare_demo_data() {
    let dir = std::env::temp_dir().join(format!("roomguard-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("ROOMGUARD_DATA_DIR", &dir);
    let mut cfg = AppConfig::default();
    cfg.setup_done = true;
    cfg.admin_name = "Kaushal".to_string();
    cfg.model = Model::Pc1832;
    cfg.camera.device = vision::DEMO_DEVICE;
    cfg.camera.mirror = false;
    cfg.camera.faces = false;
    cfg.camera.doorway = Some(NRect::new(0.10, 0.29, 0.19, 0.66));
    cfg.check_updates = false;
    cfg.keep_awake = false;
    cfg.sound.master = 0.0;
    cfg.operator.set_passcode("blue");
    cfg.save();
    let now = util::now_ms();
    let day = 86_400_000i64;
    let mut panel = PanelSave::fresh(Model::Pc1832);
    panel.ee.sections.insert("005.01".to_string(), "005045012".to_string());
    for (ago, kind, text) in [
        (day * 2, EvKind::Arm, "Armed away by master code (Kaushal)"),
        (day * 2 - 3_600_000, EvKind::Disarm, "Disarmed by master code (Kaushal)"),
        (day, EvKind::Trouble, "Trouble: AC failure"),
        (day - 900_000, EvKind::Restore, "Trouble restored: AC failure"),
        (5_400_000, EvKind::Camera, "Door opened (seen by the camera)"),
        (5_390_000, EvKind::Camera, "Door closed (seen by the camera)"),
    ] {
        panel.events.push(EventRow { ts: now - ago, kind, text: text.to_string(), cid: 0, qual: '\0', zu: 0, is_user: false });
    }
    util::save_json(&dir.join("panel-pc1832.json"), &panel);
    let chat = vec![
        ChatMsg { from: ChatFrom::System, text: "Signal received: E301 AC Loss".to_string(), ts: now - day, photo: None },
        ChatMsg {
            from: ChatFrom::Operator,
            text: "Hi Kaushal, your panel just reported a trouble: ac failure. Nothing is in alarm, but you'll want to take a look.".to_string(),
            ts: now - day + 4_000,
            photo: None,
        },
        ChatMsg { from: ChatFrom::System, text: "Signal received: R301 AC Loss".to_string(), ts: now - day + 900_000, photo: None },
    ];
    util::save_json(&dir.join("operator.json"), &serde_json::json!({ "chat": chat }));
}

struct Shots {
    dir: PathBuf,
    stage: u8,
    start: Option<f64>,
    mark: f64,
    awaiting: Option<&'static str>,
}

enum ShotAct {
    Nothing,
    Shoot,
    Model(Model, u8),
    Arm,
    Settings,
    Quit,
}

#[derive(Clone, Copy)]
enum Icon {
    Gear,
    Sun,
    Moon,
}

pub struct RoomGuardApp {
    cfg: AppConfig,
    engine: EngineHandle,
    audio: AudioHandle,
    cam: CameraHandle,
    updater: Updater,
    input: KeyInput,
    camera_view: CameraView,
    operator_view: OperatorView,
    events_view: EventsView,
    settings: SettingsView,
    setup: Option<SetupView>,
    lights: f32,
    save_at: Option<Instant>,
    applied_autostart: bool,
    close_dialog: bool,
    close_code: String,
    close_error: String,
    close_focus: bool,
    closing: bool,
    shut: bool,
    right_tab: u8,
    saw_call: bool,
    update_dismissed: bool,
    want_restart: bool,
    start_minimized: bool,
    shots: Option<Shots>,
    /// Everything was erased; nothing may be saved again before restarting.
    wiped: bool,
    /// True once the automatic fresh empty-room calibration for this app launch has started.
    startup_calib_started: bool,
    /// Last configured camera device ID so switching webcams triggers a fresh calibration.
    last_cam_device: u32,
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
    let resp = resp.on_hover_text(tip);
    let hovered = resp.hovered();
    let p = ui.painter();
    let bg = if hovered { theme::CARD_HI } else { theme::BG };
    if hovered {
        p.rect_filled(rect, 8.0, bg);
    }
    let c = rect.center();
    let col = if hovered { theme::TEXT } else { theme::MUTED };
    match icon {
        Icon::Gear => paint::icon_gear(p, c, 20.0, col),
        Icon::Sun => paint::icon_sun(p, c, 20.0, theme::AMBER),
        Icon::Moon => paint::icon_moon(p, c, 20.0, col, bg),
    }
    resp
}

fn brand(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::hover());
    let p = ui.painter();
    paint::grad_rrect(p, rect, 7.0, Color32::from_rgb(0xEE, 0xE9, 0xDC), Color32::from_rgb(0xCF, 0xC8, 0xB8), None);
    let lights = [theme::GREEN, theme::RED, theme::AMBER];
    for (i, c) in lights.iter().enumerate() {
        p.circle_filled(Pos2::new(rect.left() + 8.0 + i as f32 * 6.0, rect.top() + 8.5), 2.0, *c);
    }
    for row in 0..2 {
        for col in 0..3 {
            let min = Pos2::new(rect.left() + 6.0 + col as f32 * 6.0, rect.top() + 14.0 + row as f32 * 5.5);
            p.rect_filled(Rect::from_min_size(min, Vec2::new(4.0, 3.5)), 1.0, Color32::from_rgb(0x8A, 0x83, 0x75));
        }
    }
    ui.label(RichText::new("RoomGuard").font(theme::semi(17.0)).color(theme::TEXT));
}

impl RoomGuardApp {
    pub fn new(cc: &eframe::CreationContext<'_>, shots: Option<PathBuf>, minimized: bool) -> Self {
        theme::install(&cc.egui_ctx);
        let cfg = AppConfig::load();
        let ctx = cc.egui_ctx.clone();
        let audio = audio::start();
        let (tx, rx) = mpsc::channel();
        let notify = notify::start(tx.clone());
        let cam = vision::start(cfg.camera.clone(), tx.clone(), ctx.clone());
        let engine = engine::start(cfg.clone(), tx, rx, audio.clone(), cam.clone(), notify, ctx.clone());
        let updater = Updater::new();
        if cfg.check_updates && cfg.setup_done && shots.is_none() {
            updater.check(&ctx);
        }
        let setup = if cfg.setup_done { None } else { Some(SetupView::new(&cfg)) };
        let lights = if cfg.ui.lights_on { 1.0 } else { 0.0 };
        let right_tab = cfg.ui.right_tab.min(1);
        let applied_autostart = autostart::is_enabled();
        let last_cam_device = cfg.camera.device;
        RoomGuardApp {
            cfg,
            engine,
            audio,
            cam,
            updater,
            input: KeyInput::default(),
            camera_view: CameraView::default(),
            operator_view: OperatorView::default(),
            events_view: EventsView::default(),
            settings: SettingsView::default(),
            setup,
            lights,
            save_at: None,
            applied_autostart,
            close_dialog: false,
            close_code: String::new(),
            close_error: String::new(),
            close_focus: false,
            closing: false,
            shut: false,
            right_tab,
            saw_call: false,
            update_dismissed: false,
            want_restart: false,
            start_minimized: minimized,
            shots: shots.map(|dir| Shots { dir, stage: 0, start: None, mark: 0.0, awaiting: None }),
            wiped: false,
            startup_calib_started: false,
            last_cam_device,
        }
    }

    /// Erases everything and restarts into the setup wizard.
    fn start_fresh(&mut self, ctx: &egui::Context) {
        util::log("starting over: erasing everything and restarting");
        self.save_at = None;
        self.closing = true;
        self.shutdown();
        self.wiped = true;
        if self.applied_autostart {
            let _ = autostart::set(false);
        }
        util::wipe_data();
        update::relaunch();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn push_cfg(&mut self) {
        if self.cfg.camera.device != self.last_cam_device {
            self.last_cam_device = self.cfg.camera.device;
            self.startup_calib_started = false;
        }
        self.engine.send(EngineMsg::Config(Box::new(self.cfg.clone())));
        self.save_at = Some(Instant::now() + Duration::from_millis(700));
        if self.shots.is_none() && self.cfg.autostart != self.applied_autostart {
            match autostart::set(self.cfg.autostart) {
                Ok(()) => self.applied_autostart = self.cfg.autostart,
                Err(e) => {
                    util::log(&format!("couldn't change the open at sign in setting: {}", e));
                    self.applied_autostart = self.cfg.autostart;
                }
            }
        }
    }

    fn shutdown(&mut self) {
        if self.shut {
            return;
        }
        self.shut = true;
        self.cfg.ui.right_tab = self.right_tab;
        if self.shots.is_none() {
            self.cfg.save();
        }
        // The engine saves the panel and tells the camera to stop. Then wait
        // for the camera to save its room map, and for every queued save to
        // reach the disk, before the window goes away.
        self.engine.shutdown();
        self.cam.send(vision::CamCmd::Shutdown);
        self.cam.wait_stopped(Duration::from_secs(2));
        util::flush_saves(Duration::from_secs(4));
    }

    fn verify_code(&mut self) -> bool {
        let code = std::mem::take(&mut self.close_code);
        let (tx, rx) = mpsc::channel();
        self.engine.send(EngineMsg::VerifyCode { code, reply: tx });
        rx.recv_timeout(Duration::from_millis(800)).unwrap_or(false)
    }

    fn top_bar(&mut self, ctx: &egui::Context, view: &View, layout_locked: bool, changed: &mut bool) {
        egui::TopBottomPanel::top("top-bar")
            .exact_height(56.0)
            .frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::symmetric(16.0, 0.0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    brand(ui);
                    ui.add_space(14.0);
                    let col = views::status_color(view.status);
                    let controls_width = if layout_locked || self.cfg.ui.keypad_only { 225.0 } else { 490.0 };
                    let max_w = (ui.available_width() - controls_width).max(160.0);
                    egui::Frame::none()
                        .fill(col.gamma_multiply(0.14))
                        .rounding(16.0)
                        .inner_margin(egui::Margin::symmetric(12.0, 5.0))
                        .show(ui, |ui| {
                            ui.set_max_width(max_w);
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                                ui.painter().circle_filled(r.center(), 4.5, col);
                                ui.label(RichText::new(&view.status_title).font(theme::semi(14.0)).color(col));
                                if view.following_panel && !view.physical_unavailable {
                                    ui.label(theme::muted("Linked panel"));
                                }
                                if view.armed && self.cfg.operator.quiet_intrusion {
                                    ui.label(theme::muted("Quiet intrusion"));
                                }
                                if !view.status_detail.is_empty() {
                                    ui.add(egui::Label::new(RichText::new(&view.status_detail).color(theme::MUTED).size(13.0)).truncate());
                                }
                            });
                        });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if layout_locked {
                            let message = if view.following_panel { "Layout locked · Disarm at panel" } else { "Layout locked · Disarm to unlock" };
                            ui.label(RichText::new(message).color(theme::AMBER));
                            return;
                        }
                        if self.cfg.ui.keypad_only {
                            if ui.add_sized([140.0, 34.0], egui::Button::new("Exit keypad view")).clicked() {
                                self.cfg.ui.keypad_only = false;
                                *changed = true;
                            }
                            return;
                        }
                        if icon_button(ui, Icon::Gear, "Settings").clicked() {
                            self.settings.open = !self.settings.open;
                        }
                        let layout_menu = ui.scope(|ui| {
                            ui.spacing_mut().interact_size.y = 34.0;
                            ui.menu_button("Layout", |ui| {
                                let prefs = &mut self.cfg.ui;
                                *changed |= ui.checkbox(&mut prefs.show_camera_sidebar, "Camera & zones").changed();
                                *changed |= ui.checkbox(&mut prefs.show_monitoring_sidebar, "Monitoring & history").changed();
                                ui.separator();
                                *changed |= ui.checkbox(&mut prefs.lock_layout_while_armed, "Lock keypad view when armed").changed();
                                if ui.button("Keypad only").clicked() {
                                    prefs.keypad_only = true;
                                    self.settings.open = false;
                                    *changed = true;
                                    ui.close_menu();
                                }
                            }).response
                        }).inner;
                        layout_menu.on_hover_text("Show or hide the sidebars");
                        let on = self.cfg.ui.lights_on;
                        let tip = if on { "Turn the room lights off" } else { "Turn the room lights on" };
                        if icon_button(ui, if on { Icon::Sun } else { Icon::Moon }, tip).clicked() {
                            self.cfg.ui.lights_on = !on;
                            *changed = true;
                        }
                        ui.add_space(6.0);
                        let cover_open = match self.cfg.model {
                            Model::Pc1550 => self.cfg.ui.door_open_1550,
                            Model::Pc1832 => self.cfg.ui.door_open_5500,
                            Model::Vista20p => self.cfg.ui.door_open_6160,
                        };
                        let cover_label = if cover_open { "Close cover" } else { "Open cover" };
                        if ui.button(RichText::new(cover_label).size(12.5)).clicked() {
                            match self.cfg.model {
                                Model::Pc1550 => self.cfg.ui.door_open_1550 = !self.cfg.ui.door_open_1550,
                                Model::Pc1832 => self.cfg.ui.door_open_5500 = !self.cfg.ui.door_open_5500,
                                Model::Vista20p => self.cfg.ui.door_open_6160 = !self.cfg.ui.door_open_6160,
                            }
                            self.audio.send(AudioCmd::Click);
                            *changed = true;
                        }
                        ui.add_space(6.0);
                        let armed = view.armed;
                        let resp = ui
                            .add_enabled_ui(!armed, |ui| {
                                if ui.selectable_label(self.cfg.model == Model::Vista20p, "Voice keypad").clicked() && self.cfg.model != Model::Vista20p {
                                    self.cfg.model = Model::Vista20p;
                                    *changed = true;
                                }
                                if ui.selectable_label(self.cfg.model == Model::Pc1832, "LCD keypad").clicked() && self.cfg.model != Model::Pc1832 {
                                    self.cfg.model = Model::Pc1832;
                                    *changed = true;
                                }
                                if ui.selectable_label(self.cfg.model == Model::Pc1550, "LED keypad").clicked() && self.cfg.model != Model::Pc1550 {
                                    self.cfg.model = Model::Pc1550;
                                    *changed = true;
                                }
                            })
                            .response;
                        if armed {
                            resp.on_disabled_hover_text("Disarm first to switch panels");
                        }
                    });
                });
            });
    }

    fn left_panel(&mut self, ctx: &egui::Context, view: &View, changed: &mut bool) {
        let panel_w = 404.0;
        let inner_w = panel_w - 24.0;
        egui::SidePanel::left("left-panel")
            .exact_width(panel_w)
            .resizable(false)
            .frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::same(12.0)))
            .show(ctx, |ui| {
                ui.set_max_width(inner_w);
                egui::ScrollArea::vertical().id_salt("left-scroll").auto_shrink([false, false]).show(ui, |ui| {
                    let content_w = (ui.available_width() - 2.0).min(inner_w);
                    ui.set_max_width(content_w);
                    theme::card().show(ui, |ui| {
                        let cw = (content_w - 24.0).max(200.0);
                        ui.set_width(cw);
                        ui.set_max_width(cw);
                        self.camera_view.ui(ui, &self.cam, &mut self.cfg.camera, view, changed, self.cfg.ui.show_motion, self.cfg.ui.show_tracks);
                    });
                    ui.add_space(10.0);
                    theme::card().show(ui, |ui| {
                        let cw = (content_w - 24.0).max(200.0);
                        ui.set_width(cw);
                        ui.set_max_width(cw);
                        if let Some(z) = views::sensors::ui(ui, view, &self.cfg.camera, &self.engine) {
                            self.camera_view.start_draw_zone(z);
                        }
                    });
                });
            });
    }

    fn right_panel(&mut self, ctx: &egui::Context, view: &View) {
        let panel_w = 356.0;
        let inner_w = panel_w - 24.0;
        egui::SidePanel::right("right-panel")
            .exact_width(panel_w)
            .resizable(false)
            .frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::same(12.0)))
            .show(ctx, |ui| {
                ui.set_max_width(inner_w);
                ui.horizontal(|ui| {
                    for (i, name) in ["Monitoring", "History"].iter().enumerate() {
                        let label = if i == 0 && view.call.is_some() { format!("{}  \u{2022}", name) } else { name.to_string() };
                        if ui.selectable_label(self.right_tab == i as u8, label).clicked() {
                            self.right_tab = i as u8;
                        }
                    }
                });
                ui.add_space(6.0);
                let h = ui.available_height();
                theme::card().show(ui, |ui| {
                    let cw = (inner_w - 24.0).max(200.0);
                    ui.set_width(cw);
                    ui.set_max_width(cw);
                    ui.set_height((h - 28.0).max(200.0));
                    if self.right_tab == 0 {
                        self.operator_view.ui(ui, view, &self.engine, &self.cfg.operator.operator_name, self.cfg.operator.phone_alerts);
                    } else {
                        self.events_view.ui(ui, view, &self.engine);
                    }
                });
            });
    }

    /// The update banner along the top of the wall. Returns the height it uses.
    fn update_banner(&mut self, ctx: &egui::Context, rect: Rect) -> f32 {
        if self.update_dismissed || self.shots.is_some() {
            return 0.0;
        }
        let state = self.updater.state();
        let (text, kind) = match &state {
            UpdateState::Available(rel) if rel.version != self.cfg.skipped_version => (format!("RoomGuard {} is available", rel.version), 1),
            UpdateState::Downloading { got, total } => {
                let pct = if *total > 0 { format!(", {}%", got * 100 / total) } else { String::new() };
                (format!("Downloading the update{}", pct), 0)
            }
            UpdateState::Ready(msg) => (msg.clone(), 2),
            _ => return 0.0,
        };
        let width = (rect.width() - 32.0).max(260.0);
        egui::Area::new(egui::Id::new("update-banner")).fixed_pos(rect.min + Vec2::new(16.0, 12.0)).show(ctx, |ui| {
            egui::Frame::none()
                .fill(theme::CARD)
                .rounding(8.0)
                .stroke(Stroke::new(1.0f32, theme::LINE))
                .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                .show(ui, |ui| {
                    ui.set_width(width - 24.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(&text).color(theme::TEXT)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match (kind, &state) {
                            (1, UpdateState::Available(rel)) => {
                                if ui.button("Later").clicked() {
                                    self.update_dismissed = true;
                                }
                                if ui.button("Skip").clicked() {
                                    self.cfg.skipped_version = rel.version.clone();
                                    self.update_dismissed = true;
                                    self.save_at = Some(Instant::now());
                                }
                                if views::accent_button(ui, "Update now", theme::GREEN).clicked() {
                                    self.updater.install(rel.clone(), ui.ctx());
                                }
                            }
                            (2, _) => {
                                if ui.button("Later").clicked() {
                                    self.update_dismissed = true;
                                }
                                if !cfg!(target_os = "macos") && views::accent_button(ui, "Restart now", theme::GREEN).clicked() {
                                    self.want_restart = true;
                                }
                            }
                            _ => {}
                        });
                    });
                });
        });
        56.0
    }

    fn wall(&mut self, ui: &mut egui::Ui, view: &View, t: f64, keypad_only: bool, changed: &mut bool) {
        let rect = ui.max_rect();
        let p = ui.painter().clone();
        let l = self.lights;
        // Painted plaster wall gradient + subtle overhead room light glow
        paint::grad_rrect(
            &p,
            rect,
            0.0,
            paint::dim(Color32::from_rgb(0xD8, 0xD2, 0xC5), l),
            paint::dim(Color32::from_rgb(0xC2, 0xBB, 0xAC), l),
            Some(Stroke::NONE),
        );
        if l > 0.02 {
            paint::glow(&p, Pos2::new(rect.center().x, rect.top() - 60.0), rect.width() * 0.85, paint::alpha(Color32::WHITE, 0.11 * l));
        }
        // Wall baseboard trim
        let base = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 20.0), rect.right_bottom());
        paint::grad_rrect(&p, base, 0.0, paint::dim(Color32::from_rgb(0xEA, 0xE7, 0xE0), l), paint::dim(Color32::from_rgb(0xBC, 0xB7, 0xAD), l), Some(Stroke::NONE));
        p.line_segment([base.left_top(), base.right_top()], Stroke::new(1.0f32, paint::dim(Color32::from_rgb(0xAA, 0xA4, 0x99), l)));

        let banner = if keypad_only { 0.0 } else { self.update_banner(ui.ctx(), rect) };
        let bottom_margin = if keypad_only { 22.0 } else { 68.0 };
        let area = Rect::from_min_max(
            Pos2::new(rect.left() + 14.0, rect.top() + banner + 10.0),
            Pos2::new(rect.right() - 14.0, rect.bottom() - bottom_margin),
        );
        let toggled = match view.model {
            Model::Pc1550 => keypads::classic::draw(ui, area, view, &mut self.input, &self.engine, &self.audio, l, t),
            Model::Pc1832 => keypads::lcd::draw(ui, area, view, &mut self.input, &self.engine, &self.audio, l, t),
            Model::Vista20p => keypads::vista::draw(ui, area, view, &mut self.input, &self.engine, &self.audio, l, t),
        };
        if toggled {
            match view.model {
                Model::Pc1550 => self.cfg.ui.door_open_1550 = !self.cfg.ui.door_open_1550,
                Model::Pc1832 => self.cfg.ui.door_open_5500 = !self.cfg.ui.door_open_5500,
                Model::Vista20p => self.cfg.ui.door_open_6160 = !self.cfg.ui.door_open_6160,
            }
            self.audio.send(AudioCmd::Click);
            *changed = true;
        }

        if !keypad_only {
            self.wall_hints(&p, rect, view);
        }
        if view.bell {
            let a = ((t * 3.5).sin() * 0.5 + 0.5) as f32;
            p.rect_stroke(rect.shrink(4.0), 0.0, Stroke::new(8.0f32, paint::alpha(theme::RED, 0.25 + 0.45 * a)));
        }
    }

    fn wall_hints(&self, p: &egui::Painter, rect: Rect, view: &View) {
        let caption = match view.model {
            Model::Pc1550 => "RmG Classic LED Keypad  \u{00b7}  PC1550 compatible  \u{00b7}  Click cover to swing open/closed",
            Model::Pc1832 => "GuardSeries LCD Keypad  \u{00b7}  PC1832 compatible  \u{00b7}  Click cover to flip open/closed",
            Model::Vista20p => "VistaGuard Voice LCD Keypad  \u{00b7}  Vista 20P compatible  \u{00b7}  Click cover to flip open/closed",
        };
        let keys_hint = match view.model {
            Model::Pc1550 => "Keyboard: 0-9, *, #  \u{00b7}  Hold F, A or P for emergency keys",
            Model::Pc1832 => "Keyboard: 0-9, *, #  \u{00b7}  Arrows scroll  \u{00b7}  F1-F5 function keys  \u{00b7}  Hold F, A, P",
            Model::Vista20p => "Commands: [Code]+1 OFF, +2 AWAY, +3 STAY, +9 CHIME  \u{00b7}  # + * speaks status  \u{00b7}  Hold F, P, A",
        };
        // While a menu or delay is running, say what the keypad is waiting for.
        let hint: &str = if view.hint.is_empty() { keys_hint } else { view.hint.as_str() };
        let ink = paint::dim(Color32::from_rgb(0x5E, 0x58, 0x4E), self.lights.max(0.75));
        let max_text_w = (rect.width() - 32.0).max(160.0);
        let cap_galley = p.fonts(|f| f.layout(caption.to_string(), theme::semi(12.0), ink, max_text_w));
        let hint_galley = p.fonts(|f| f.layout(hint.to_string(), theme::regular(11.0), paint::alpha(ink, 0.85), max_text_w));
        let hint_y = rect.bottom() - 25.0 - hint_galley.size().y;
        let cap_y = hint_y - 3.0 - cap_galley.size().y;
        p.galley(Pos2::new(rect.center().x - cap_galley.size().x * 0.5, cap_y), cap_galley, ink);
        p.galley(Pos2::new(rect.center().x - hint_galley.size().x * 0.5, hint_y), hint_galley, paint::alpha(ink, 0.85));
    }

    fn close_prompt(&mut self, ctx: &egui::Context, layout_locked: bool) {
        if !self.close_dialog {
            return;
        }
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("close-dim")).fixed_pos(screen.min).order(egui::Order::Middle).show(ctx, |ui| {
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(160));
            ui.allocate_rect(screen, Sense::click());
        });
        let needs_code = self.cfg.close_needs_code || layout_locked;
        egui::Window::new("close-armed")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new("RoomGuard is armed").font(theme::semi(19.0)));
                ui.add_space(4.0);
                if needs_code {
                    ui.label("Enter your code to close RoomGuard.");
                }
                ui.label("Monitoring stops when RoomGuard closes.");
                ui.add_space(8.0);
                let mut submit = false;
                if needs_code {
                    let r = ui.add(egui::TextEdit::singleline(&mut self.close_code).password(true).hint_text("Your code").desired_width(150.0));
                    if self.close_focus {
                        r.request_focus();
                        self.close_focus = false;
                    }
                    submit = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if !self.close_error.is_empty() {
                        ui.label(RichText::new(&self.close_error).color(theme::AMBER));
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if views::accent_button(ui, "Close RoomGuard", theme::RED).clicked() {
                        submit = true;
                    }
                    if ui.button("Minimize instead").clicked() {
                        self.close_dialog = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    if ui.button("Keep it open").clicked() {
                        self.close_dialog = false;
                    }
                });
                if submit {
                    let ok = if needs_code { !self.close_code.is_empty() && self.verify_code() } else { true };
                    if ok {
                        self.close_dialog = false;
                        self.closing = true;
                        self.shutdown();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        self.close_code.clear();
                        self.close_error = "That code isn't right. Wrong codes count toward the keypad lockout.".to_string();
                        self.close_focus = true;
                    }
                }
            });
    }

    fn drive_shots(&mut self, ctx: &egui::Context, view: &View) {
        let act = {
            let Some(sh) = self.shots.as_mut() else {
                return;
            };
            ctx.request_repaint();
            let t = ctx.input(|i| i.time);
            let start = *sh.start.get_or_insert(t);
            let el = t - start;
            if let Some(name) = sh.awaiting {
                let img = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(img) = img {
                    let mut rgba = Vec::with_capacity(img.pixels.len() * 4);
                    for px in &img.pixels {
                        rgba.extend_from_slice(&px.to_array());
                    }
                    let path = sh.dir.join(format!("{}.png", name));
                    match png::write(&path, img.size[0] as u32, img.size[1] as u32, &rgba) {
                        Ok(()) => util::log(&format!("saved {}", path.display())),
                        Err(e) => util::log(&format!("couldn't save {}: {}", path.display(), e)),
                    }
                    sh.awaiting = None;
                    sh.stage += 1;
                    sh.mark = el;
                } else if el - sh.mark > 15.0 {
                    util::log("a screenshot never arrived, skipping it");
                    sh.awaiting = None;
                    sh.stage += 1;
                    sh.mark = el;
                }
                ShotAct::Nothing
            } else {
                let since = el - sh.mark;
                let mut act = ShotAct::Nothing;
                let mut shoot = None;
                match sh.stage {
                    0 if el > 9.0 => shoot = Some("app-lcd"),
                    1 => {
                        act = ShotAct::Model(Model::Pc1550, 0);
                        sh.stage = 2;
                        sh.mark = el;
                    }
                    2 if since > 3.0 => shoot = Some("app-led"),
                    3 => {
                        act = ShotAct::Model(Model::Pc1832, 1);
                        sh.stage = 4;
                        sh.mark = el;
                    }
                    4 if el > 72.0 => shoot = Some("room-map"),
                    5 => {
                        act = ShotAct::Model(Model::Pc1832, 0);
                        sh.stage = 6;
                        sh.mark = el;
                    }
                    6 if since > 2.0 && view.status == StatusKind::Ready => {
                        act = ShotAct::Arm;
                        sh.stage = 7;
                        sh.mark = el;
                    }
                    7 if view.call.is_some() || since > 110.0 => {
                        sh.stage = 8;
                        sh.mark = el;
                    }
                    8 if since > 2.0 => shoot = Some("alarm"),
                    9 => {
                        act = ShotAct::Settings;
                        sh.stage = 10;
                        sh.mark = el;
                    }
                    10 if since > 1.5 => shoot = Some("settings"),
                    11 => act = ShotAct::Quit,
                    _ => {}
                }
                if let Some(name) = shoot {
                    sh.awaiting = Some(name);
                    sh.mark = el;
                    act = ShotAct::Shoot;
                }
                act
            }
        };
        match act {
            ShotAct::Nothing => {}
            ShotAct::Shoot => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot),
            ShotAct::Model(m, tab) => {
                self.camera_view.tab = tab;
                if self.cfg.model != m {
                    self.cfg.model = m;
                    self.push_cfg();
                }
            }
            ShotAct::Arm => {
                for d in [1u8, 2, 3, 4] {
                    self.engine.send(EngineMsg::Key { key: Key::Digit(d), down: true });
                    self.engine.send(EngineMsg::Key { key: Key::Digit(d), down: false });
                }
            }
            ShotAct::Settings => {
                self.settings.open = true;
                self.settings.tab = 4;
            }
            ShotAct::Quit => {
                self.closing = true;
                self.shutdown();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl Drop for RoomGuardApp {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl eframe::App for RoomGuardApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Never let a UI bug crash the alarm panel: catch any panic, log it
        // (the global panic hook records file & line in roomguard.log), and
        // keep the window alive.
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.update_inner(ctx, frame);
        }));
        if res.is_err() {
            util::log("the UI recovered from an internal error (the panic line above says where)");
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl RoomGuardApp {
    fn update_inner(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let view = self.engine.view();
        let t = ctx.input(|i| i.time);
        if self.start_minimized {
            self.start_minimized = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            if view.armed && self.shots.is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.close_dialog = true;
                self.close_focus = true;
                self.close_error.clear();
            } else {
                self.closing = true;
                self.shutdown();
            }
        }
        if view.call.is_some() && !self.saw_call {
            self.saw_call = true;
            self.right_tab = 0;
        } else if view.call.is_none() {
            self.saw_call = false;
        }
        let target = if self.cfg.ui.lights_on { 1.0 } else { 0.0 };
        if (self.lights - target).abs() > 0.002 {
            self.lights += (target - self.lights) * 0.18;
            ctx.request_repaint();
        } else {
            self.lights = target;
        }
        self.input.sync_doors(ctx, self.cfg.ui.door_open_1550, self.cfg.ui.door_open_5500, self.cfg.ui.door_open_6160);

        let mut changed = false;
        let layout_locked = view.armed && self.cfg.ui.lock_layout_while_armed;
        if layout_locked {
            self.settings.open = false;
        }
        self.top_bar(ctx, &view, layout_locked, &mut changed);
        let keypad_only = layout_locked || self.cfg.ui.keypad_only;
        if !keypad_only && self.cfg.ui.show_camera_sidebar {
            self.left_panel(ctx, &view, &mut changed);
        } else {
            self.camera_view.draw = views::camera::Draw::Off;
        }
        if !keypad_only && self.cfg.ui.show_monitoring_sidebar {
            self.right_panel(ctx, &view);
        }
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG)).show(ctx, |ui| {
            self.wall(ui, &view, t, keypad_only, &mut changed);
        });

        // Trigger a fresh empty-room calibration automatically once per app launch
        // as soon as setup is done, the real webcam is streaming, and the panel is disarmed.
        if !self.startup_calib_started
            && self.setup.is_none()
            && self.shots.is_none()
            && !view.armed
            && self.cfg.camera.enabled
            && self.cfg.camera.device != vision::DEMO_DEVICE
        {
            let ready_for_calib = self
                .cam
                .shared
                .lock()
                .map(|s| matches!(s.status, vision::CamStatus::Running) && s.preview.is_some() && !s.room_calibrated && s.calib.is_none())
                .unwrap_or(false);
            if ready_for_calib {
                self.startup_calib_started = true;
                self.cam.send(vision::CamCmd::CalibrateRoom);
            }
        }

        // Pressing Escape while drawing a zone cancels draw mode instead of pressing '#' on the keypad.
        if self.camera_view.draw != views::camera::Draw::Off && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.camera_view.draw = views::camera::Draw::Off;
        }

        let setup_visible = self.setup.is_some() && !layout_locked;
        let modal = setup_visible || self.settings.open || self.close_dialog || self.camera_view.draw != views::camera::Draw::Off;
        if layout_locked && !self.close_dialog {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
        if !modal && self.input.keyboard(ctx, &self.engine, &self.audio, view.model) {
            // Auto-open the keypad cover when the user types on their physical keyboard.
            match view.model {
                Model::Pc1550 if !self.cfg.ui.door_open_1550 => {
                    self.cfg.ui.door_open_1550 = true;
                    changed = true;
                }
                Model::Pc1832 if !self.cfg.ui.door_open_5500 => {
                    self.cfg.ui.door_open_5500 = true;
                    changed = true;
                }
                Model::Vista20p if !self.cfg.ui.door_open_6160 => {
                    self.cfg.ui.door_open_6160 = true;
                    changed = true;
                }
                _ => {}
            }
        }
        self.settings.window(ctx, &mut self.cfg, &mut changed, &view, &self.engine, &self.cam, &self.audio, &self.updater);
        if self.settings.restart {
            self.settings.restart = false;
            self.want_restart = true;
        }
        if self.settings.rerun_setup {
            self.settings.rerun_setup = false;
            self.setup = Some(SetupView::new(&self.cfg));
        }
        if self.settings.wipe {
            self.settings.wipe = false;
            self.start_fresh(ctx);
            return;
        }
        let mut setup_done = false;
        if let Some(setup) = self.setup.as_mut().filter(|_| !layout_locked) {
            setup_done = setup.ui(ctx, &mut self.cfg, &mut changed, &self.engine, &self.cam);
        }
        if setup_done {
            self.setup = None;
            if self.cfg.check_updates {
                self.updater.check(ctx);
            }
        }
        self.close_prompt(ctx, layout_locked);
        if changed {
            self.push_cfg();
        }
        if self.save_at.map_or(false, |at| Instant::now() >= at) {
            self.save_at = None;
            if self.shots.is_none() && !self.wiped {
                self.cfg.save();
            }
        }
        if self.want_restart {
            self.want_restart = false;
            self.shutdown();
            update::relaunch();
            self.closing = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.drive_shots(ctx, &view);
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        self.cam.set_minimized(minimized);
        let ms = if minimized {
            1000
        } else if view.bell || view.status == StatusKind::Alarm || view.hold.is_some() {
            50
        } else {
            250
        };
        ctx.request_repaint_after(Duration::from_millis(ms));
    }
}
