//! Home Assistant setup, entity-to-zone rules, and camera previews.

use eframe::egui::{self, RichText};

use crate::config::AppConfig;
use crate::engine::{EngineHandle, EngineMsg, View};
use crate::engine::types::ZoneKind;
use crate::home_assistant::{AlarmState, Binding, Rule};
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

impl HomeUi {
    pub fn ui(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, view: &View, engine: &EngineHandle) {
        let home = &mut cfg.home_assistant;
        ui.label(theme::heading("Home Assistant"));
        ui.label(if !home.enabled { "Disconnected" } else if view.home.status.is_empty() { "Connecting…" } else { &view.home.status });
        if view.armed {
            ui.label(theme::muted("Disarm before changing the connection, followed panel, or zone assignments."));
        }

        ui.collapsing("Connection", |ui| {
            ui.add_enabled_ui(!view.armed, |ui| {
                let (url, token) = self.credentials.get_or_insert_with(|| (home.url.clone(), home.token.clone()));
                ui.label("Home Assistant URL");
                ui.add(egui::TextEdit::singleline(url).hint_text("http://homeassistant.local:8123").desired_width(380.0));
                ui.label("Long-lived access token");
                ui.add(egui::TextEdit::singleline(token).password(true).desired_width(380.0));
                ui.label(theme::muted("Create a token in your Home Assistant profile. Use HTTPS for remote connections."));
                ui.horizontal(|ui| {
                    if ui.add_enabled(!url.trim().is_empty() && !token.trim().is_empty(), egui::Button::new("Connect")).clicked() {
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
            ui.label(theme::muted("Webcam capture is off. Home Assistant zones and text alerts still work."));
        }

        ui.add_space(12.0);
        ui.label(theme::heading("Follow physical panel"));
        let partition = home.follow_panel.as_deref().and_then(|id| view.home.entities.get(id));
        let label = partition.map(|entity| entity.name()).or(home.follow_panel.as_deref())
            .unwrap_or("Use RoomGuard independently").to_owned();
        ui.add_enabled_ui(!view.armed, |ui| {
            egui::ComboBox::from_id_salt("ha-physical-panel").selected_text(label).width(375.0).show_ui(ui, |ui| {
                *changed |= ui.selectable_value(&mut home.follow_panel, None, "Use RoomGuard independently").changed();
                if view.home.online {
                    for entity in view.home.entities.values().filter(|entity| entity.alarm_panel()) {
                        *changed |= ui.selectable_value(
                            &mut home.follow_panel, Some(entity.entity_id.clone()), format!("{} ({})", entity.name(), entity.entity_id),
                        ).changed();
                    }
                }
            });
        });
        ui.label(theme::muted("Arm and disarm from the real panel. The RoomGuard keypad shows that partition’s state and the room still uses its own zone rules."));
        if view.following_panel {
            if view.physical_unavailable {
                ui.label(RichText::new("Physical panel unavailable. RoomGuard keeps its last known arming mode.").color(theme::AMBER));
            } else if let Some(state) = partition.and_then(|entity| AlarmState::parse(&entity.state)) {
                ui.label(theme::muted(state.label()));
            }
        } else if !view.home.online {
            ui.label(theme::muted("Connect to Home Assistant to find your alarm partitions."));
        } else if !view.home.entities.values().any(|entity| entity.alarm_panel()) {
            ui.label(theme::muted("No alarm control panel entities found in Home Assistant."));
        }

        ui.add_space(12.0);
        ui.label(theme::heading("Entities"));
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search names or entity IDs").desired_width(380.0));
        let filter = self.filter.trim().to_lowercase();
        let selection = view.home.entities.get(&self.entity_id);
        let label = selection.map(|entity| entity.name()).unwrap_or("Choose an entity");
        egui::ComboBox::from_id_salt("ha-entity").selected_text(label).width(375.0).show_ui(ui, |ui| {
            for entity in view.home.entities.values().filter(|entity| {
                filter.is_empty() || entity.entity_id.to_lowercase().contains(&filter) || entity.name().to_lowercase().contains(&filter)
            }) {
                if ui.selectable_label(self.entity_id == entity.entity_id, format!("{} ({})", entity.name(), entity.entity_id)).clicked() {
                    self.entity_id = entity.entity_id.clone();
                    self.rule = Some(if entity.entity_id.starts_with("event.") { Rule::Changes } else { Rule::Equals });
                    self.operand = if entity.binary() { "on".into() } else { String::new() };
                    self.texture = None;
                }
            }
        });

        if let Some(entity) = view.home.entities.get(&self.entity_id) {
            ui.label(theme::muted(format!("{} · {}", entity.entity_id, if view.home.online { entity.description() } else { "Unavailable".into() })));
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
                            Err(error) => { ui.label(theme::muted(error)); }
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
                            ui.add(egui::TextEdit::singleline(&mut self.operand).hint_text(if entity.binary() { "on or off" } else { "State or number" }).desired_width(140.0));
                        }
                    });
                    let label = zones.iter().find(|zone| zone.num == self.zone)
                        .map(|zone| format!("{:02} · {}", zone.num, zone.label))
                        .unwrap_or_else(|| "Choose an enabled zone".into());
                    egui::ComboBox::from_id_salt("ha-zone").selected_text(label).width(250.0).show_ui(ui, |ui| {
                        for zone in zones {
                            ui.selectable_value(&mut self.zone, zone.num, format!("{:02} · {}", zone.num, zone.label));
                        }
                    });
                    let existing = home.binding(view.model, self.zone);
                    let text = if existing.is_some() { "Replace zone input" } else { "Assign to zone" };
                    if ui.add_enabled(self.zone != 0 && rule.accepts(&self.operand), egui::Button::new(text)).clicked() {
                        home.bindings.retain(|binding| binding.model != view.model || binding.zone != self.zone);
                        home.bindings.push(Binding { model: view.model, zone: self.zone, entity: self.entity_id.clone(), rule: *rule, operand: self.operand.trim().into() });
                        *changed = true;
                    }
                });
                ui.label(theme::muted("A Home Assistant assignment supplies that zone’s input. Set its alarm type in Panel settings."));
            }
        }

        ui.add_space(12.0);
        ui.label(theme::heading("Zone assignments"));
        let mut remove = None;
        for binding in home.bindings.iter().filter(|binding| binding.model == view.model) {
            let entity = view.home.entities.get(&binding.entity);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{:02}", binding.zone)).strong());
                ui.label(entity.map(|entity| entity.name()).unwrap_or(&binding.entity));
                ui.label(theme::muted(format!("{} {}", binding.rule.label(), binding.operand)));
                if ui.add_enabled(!view.armed, egui::Button::new("Remove")).clicked() {
                    remove = Some(binding.zone);
                }
            });
        }
        if let Some(zone) = remove {
            home.bindings.retain(|binding| binding.model != view.model || binding.zone != zone);
            *changed = true;
        }
    }
}
