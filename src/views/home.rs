//! Home Assistant setup, two-way alarm panel sync (standard HA + ESPHome DSC/Vista),
//! proper elevated searchable dropdown pickers, two-way siren/bell sync,
//! conflict-free real-panel mirroring, and on-demand camera previews.

use std::f32::consts::TAU;

use eframe::egui::{self, Color32, Pos2, RichText, Sense, Stroke, Vec2};

use crate::config::AppConfig;
use crate::engine::types::{Model, ZoneKind};
use crate::engine::{EngineHandle, EngineMsg, PanelEdit, View};
use crate::home_assistant::{AlarmState, Binding, ConnectionState, Entity, PanelSyncMode, Rule};
use crate::theme;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EntityTab {
    #[default]
    Sensors,
    Contacts,
    Cameras,
    All,
}

#[derive(Default)]
pub struct HomeUi {
    credentials: Option<(String, String)>,
    last_model: Option<Model>,
    show_pin: bool,
    // Sensor dropdown state
    sensor_open: bool,
    sensor_just_opened: bool,
    filter: String,
    tab: EntityTab,
    entity_id: String,
    // Siren/output dropdown state
    siren_open: bool,
    siren_just_opened: bool,
    siren_filter: String,
    // Zone assignment builder
    zone: u8,
    zone_code: String,
    zone_name: String,
    rename_zone: bool,
    rule: Option<Rule>,
    operand: String,
    texture: Option<(std::sync::Arc<crate::vision::frame::Rgb>, egui::TextureHandle)>,
}

fn zone_type_choices(model: Model) -> &'static [(&'static str, &'static str)] {
    match model {
        Model::Pc1832 => &[
            ("01", "Delay 1 (Entry/Exit Door)"),
            ("02", "Delay 2 (Secondary Entry)"),
            ("03", "Instant (Window / Perimeter)"),
            ("05", "Interior Stay/Away (Motion)"),
            ("04", "Interior Follower"),
            ("08", "24h Fire / Smoke"),
            ("41", "24h Carbon Monoxide"),
            ("19", "24h Water / Leak"),
            ("13", "24h Gas"),
            ("11", "24h Burglary"),
            ("16", "24h Panic"),
            ("21", "24h Tamper"),
            ("26", "24h Non-Alarm (Monitor Only)"),
            ("37", "Night Zone"),
        ],
        Model::Vista20p => &[
            ("01", "Entry/Exit #1 (Door)"),
            ("02", "Entry/Exit #2 (Secondary Door)"),
            ("03", "Perimeter (Window / Instant)"),
            ("04", "Interior Follower (Motion)"),
            ("10", "Interior w/ Delay"),
            ("09", "24h Fire / Smoke"),
            ("14", "24h Carbon Monoxide"),
            ("07", "24h Audible Alarm"),
            ("06", "24h Silent Alarm"),
            ("08", "24h Auxiliary / Medical"),
            ("05", "Trouble Day / Alarm Night"),
        ],
        Model::Pc1550 => &[
            ("00", "Standard Delay (Door)"),
            ("01", "Instant (Window / Perimeter)"),
            ("03", "Interior Home-Away (Motion)"),
            ("02", "Interior"),
            ("07", "Auxiliary Delay"),
            ("08", "24h Fire / Smoke"),
            ("04", "24h Bell"),
            ("05", "24h Bell/Buzzer"),
            ("06", "24h Buzzer"),
        ],
    }
}

fn zone_type_label(model: Model, code: &str) -> &'static str {
    zone_type_choices(model)
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, l)| *l)
        .unwrap_or("Instant (Perimeter)")
}

fn current_zone_code(view: &View, zone: u8) -> Option<String> {
    let z = usize::from(zone);
    if z == 0 || z > view.model.zone_count() {
        return None;
    }
    match view.model {
        Model::Pc1550 => {
            let raw = view.prog.sections.get("11").map(String::as_str).unwrap_or("000301000308");
            raw.get((z - 1) * 2..(z - 1) * 2 + 2).map(str::to_owned)
        }
        Model::Pc1832 => {
            let (sec, idx) = crate::engine::eeprom::pc1832_zone_section(z);
            let default = crate::engine::eeprom::spec(Model::Pc1832, sec).map_or("", |s| s.default);
            let raw = view.prog.sections.get(sec).map(String::as_str).unwrap_or(default);
            raw.get(idx * 2..idx * 2 + 2).map(str::to_owned)
        }
        Model::Vista20p => {
            let default = crate::engine::eeprom::spec(Model::Vista20p, "56").map_or("", |s| s.default);
            let raw = view.prog.sections.get("56").map(String::as_str).unwrap_or(default);
            raw.get((z - 1) * 2..(z - 1) * 2 + 2).map(str::to_owned)
        }
    }
}

fn is_null_code(model: Model, code: &str) -> bool {
    match model {
        Model::Pc1550 => code.ends_with('9'),
        Model::Pc1832 | Model::Vista20p => code == "00" || code.is_empty(),
    }
}

/// Paints a crisp 18x18 vector status badge: green checkmark, spinning blue
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

fn wrapped_muted(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(theme::muted(text)).wrap());
}

fn trim_label(name: &str) -> String {
    name.trim().chars().take(24).collect()
}

fn domain_sort_rank(e: &Entity) -> u8 {
    match e.domain() {
        "binary_sensor" => 0,
        "cover" | "lock" => 1,
        "sensor" => 2,
        "input_boolean" | "input_number" => 3,
        "siren" | "switch" | "light" => 4,
        "camera" => 5,
        _ => 6,
    }
}

fn small_badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.16))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(egui::Margin::symmetric(6.0, 1.5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(color));
        });
}

struct RowBadge<'a> {
    text: &'a str,
    color: Color32,
}

/// Renders a crisp, fixed-height (44 px) dropdown row using exact measured
/// geometry: left status dot + truncated two-line title/entity_id, right-aligned
/// live state + compact pills. Never stretches vertically inside a ScrollArea
/// and never lets text collide with badges.
#[allow(clippy::too_many_arguments)]
fn entity_dropdown_row(
    ui: &mut egui::Ui,
    width: f32,
    selected: bool,
    accent_border: bool,
    dot_color: Color32,
    title: &str,
    subtitle: &str,
    state_text: &str,
    state_color: Color32,
    badges: &[RowBadge<'_>],
) -> egui::Response {
    let height = 44.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width.max(180.0), height), Sense::click());
    let hovered = resp.hovered();
    let fill = if selected {
        Color32::from_rgb(0x23, 0x3D, 0x31)
    } else if hovered {
        Color32::from_rgb(0x28, 0x30, 0x3B)
    } else {
        Color32::from_rgb(0x21, 0x26, 0x2F)
    };
    let border = if selected || accent_border {
        theme::GREEN
    } else if hovered {
        theme::BLUE.gamma_multiply(0.85)
    } else {
        theme::LINE
    };
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 6.0, fill);
    p.rect_stroke(rect, 6.0, Stroke::new(1.0f32, border));

    let cy = rect.center().y;
    let dot_x = rect.left() + 12.0;
    p.circle_filled(Pos2::new(dot_x, cy), 4.0, dot_color);

    // Measure and place right-hand state + badges FIRST so the left text column
    // gets an exact non-overlapping max width.
    let mut right_x = rect.right() - 10.0;
    let mut state_job = egui::text::LayoutJob::single_section(
        state_text.to_string(),
        egui::TextFormat::simple(theme::semi(11.5), state_color),
    );
    state_job.wrap = egui::text::TextWrapping::truncate_at_width(82.0);
    let state_galley = ui.fonts(|f| f.layout_job(state_job));
    let state_w = state_galley.size().x.min(82.0);
    p.galley(Pos2::new(right_x - state_w, cy - state_galley.size().y * 0.5), state_galley, state_color);
    right_x -= state_w + 7.0;

    for badge in badges {
        let mut bjob = egui::text::LayoutJob::single_section(
            badge.text.to_string(),
            egui::TextFormat::simple(theme::regular(10.5), badge.color),
        );
        bjob.wrap = egui::text::TextWrapping::truncate_at_width(78.0);
        let bg = ui.fonts(|f| f.layout_job(bjob));
        let bw = (bg.size().x + 12.0).min(90.0);
        let bh = 18.0;
        let br = egui::Rect::from_min_size(Pos2::new(right_x - bw, cy - bh * 0.5), Vec2::new(bw, bh));
        p.rect_filled(br, 5.0, badge.color.gamma_multiply(0.18));
        p.galley(
            Pos2::new(br.center().x - bg.size().x * 0.5, cy - bg.size().y * 0.5),
            bg,
            badge.color,
        );
        right_x -= bw + 5.0;
    }

    let left_x = dot_x + 12.0;
    let max_left_w = (right_x - left_x - 6.0).max(60.0);
    let title_color = if selected { Color32::WHITE } else { theme::TEXT };
    let mut title_job = egui::text::LayoutJob::single_section(
        title.to_string(),
        egui::TextFormat::simple(theme::semi(12.5), title_color),
    );
    title_job.wrap = egui::text::TextWrapping::truncate_at_width(max_left_w);
    let title_galley = ui.fonts(|f| f.layout_job(title_job));

    let mut sub_job = egui::text::LayoutJob::single_section(
        subtitle.to_string(),
        egui::TextFormat::simple(theme::regular(10.5), theme::MUTED),
    );
    sub_job.wrap = egui::text::TextWrapping::truncate_at_width(max_left_w);
    let sub_galley = ui.fonts(|f| f.layout_job(sub_job));

    let title_h = title_galley.size().y;
    let total_h = title_h + 2.0 + sub_galley.size().y;
    let top_y = cy - total_h * 0.5;
    let lp = ui.painter_at(egui::Rect::from_min_max(
        Pos2::new(left_x, rect.top() + 2.0),
        Pos2::new(left_x + max_left_w, rect.bottom() - 2.0),
    ));
    lp.galley(Pos2::new(left_x, top_y), title_galley, title_color);
    lp.galley(Pos2::new(left_x, top_y + title_h + 2.0), sub_galley, theme::MUTED);
    resp
}

/// Draws a crisp, proper full-width dropdown trigger button with left-aligned label,
/// optional badge, and right-aligned chevron arrow. Returns its `Response`.
fn dropdown_trigger(
    ui: &mut egui::Ui,
    _id: impl std::hash::Hash,
    width: f32,
    open: bool,
    dot: Option<Color32>,
    label: &str,
    badge: Option<&str>,
    placeholder: bool,
) -> egui::Response {
    let height = 34.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width.max(180.0), height), Sense::click());
    let hovered = resp.hovered();
    let fill = if open {
        Color32::from_rgb(0x23, 0x2A, 0x33)
    } else if hovered {
        theme::CARD_HI
    } else {
        theme::CARD
    };
    let border = if open {
        theme::GREEN
    } else if hovered {
        theme::BLUE.gamma_multiply(0.75)
    } else {
        theme::LINE
    };
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 7.0, fill);
    p.rect_stroke(rect, 7.0, Stroke::new(1.0f32, border));

    let mut x = rect.left() + 10.0;
    let cy = rect.center().y;
    if let Some(col) = dot {
        p.circle_filled(Pos2::new(x + 4.0, cy), 4.0, col);
        x += 15.0;
    }
    let badge_layout = badge.map(|b| {
        let mut job = egui::text::LayoutJob::single_section(
            b.to_string(),
            egui::TextFormat::simple(theme::regular(10.5), theme::BLUE),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(84.0);
        let g = ui.fonts(|f| f.layout_job(job));
        let bw = (g.size().x + 12.0).min(96.0);
        (g, bw)
    });
    let right_margin = 28.0 + badge_layout.as_ref().map_or(0.0, |(_, bw)| bw + 8.0);
    let max_text_w = (rect.right() - right_margin - x).max(60.0);
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat::simple(
            if placeholder { theme::regular(13.0) } else { theme::semi(13.0) },
            if placeholder { theme::MUTED } else { theme::TEXT },
        ),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_text_w);
    let galley = ui.fonts(|f| f.layout_job(job));
    let text_pos = Pos2::new(x, cy - galley.size().y * 0.5);
    p.galley(text_pos, galley, if placeholder { theme::MUTED } else { theme::TEXT });

    if let Some((g, bw)) = badge_layout {
        let br = egui::Rect::from_center_size(Pos2::new(rect.right() - 26.0 - bw * 0.5, cy), Vec2::new(bw, 18.0));
        p.rect_filled(br, 5.0, theme::BLUE.gamma_multiply(0.16));
        p.galley(Pos2::new(br.center().x - g.size().x * 0.5, cy - g.size().y * 0.5), g, theme::BLUE);
    }
    // Crisp vector chevron on the right
    let cx = rect.right() - 14.0;
    let stroke = Stroke::new(1.6f32, if open { theme::GREEN } else { theme::MUTED });
    if open {
        p.line_segment([Pos2::new(cx - 4.0, cy + 2.0), Pos2::new(cx, cy - 2.5)], stroke);
        p.line_segment([Pos2::new(cx, cy - 2.5), Pos2::new(cx + 4.0, cy + 2.0)], stroke);
    } else {
        p.line_segment([Pos2::new(cx - 4.0, cy - 2.0), Pos2::new(cx, cy + 2.5)], stroke);
        p.line_segment([Pos2::new(cx, cy + 2.5), Pos2::new(cx + 4.0, cy - 2.0)], stroke);
    }
    resp
}

/// Computes an elevated popup position (below the button if room, otherwise cleanly above,
/// clamped inside the viewport so it is ALWAYS full height and never squished to 2 lines).
fn popup_pos(ctx: &egui::Context, anchor: egui::Rect, popup_w: f32, popup_h: f32) -> Pos2 {
    let screen = ctx.screen_rect().shrink(10.0);
    let x = anchor.left().clamp(screen.left(), (screen.right() - popup_w).max(screen.left()));
    let space_below = screen.bottom() - anchor.bottom();
    let space_above = anchor.top() - screen.top();
    let y = if space_below >= popup_h || space_below >= space_above {
        (anchor.bottom() + 4.0).min((screen.bottom() - popup_h).max(screen.top()))
    } else {
        (anchor.top() - popup_h - 4.0).max(screen.top())
    };
    Pos2::new(x, y)
}

impl HomeUi {
    pub fn close_popups(&mut self) {
        self.sensor_open = false;
        self.siren_open = false;
    }

    fn select_entity(&mut self, entity: &Entity, cfg: &AppConfig, view: &View, first_unused: Option<u8>) {
        self.entity_id = entity.entity_id.clone();
        let (default_rule, default_operand) = entity.default_rule_and_operand();
        self.rule = Some(default_rule);
        self.operand = default_operand;
        self.zone_code = entity.suggested_zone_code(view.model).to_string();
        self.zone_name = trim_label(entity.name());
        let max_z = view.model.zone_count() as u8;
        if let Some(existing) = cfg
            .home_assistant
            .bindings
            .iter()
            .find(|b| b.model == view.model && b.entity == self.entity_id)
        {
            self.zone = existing.zone;
            self.rename_zone = false;
        } else if let Some(hint_z) = entity.esphome_zone_hint(max_z) {
            // ESPHome DSC/Vista zone entity (e.g. `_z1`, `_z9`, `_z17`): pre-select that exact zone!
            self.zone = hint_z;
            self.rename_zone = true;
        } else if self.zone == 0 || self.zone > max_z {
            self.zone = first_unused.unwrap_or(1);
            self.rename_zone = first_unused.is_some();
        }
        self.texture = None;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, view: &View, engine: &EngineHandle) {
        let content_w = ui.available_width().min(ui.max_rect().width()).max(280.0);
        ui.set_max_width(content_w);
        let card_inner_w = (content_w - 28.0).max(240.0);
        let field_w = (content_w - 6.0).clamp(240.0, 520.0);

        if self.last_model != Some(view.model) {
            self.last_model = Some(view.model);
            self.zone = 0;
            self.zone_code.clear();
            self.rename_zone = false;
        }

        ui.label(theme::heading("Home Assistant"));
        ui.add_space(4.0);

        // Live connection status banner with clear vector icon.
        let conn_state = if !cfg.home_assistant.enabled {
            ConnectionState::Disconnected
        } else {
            view.home.state.clone()
        };
        let (title, title_color, subtitle) = match &conn_state {
            ConnectionState::Connected => {
                let count = view.home.entities.len();
                ("Connected", theme::GREEN, format!("{count} entities live"))
            }
            ConnectionState::Connecting => ("Connecting...", theme::BLUE, "Connecting to Home Assistant...".to_string()),
            ConnectionState::Error(err) => ("Disconnected", theme::RED, format!("{err} (retrying automatically)")),
            ConnectionState::Disconnected => (
                "Disconnected",
                theme::RED,
                if !cfg.home_assistant.url.trim().is_empty() && !cfg.home_assistant.token.trim().is_empty() {
                    "Manually disconnected".to_string()
                } else {
                    "Enter your Home Assistant URL and token below to connect".to_string()
                },
            ),
        };

        theme::card().show(ui, |ui| {
            ui.set_width(card_inner_w);
            ui.horizontal(|ui| {
                status_badge(ui, &conn_state);
                let text_w = (card_inner_w - 130.0).max(120.0);
                ui.vertical(|ui| {
                    ui.set_max_width(text_w);
                    ui.label(RichText::new(title).font(theme::semi(14.5)).color(title_color));
                    ui.add(egui::Label::new(theme::muted(subtitle)).wrap());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if cfg.home_assistant.enabled {
                        if ui
                            .add_enabled(!view.armed, egui::Button::new("Disconnect"))
                            .on_disabled_hover_text("Disarm RoomGuard before disconnecting")
                            .clicked()
                        {
                            cfg.home_assistant.enabled = false;
                            *changed = true;
                        }
                    } else if !cfg.home_assistant.url.trim().is_empty() && !cfg.home_assistant.token.trim().is_empty() {
                        if ui
                            .add_enabled(!view.armed, egui::Button::new("Reconnect"))
                            .on_disabled_hover_text("Disarm RoomGuard before connecting")
                            .clicked()
                        {
                            cfg.home_assistant.enabled = true;
                            *changed = true;
                        }
                    }
                });
            });
        });

        if view.armed {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new("Panel is armed. You can still edit HA panel PIN and test sirens; disarm to change the linked panel or zone bindings.").color(theme::AMBER),
                )
                .wrap(),
            );
        }

        ui.add_space(6.0);
        egui::CollapsingHeader::new("Connection")
            .id_salt("ha-connection-section")
            .default_open(!view.home.online)
            .show(ui, |ui| {
                ui.set_max_width(content_w - 16.0);
                ui.add_enabled_ui(!view.armed, |ui| {
                    let (url, token) = self
                        .credentials
                        .get_or_insert_with(|| (cfg.home_assistant.url.clone(), cfg.home_assistant.token.clone()));
                    ui.label("Home Assistant URL");
                    ui.add(
                        egui::TextEdit::singleline(url)
                            .hint_text("http://homeassistant.local:8123 or http://192.168.1.45:8123")
                            .desired_width(field_w),
                    );
                    ui.label("Long-lived access token");
                    ui.add(
                        egui::TextEdit::singleline(token)
                            .password(true)
                            .hint_text("Paste token from HA Profile -> Security")
                            .desired_width(field_w),
                    );
                    wrapped_muted(
                        ui,
                        "In Home Assistant: click your profile (bottom-left) -> Security -> Long-lived access tokens -> Create token.",
                    );
                    ui.horizontal(|ui| {
                        let ready = !url.trim().is_empty() && !token.trim().is_empty();
                        if ui.add_enabled(ready, egui::Button::new("Connect")).clicked() {
                            cfg.home_assistant.url = url.trim().trim_end_matches('/').to_owned();
                            cfg.home_assistant.token = token.trim().to_owned();
                            cfg.home_assistant.enabled = true;
                            *changed = true;
                        }
                        if ui.add_enabled(cfg.home_assistant.enabled, egui::Button::new("Disconnect")).clicked() {
                            cfg.home_assistant.enabled = false;
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
            wrapped_muted(
                ui,
                "Webcam capture is off. Home Assistant wireless sensors, sirens, panel sync, and phone alerts still work.",
            );
        }

        // --- Section 2: Alarm Panel Sync (Standard HA + ESPHome DSC/Vista, PIN always editable!) ---
        ui.add_space(12.0);
        ui.label(theme::heading("Alarm panel sync"));
        wrapped_muted(
            ui,
            "Link a Home Assistant alarm panel (Test Panel, Alarmo, Envisalink, or ESPHome DSC/Vista bridge). Leave Independent to avoid any conflict with a house alarm.",
        );
        let partition = cfg.home_assistant.follow_panel.as_deref().and_then(|id| view.home.entities.get(id));
        let panel_label = partition
            .map(|entity| format!("{} ({})", entity.name(), entity.entity_id))
            .or_else(|| cfg.home_assistant.follow_panel.clone())
            .unwrap_or_else(|| "Do not sync with a real panel (Independent)".to_owned());
        ui.add_enabled_ui(!view.armed, |ui| {
            let home = &mut cfg.home_assistant;
            egui::ComboBox::from_id_salt("ha-physical-panel")
                .selected_text(panel_label)
                .truncate()
                .width(field_w)
                .height(260.0)
                .show_ui(ui, |ui| {
                    *changed |= ui
                        .selectable_value(&mut home.follow_panel, None, "Do not sync with a real panel (Independent)")
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
                        .radio_value(&mut home.panel_sync, PanelSyncMode::TwoWay, "Two-way sync (arm & disarm both)")
                        .changed()
                    {
                        *changed = true;
                    }
                    if ui
                        .radio_value(&mut home.panel_sync, PanelSyncMode::FollowOnly, "Follow real panel only (read-only)")
                        .changed()
                    {
                        *changed = true;
                    }
                });
            }
        });

        // Keep HA panel PIN and alarm-ownership checkbox ALWAYS editable even while armed
        // so a mismatched PIN can never trap the user out of disarming!
        if cfg.home_assistant.follow_panel.is_some() {
            let home = &mut cfg.home_assistant;
            if home.panel_sync == PanelSyncMode::TwoWay {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("HA panel PIN");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut home.panel_code)
                                .password(!self.show_pin)
                                .hint_text("Leave blank to use keypad code")
                                .desired_width(175.0),
                        )
                        .changed()
                    {
                        *changed = true;
                    }
                    ui.checkbox(&mut self.show_pin, "Show");
                    if !home.panel_code.is_empty() && ui.small_button("Clear").clicked() {
                        home.panel_code.clear();
                        *changed = true;
                    }
                });
                wrapped_muted(
                    ui,
                    "Recommended: leave blank so RoomGuard forwards whatever code you type on the keypad directly to your panel (no PIN saved in config or YAML). Only enter a PIN here if your Home Assistant test panel uses a separate code.",
                );
            } else {
                wrapped_muted(
                    ui,
                    "Read-only mirror: RoomGuard never sends arm or disarm commands to your real panel.",
                );
            }

            ui.add_space(4.0);
            let mut real_panel_owns = !home.ha_zones_trigger_alarm;
            if ui
                .checkbox(
                    &mut real_panel_owns,
                    "Real panel handles alarms for HA zones (prevent double alarms)",
                )
                .changed()
            {
                home.ha_zones_trigger_alarm = !real_panel_owns;
                *changed = true;
            }
            wrapped_muted(
                ui,
                if real_panel_owns {
                    "Safe mirror mode: Home Assistant sensors show live open/closed status and chimes on RoomGuard, while the real panel handles entry delays and alarms."
                } else {
                    "Active zone mode: Home Assistant sensors also trigger RoomGuard entry delays, alarms, and sirens (best for the Test Lab or standalone wireless sensors)."
                },
            );
            ui.add_space(4.0);
            if ui
                .checkbox(
                    &mut home.sync_bypasses,
                    "Sync zone bypasses two-way with linked panel",
                )
                .changed()
            {
                *changed = true;
            }
            wrapped_muted(
                ui,
                if home.sync_bypasses {
                    "Bypasses on a linked ESPHome Keybus/ECP panel (via Zone Status zs) or Lab bypass helper reflect in RoomGuard and vice versa."
                } else {
                    "Bypass sync paused: zone bypasses stay local to RoomGuard."
                },
            );
        }

        if cfg.home_assistant.follow_panel.is_some() && cfg.home_assistant.enabled {
            ui.add_space(4.0);
            if view.home.detected_esphome.is_some() || view.home.detected_bypass_entity.is_some() {
                let mut parts: Vec<String> = Vec::new();
                if let Some(esp) = &view.home.detected_esphome {
                    parts.push(format!("ESPHome bridge: {esp}"));
                }
                if let Some(byp) = &view.home.detected_bypass_entity {
                    parts.push(format!("Bypass sync: {byp}"));
                }
                ui.horizontal_wrapped(|ui| {
                    live_dot(ui, theme::GREEN);
                    ui.label(RichText::new(parts.join(" · ")).size(12.0).color(theme::GREEN));
                });
            }
            if view.home.learned_exit_away.is_some()
                || view.home.learned_exit_stay.is_some()
                || view.home.learned_entry.is_some()
            {
                let mut tparts: Vec<String> = Vec::new();
                if let Some(s) = view.home.learned_exit_away {
                    tparts.push(format!("Away exit {s}s"));
                }
                if let Some(s) = view.home.learned_exit_stay {
                    tparts.push(format!("Stay exit {s}s"));
                }
                if let Some(s) = view.home.learned_entry {
                    tparts.push(format!("Entry {s}s"));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::muted(format!("Lockstep timers synced to HA panel: {}", tparts.join(" · "))));
                    if ui
                        .small_button("Reset")
                        .on_hover_text("Clear learned HA delay timers so RoomGuard re-measures them on the next arm")
                        .clicked()
                    {
                        engine.send(EngineMsg::ResetLearnedHaTimers);
                    }
                });
            }
            if view.physical_unavailable {
                ui.horizontal(|ui| {
                    live_dot(ui, theme::AMBER);
                    ui.add(
                        egui::Label::new(
                            RichText::new("Linked panel unavailable. RoomGuard keeps its last known arming mode.").color(theme::AMBER),
                        )
                        .wrap(),
                    );
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
                ui.add(
                    egui::Label::new(RichText::new(msg).color(if *ok { theme::MUTED } else { theme::AMBER }).size(12.5)).wrap(),
                );
            }
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(theme::CARD_HI)
                .rounding(egui::Rounding::same(8.0))
                .stroke(Stroke::new(1.0f32, theme::LINE))
                .inner_margin(egui::Margin::same(9.0))
                .show(ui, |ui| {
                    ui.set_width((content_w - 22.0).max(240.0));
                    ui.add(
                        egui::Label::new(
                            RichText::new("Tip for real house panels: Copy your zone numbers, names, and types from the zone list card written by your installer inside your wall keypad cover. Turn on 'Real panel handles alarms' above if those sensors already belong to your real panel.")
                                .color(theme::MUTED)
                                .size(12.0),
                        )
                        .wrap(),
                    );
                });
        } else if !view.home.online {
            wrapped_muted(ui, "Connect to Home Assistant to discover your alarm panels.");
        } else if !view.home.entities.values().any(|entity| entity.alarm_panel()) {
            wrapped_muted(ui, "No alarm_control_panel entities found in Home Assistant.");
        }

        // --- Section 3: Wireless Sensors -> Zones (Proper Elevated Searchable Dropdown!) ---
        ui.add_space(14.0);
        ui.label(theme::heading("Wireless sensors -> Zones"));
        wrapped_muted(
            ui,
            "Select any real Zigbee, Z-Wave, Matter, Wi-Fi, ESPHome, or Lab sensor from the dropdown below and assign it to any zone (or click + Next unused to add a new zone).",
        );

        let first_unused = view.zones.iter().find(|z| z.kind == ZoneKind::Null).map(|z| z.num);
        if view.home.entities.get(&self.entity_id).is_some_and(|e| e.alarm_panel()) {
            self.entity_id.clear();
        }
        let selected_ent = view.home.entities.get(&self.entity_id);
        let trig_dot = selected_ent.map(|e| match e.live_active_hint() {
            Some(true) => theme::AMBER,
            Some(false) => theme::GREEN,
            None => theme::MUTED,
        });
        let trig_label = selected_ent
            .map(|e| format!("{}  ({})", e.name(), e.entity_id))
            .unwrap_or_else(|| {
                if view.home.online {
                    "Click to choose a Home Assistant sensor or camera...".to_string()
                } else {
                    "Connect to Home Assistant to browse sensors".to_string()
                }
            });
        let trig_badge = selected_ent.map(Entity::category_badge);
        ui.add_space(4.0);
        let sensor_btn = dropdown_trigger(
            ui,
            "ha-sensor-dd-trigger",
            field_w,
            self.sensor_open,
            trig_dot,
            &trig_label,
            trig_badge,
            selected_ent.is_none(),
        );
        if sensor_btn.clicked() && view.home.online {
            self.sensor_open = !self.sensor_open;
            self.sensor_just_opened = self.sensor_open;
            if self.sensor_open {
                self.siren_open = false;
            }
        }
        if !view.home.online {
            self.sensor_open = false;
        }

        // Elevated floating searchable dropdown popup (fixed 44px rows, zero overlap, adaptive height)
        if self.sensor_open {
            let popup_w = field_w.clamp(320.0, 520.0);
            let total_non_panel = view.home.entities.values().filter(|e| !e.alarm_panel()).count();
            let mut matched: Vec<(u8, u8, &Entity)> = view
                .home
                .entities
                .values()
                .filter(|e| !e.alarm_panel())
                .filter(|e| match self.tab {
                    EntityTab::Sensors => e.is_sensor_candidate(),
                    EntityTab::Contacts => e.is_contact_or_motion(),
                    EntityTab::Cameras => e.camera(),
                    EntityTab::All => true,
                })
                .filter_map(|e| e.matches_search(&self.filter).map(|r| (r, domain_sort_rank(e), e)))
                .collect();
            matched.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1.cmp(&b.1))
                    .then_with(|| a.2.name().to_ascii_lowercase().cmp(&b.2.name().to_ascii_lowercase()))
            });
            let list_h = if matched.is_empty() {
                56.0
            } else {
                ((matched.len() as f32) * 48.0 + 4.0).clamp(54.0, 232.0)
            };
            let popup_h = list_h + 104.0;
            let pos = popup_pos(ui.ctx(), sensor_btn.rect, popup_w, popup_h);
            let mut picked_id: Option<String> = None;
            let mut close_now = ui.input(|i| i.key_pressed(egui::Key::Escape));

            let area_resp = egui::Area::new(egui::Id::new("ha-sensor-dropdown-popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(Color32::from_rgb(0x1B, 0x20, 0x28))
                        .stroke(Stroke::new(1.2f32, theme::GREEN.gamma_multiply(0.85)))
                        .rounding(egui::Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            let inner_w = popup_w - 20.0;
                            ui.set_width(inner_w);
                            // Search bar inside the dropdown
                            ui.horizontal(|ui| {
                                let sw = if self.filter.is_empty() { inner_w - 4.0 } else { inner_w - 64.0 };
                                let edit = ui.add(
                                    egui::TextEdit::singleline(&mut self.filter)
                                        .hint_text("Search by name, room, door/motion/smoke, or entity ID...")
                                        .desired_width(sw.max(160.0)),
                                );
                                if self.sensor_just_opened {
                                    edit.request_focus();
                                    self.sensor_just_opened = false;
                                }
                                if !self.filter.is_empty() && ui.button("Clear").clicked() {
                                    self.filter.clear();
                                }
                            });

                            ui.add_space(5.0);
                            ui.horizontal_wrapped(|ui| {
                                for (tab, lbl) in [
                                    (EntityTab::Sensors, "Sensors"),
                                    (EntityTab::Contacts, "Doors & Motion"),
                                    (EntityTab::Cameras, "Cameras"),
                                    (EntityTab::All, "All"),
                                ] {
                                    let on = self.tab == tab;
                                    let btn = egui::Button::new(
                                        RichText::new(lbl)
                                            .size(11.5)
                                            .color(if on { Color32::WHITE } else { theme::MUTED }),
                                    )
                                    .fill(if on { Color32::from_rgb(0x2C, 0x5A, 0x44) } else { theme::CARD_HI })
                                    .stroke(Stroke::new(1.0f32, if on { theme::GREEN } else { theme::LINE }));
                                    if ui.add(btn).clicked() {
                                        self.tab = tab;
                                    }
                                }
                                ui.label(theme::muted(format!("{} of {}", matched.len(), total_non_panel)));
                            });

                            ui.add_space(5.0);
                            ui.separator();
                            ui.add_space(4.0);

                            if matched.is_empty() {
                                ui.add_space(10.0);
                                wrapped_muted(ui, "No sensors match. Clear your search or switch to 'All'.");
                                ui.add_space(10.0);
                            } else {
                                egui::ScrollArea::vertical()
                                    .id_salt("ha-sensor-dd-scroll")
                                    .max_height(list_h)
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        let row_w = (inner_w - 12.0).max(200.0);
                                        for (_, _, entity) in &matched {
                                            let is_sel = self.entity_id == entity.entity_id;
                                            let bound_zone = cfg
                                                .home_assistant
                                                .bindings
                                                .iter()
                                                .find(|b| b.model == view.model && b.entity == entity.entity_id)
                                                .map(|b| b.zone);
                                            let dot_col = match entity.live_active_hint() {
                                                Some(true) => theme::AMBER,
                                                Some(false) => theme::GREEN,
                                                None => theme::MUTED,
                                            };
                                            let z_str = bound_zone.map(|z| format!("Z{:02}", z));
                                            let mut badges = Vec::with_capacity(2);
                                            if let Some(ref zs) = z_str {
                                                badges.push(RowBadge { text: zs, color: theme::GREEN });
                                            }
                                            badges.push(RowBadge { text: entity.category_badge(), color: theme::BLUE });
                                            let state_txt = entity.short_state();
                                            let row = entity_dropdown_row(
                                                ui,
                                                row_w,
                                                is_sel,
                                                bound_zone.is_some(),
                                                dot_col,
                                                entity.name(),
                                                &entity.entity_id,
                                                &state_txt,
                                                dot_col,
                                                &badges,
                                            );
                                            if row.clicked() {
                                                picked_id = Some(entity.entity_id.clone());
                                            }
                                            ui.add_space(4.0);
                                        }
                                    });
                            }
                        });
                });

            if let Some(id) = picked_id {
                if let Some(entity) = view.home.entities.get(&id) {
                    self.select_entity(entity, cfg, view, first_unused);
                }
                close_now = true;
            } else if ui.input(|i| i.pointer.any_pressed()) {
                if let Some(ptr) = ui.input(|i| i.pointer.interact_pos()) {
                    if !area_resp.response.rect.contains(ptr) && !sensor_btn.rect.contains(ptr) {
                        close_now = true;
                    }
                }
            }
            if close_now {
                self.sensor_open = false;
            }
        }

        // Selected entity configuration card
        if let Some(entity) = view.home.entities.get(&self.entity_id) {
            ui.add_space(6.0);
            theme::card().show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(entity.name()).font(theme::semi(13.5)));
                    small_badge(ui, entity.category_badge(), theme::BLUE);
                    ui.label(theme::muted(format!(
                        "{} · live: {}",
                        entity.entity_id,
                        if view.home.online { entity.short_state() } else { "Unavailable".into() }
                    )));
                });
                ui.add_space(4.0);

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
                                        let scale = ((content_w - 44.0) / rgb.w as f32).min(1.0);
                                        ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(rgb.w as f32, rgb.h as f32) * scale));
                                    }
                                }
                                Err(error) => {
                                    wrapped_muted(ui, error);
                                }
                            }
                        }
                    }
                } else {
                    let max_z = view.model.zone_count() as u8;
                    if self.zone == 0 || self.zone > max_z {
                        self.zone = first_unused.unwrap_or(1);
                    }
                    if self.zone_code.is_empty() || !zone_type_choices(view.model).iter().any(|(c, _)| *c == self.zone_code) {
                        self.zone_code = entity.suggested_zone_code(view.model).to_string();
                    }
                    let rule = self.rule.get_or_insert(Rule::Equals);
                    ui.add_enabled_ui(!view.armed && view.home.online, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Trigger when");
                            egui::ComboBox::from_id_salt("ha-rule")
                                .selected_text(rule.label())
                                .width(92.0)
                                .height(220.0)
                                .show_ui(ui, |ui| {
                                    for choice in Rule::ALL {
                                        ui.selectable_value(rule, choice, choice.label());
                                    }
                                });
                            if *rule != Rule::Changes {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.operand)
                                        .hint_text(if entity.binary() { "on or off" } else { "State or number" })
                                        .desired_width(120.0),
                                );
                            }
                        });

                        let sel_zone = view.zones.get(usize::from(self.zone) - 1);
                        let zone_label = sel_zone
                            .map(|z| {
                                if z.kind == ZoneKind::Null {
                                    format!("{:02} · (Unused - will enable)", z.num)
                                } else if cfg.home_assistant.binding(view.model, z.num).is_some() {
                                    format!("{:02} · {} [HA]", z.num, z.label)
                                } else {
                                    format!("{:02} · {} ({})", z.num, z.label, z.kind.short())
                                }
                            })
                            .unwrap_or_else(|| "Choose a zone".into());
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Zone slot");
                            egui::ComboBox::from_id_salt("ha-zone")
                                .selected_text(zone_label)
                                .width((field_w - 170.0).clamp(160.0, 240.0))
                                .height(260.0)
                                .show_ui(ui, |ui| {
                                    for z in &view.zones {
                                        let text = if z.kind == ZoneKind::Null {
                                            format!("{:02} · (Unused - will enable)", z.num)
                                        } else if cfg.home_assistant.binding(view.model, z.num).is_some() {
                                            format!("{:02} · {} [HA]", z.num, z.label)
                                        } else {
                                            format!("{:02} · {} ({})", z.num, z.label, z.kind.short())
                                        };
                                        if ui.selectable_value(&mut self.zone, z.num, text).changed() {
                                            if z.kind == ZoneKind::Null {
                                                self.rename_zone = true;
                                                self.zone_name = trim_label(entity.name());
                                                self.zone_code = entity.suggested_zone_code(view.model).to_string();
                                            } else {
                                                if let Some(cur) = current_zone_code(view, z.num) {
                                                    if !is_null_code(view.model, &cur) {
                                                        self.zone_code = cur;
                                                    }
                                                }
                                                self.zone_name = z.label.clone();
                                                self.rename_zone = false;
                                            }
                                        }
                                    }
                                });
                            if let Some(next_free) = first_unused {
                                if ui
                                    .button("+ Next unused")
                                    .on_hover_text(format!("Pick Zone {:02} (the next unused slot)", next_free))
                                    .clicked()
                                {
                                    self.zone = next_free;
                                    self.rename_zone = true;
                                    self.zone_name = trim_label(entity.name());
                                    self.zone_code = entity.suggested_zone_code(view.model).to_string();
                                }
                            }
                        });

                        let is_unused = sel_zone.is_some_and(|z| z.kind == ZoneKind::Null);
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Zone type");
                            egui::ComboBox::from_id_salt("ha-zone-type")
                                .selected_text(zone_type_label(view.model, &self.zone_code))
                                .width((field_w - 92.0).clamp(180.0, 270.0))
                                .height(260.0)
                                .show_ui(ui, |ui| {
                                    for &(code, lbl) in zone_type_choices(view.model) {
                                        ui.selectable_value(&mut self.zone_code, code.to_string(), lbl);
                                    }
                                });
                        });

                        ui.horizontal_wrapped(|ui| {
                            let mut do_rename = self.rename_zone || is_unused;
                            if ui.checkbox(&mut do_rename, "Name zone").changed() {
                                self.rename_zone = do_rename;
                            }
                            if do_rename {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.zone_name)
                                        .char_limit(24)
                                        .hint_text("e.g. Back Door")
                                        .desired_width(170.0),
                                );
                                if ui.small_button("Use HA name").clicked() {
                                    self.zone_name = trim_label(entity.name());
                                }
                            }
                        });
                        if view.model == Model::Vista20p && (self.rename_zone || is_unused) {
                            let (spoken, matched) = crate::audio::voice::spoken_zone_preview(&self.zone_name, self.zone);
                            if matched {
                                ui.label(RichText::new(format!("6160V voice will speak: \"{}\"", spoken)).size(11.5).color(theme::GREEN));
                            } else {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "Not in 6160V voice ROM. LCD shows your name, voice falls back to \"{}\".",
                                            spoken
                                        ))
                                        .size(11.5)
                                        .color(theme::AMBER),
                                    )
                                    .wrap(),
                                );
                            }
                            egui::CollapsingHeader::new("Show 48 speakable 6160V voice words")
                                .id_salt("vista-vocab-ha")
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(crate::audio::voice::vocab_display_list()).size(11.5).color(theme::MUTED))
                                            .wrap(),
                                    );
                                });
                        }

                        let existing = cfg.home_assistant.binding(view.model, self.zone).is_some();
                        let btn_text = if is_unused {
                            format!("Enable & assign to Zone {:02}", self.zone)
                        } else if existing {
                            format!("Replace Zone {:02} sensor", self.zone)
                        } else {
                            format!("Assign to Zone {:02}", self.zone)
                        };
                        ui.add_space(2.0);
                        if ui
                            .add_enabled(
                                self.zone != 0 && rule.accepts(&self.operand),
                                egui::Button::new(RichText::new(btn_text).color(Color32::WHITE))
                                    .fill(Color32::from_rgb(0x2C, 0x5A, 0x44)),
                            )
                            .clicked()
                        {
                            let target_zone = self.zone;
                            if !self.zone_code.is_empty() {
                                engine.send(EngineMsg::EditPanel(PanelEdit::SetZoneType {
                                    zone: target_zone,
                                    code: self.zone_code.clone(),
                                }));
                            }
                            if (self.rename_zone || is_unused) && !self.zone_name.trim().is_empty() {
                                let labels = cfg.labels_mut(view.model);
                                if let Some(slot) = labels.get_mut(usize::from(target_zone) - 1) {
                                    *slot = trim_label(&self.zone_name);
                                }
                            }
                            cfg.home_assistant
                                .bindings
                                .retain(|binding| binding.model != view.model || binding.zone != target_zone);
                            cfg.home_assistant.bindings.push(Binding {
                                model: view.model,
                                zone: target_zone,
                                entity: self.entity_id.clone(),
                                rule: *rule,
                                operand: self.operand.trim().into(),
                            });
                            *changed = true;
                        }
                    });
                }
            });
        }

        // --- Section 4: Active Zone Assignments & Unused Demo Cleanup ---
        ui.add_space(12.0);
        ui.label(theme::heading("Zone assignments"));
        let mut remove = None;
        let mut count = 0usize;
        for binding in cfg.home_assistant.bindings.iter().filter(|binding| binding.model == view.model) {
            count += 1;
            let entity = view.home.entities.get(&binding.entity);
            let zone_state = if !view.home.online {
                (theme::RED, "Offline".to_string())
            } else if let Some(ent) = entity {
                match binding.rule.active(&ent.state, &binding.operand) {
                    Some(true) => (theme::AMBER, format!("{} (OPEN)", ent.short_state())),
                    Some(false) => (theme::GREEN, format!("{} (Closed)", ent.short_state())),
                    None => (theme::AMBER, "Unavailable".to_string()),
                }
            } else {
                (theme::AMBER, "Missing entity".to_string())
            };
            let zone_info = view.zones.iter().find(|z| z.num == binding.zone);
            let zone_name = zone_info.map(|z| z.label.as_str()).unwrap_or("");
            let zone_kind = zone_info.map(|z| z.kind.short()).unwrap_or("");
            ui.horizontal_wrapped(|ui| {
                live_dot(ui, zone_state.0);
                ui.label(RichText::new(format!("{:02}", binding.zone)).strong());
                if !zone_name.is_empty() {
                    ui.label(zone_name);
                }
                if !zone_kind.is_empty() {
                    ui.label(theme::muted(format!("[{}]", zone_kind)));
                }
                ui.label(theme::muted("<-"));
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
            wrapped_muted(ui, "No Home Assistant sensors assigned to zones on this panel yet.");
        }
        if let Some(zone) = remove {
            cfg.home_assistant
                .bindings
                .retain(|binding| binding.model != view.model || binding.zone != zone);
            *changed = true;
        }

        // One-click cleanup for unused simulated zones
        let unused_sim: Vec<u8> = view
            .zones
            .iter()
            .filter(|z| z.kind != ZoneKind::Null && !z.camera && cfg.home_assistant.binding(view.model, z.num).is_none())
            .map(|z| z.num)
            .collect();
        if !unused_sim.is_empty() && (count > 0 || !cfg.camera.enabled) {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!view.armed, egui::Button::new(format!("Disable {} unused simulated zones", unused_sim.len())))
                    .on_hover_text("Sets unused default zones (with no HA sensor or camera box) to Not Used so your Zones list stays clean. You can re-enable any zone anytime.")
                    .clicked()
                {
                    engine.send(EngineMsg::EditPanel(PanelEdit::DisableZones(unused_sim)));
                }
            });
        }

        // --- Alarm notifications: tell Home Assistant about RoomGuard itself ---
        ui.add_space(14.0);
        ui.label(theme::heading("Alarm notifications"));
        wrapped_muted(
            ui,
            "RoomGuard tells Home Assistant what it's doing, even with no panel linked: a roomguard_state event on every change (disarmed, arming, armed_away, armed_home, armed_night, pending or triggered), and a roomguard_alarm event when an alarm starts or clears, for your automations. Each alarm also shows a notification in Home Assistant until it's disarmed. Duress and silent alarms send the event only.",
        );
        if ui
            .checkbox(&mut cfg.home_assistant.announce_alarms, "Tell Home Assistant about RoomGuard's alarms and arming")
            .changed()
        {
            *changed = true;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Phone alert (optional)");
            let field = egui::TextEdit::singleline(&mut cfg.home_assistant.notify_service)
                .hint_text("notify.mobile_app_your_phone")
                .desired_width(260.0);
            if ui.add(field).changed() {
                *changed = true;
            }
        });
        wrapped_muted(
            ui,
            "Find your phone's service in Home Assistant under Developer tools, Actions: search for notify.mobile_app. Alarms arrive as urgent notifications with an alarm sound, and clear themselves when RoomGuard is disarmed. Events need an administrator's access token.",
        );

        // --- Section 5: Two-Way Home Assistant Sirens & Alarm Outputs (One-Click Linking!) ---
        ui.add_space(14.0);
        ui.label(theme::heading("Sirens & alarm outputs (two-way)"));
        wrapped_muted(
            ui,
            "Link any Home Assistant siren, switch, strobe, lab siren toggle, or ESPHome Keybus/ECP alarm/fire sensor. RoomGuard turns it ON during alarms, AND flipping it ON in Home Assistant immediately sounds RoomGuard's PC siren live.",
        );

        if view.ha_external_siren {
            ui.add_space(5.0);
            egui::Frame::none()
                .fill(theme::RED.gamma_multiply(0.18))
                .rounding(egui::Rounding::same(8.0))
                .stroke(Stroke::new(1.0f32, theme::RED))
                .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width((content_w - 24.0).max(240.0));
                    ui.horizontal_wrapped(|ui| {
                        live_dot(ui, theme::RED);
                        ui.label(RichText::new("External Home Assistant siren is sounding").font(theme::semi(13.0)).color(theme::RED));
                        if ui
                            .add(egui::Button::new(RichText::new("Silence siren").color(Color32::WHITE)).fill(theme::RED))
                            .clicked()
                        {
                            engine.send(EngineMsg::SilenceHaSiren);
                        }
                    });
                });
        }

        let mut remove_output: Option<String> = None;
        if !cfg.home_assistant.siren_outputs.is_empty() {
            ui.add_space(6.0);
            for entity_id in &cfg.home_assistant.siren_outputs {
                let ent = view.home.entities.get(entity_id);
                let is_on = ent.is_some_and(Entity::is_output_on);
                let dot_col = if !view.home.online {
                    theme::RED
                } else if is_on {
                    theme::RED
                } else {
                    theme::GREEN
                };
                egui::Frame::none()
                    .fill(if is_on { Color32::from_rgb(0x36, 0x20, 0x22) } else { theme::CARD })
                    .rounding(egui::Rounding::same(7.0))
                    .stroke(Stroke::new(1.0f32, if is_on { theme::RED } else { theme::LINE }))
                    .inner_margin(egui::Margin::symmetric(9.0, 6.0))
                    .show(ui, |ui| {
                        ui.set_width((content_w - 22.0).max(240.0));
                        ui.horizontal_wrapped(|ui| {
                            live_dot(ui, dot_col);
                            ui.label(RichText::new(ent.map(|e| e.name()).unwrap_or(entity_id.as_str())).font(theme::semi(13.0)));
                            ui.label(theme::muted(format!(
                                "{} · {}",
                                entity_id,
                                ent.map(|e| e.short_state()).unwrap_or_else(|| "Offline".into())
                            )));
                            if ui
                                .add_enabled(view.home.online, egui::Button::new("Test (3s)"))
                                .on_hover_text("Sounds RoomGuard's siren and turns this HA output ON for 3 seconds, then OFF")
                                .clicked()
                            {
                                engine.send(EngineMsg::TestHaOutput {
                                    entity_id: entity_id.clone(),
                                    on: true,
                                    auto_off_secs: Some(3),
                                });
                            }
                            if ui
                                .add_enabled(view.home.online, egui::Button::new("Off"))
                                .on_hover_text("Silence and turn OFF immediately")
                                .clicked()
                            {
                                engine.send(EngineMsg::TestHaOutput {
                                    entity_id: entity_id.clone(),
                                    on: false,
                                    auto_off_secs: None,
                                });
                            }
                            if ui.button("Remove").clicked() {
                                remove_output = Some(entity_id.clone());
                            }
                        });
                    });
                ui.add_space(4.0);
            }
        } else if view.home.online {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::muted("No sirens linked yet. Click an output below to link it right away."));
                if view.home.entities.contains_key("input_boolean.roomguard_lab_siren") {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("+ Quick-link Lab siren").size(12.0).color(Color32::WHITE))
                                .fill(Color32::from_rgb(0x2C, 0x5A, 0x44)),
                        )
                        .clicked()
                    {
                        cfg.home_assistant.siren_outputs.push("input_boolean.roomguard_lab_siren".into());
                        *changed = true;
                    }
                }
            });
        }
        if let Some(id) = remove_output {
            cfg.home_assistant.siren_outputs.retain(|x| x != &id);
            *changed = true;
        }
        if let Some((ok, msg)) = &view.home.output_notice {
            ui.add_space(3.0);
            ui.add(
                egui::Label::new(RichText::new(msg).color(if *ok { theme::GREEN } else { theme::AMBER }).size(12.5)).wrap(),
            );
        }

        // One-click elevated searchable dropdown: clicking any row immediately links/unlinks it!
        if view.home.online {
            ui.add_space(6.0);
            let linked_count = cfg.home_assistant.siren_outputs.len();
            let siren_label = if linked_count == 0 {
                "Click to link a siren, lab siren toggle, switch, or alarm sensor...".to_string()
            } else {
                format!("Add or remove another siren / alarm output ({linked_count} linked)...")
            };
            let siren_badge = (linked_count > 0).then_some("two-way");

            let siren_btn = dropdown_trigger(
                ui,
                "ha-siren-dd-trigger",
                field_w,
                self.siren_open,
                (linked_count > 0).then_some(theme::GREEN),
                &siren_label,
                siren_badge,
                linked_count == 0,
            );
            if siren_btn.clicked() {
                self.siren_open = !self.siren_open;
                self.siren_just_opened = self.siren_open;
                if self.siren_open {
                    self.sensor_open = false;
                }
            }

            if self.siren_open {
                let mut outputs: Vec<(u8, u8, &Entity)> = view
                    .home
                    .entities
                    .values()
                    .filter(|e| e.is_output())
                    .filter_map(|e| {
                        let rank = e.matches_search(&self.siren_filter)?;
                        let dom_rank = match e.domain() {
                            "siren" => 0,
                            "binary_sensor" => 1,
                            "input_boolean" if e.entity_id.contains("siren") || e.name().to_ascii_lowercase().contains("siren") => 1,
                            "switch" => 2,
                            "input_boolean" => 3,
                            _ => 4,
                        };
                        Some((rank, dom_rank, e))
                    })
                    .collect();
                outputs.sort_by(|a, b| {
                    a.0.cmp(&b.0)
                        .then_with(|| a.1.cmp(&b.1))
                        .then_with(|| a.2.name().to_ascii_lowercase().cmp(&b.2.name().to_ascii_lowercase()))
                });

                let popup_w = field_w.clamp(320.0, 520.0);
                let list_h = if outputs.is_empty() {
                    52.0
                } else {
                    ((outputs.len() as f32) * 48.0 + 4.0).clamp(54.0, 232.0)
                };
                let popup_h = list_h + 62.0;
                let pos = popup_pos(ui.ctx(), siren_btn.rect, popup_w, popup_h);
                let mut toggle_id: Option<String> = None;
                let mut close_now = ui.input(|i| i.key_pressed(egui::Key::Escape));

                let area_resp = egui::Area::new(egui::Id::new("ha-siren-dropdown-popup"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(pos)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style())
                            .fill(Color32::from_rgb(0x1B, 0x20, 0x28))
                            .stroke(Stroke::new(1.2f32, theme::GREEN.gamma_multiply(0.85)))
                            .rounding(egui::Rounding::same(10.0))
                            .inner_margin(egui::Margin::same(10.0))
                            .show(ui, |ui| {
                                let inner_w = popup_w - 20.0;
                                ui.set_width(inner_w);
                                ui.horizontal(|ui| {
                                    let sw = if self.siren_filter.is_empty() { inner_w - 4.0 } else { inner_w - 64.0 };
                                    let edit = ui.add(
                                        egui::TextEdit::singleline(&mut self.siren_filter)
                                            .hint_text("Search sirens, switches, lights, or ESPHome alarm sensors...")
                                            .desired_width(sw.max(160.0)),
                                    );
                                    if self.siren_just_opened {
                                        edit.request_focus();
                                        self.siren_just_opened = false;
                                    }
                                    if !self.siren_filter.is_empty() && ui.button("Clear").clicked() {
                                        self.siren_filter.clear();
                                    }
                                });
                                ui.add_space(6.0);
                                if outputs.is_empty() {
                                    ui.add_space(8.0);
                                    wrapped_muted(ui, "No siren, switch, light, input_boolean, or alarm sensor entities found.");
                                    ui.add_space(8.0);
                                } else {
                                    egui::ScrollArea::vertical()
                                        .id_salt("ha-siren-dd-scroll")
                                        .max_height(list_h)
                                        .auto_shrink([false, true])
                                        .show(ui, |ui| {
                                            let row_w = (inner_w - 12.0).max(200.0);
                                            for (_, _, ent) in &outputs {
                                                let linked = cfg.home_assistant.siren_outputs.iter().any(|x| x == &ent.entity_id);
                                                let is_on = ent.is_output_on();
                                                let dot_col = if is_on { theme::RED } else { theme::GREEN };
                                                let mut badges = Vec::with_capacity(2);
                                                if linked {
                                                    badges.push(RowBadge { text: "Linked", color: theme::GREEN });
                                                }
                                                badges.push(RowBadge { text: ent.category_badge(), color: theme::AMBER });
                                                let state_txt = ent.short_state();
                                                let row = entity_dropdown_row(
                                                    ui,
                                                    row_w,
                                                    linked,
                                                    linked,
                                                    dot_col,
                                                    ent.name(),
                                                    &ent.entity_id,
                                                    &state_txt,
                                                    dot_col,
                                                    &badges,
                                                );
                                                if row.clicked() {
                                                    toggle_id = Some(ent.entity_id.clone());
                                                }
                                                ui.add_space(4.0);
                                            }
                                        });
                                }
                            });
                    });

                if let Some(id) = toggle_id {
                    if let Some(idx) = cfg.home_assistant.siren_outputs.iter().position(|x| x == &id) {
                        cfg.home_assistant.siren_outputs.remove(idx);
                    } else {
                        cfg.home_assistant.siren_outputs.push(id);
                    }
                    *changed = true;
                    close_now = true;
                } else if ui.input(|i| i.pointer.any_pressed()) {
                    if let Some(ptr) = ui.input(|i| i.pointer.interact_pos()) {
                        if !area_resp.response.rect.contains(ptr) && !siren_btn.rect.contains(ptr) {
                            close_now = true;
                        }
                    }
                }
                if close_now {
                    self.siren_open = false;
                }
            }
        }
    }
}
