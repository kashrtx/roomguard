//! Home Assistant setup, two-way alarm panel sync, wireless sensor-to-zone rules,
//! and on-demand camera previews.

use std::f32::consts::TAU;

use eframe::egui::{self, Color32, Pos2, RichText, Sense, Stroke, Vec2};

use crate::config::AppConfig;
use crate::engine::types::ZoneKind;
use crate::engine::{EngineHandle, EngineMsg, View};
use crate::home_assistant::{AlarmState, Binding, ConnectionState, PanelSyncMode, Rule};
use crate::theme;

#[derive(Default)]
pub struct HomeUi {
    credentials: Option<(String, String)>,
    filter: String,
    entity_id: String,
    zone: u8,
    rule: Option<Rule>,
    operand: String,
    texture: Option<(std::sync::Arc<crate::vision::frame::Rgb>, egui::TextureHandle)>,
}

/// Paints a crisp 18x18 vector status badge: green checkmark, spinning blue/amber
/// connecting ring, or red X for automatic/manual disconnect.
fn status_badge(ui: &mut egui::Ui, state: &ConnectionState) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
    let p = ui.painter_at(rect);
    let c = rect.center();
    let r = 8.5;
    match state {
        ConnectionState::Connected => {
            p.circle_filled(c, r, theme::GREEN);
            let s = Stroke::new(2.1f32, Color32::WHITE);
            let a = Pos2::new(c.x - 4.0, c.y + 0.3);
            let b = Pos2::new(c.x - 1.2, c.y + 3.2);
            let d = Pos2::new(c.x + 4.3, c.y - 3.1);
            p.line_segment([a, b], s);
            p.line_segment([b, d], s);
        }
        ConnectionState::Connecting => {
            ui.ctx().request_repaint();
            let t = ui.input(|i| i.time) as f32;
            p.circle_stroke(c, r - 1.0, Stroke::new(2.0f32, theme::LINE));
            let start = t * 4.5;
            let sweep = TAU * 0.34;
            let steps = 14;
            let pts: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let ang = start + sweep * (i as f32 / steps as f32);
                    Pos2::new(c.x + (r - 1.0) * ang.cos(), c.y + (r - 1.0) * ang.sin())
                })
                .collect();
            p.add(egui::Shape::line(pts, Stroke::new(2.4f32, theme::BLUE)));
        }
        ConnectionState::Disconnected | ConnectionState::Error(_) => {
            p.circle_filled(c, r, theme::RED);
            let s = Stroke::new(2.0f32, Color32::WHITE);
            let d = 3.5;
            p.line_segment([Pos2::new(c.x - d, c.y - d), Pos2::new(c.x + d, c.y + d)], s);
            p.line_segment([Pos2::new(c.x + d, c.y - d), Pos2::new(c.x - d, c.y + d)], s);
        }
    }
}

fn live_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

impl HomeUi {
    pub fn ui(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, view: &View, engine: &EngineHandle) {
        let home = &mut cfg.home_assistant;
        ui.label(theme::heading("Home Assistant"));
        ui.add_space(4.0);

        // Live connection status banner with clear visual icon.
        let conn_state = if !home.enabled {
            ConnectionState::Disconnected
        } else {
            view.home.state.clone()
        };
        let (title, title_color, subtitle) = match &conn_state {
            ConnectionState::Connected => {
                let count = view.home.entities.len();
                (
                    "Connected",
                    theme::GREEN,
                    format!("{count} entities live"),
                )
            }
            ConnectionState::Connecting => (
                "Connecting...",
                theme::BLUE,
                "Connecting to Home Assistant...".to_string(),
            ),
            ConnectionState::Error(err) => (
                "Disconnected",
                theme::RED,
                format!("{err} (retrying automatically)"),
            ),
            ConnectionState::Disconnected => (
                "Disconnected",
                theme::RED,
                if !home.url.trim().is_empty() && !home.token.trim().is_empty() {
                    "Manually disconnected".to_string()
                } else {
                    "Enter your Home Assistant URL and token below to connect".to_string()
                },
            ),
        };

        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                status_badge(ui, &conn_state);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).font(theme::semi(14.5)).color(title_color));
                    ui.label(theme::muted(subtitle));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if home.enabled {
                        if ui
                            .add_enabled(!view.armed, egui::Button::new("Disconnect"))
                            .on_disabled_hover_text("Disarm RoomGuard before disconnecting")
                            .clicked()
                        {
                            home.enabled = false;
                            *changed = true;
                        }
                    } else if !home.url.trim().is_empty() && !home.token.trim().is_empty() {
                        if ui
                            .add_enabled(!view.armed, egui::Button::new("Reconnect"))
                            .on_disabled_hover_text("Disarm RoomGuard before connecting")
                            .clicked()
                        {
                            home.enabled = true;
                            *changed = true;
                        }
                    }
                });
            });
        });

        if view.armed {
            ui.add_space(4.0);
            ui.label(RichText::new("Disarm RoomGuard before changing the connection, linked panel, or zone assignments.").color(theme::AMBER));
        }

        ui.add_space(6.0);
        egui::CollapsingHeader::new("Connection")
            .id_salt("ha-connection-section")
            .default_open(!view.home.online)
            .show(ui, |ui| {
                ui.add_enabled_ui(!view.armed, |ui| {
                    let (url, token) = self.credentials.get_or_insert_with(|| (home.url.clone(), home.token.clone()));
                    ui.label("Home Assistant URL");
                    ui.add(
                        egui::TextEdit::singleline(url)
                            .hint_text("http://homeassistant.local:8123 or http://192.168.1.45:8123")
                            .desired_width(380.0),
                    );
                    ui.label("Long-lived access token");
                    ui.add(
                        egui::TextEdit::singleline(token)
                            .password(true)
                            .hint_text("Paste token from HA Profile -> Security")
                            .desired_width(380.0),
                    );
                    ui.label(theme::muted(
                        "In Home Assistant: click your profile (bottom-left) -> Security -> Long-lived access tokens -> Create token.",
                    ));
                    ui.horizontal(|ui| {
                        let ready = !url.trim().is_empty() && !token.trim().is_empty();
                        if ui.add_enabled(ready, egui::Button::new("Connect")).clicked() {
                            home.url = url.trim().trim_end_matches('/').to_owned();
                            home.token = token.trim().to_owned();
                            home.enabled = true;
                            *changed = true;
                        }
                        if ui.add_enabled(home.enabled, egui::Button::new("Disconnect")).clicked() {
                            home.enabled = false;
                            *changed = true;
                        }
                    });
                });
            });

        ui.add_space(8.0);
        if ui.checkbox(&mut cfg.camera.enabled, "Use webcam").changed() {
            *changed = true;
        }
        if !cfg.camera.enabled {
            ui.label(theme::muted("Webcam capture is off. Home Assistant wireless sensors, panel sync, and phone alerts still work."));
        }

        // --- Section 2: Alarm Panel Sync (Two-Way or Read-Only Follow) ---
        ui.add_space(12.0);
        ui.label(theme::heading("Alarm panel sync"));
        ui.label(theme::muted(
            "Link a Home Assistant alarm panel (like a real house panel or RoomGuard Test Panel) so arming and disarming stay in sync.",
        ));
        let partition = home.follow_panel.as_deref().and_then(|id| view.home.entities.get(id));
        let panel_label = partition
            .map(|entity| format!("{} ({})", entity.name(), entity.entity_id))
            .or_else(|| home.follow_panel.clone())
            .unwrap_or_else(|| "Use RoomGuard independently".to_owned());
        ui.add_enabled_ui(!view.armed, |ui| {
            egui::ComboBox::from_id_salt("ha-physical-panel")
                .selected_text(panel_label)
                .width(380.0)
                .show_ui(ui, |ui| {
                    *changed |= ui
                        .selectable_value(&mut home.follow_panel, None, "Use RoomGuard independently")
                        .changed();
                    if view.home.online {
                        for entity in view.home.entities.values().filter(|entity| entity.alarm_panel()) {
                            *changed |= ui
                                .selectable_value(
                                    &mut home.follow_panel,
                                    Some(entity.entity_id.clone()),
                                    format!("{} ({})", entity.name(), entity.entity_id),
                                )
                                .changed();
                        }
                    }
                });
            if home.follow_panel.is_some() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .radio_value(&mut home.panel_sync, PanelSyncMode::TwoWay, "Two-way sync (recommended)")
                        .changed()
                    {
                        *changed = true;
                    }
                    if ui
                        .radio_value(&mut home.panel_sync, PanelSyncMode::FollowOnly, "Follow physical panel only")
                        .changed()
                    {
                        *changed = true;
                    }
                });
                if home.panel_sync == PanelSyncMode::TwoWay {
                    ui.horizontal(|ui| {
                        ui.label("HA panel PIN");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut home.panel_code)
                                    .password(true)
                                    .hint_text("1234 for Test Panel (or blank)")
                                    .desired_width(210.0),
                            )
                            .changed()
                        {
                            *changed = true;
                        }
                    });
                    ui.label(theme::muted(
                        "Arm or disarm from EITHER RoomGuard's keypad OR Home Assistant. Leave PIN blank to forward the code you type on RoomGuard's keypad.",
                    ));
                } else {
                    ui.label(theme::muted(
                        "Read-only mirror: arm and disarm from the real panel. RoomGuard's keypad shows its state while still guarding your room zones.",
                    ));
                }
            }
        });

        if home.follow_panel.is_some() && home.enabled {
            if view.physical_unavailable {
                ui.horizontal(|ui| {
                    live_dot(ui, theme::AMBER);
                    ui.label(RichText::new("Linked panel unavailable. RoomGuard keeps its last known arming mode.").color(theme::AMBER));
                });
            } else if let Some(state) = partition.and_then(|entity| AlarmState::parse(&entity.state)) {
                let col = match state {
                    AlarmState::Disarmed => theme::GREEN,
                    AlarmState::Arming | AlarmState::Pending | AlarmState::Disarming => theme::AMBER,
                    AlarmState::Triggered => theme::RED,
                    _ => theme::BLUE,
                };
                ui.horizontal(|ui| {
                    live_dot(ui, col);
                    ui.label(RichText::new(format!("Home Assistant panel: {}", state.label())).color(col));
                });
            }
            if let Some((ok, msg)) = &view.home.service_notice {
                ui.label(RichText::new(msg).color(if *ok { theme::MUTED } else { theme::AMBER }).size(12.5));
            }
        } else if !view.home.online {
            ui.label(theme::muted("Connect to Home Assistant to discover your alarm panels."));
        } else if !view.home.entities.values().any(|entity| entity.alarm_panel()) {
            ui.label(theme::muted("No alarm_control_panel entities found in Home Assistant."));
        }

        // --- Section 3: Wireless Sensors -> Zones (excludes alarm_control_panel.*) ---
        ui.add_space(12.0);
        ui.label(theme::heading("Wireless sensors -> Zones"));
        ui.label(theme::muted(
            "Connect Zigbee, Z-Wave, Matter, Wi-Fi, ESPHome, or Lab sensors to any RoomGuard zone. (Alarm panels go in Alarm panel sync above.)",
        ));
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search sensor names or entity IDs").desired_width(380.0));
        let filter = self.filter.trim().to_lowercase();
        if view.home.entities.get(&self.entity_id).is_some_and(|e| e.alarm_panel()) {
            self.entity_id.clear();
        }
        let selection = view.home.entities.get(&self.entity_id);
        let label = selection
            .map(|entity| format!("{} ({})", entity.name(), entity.entity_id))
            .unwrap_or_else(|| "Choose a wireless sensor or camera".into());
        egui::ComboBox::from_id_salt("ha-entity").selected_text(label).width(380.0).show_ui(ui, |ui| {
            for entity in view.home.entities.values().filter(|entity| {
                !entity.alarm_panel()
                    && (filter.is_empty()
                        || entity.entity_id.to_lowercase().contains(&filter)
                        || entity.name().to_lowercase().contains(&filter))
            }) {
                let item = format!("{} ({}) · {}", entity.name(), entity.entity_id, entity.description());
                if ui.selectable_label(self.entity_id == entity.entity_id, item).clicked() {
                    self.entity_id = entity.entity_id.clone();
                    let (default_rule, default_operand) = entity.default_rule_and_operand();
                    self.rule = Some(default_rule);
                    self.operand = default_operand;
                    self.texture = None;
                }
            }
        });

        if let Some(entity) = view.home.entities.get(&self.entity_id) {
            ui.label(theme::muted(format!(
                "{} · current state: {}",
                entity.entity_id,
                if view.home.online { entity.description() } else { "Unavailable".into() }
            )));
            if entity.camera() {
                ui.horizontal(|ui| {
                    if ui.add_enabled(view.home.online, egui::Button::new("Load snapshot")).clicked() {
                        engine.send(EngineMsg::HomePreview(self.entity_id.clone()));
                    }
                    ui.label(theme::muted("On demand"));
                });
                if let Some((id, preview)) = &view.home.preview {
                    if id == &self.entity_id {
                        match preview {
                            Ok(rgb) => {
                                if !self.texture.as_ref().is_some_and(|(image, _)| std::sync::Arc::ptr_eq(image, rgb)) {
                                    let image = egui::ColorImage::from_rgb([rgb.w, rgb.h], &rgb.data);
                                    let texture = ui.ctx().load_texture("ha-camera", image, egui::TextureOptions::LINEAR);
                                    self.texture = Some((rgb.clone(), texture));
                                }
                                if let Some((_, texture)) = &self.texture {
                                    let scale = (ui.available_width() / rgb.w as f32).min(1.0);
                                    ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(rgb.w as f32, rgb.h as f32) * scale));
                                }
                            }
                            Err(error) => {
                                ui.label(theme::muted(error));
                            }
                        }
                    }
                }
            } else {
                let zones: Vec<_> = view.zones.iter().filter(|zone| zone.kind != ZoneKind::Null).collect();
                if !zones.iter().any(|zone| zone.num == self.zone) {
                    self.zone = zones.first().map_or(0, |zone| zone.num);
                }
                let rule = self.rule.get_or_insert(Rule::Equals);
                ui.add_enabled_ui(!view.armed && view.home.online, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("ha-rule").selected_text(rule.label()).show_ui(ui, |ui| {
                            for choice in Rule::ALL {
                                ui.selectable_value(rule, choice, choice.label());
                            }
                        });
                        if *rule != Rule::Changes {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.operand)
                                    .hint_text(if entity.binary() { "on or off" } else { "State or number" })
                                    .desired_width(140.0),
                            );
                        }
                    });
                    let label = zones
                        .iter()
                        .find(|zone| zone.num == self.zone)
                        .map(|zone| format!("{:02} · {}", zone.num, zone.label))
                        .unwrap_or_else(|| "Choose an enabled zone".into());
                    egui::ComboBox::from_id_salt("ha-zone").selected_text(label).width(260.0).show_ui(ui, |ui| {
                        for zone in zones {
                            ui.selectable_value(&mut self.zone, zone.num, format!("{:02} · {}", zone.num, zone.label));
                        }
                    });
                    let existing = home.binding(view.model, self.zone);
                    let text = if existing.is_some() { "Replace zone input" } else { "Assign to zone" };
                    if ui.add_enabled(self.zone != 0 && rule.accepts(&self.operand), egui::Button::new(text)).clicked() {
                        home.bindings.retain(|binding| binding.model != view.model || binding.zone != self.zone);
                        home.bindings.push(Binding {
                            model: view.model,
                            zone: self.zone,
                            entity: self.entity_id.clone(),
                            rule: *rule,
                            operand: self.operand.trim().into(),
                        });
                        *changed = true;
                    }
                });
                ui.label(theme::muted("A Home Assistant sensor supplies that zone's input. Set its zone type (Delay, Instant, Fire, etc.) in Panel settings."));
            }
        }

        ui.add_space(12.0);
        ui.label(theme::heading("Zone assignments"));
        let mut remove = None;
        let mut count = 0usize;
        for binding in home.bindings.iter().filter(|binding| binding.model == view.model) {
            count += 1;
            let entity = view.home.entities.get(&binding.entity);
            let zone_state = if !view.home.online {
                (theme::RED, "Offline".to_string())
            } else if let Some(ent) = entity {
                match binding.rule.active(&ent.state, &binding.operand) {
                    Some(true) => (theme::AMBER, format!("{} (OPEN)", ent.description())),
                    Some(false) => (theme::GREEN, format!("{} (Closed)", ent.description())),
                    None => (theme::AMBER, "Unavailable".to_string()),
                }
            } else {
                (theme::AMBER, "Missing entity".to_string())
            };
            let zone_name = view
                .zones
                .iter()
                .find(|z| z.num == binding.zone)
                .map(|z| z.label.as_str())
                .unwrap_or("");
            ui.horizontal_wrapped(|ui| {
                live_dot(ui, zone_state.0);
                ui.label(RichText::new(format!("{:02}", binding.zone)).strong());
                if !zone_name.is_empty() {
                    ui.label(theme::muted(zone_name));
                }
                ui.label(entity.map(|e| e.name()).unwrap_or(&binding.entity));
                let rule_txt = if binding.rule == Rule::Changes {
                    binding.rule.label().to_string()
                } else {
                    format!("{} {}", binding.rule.label(), binding.operand)
                };
                ui.label(theme::muted(format!("({}) · {}", rule_txt, zone_state.1)));
                if ui.add_enabled(!view.armed, egui::Button::new("Remove")).clicked() {
                    remove = Some(binding.zone);
                }
            });
        }
        if count == 0 {
            ui.label(theme::muted("No Home Assistant sensors assigned to zones on this panel yet."));
        }
        if let Some(zone) = remove {
            home.bindings.retain(|binding| binding.model != view.model || binding.zone != zone);
            *changed = true;
        }
    }
}
