//! Home Assistant entity discovery, live states, two-way alarm panel control,
//! and on-demand camera snapshots.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{ErrorKind, Read};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::engine::{types::Model, EngineMsg};
use crate::vision::frame::{decode_jpeg_file, Rgb};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelSyncMode {
    /// Arm and disarm from either the physical Home Assistant panel or RoomGuard's keypad.
    #[default]
    TwoWay,
    /// Read-only mirror: arm and disarm only at the physical panel.
    FollowOnly,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelLink {
    #[default]
    Independent,
    FollowOnly,
    TwoWay,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub url: String,
    pub token: String,
    pub bindings: Vec<Binding>,
    pub follow_panel: Option<String>,
    pub panel_sync: PanelSyncMode,
    pub panel_code: String,
    /// When linked to a real Home Assistant alarm panel, set to `false` to mirror
    /// HA zones on the keypad (live open/closed + chimes) without firing duplicate
    /// RoomGuard alarms on top of the real panel.
    #[serde(default = "default_true")]
    pub ha_zones_trigger_alarm: bool,
    /// Home Assistant entities (`siren.*`, `switch.*`, `light.*`, `input_boolean.*`)
    /// turned ON automatically when RoomGuard's alarm siren sounds and OFF when silenced.
    #[serde(default)]
    pub siren_outputs: Vec<String>,
    /// Sync zone bypasses two-way with a linked ESPHome DSC/Vista panel (via `zs` and
    /// `alarm_keypress`) or the Home Assistant Test Lab (`input_text.roomguard_test_bypasses`).
    #[serde(default = "default_true")]
    pub sync_bypasses: bool,
    /// Automatically measured exit/entry delay durations (seconds) of the linked HA panel
    /// so RoomGuard's countdown stays in lockstep with HA without skipping the final seconds.
    #[serde(default)]
    pub learned_exit_away: Option<u64>,
    #[serde(default)]
    pub learned_exit_stay: Option<u64>,
    #[serde(default)]
    pub learned_entry: Option<u64>,
    /// Tell Home Assistant about RoomGuard itself: a `roomguard_state` event on
    /// every change, and on an alarm a `roomguard_alarm` event plus a notification.
    #[serde(default = "default_true")]
    pub announce_alarms: bool,
    /// Optional phone push for alarms, e.g. `notify.mobile_app_pixel_8`.
    #[serde(default)]
    pub notify_service: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            token: String::new(),
            bindings: Vec::new(),
            follow_panel: None,
            panel_sync: PanelSyncMode::TwoWay,
            panel_code: String::new(),
            ha_zones_trigger_alarm: true,
            siren_outputs: Vec::new(),
            sync_bypasses: true,
            learned_exit_away: None,
            learned_exit_stay: None,
            learned_entry: None,
            announce_alarms: true,
            notify_service: String::new(),
        }
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HomeAssistant")
            .field("enabled", &self.enabled)
            .field("bindings", &self.bindings.len())
            .field("panel_sync", &self.panel_sync)
            .finish_non_exhaustive()
    }
}

impl Config {
    pub fn followed_panel(&self) -> Option<&str> {
        self.follow_panel.as_deref().filter(|id| {
            self.enabled
                && (id.starts_with("alarm_control_panel.") || id.starts_with("sensor."))
        })
    }

    pub fn panel_link(&self) -> PanelLink {
        if self.followed_panel().is_none() {
            PanelLink::Independent
        } else {
            match self.panel_sync {
                PanelSyncMode::FollowOnly => PanelLink::FollowOnly,
                PanelSyncMode::TwoWay => PanelLink::TwoWay,
            }
        }
    }

    pub fn binding(&self, model: Model, zone: u8) -> Option<&Binding> {
        self.bindings.iter().find(|binding| binding.model == model && binding.zone == zone)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConnectionState {
    /// Manually disconnected or not yet configured.
    #[default]
    Disconnected,
    /// Establishing or re-establishing the WebSocket session.
    Connecting,
    /// Live WebSocket authenticated and synced with entity states.
    Connected,
    /// Automatically detected disconnect, timeout, or authentication error.
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmState {
    Disarmed,
    Arming,
    Stay,
    Away,
    Night,
    StayInstant,
    AwayMax,
    Vacation,
    CustomBypass,
    Pending,
    Triggered,
    Disarming,
}

impl AlarmState {
    pub fn parse(state: &str) -> Option<Self> {
        Some(match state.trim().to_ascii_lowercase().as_str() {
            "disarmed" | "ready" | "not_ready" => Self::Disarmed,
            "arming" | "exit_delay" => Self::Arming,
            "armed_home" | "armed_stay" => Self::Stay,
            "armed_instant" => Self::StayInstant,
            "armed_away" => Self::Away,
            "armed_max" | "armed_maximum" => Self::AwayMax,
            "armed_night" => Self::Night,
            "armed_vacation" => Self::Vacation,
            "armed_custom_bypass" => Self::CustomBypass,
            "pending" | "entry_delay" => Self::Pending,
            "triggered" | "alarm" => Self::Triggered,
            "disarming" => Self::Disarming,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Disarmed => "Disarmed",
            Self::Arming => "Exit delay",
            Self::Stay => "Armed stay",
            Self::StayInstant => "Armed instant",
            Self::Away => "Armed away",
            Self::AwayMax => "Armed maximum",
            Self::Night => "Armed night",
            Self::Vacation => "Armed vacation",
            Self::CustomBypass => "Armed with custom bypass",
            Self::Pending => "Entry delay",
            Self::Triggered => "Alarm triggered",
            Self::Disarming => "Disarming",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmAction {
    Disarm,
    ArmHome,
    ArmAway,
    ArmNight,
    Trigger,
}

impl AlarmAction {
    pub fn service(self) -> &'static str {
        match self {
            Self::Disarm => "alarm_disarm",
            Self::ArmHome => "alarm_arm_home",
            Self::ArmAway => "alarm_arm_away",
            Self::ArmNight => "alarm_arm_night",
            Self::Trigger => "alarm_trigger",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Disarm => "Disarm",
            Self::ArmHome => "Arm Stay",
            Self::ArmAway => "Arm Away",
            Self::ArmNight => "Arm Night",
            Self::Trigger => "Trigger Alarm",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub model: Model,
    pub zone: u8,
    pub entity: String,
    pub rule: Rule,
    pub operand: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rule {
    Equals,
    NotEquals,
    Above,
    Below,
    Changes,
}

impl Rule {
    pub const ALL: [Self; 5] = [Self::Equals, Self::NotEquals, Self::Above, Self::Below, Self::Changes];

    pub fn label(self) -> &'static str {
        match self {
            Self::Equals => "Is",
            Self::NotEquals => "Is not",
            Self::Above => "Above",
            Self::Below => "Below",
            Self::Changes => "Changes",
        }
    }

    pub fn accepts(self, operand: &str) -> bool {
        match self {
            Self::Changes => true,
            Self::Above | Self::Below => number(operand).is_some(),
            _ => !operand.trim().is_empty(),
        }
    }

    pub fn active(self, state: &str, operand: &str) -> Option<bool> {
        let s = state.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("unknown") || s.eq_ignore_ascii_case("unavailable") {
            return None;
        }
        Some(match self {
            Self::Equals => s.eq_ignore_ascii_case(operand.trim()),
            Self::NotEquals => !s.eq_ignore_ascii_case(operand.trim()),
            Self::Above => number(s)? > number(operand)?,
            Self::Below => number(s)? < number(operand)?,
            Self::Changes => false,
        })
    }
}

fn number(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|value| value.is_finite())
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Entity {
    pub entity_id: String,
    pub state: String,
    #[serde(default)]
    attributes: Attributes,
    #[serde(default)]
    last_updated: String,
    #[serde(default)]
    last_changed: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct Attributes {
    friendly_name: Option<String>,
    unit_of_measurement: Option<String>,
    device_class: Option<String>,
    supported_features: Option<u32>,
    code_format: Option<String>,
    code_arm_required: Option<bool>,
    bypassed: Option<bool>,
    bypassed_zones: Option<Vec<Value>>,
    /// Exposed by Home Assistant `manual` alarm_control_panel and Alarmo during `arming`/`pending`.
    next_state: Option<String>,
    /// Exposed by Alarmo (and some custom panels) in seconds for active exit/entry delays.
    delay: Option<Value>,
}

impl Entity {
    pub fn name(&self) -> &str {
        self.attributes.friendly_name.as_deref().unwrap_or(&self.entity_id)
    }

    pub fn domain(&self) -> &str {
        self.entity_id.split('.').next().unwrap_or("")
    }

    pub fn camera(&self) -> bool {
        self.entity_id.starts_with("camera.")
    }

    pub fn alarm_panel(&self) -> bool {
        if self.entity_id.starts_with("alarm_control_panel.") {
            return true;
        }
        // Also recognize ESPHome DSC / Vista partition status text sensors
        // (e.g. sensor.dscalarm_status_partition_1, sensor.vistaalarm_system_status_partition_1).
        self.entity_id.starts_with("sensor.")
            && (self.entity_id.contains("status_partition") || self.entity_id.contains("alarm_status"))
            && AlarmState::parse(&self.state).is_some()
    }

    pub fn supports_alarm_services(&self) -> bool {
        self.entity_id.starts_with("alarm_control_panel.")
    }

    pub fn binary(&self) -> bool {
        self.entity_id.starts_with("binary_sensor.")
            || self.entity_id.starts_with("input_boolean.")
            || self.entity_id.starts_with("switch.")
    }

    /// True for Home Assistant entities that RoomGuard can turn ON/OFF via `turn_on`/`turn_off`.
    pub fn is_writable_output(&self) -> bool {
        matches!(self.domain(), "siren" | "switch" | "light" | "input_boolean")
    }

    /// True for entities usable in Sirens & alarm outputs: writable sirens/switches/lights/toggles
    /// PLUS read-only ESPHome DSC/Vista alarm, bell, and fire binary sensors (`al_1`, `alm_1`, `fa_1`, `fire_1`, `bell`).
    pub fn is_output(&self) -> bool {
        if self.is_writable_output() {
            return true;
        }
        if self.domain() == "binary_sensor" {
            let id = self.entity_id.to_ascii_lowercase();
            let name = self.name().to_ascii_lowercase();
            return id.contains("siren")
                || id.contains("bell")
                || id.contains("alarm")
                || id.contains("fire")
                || id.ends_with("_al_1")
                || id.ends_with("_alm_1")
                || id.ends_with("_fa_1")
                || name.contains("siren")
                || name.contains("bell")
                || name.contains("alarm status")
                || name.contains("fire status");
        }
        false
    }

    /// True when this siren/alarm output represents a fire alarm (plays Temporal-3).
    pub fn is_fire_output(&self) -> bool {
        let id = self.entity_id.to_ascii_lowercase();
        let name = self.name().to_ascii_lowercase();
        let dc = self.device_class().unwrap_or("").to_ascii_lowercase();
        dc == "smoke" || dc == "fire" || id.contains("fire") || id.contains("_fa_") || name.contains("fire")
    }

    /// True when a Home Assistant siren, switch, toggle, or alarm binary_sensor is currently ON/active.
    pub fn is_output_on(&self) -> bool {
        matches!(
            self.state.trim().to_ascii_lowercase().as_str(),
            "on" | "true" | "triggered" | "active" | "alarm"
        )
    }

    /// Target alarm state exposed by Home Assistant `manual` / Alarmo during `arming` or `pending`.
    pub fn alarm_next_state(&self) -> Option<AlarmState> {
        self.attributes.next_state.as_deref().and_then(AlarmState::parse)
    }

    /// Remaining or total delay seconds exposed by Alarmo / custom HA alarm panels (`attributes.delay`).
    pub fn alarm_delay_secs(&self) -> Option<u64> {
        let v = self.attributes.delay.as_ref()?;
        let secs = v
            .as_u64()
            .or_else(|| v.as_f64().filter(|f| f.is_finite() && *f > 0.0).map(|f| f.round() as u64))
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))?;
        (secs > 0 && secs <= 600).then_some(secs)
    }

    /// Whole seconds elapsed since HA's `last_changed` RFC-3339 timestamp (clamped to 0..=600).
    pub fn elapsed_since_last_changed_secs(&self) -> Option<u64> {
        let raw = self.last_changed.trim();
        if raw.is_empty() {
            return None;
        }
        let changed = chrono::DateTime::parse_from_rfc3339(raw).ok()?;
        let now = chrono::Utc::now();
        let diff = now.signed_duration_since(changed.with_timezone(&chrono::Utc)).num_seconds();
        if diff < 0 {
            Some(0)
        } else {
            Some((diff as u64).min(600))
        }
    }

    /// Detects ESPHome DSC/Vista zone numbers from entity IDs or names like `_z1`, `_z17`, `(z9)`.
    pub fn esphome_zone_hint(&self, max_zone: u8) -> Option<u8> {
        for source in [self.entity_id.as_str(), self.name()] {
            let low = source.to_ascii_lowercase();
            for part in low.split(|c: char| !c.is_ascii_alphanumeric()) {
                if let Some(rest) = part.strip_prefix('z') {
                    if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                        if let Ok(n) = rest.parse::<u8>() {
                            if n >= 1 && n <= max_zone {
                                return Some(n);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// True for typical contact, motion, lock, or door/window binary sensors.
    pub fn is_contact_or_motion(&self) -> bool {
        matches!(self.domain(), "binary_sensor" | "cover" | "lock" | "input_boolean")
    }

    /// Filters out noisy internal Home Assistant entities (conversation, sun, backup events,
    /// timestamps, updates, weather, TTS, zones) from the default sensor browser.
    pub fn is_sensor_candidate(&self) -> bool {
        if self.entity_id.starts_with("sensor.sun_") || self.device_class() == Some("timestamp") {
            return false;
        }
        match self.domain() {
            "binary_sensor" | "sensor" | "cover" | "lock" | "input_boolean" | "input_number" | "switch" | "camera"
            | "siren" | "light" => true,
            "event" => !self.entity_id.contains("backup") && !self.entity_id.contains("update"),
            _ => false,
        }
    }

    /// Short, readable category tag for the entity browser row.
    pub fn category_badge(&self) -> &str {
        if let Some(dc) = self.device_class() {
            if !dc.trim().is_empty() {
                return dc;
            }
        }
        match self.domain() {
            "binary_sensor" => "binary",
            "input_boolean" => "toggle",
            "input_number" => "number",
            "device_tracker" => "tracker",
            other => other,
        }
    }

    /// Concise live state string (shortens raw ISO timestamps so rows stay clean).
    pub fn short_state(&self) -> String {
        let raw = self.description();
        // Trim ISO-8601 timestamps like "2026-11-27T18:44:21.398665+00:00" to "18:44:21".
        if raw.len() >= 19 && raw.as_bytes().get(4) == Some(&b'-') && raw.as_bytes().get(10) == Some(&b'T') {
            return raw[11..19].to_string();
        }
        let mut s: String = raw.chars().take(20).collect();
        if raw.chars().count() > 20 {
            s.push_str("...");
        }
        s
    }

    /// Visual live status hint for the entity row dot (`Some(true)` = active/open/on, `Some(false)` = closed/off).
    pub fn live_active_hint(&self) -> Option<bool> {
        let st = self.state.trim().to_ascii_lowercase();
        if st.is_empty() || matches!(st.as_str(), "unavailable" | "unknown") {
            return None;
        }
        if matches!(st.as_str(), "on" | "open" | "opening" | "unlocked" | "detected" | "triggered" | "home" | "active") {
            Some(true)
        } else if matches!(st.as_str(), "off" | "closed" | "locked" | "clear" | "idle" | "disarmed" | "not_home") {
            Some(false)
        } else {
            Some(false)
        }
    }

    /// Multi-word relevance search across friendly name, entity_id, device_class, domain, and state.
    /// Returns a sort priority (`0` = best match) when all words match.
    pub fn matches_search(&self, query: &str) -> Option<u8> {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return Some(2);
        }
        let name = self.name().to_ascii_lowercase();
        let id = self.entity_id.to_ascii_lowercase();
        let dc = self.device_class().unwrap_or("").to_ascii_lowercase();
        let st = self.state.to_ascii_lowercase();
        for term in q.split_whitespace() {
            if !name.contains(term) && !id.contains(term) && !dc.contains(term) && !st.contains(term) {
                return None;
            }
        }
        if name.starts_with(&q) || id.starts_with(&q) {
            Some(0)
        } else if name.contains(&q) || dc == q {
            Some(1)
        } else {
            Some(2)
        }
    }

    pub fn supports_night(&self) -> bool {
        // HA AlarmControlPanelEntityFeature.ARM_NIGHT = 4
        self.attributes.supported_features.map_or(true, |f| (f & 4) != 0)
    }

    pub fn supports_trigger(&self) -> bool {
        // HA AlarmControlPanelEntityFeature.TRIGGER = 32
        self.entity_id.starts_with("alarm_control_panel.")
            && self.attributes.supported_features.is_some_and(|f| (f & 32) != 0)
    }

    /// True if this entity reports its zone is bypassed (e.g. ESPHome Vista ECP
    /// zone text_sensor state `"B"`, `"bypassed"`, or `attributes.bypassed == true`).
    pub fn is_zone_bypassed(&self) -> bool {
        if self.attributes.bypassed == Some(true) {
            return true;
        }
        let s = self.state.trim();
        if self.domain() == "sensor" {
            return s.eq_ignore_ascii_case("b")
                || s.eq_ignore_ascii_case("bypass")
                || s.eq_ignore_ascii_case("bypassed");
        }
        false
    }

    /// Extracts any `bypassed_zones` integer/string attribute exposed by Alarmo or custom HA panels.
    pub fn attribute_bypassed_zones(&self, max_zone: u8) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(list) = &self.attributes.bypassed_zones {
            for item in list {
                let n = item
                    .as_u64()
                    .and_then(|v| u8::try_from(v).ok())
                    .or_else(|| item.as_str().and_then(|s| s.trim().parse::<u8>().ok()));
                if let Some(z) = n {
                    if z >= 1 && z <= max_zone && !out.contains(&z) {
                        out.push(z);
                    }
                }
            }
        }
        out
    }

    pub fn needs_code(&self) -> bool {
        self.attributes.code_format.as_deref().is_some_and(|fmt| !fmt.trim().is_empty())
    }

    pub fn code_arm_required(&self) -> bool {
        self.attributes.code_arm_required.unwrap_or(false)
    }

    pub fn default_rule_and_operand(&self) -> (Rule, String) {
        let domain = self.entity_id.split('.').next().unwrap_or("");
        match domain {
            "event" => (Rule::Changes, String::new()),
            "binary_sensor" | "input_boolean" | "switch" | "light" | "fan" => (Rule::Equals, "on".into()),
            "cover" => (Rule::Equals, "open".into()),
            "lock" => (Rule::Equals, "unlocked".into()),
            "person" | "device_tracker" => (Rule::Equals, "home".into()),
            "sensor" | "input_number" | "number" if number(&self.state).is_some() => (Rule::Above, self.state.clone()),
            _ => (Rule::Equals, "on".into()),
        }
    }

    pub fn device_class(&self) -> Option<&str> {
        self.attributes.device_class.as_deref()
    }

    /// Suggests the best EEPROM zone-type code for `model` based on this sensor's
    /// Home Assistant `device_class` and name.
    pub fn suggested_zone_code(&self, model: Model) -> &'static str {
        let dc = self.device_class().unwrap_or("").to_ascii_lowercase();
        let name = self.name().to_ascii_lowercase();
        let id = self.entity_id.to_ascii_lowercase();
        let has = |word: &str| dc == word || name.contains(word) || id.contains(word);
        let is_co = has("carbon_monoxide")
            || name == "co"
            || name.starts_with("co ")
            || name.ends_with(" co")
            || name.contains(" co ")
            || id.ends_with("_co")
            || id.contains("_co_");
        match model {
            Model::Pc1832 => {
                if has("smoke") || has("fire") || has("heat") {
                    "08" // 24h Standard Fire
                } else if is_co {
                    "41" // 24h Carbon Monoxide
                } else if has("moisture") || has("water") || has("leak") {
                    "19" // 24h Water
                } else if has("gas") {
                    "13" // 24h Gas
                } else if has("tamper") {
                    "21" // 24h Tamper
                } else if has("panic") || has("siren") {
                    "16" // 24h Panic
                } else if has("motion") || has("occupancy") || has("presence") {
                    "05" // Interior Stay/Away
                } else if has("door") || has("garage") || has("lock") {
                    "01" // Delay 1 (Entry/Exit)
                } else {
                    "03" // Instant (Perimeter / Window / Vibration)
                }
            }
            Model::Vista20p => {
                if has("smoke") || has("fire") || has("heat") {
                    "09" // Fire
                } else if is_co {
                    "14" // Carbon Monoxide
                } else if has("moisture") || has("water") || has("leak") || has("gas") || has("tamper") || has("panic") || has("siren") {
                    "07" // 24h Audible Alarm
                } else if has("motion") || has("occupancy") || has("presence") {
                    "04" // Interior Follower
                } else if has("door") || has("garage") || has("lock") {
                    "01" // Entry/Exit #1
                } else {
                    "03" // Perimeter (Instant / Window / Vibration)
                }
            }
            Model::Pc1550 => {
                if has("smoke") || has("fire") || has("heat") {
                    "08" // 24h Fire
                } else if is_co || has("moisture") || has("water") || has("leak") || has("gas") || has("tamper") || has("panic") || has("siren") {
                    "04" // 24h Bell
                } else if has("motion") || has("occupancy") || has("presence") {
                    "03" // Interior Home-Away
                } else if has("door") || has("garage") || has("lock") {
                    "00" // Standard Delay
                } else {
                    "01" // Instant
                }
            }
        }
    }

    pub fn description(&self) -> String {
        match &self.attributes.unit_of_measurement {
            Some(unit) => format!("{} {}", self.state, unit),
            None => self.state.clone(),
        }
    }
}

pub type Entities = BTreeMap<String, Entity>;

/// Parsed snapshot of ESPHome DSC Keybus / Vista ECP `zs` (`sensor.*_zone_status`)
/// or Home Assistant Test Lab `input_text.roomguard_test_bypasses`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZoneStatusSnapshot {
    pub bypassed: BTreeSet<u8>,
    pub alarmed: BTreeSet<u8>,
    pub open: BTreeSet<u8>,
    pub tampered: BTreeSet<u8>,
    pub faulted: BTreeSet<u8>,
}

/// Parses ESPHome `zs` (`OP:1,BY:3,AL:2,TA:4,CK:5,ZF:6`) and optionally bare zone numbers
/// (`"1, 3, 5"` in `input_text.roomguard_test_bypasses`) into structured zone sets.
pub fn parse_zone_status(raw: &str, max_zone: u8, allow_bare_nums: bool) -> ZoneStatusSnapshot {
    let mut snap = ZoneStatusSnapshot::default();
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("unknown")
        || trimmed.eq_ignore_ascii_case("unavailable")
        || trimmed.eq_ignore_ascii_case("none")
    {
        return snap;
    }
    for token in trimmed.split(|c: char| c == ',' || c == ';' || c.is_ascii_whitespace()) {
        let tok = token.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some((prefix, num_str)) = tok.split_once(':') {
            if let Ok(z) = num_str.trim().parse::<u8>() {
                if z >= 1 && z <= max_zone {
                    match prefix.trim().to_ascii_uppercase().as_str() {
                        "BY" | "BYP" | "BYPASS" => { snap.bypassed.insert(z); }
                        "AL" | "ALM" | "ALARM" => { snap.alarmed.insert(z); }
                        "OP" | "OPEN" => { snap.open.insert(z); }
                        "TA" | "TAMP" | "TAMPER" => { snap.tampered.insert(z); }
                        "CK" | "ZF" | "FAULT" | "CHECK" => { snap.faulted.insert(z); }
                        _ => {}
                    }
                }
            }
        } else if allow_bare_nums {
            let digits = tok.strip_prefix('z').or_else(|| tok.strip_prefix('Z')).unwrap_or(tok);
            if let Ok(z) = digits.parse::<u8>() {
                if z >= 1 && z <= max_zone {
                    snap.bypassed.insert(z);
                }
            }
        }
    }
    snap
}

/// Formats a set of bypassed zones into canonical ESPHome/Lab `BY:1,BY:3` format.
pub fn format_bypass_list(zones: &[u8]) -> String {
    zones
        .iter()
        .map(|z| format!("BY:{z}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Detects the ESPHome node prefix (e.g. `"dscalarm"` or `"vistaalarm"`) from discovered
/// `esphome.*` services matching the linked alarm panel.
pub fn detect_esphome_prefix(followed_panel: &str, esphome_services: &[String]) -> Option<String> {
    let mut prefixes: Vec<String> = Vec::new();
    for svc in esphome_services {
        for suffix in ["_alarm_keypress_partition", "_alarm_keypress", "_set_alarm_state"] {
            if let Some(prefix) = svc.strip_suffix(suffix) {
                if !prefix.is_empty() && !prefixes.iter().any(|p| p == prefix) {
                    prefixes.push(prefix.to_string());
                }
            }
        }
    }
    let fp = followed_panel.to_ascii_lowercase();
    if let Some(matched) = prefixes.iter().find(|p| fp.contains(&p.to_ascii_lowercase())) {
        return Some(matched.clone());
    }
    if prefixes.len() == 1 {
        return Some(prefixes[0].clone());
    }
    None
}

/// Extracts a 1-based partition number from an entity ID like `sensor.dscalarm_status_partition_2` (defaults to 1).
pub fn detect_partition(entity_id: &str) -> u8 {
    let low = entity_id.to_ascii_lowercase();
    if let Some((_, rest)) = low.rsplit_once("partition_") {
        if let Some(d) = rest.chars().next().and_then(|c| c.to_digit(10)) {
            if (1..=8).contains(&d) {
                return d as u8;
            }
        }
    }
    if let Some((_, tail)) = low.rsplit_once('_') {
        if tail.len() == 1 {
            if let Some(d) = tail.chars().next().and_then(|c| c.to_digit(10)) {
                if (1..=8).contains(&d) {
                    return d as u8;
                }
            }
        }
    }
    1
}

/// Companion entities automatically discovered alongside a linked alarm panel
/// (ESPHome DSC Keybus / Vista ECP `zs`, `byp_p`, `armi_p`, `fa_p`/`fire_p`, `al_p`/`alm_p`,
/// or the RoomGuard Test Lab `input_text.roomguard_test_bypasses`).
#[derive(Clone, Debug, Default)]
pub struct PanelCompanions {
    pub zone_status: Option<String>,
    pub lab_bypasses: Option<String>,
    pub bypass_flag: Option<String>,
    pub instant_flag: Option<String>,
    pub fire_flag: Option<String>,
    pub alarm_flag: Option<String>,
}

pub fn find_panel_companions(
    followed_panel: &str,
    entities: &Entities,
    esphome_prefix: Option<&str>,
) -> PanelCompanions {
    let part = detect_partition(followed_panel);
    let pfx = esphome_prefix.map(|s| s.to_ascii_lowercase());
    let matches_prefix = |id: &str| -> bool {
        match &pfx {
            Some(p) => id.contains(p),
            None => true,
        }
    };
    let mut comp = PanelCompanions::default();
    let is_lab = followed_panel.contains("roomguard_test_panel") || followed_panel.contains("roomguard_lab");
    if is_lab && entities.contains_key("input_text.roomguard_test_bypasses") {
        comp.lab_bypasses = Some("input_text.roomguard_test_bypasses".to_string());
    }
    for (id, ent) in entities {
        let low = id.to_ascii_lowercase();
        let dom = ent.domain();
        if dom == "input_text" && comp.lab_bypasses.is_none() && (low.contains("roomguard") && low.contains("bypass")) {
            comp.lab_bypasses = Some(id.clone());
        }
        if !matches_prefix(&low) {
            continue;
        }
        if dom == "sensor" {
            if comp.zone_status.is_none()
                && (low.ends_with("_zone_status") || low.ends_with("_zs"))
            {
                comp.zone_status = Some(id.clone());
            }
        } else if dom == "binary_sensor" {
            let byp_suf = format!("_byp_{part}");
            let byp_suf2 = format!("_bypass_partition_{part}");
            if comp.bypass_flag.is_none() && (low.ends_with(&byp_suf) || low.ends_with(&byp_suf2)) {
                comp.bypass_flag = Some(id.clone());
            }
            let inst_suf = format!("_armi_{part}");
            let inst_suf2 = format!("_instant_partition_{part}");
            if comp.instant_flag.is_none() && (low.ends_with(&inst_suf) || low.ends_with(&inst_suf2)) {
                comp.instant_flag = Some(id.clone());
            }
            let fa_suf = format!("_fa_{part}");
            let fire_suf = format!("_fire_partition_{part}");
            let fire_suf2 = format!("_fire_status_partition_{part}");
            if comp.fire_flag.is_none()
                && (low.ends_with(&fa_suf) || low.ends_with(&fire_suf) || low.ends_with(&fire_suf2))
            {
                comp.fire_flag = Some(id.clone());
            }
            let al_suf = format!("_al_{part}");
            let alm_suf = format!("_alm_{part}");
            let alm_suf2 = format!("_alarm_partition_{part}");
            let alm_suf3 = format!("_alarm_status_partition_{part}");
            if comp.alarm_flag.is_none()
                && (low.ends_with(&al_suf)
                    || low.ends_with(&alm_suf)
                    || low.ends_with(&alm_suf2)
                    || low.ends_with(&alm_suf3))
            {
                comp.alarm_flag = Some(id.clone());
            }
        }
    }
    comp
}

pub enum Update {
    Connection(ConnectionState),
    Snapshot(Entities),
    /// Periodic / post-command live state refresh so RoomGuard and Home Assistant never drift out of sync.
    Refresh(Entities),
    /// Discovered `esphome.*` service names (e.g. `dscalarm_alarm_keypress`, `vistaalarm_set_alarm_state`).
    Services(Vec<String>),
    State { id: String, entity: Option<Entity> },
    ServiceResult { action: AlarmAction, result: Result<(), String> },
    OutputResult { entity_id: String, on: bool, result: Result<(), String> },
    KeypressResult { service: String, result: Result<(), String> },
    /// Home Assistant refused one of RoomGuard's announcements: an event
    /// (which needs an administrator's token) or a notification service.
    AnnounceFailed { what: String, error: String },
    Preview { id: String, result: Result<Arc<Rgb>, String> },
}

struct Connection {
    epoch: u64,
    url: String,
    token: String,
}

#[derive(Clone, Debug)]
enum CallKind {
    Alarm { action: AlarmAction, code: Option<String> },
    Output { on: bool },
    EspHomeKeypress { service: String, keys: String, partition: Option<u8> },
    EspHomeAlarmState { service: String, state: String, code: String, partition: u8, action: AlarmAction },
    InputText { value: String },
    /// A Home Assistant event for automations, e.g. `roomguard_alarm`.
    Event { event_type: String, data: Value },
    /// Any other service, e.g. `persistent_notification.create` or a phone's notify service.
    Service { domain: String, service: String, data: Value },
}

#[derive(Clone, Debug)]
struct ServiceCall {
    epoch: u64,
    entity_id: String,
    kind: CallKind,
}

pub struct Handle {
    config: mpsc::Sender<Option<Connection>>,
    calls: mpsc::Sender<ServiceCall>,
    previews: mpsc::SyncSender<(Connection, String)>,
}

impl Handle {
    pub fn configure(&self, cfg: &Config, epoch: u64) {
        let _ = self.config.send(cfg.enabled.then(|| connection(cfg, epoch)));
    }

    pub fn call_alarm(&self, epoch: u64, entity_id: String, action: AlarmAction, code: Option<String>) {
        let _ = self.calls.send(ServiceCall {
            epoch,
            entity_id,
            kind: CallKind::Alarm { action, code },
        });
    }

    pub fn set_output(&self, epoch: u64, entity_id: String, on: bool) {
        let _ = self.calls.send(ServiceCall {
            epoch,
            entity_id,
            kind: CallKind::Output { on },
        });
    }

    pub fn call_esphome_keypress(&self, epoch: u64, service: String, keys: String, partition: Option<u8>) {
        let _ = self.calls.send(ServiceCall {
            epoch,
            entity_id: String::new(),
            kind: CallKind::EspHomeKeypress { service, keys, partition },
        });
    }

    pub fn call_esphome_alarm_state(
        &self,
        epoch: u64,
        service: String,
        state: String,
        code: String,
        partition: u8,
        action: AlarmAction,
    ) {
        let _ = self.calls.send(ServiceCall {
            epoch,
            entity_id: String::new(),
            kind: CallKind::EspHomeAlarmState { service, state, code, partition, action },
        });
    }

    pub fn set_input_text(&self, epoch: u64, entity_id: String, value: String) {
        let _ = self.calls.send(ServiceCall {
            epoch,
            entity_id,
            kind: CallKind::InputText { value },
        });
    }

    /// Fires a Home Assistant event (an automation trigger).
    pub fn fire_event(&self, epoch: u64, event_type: &str, data: Value) {
        let _ = self.calls.send(ServiceCall {
            epoch,
            entity_id: String::new(),
            kind: CallKind::Event { event_type: event_type.into(), data },
        });
    }

    /// Calls any Home Assistant service with the given data.
    pub fn call_service(&self, epoch: u64, domain: &str, service: &str, data: Value) {
        let _ = self.calls.send(ServiceCall {
            epoch,
            entity_id: String::new(),
            kind: CallKind::Service { domain: domain.into(), service: service.into(), data },
        });
    }

    pub fn preview(&self, cfg: &Config, epoch: u64, id: String) -> bool {
        self.previews.try_send((connection(cfg, epoch), id)).is_ok()
    }
}

fn connection(cfg: &Config, epoch: u64) -> Connection {
    Connection { epoch, url: normalized_url(&cfg.url), token: cfg.token.trim().into() }
}

/// `homeassistant.local:8123` typed without a scheme means plain http, like a
/// browser assumes. Trailing slashes are dropped.
fn normalized_url(raw: &str) -> String {
    let url = raw.trim().trim_end_matches('/');
    if url.is_empty() || url.contains("://") {
        url.into()
    } else {
        format!("http://{}", url)
    }
}

pub fn start(engine: mpsc::Sender<EngineMsg>) -> Handle {
    let (config, settings) = mpsc::channel();
    let (calls, call_rx) = mpsc::channel();
    let (previews, requests) = mpsc::sync_channel::<(Connection, String)>(1);
    let updates = engine.clone();
    let _ = std::thread::Builder::new()
        .name("home-assistant".into())
        .spawn(move || run(settings, call_rx, updates));
    let _ = std::thread::Builder::new().name("home-assistant-preview".into()).spawn(move || {
        for (cfg, id) in requests {
            let result = preview(&cfg, &id).map(Arc::new);
            emit(&engine, cfg.epoch, Update::Preview { id, result });
        }
    });
    Handle { config, calls, previews }
}

fn emit(engine: &mpsc::Sender<EngineMsg>, epoch: u64, update: Update) {
    let _ = engine.send(EngineMsg::HomeAssistant { epoch, update });
}

fn run(
    settings: mpsc::Receiver<Option<Connection>>,
    calls: mpsc::Receiver<ServiceCall>,
    engine: mpsc::Sender<EngineMsg>,
) {
    let mut selected = None;
    loop {
        while let Ok(newest) = settings.try_recv() {
            selected = newest;
        }
        let Some(cfg) = selected.as_ref() else {
            // Drain any stale queued calls while disconnected.
            while calls.try_recv().is_ok() {}
            match settings.recv() {
                Ok(cfg) => selected = cfg,
                Err(_) => return,
            }
            continue;
        };
        emit(&engine, cfg.epoch, Update::Connection(ConnectionState::Connecting));
        match session(cfg, &settings, &calls, &engine) {
            Ok(next) => selected = next,
            Err(error) => {
                emit(&engine, cfg.epoch, Update::Connection(ConnectionState::Error(error)));
                match settings.recv_timeout(Duration::from_secs(4)) {
                    Ok(next) => selected = next,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        }
    }
}

fn endpoint(cfg: &Connection) -> Result<tungstenite::http::Uri, String> {
    let uri = cfg
        .url
        .parse::<tungstenite::http::Uri>()
        .map_err(|_| "Enter a Home Assistant URL starting with http:// or https://".to_string())?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri.host().is_none()
        || uri.authority().is_some_and(|authority| authority.as_str().contains('@'))
        || uri.query().is_some()
        || cfg.url.contains('#')
    {
        return Err("Enter a Home Assistant base URL without credentials, a query, or a fragment".into());
    }
    if cfg.token.is_empty() {
        return Err("Enter a Home Assistant long-lived access token".into());
    }
    Ok(uri)
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

fn open(cfg: &Connection) -> Result<Socket, String> {
    let uri = endpoint(cfg)?;
    let secure = uri.scheme_str() == Some("https");
    let hostname = uri.host().unwrap_or_default().trim_matches(&['[', ']'][..]);
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let addresses = (hostname, port).to_socket_addrs().map_err(|_| "Home Assistant host could not be resolved")?;
    let tcp = addresses
        .take(4)
        .find_map(|address| TcpStream::connect_timeout(&address, Duration::from_secs(3)).ok())
        .ok_or("Home Assistant is unreachable")?;
    tcp.set_read_timeout(Some(Duration::from_secs(4))).map_err(|_| "Could not set Home Assistant read timeout")?;
    tcp.set_write_timeout(Some(Duration::from_secs(4))).map_err(|_| "Could not set Home Assistant write timeout")?;
    let prefix = if secure { "wss" } else { "ws" };
    // Never panic on the typed URL: that would end this thread silently.
    let rest = cfg.url.split_once("://").map_or(cfg.url.as_str(), |(_, rest)| rest);
    let url = format!("{}://{}/api/websocket", prefix, rest);
    let request = url.into_client_request().map_err(|_| "Invalid Home Assistant WebSocket URL")?;
    let limits = tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(16 * 1024 * 1024),
        max_frame_size: Some(16 * 1024 * 1024),
        ..Default::default()
    };
    let (mut socket, _) = tungstenite::client_tls_with_config(request, tcp, Some(limits), None)
        .map_err(|_| "Could not open Home Assistant WebSocket (check URL and TLS certificate)")?;
    let tcp = match socket.get_mut() {
        MaybeTlsStream::Plain(tcp) => tcp,
        MaybeTlsStream::Rustls(tls) => &mut tls.sock,
        _ => return Err("Unsupported Home Assistant connection".into()),
    };
    tcp.set_read_timeout(Some(Duration::from_millis(100))).map_err(|_| "Could not set Home Assistant read timeout")?;
    Ok(socket)
}

fn send(socket: &mut Socket, value: Value) -> Result<(), String> {
    socket.send(Message::Text(value.to_string())).map_err(|_| "Home Assistant connection lost".into())
}

fn read(socket: &mut Socket) -> Result<Option<Value>, String> {
    match socket.read() {
        Ok(Message::Text(text)) => serde_json::from_str(&text).map(Some).map_err(|_| "Invalid message from Home Assistant".into()),
        Ok(Message::Ping(payload)) => {
            let _ = socket.send(Message::Pong(payload));
            Ok(None)
        }
        Ok(Message::Close(_)) => Err("Home Assistant closed the connection".into()),
        Ok(_) => Ok(None),
        Err(tungstenite::Error::Io(error)) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => Ok(None),
        Err(_) => Err("Home Assistant connection lost".into()),
    }
}

fn wait_for(socket: &mut Socket, accepts: impl Fn(&Value) -> bool) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Some(message) = read(socket)? {
            if accepts(&message) {
                return Ok(message);
            }
        }
    }
    Err("Home Assistant did not answer in time".into())
}

fn state_change(message: &Value) -> Option<(String, Option<Entity>)> {
    if message["type"] != "event" || message["event"]["event_type"] != "state_changed" {
        return None;
    }
    let data = &message["event"]["data"];
    let id = data["entity_id"].as_str()?.to_owned();
    let entity = if data["new_state"].is_null() {
        None
    } else {
        Some(serde_json::from_value::<Entity>(data["new_state"].clone()).ok()?)
    };
    Some((id, entity))
}

fn service_call_payload(id: u64, call: &ServiceCall) -> Option<Value> {
    match &call.kind {
        CallKind::Alarm { action, code } => {
            let mut service_data = serde_json::Map::new();
            if let Some(code) = code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                service_data.insert("code".into(), json!(code));
            }
            let mut payload = json!({
                "id": id,
                "type": "call_service",
                "domain": "alarm_control_panel",
                "service": action.service(),
                "target": {
                    "entity_id": call.entity_id,
                },
            });
            if !service_data.is_empty() {
                payload["service_data"] = Value::Object(service_data);
            }
            Some(payload)
        }
        CallKind::Output { on } => {
            let domain = call.entity_id.split('.').next().unwrap_or("");
            if !matches!(domain, "siren" | "switch" | "light" | "input_boolean") {
                return None;
            }
            Some(json!({
                "id": id,
                "type": "call_service",
                "domain": domain,
                "service": if *on { "turn_on" } else { "turn_off" },
                "target": {
                    "entity_id": call.entity_id,
                },
            }))
        }
        CallKind::EspHomeKeypress { service, keys, partition } => {
            if service.trim().is_empty() || keys.trim().is_empty() {
                return None;
            }
            let mut data = serde_json::Map::new();
            data.insert("keys".into(), json!(keys));
            if let Some(p) = partition {
                if service.ends_with("_partition") {
                    data.insert("partition".into(), json!(p));
                }
            }
            Some(json!({
                "id": id,
                "type": "call_service",
                "domain": "esphome",
                "service": service,
                "service_data": Value::Object(data),
            }))
        }
        CallKind::EspHomeAlarmState { service, state, code, partition, .. } => {
            if service.trim().is_empty() {
                return None;
            }
            Some(json!({
                "id": id,
                "type": "call_service",
                "domain": "esphome",
                "service": service,
                "service_data": {
                    "state": state,
                    "code": code,
                    "partition": partition,
                },
            }))
        }
        CallKind::Event { event_type, data } => {
            if event_type.trim().is_empty() {
                return None;
            }
            Some(json!({
                "id": id,
                "type": "fire_event",
                "event_type": event_type,
                "event_data": data,
            }))
        }
        CallKind::Service { domain, service, data } => {
            if domain.trim().is_empty() || service.trim().is_empty() {
                return None;
            }
            Some(json!({
                "id": id,
                "type": "call_service",
                "domain": domain,
                "service": service,
                "service_data": data,
            }))
        }
        CallKind::InputText { value } => {
            if !call.entity_id.starts_with("input_text.") {
                return None;
            }
            Some(json!({
                "id": id,
                "type": "call_service",
                "domain": "input_text",
                "service": "set_value",
                "target": {
                    "entity_id": call.entity_id,
                },
                "service_data": {
                    "value": value,
                },
            }))
        }
    }
}

pub fn extract_esphome_services(result: &Value) -> Vec<String> {
    let Some(map) = result.get("esphome").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<String> = map.keys().cloned().collect();
    out.sort();
    out
}

fn session(
    cfg: &Connection,
    settings: &mpsc::Receiver<Option<Connection>>,
    calls: &mpsc::Receiver<ServiceCall>,
    engine: &mpsc::Sender<EngineMsg>,
) -> Result<Option<Connection>, String> {
    let mut socket = open(cfg)?;
    wait_for(&mut socket, |message| message["type"] == "auth_required")?;
    send(&mut socket, json!({"type":"auth","access_token":cfg.token}))?;
    let auth = wait_for(&mut socket, |message| matches!(message["type"].as_str(), Some("auth_ok" | "auth_invalid")))?;
    if auth["type"] != "auth_ok" {
        return Err("Home Assistant rejected the access token".into());
    }

    // Subscribe before reading the baseline so changes around connection
    // setup can be merged without treating the baseline as fresh motion.
    send(&mut socket, json!({"id":1,"type":"subscribe_events","event_type":"state_changed"}))?;
    let ack = wait_for(&mut socket, |message| message["type"] == "result" && message["id"] == 1)?;
    if ack["success"] != true {
        return Err("Home Assistant denied the state subscription".into());
    }
    send(&mut socket, json!({"id":2,"type":"get_states"}))?;

    let deadline = Instant::now() + Duration::from_secs(12);
    let mut buffered: BTreeMap<String, Option<Entity>> = BTreeMap::new();
    let mut pending_calls: HashMap<u64, ServiceCall> = HashMap::new();
    let mut online = false;
    let mut next_id = 2u64;
    let mut active_ping: Option<(u64, Instant)> = None;
    let mut next_ping = Instant::now() + Duration::from_secs(6);
    let mut refresh_id: Option<u64> = None;
    let mut next_refresh = Instant::now() + Duration::from_secs(10);
    let mut services_id: Option<u64> = None;
    let mut services_at: Option<Instant> = None;
    loop {
        match settings.try_recv() {
            Ok(next) => return Ok(next),
            Err(mpsc::TryRecvError::Disconnected) => return Ok(None),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let now = Instant::now();
        if !online && now >= deadline {
            return Err("Home Assistant did not return entity states".into());
        }
        // Send any outbound alarm panel or siren/output service calls once online.
        if online {
            while let Ok(call) = calls.try_recv() {
                if call.epoch != cfg.epoch {
                    continue;
                }
                next_id += 1;
                if let Some(payload) = service_call_payload(next_id, &call) {
                    send(&mut socket, payload)?;
                    pending_calls.insert(next_id, call);
                }
            }
            if services_id.is_none() && services_at.is_some_and(|at| now >= at) {
                services_at = None;
                next_id += 1;
                send(&mut socket, json!({"id":next_id,"type":"get_services"}))?;
                services_id = Some(next_id);
            }
            if refresh_id.is_none() && now >= next_refresh {
                next_id += 1;
                send(&mut socket, json!({"id":next_id,"type":"get_states"}))?;
                refresh_id = Some(next_id);
            }
        }
        if let Some((_, sent_at)) = active_ping {
            if now.saturating_duration_since(sent_at) >= Duration::from_secs(5) {
                return Err("Home Assistant stopped responding".into());
            }
        } else if now >= next_ping {
            next_id += 1;
            send(&mut socket, json!({"id":next_id,"type":"ping"}))?;
            active_ping = Some((next_id, now));
        }
        let Some(message) = read(&mut socket)? else { continue };
        // Any valid frame proves the connection is alive; schedule next ping cleanly.
        if message["type"] == "pong" && active_ping.is_some_and(|(pid, _)| message["id"] == pid) {
            active_ping = None;
            next_ping = Instant::now() + Duration::from_secs(6);
        } else if !online && message["type"] == "result" && message["id"] == 2 {
            if message["success"] != true {
                return Err("Home Assistant denied access to entity states".into());
            }
            let states = message["result"].as_array().ok_or("Invalid entity list from Home Assistant")?;
            let mut entities: Entities = states
                .iter()
                .filter_map(|state| serde_json::from_value::<Entity>(state.clone()).ok())
                .map(|entity| (entity.entity_id.clone(), entity))
                .collect();
            for (id, entity) in std::mem::take(&mut buffered) {
                match entity {
                    Some(entity) if entities.get(&id).map_or(true, |baseline| baseline.last_updated <= entity.last_updated) => {
                        entities.insert(id, entity);
                    }
                    None => {
                        entities.remove(&id);
                    }
                    _ => {}
                }
            }
            emit(engine, cfg.epoch, Update::Snapshot(entities));
            online = true;
            next_refresh = Instant::now() + Duration::from_secs(10);
            services_at = Some(Instant::now() + Duration::from_millis(600));
        } else if message["type"] == "result" && services_id.is_some_and(|sid| message["id"] == sid) {
            services_id = None;
            if message["success"] == true {
                let svcs = extract_esphome_services(&message["result"]);
                emit(engine, cfg.epoch, Update::Services(svcs));
            }
        } else if message["type"] == "result" && refresh_id.is_some_and(|rid| message["id"] == rid) {
            refresh_id = None;
            next_refresh = Instant::now() + Duration::from_secs(10);
            if message["success"] == true {
                if let Some(states) = message["result"].as_array() {
                    let entities: Entities = states
                        .iter()
                        .filter_map(|state| serde_json::from_value::<Entity>(state.clone()).ok())
                        .map(|entity| (entity.entity_id.clone(), entity))
                        .collect();
                    emit(engine, cfg.epoch, Update::Refresh(entities));
                }
            }
        } else if message["type"] == "result" {
            if let Some(id) = message["id"].as_u64() {
                if let Some(call) = pending_calls.remove(&id) {
                    let result = if message["success"] == true {
                        Ok(())
                    } else {
                        let detail = message["error"]["message"]
                            .as_str()
                            .unwrap_or("Home Assistant rejected the command");
                        Err(detail.to_owned())
                    };
                    match call.kind {
                        CallKind::Alarm { action, .. } | CallKind::EspHomeAlarmState { action, .. } => {
                            emit(engine, cfg.epoch, Update::ServiceResult { action, result });
                            // Verify live panel state shortly after every outbound command in case HA
                            // ignored a wrong PIN silently (manual alarm_control_panel returns success:true).
                            let soon = Instant::now() + Duration::from_millis(1200);
                            if soon < next_refresh {
                                next_refresh = soon;
                            }
                        }
                        CallKind::Output { on } => {
                            emit(
                                engine,
                                cfg.epoch,
                                Update::OutputResult { entity_id: call.entity_id, on, result },
                            );
                        }
                        CallKind::EspHomeKeypress { service, .. } => {
                            emit(engine, cfg.epoch, Update::KeypressResult { service, result });
                            let soon = Instant::now() + Duration::from_millis(1200);
                            if soon < next_refresh {
                                next_refresh = soon;
                            }
                        }
                        CallKind::InputText { .. } => {}
                        CallKind::Event { event_type, .. } => {
                            if let Err(e) = result {
                                // Firing events needs an administrator's token.
                                crate::util::log(&format!("Home Assistant refused the {} event: {}", event_type, e));
                                emit(engine, cfg.epoch, Update::AnnounceFailed { what: format!("the {} event", event_type), error: e });
                            }
                        }
                        CallKind::Service { domain, service, .. } => {
                            if let Err(e) = result {
                                crate::util::log(&format!("Home Assistant {}.{} failed: {}", domain, service, e));
                                emit(engine, cfg.epoch, Update::AnnounceFailed { what: format!("{}.{}", domain, service), error: e });
                            }
                        }
                    }
                }
            }
        } else if let Some((id, entity)) = state_change(&message) {
            if online {
                emit(engine, cfg.epoch, Update::State { id, entity });
            } else {
                buffered.insert(id, entity);
            }
        }
    }
}

fn preview(cfg: &Connection, id: &str) -> Result<Rgb, String> {
    endpoint(cfg)?;
    if !id.starts_with("camera.") || !id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._".contains(&byte)) {
        return Err("Invalid Home Assistant camera".into());
    }
    let response = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(6))
        .build()
        .get(&format!("{}/api/camera_proxy/{}", cfg.url, id))
        .set("Authorization", &format!("Bearer {}", cfg.token))
        .call()
        .map_err(|_| "Home Assistant could not provide a camera snapshot")?;
    let mut bytes = Vec::new();
    response.into_reader().take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| "Camera snapshot could not be read")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Camera snapshot exceeds 8 MB".into());
    }
    let mut decoder = jpeg_decoder::Decoder::new(bytes.as_slice());
    decoder.read_info().map_err(|_| "Camera did not return a supported JPEG snapshot")?;
    let info = decoder.info().ok_or("Camera snapshot has no dimensions")?;
    if u32::from(info.width) * u32::from(info.height) > 3840 * 2160 {
        return Err("Camera snapshot exceeds 8 megapixels".into());
    }
    decode_jpeg_file(&bytes).ok_or_else(|| "Camera did not return a supported JPEG snapshot".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_without_a_scheme_is_treated_as_http() {
        assert_eq!(normalized_url(" homeassistant.local:8123/ "), "http://homeassistant.local:8123");
        assert_eq!(normalized_url("https://ha.example.com/"), "https://ha.example.com");
        assert_eq!(normalized_url(""), "");
    }

    #[test]
    fn events_and_notifications_use_home_assistant_message_formats() {
        let event = service_call_payload(
            7,
            &ServiceCall {
                epoch: 1,
                entity_id: String::new(),
                kind: CallKind::Event { event_type: "roomguard_alarm".into(), data: json!({"state": "triggered"}) },
            },
        )
        .unwrap();
        assert_eq!(event["type"], "fire_event");
        assert_eq!(event["event_type"], "roomguard_alarm");
        assert_eq!(event["event_data"]["state"], "triggered");
        let note = service_call_payload(
            8,
            &ServiceCall {
                epoch: 1,
                entity_id: String::new(),
                kind: CallKind::Service {
                    domain: "persistent_notification".into(),
                    service: "create".into(),
                    data: json!({"message": "x"}),
                },
            },
        )
        .unwrap();
        assert_eq!(note["type"], "call_service");
        assert_eq!(note["domain"], "persistent_notification");
        assert_eq!(note["service"], "create");
        assert_eq!(note["service_data"]["message"], "x");
    }

    #[test]
    fn sensor_rules_handle_binary_numeric_case_and_unavailable_states() {
        assert_eq!(Rule::Equals.active("on", "on"), Some(true));
        assert_eq!(Rule::Equals.active("ON", "on"), Some(true));
        assert_eq!(Rule::Equals.active("off", "on"), Some(false));
        assert_eq!(Rule::Above.active("20.1", "20"), Some(true));
        assert_eq!(Rule::Below.active("19.9", "20"), Some(true));
        assert_eq!(Rule::Below.active("20", "20"), Some(false));
        assert_eq!(Rule::Below.active("NaN", "20"), None);
        assert_eq!(Rule::NotEquals.active("unavailable", "off"), None);
        assert_eq!(Rule::Changes.active("unknown", ""), None);
    }

    #[test]
    fn state_change_reads_new_entities_and_removals() {
        let message = json!({"type":"event","event":{"event_type":"state_changed","data":{
            "entity_id":"binary_sensor.door",
            "new_state":{"entity_id":"binary_sensor.door","state":"on","attributes":{"friendly_name":"Door"}}
        }}});
        let (id, entity) = state_change(&message).unwrap();
        assert_eq!(id, "binary_sensor.door");
        assert_eq!(entity.unwrap().name(), "Door");
        let mut removed = message;
        removed["event"]["data"]["new_state"] = Value::Null;
        assert!(state_change(&removed).unwrap().1.is_none());
    }

    #[test]
    fn service_call_payload_includes_code_only_when_provided() {
        let with_code = service_call_payload(
            7,
            &ServiceCall {
                epoch: 1,
                entity_id: "alarm_control_panel.roomguard_test_panel".into(),
                kind: CallKind::Alarm { action: AlarmAction::Disarm, code: Some("1234".into()) },
            },
        )
        .unwrap();
        assert_eq!(with_code["type"], "call_service");
        assert_eq!(with_code["domain"], "alarm_control_panel");
        assert_eq!(with_code["service"], "alarm_disarm");
        assert_eq!(with_code["target"]["entity_id"], "alarm_control_panel.roomguard_test_panel");
        assert_eq!(with_code["service_data"]["code"], "1234");

        let without_code = service_call_payload(
            8,
            &ServiceCall {
                epoch: 1,
                entity_id: "alarm_control_panel.roomguard_test_panel".into(),
                kind: CallKind::Alarm { action: AlarmAction::ArmAway, code: Some("   ".into()) },
            },
        )
        .unwrap();
        assert_eq!(without_code["service"], "alarm_arm_away");
        assert_eq!(without_code["target"]["entity_id"], "alarm_control_panel.roomguard_test_panel");
        assert!(without_code.get("service_data").is_none());

        let siren_on = service_call_payload(
            9,
            &ServiceCall {
                epoch: 1,
                entity_id: "siren.hallway_horn".into(),
                kind: CallKind::Output { on: true },
            },
        )
        .unwrap();
        assert_eq!(siren_on["domain"], "siren");
        assert_eq!(siren_on["service"], "turn_on");
        assert_eq!(siren_on["target"]["entity_id"], "siren.hallway_horn");

        let bool_off = service_call_payload(
            10,
            &ServiceCall {
                epoch: 1,
                entity_id: "input_boolean.siren".into(),
                kind: CallKind::Output { on: false },
            },
        )
        .unwrap();
        assert_eq!(bool_off["domain"], "input_boolean");
        assert_eq!(bool_off["service"], "turn_off");

        assert!(
            service_call_payload(
                11,
                &ServiceCall {
                    epoch: 1,
                    entity_id: "conversation.home_assistant".into(),
                    kind: CallKind::Output { on: true },
                },
            )
            .is_none(),
            "non-output domains must be rejected for siren output calls"
        );
    }

    #[test]
    fn entity_browser_filtering_and_multiword_search_hide_ha_noise() {
        let mk = |entity_id: &str, name: &str, dc: Option<&str>, state: &str| Entity {
            entity_id: entity_id.into(),
            state: state.into(),
            attributes: Attributes {
                friendly_name: Some(name.into()),
                device_class: dc.map(str::to_owned),
                ..Attributes::default()
            },
            last_updated: String::new(),
            last_changed: String::new(),
        };
        let conv = mk("conversation.home_assistant", "Home Assistant", None, "2026-11-27T18:44:21.398665+00:00");
        let backup = mk("event.backup_automatic_backup", "Backup Automatic backup", None, "unknown");
        let door = mk("binary_sensor.front_door", "Front Door", Some("door"), "off");
        let siren = mk("input_boolean.siren", "siren", None, "off");

        assert!(!conv.is_sensor_candidate());
        assert!(!backup.is_sensor_candidate());
        assert!(door.is_sensor_candidate());
        assert!(siren.is_sensor_candidate());
        assert!(siren.is_output());
        assert!(!door.is_output());
        assert_eq!(conv.short_state(), "18:44:21");
        assert_eq!(door.matches_search("front door"), Some(0));
        assert_eq!(door.matches_search("door binary"), Some(2));
        assert_eq!(door.matches_search("garage"), None);
    }

    #[test]
    fn entity_smart_defaults_cover_common_wireless_sensor_domains() {
        let mk = |entity_id: &str, state: &str| Entity {
            entity_id: entity_id.into(),
            state: state.into(),
            attributes: Attributes::default(),
            last_updated: String::new(),
            last_changed: String::new(),
        };
        assert_eq!(mk("binary_sensor.front_door", "off").default_rule_and_operand(), (Rule::Equals, "on".into()));
        assert_eq!(mk("input_boolean.roomguard_lab_door", "off").default_rule_and_operand(), (Rule::Equals, "on".into()));
        assert_eq!(mk("cover.garage", "closed").default_rule_and_operand(), (Rule::Equals, "open".into()));
        assert_eq!(mk("lock.front_door", "locked").default_rule_and_operand(), (Rule::Equals, "unlocked".into()));
        assert_eq!(mk("sensor.temp", "22.5").default_rule_and_operand(), (Rule::Above, "22.5".into()));
        assert_eq!(mk("event.doorbell", "unknown").default_rule_and_operand(), (Rule::Changes, String::new()));
    }

    #[test]
    #[ignore = "requires local loopback sockets"]
    fn websocket_merges_initial_state_delivers_live_updates_and_calls_services() {
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let send = |socket: &mut WebSocket<TcpStream>, value: Value| {
                socket.send(Message::Text(value.to_string())).unwrap();
            };
            let read = |socket: &mut WebSocket<TcpStream>| -> Value {
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap()
            };
            send(&mut socket, json!({"type":"auth_required"}));
            assert_eq!(read(&mut socket), json!({"type":"auth","access_token":"fixture"}));
            send(&mut socket, json!({"type":"auth_ok"}));
            assert_eq!(read(&mut socket)["event_type"], "state_changed");
            send(&mut socket, json!({"id":1,"type":"result","success":true,"result":null}));
            assert_eq!(read(&mut socket)["type"], "get_states");
            let state = |value: &str, minute: &str| json!({
                "entity_id":"binary_sensor.door","state":value,"last_updated":format!("2026-11-27T10:{minute}:00+00:00")
            });
            let alarm = |value: &str| json!({"entity_id":"alarm_control_panel.house","state":value});
            let event = |new: Value| json!({"type":"event","id":1,"event":{
                "event_type":"state_changed","data":{"entity_id":new["entity_id"],"new_state":new}
            }});
            send(&mut socket, event(state("on", "11")));
            send(&mut socket, json!({"id":2,"type":"result","success":true,"result":[state("off", "10"),alarm("disarmed")]}));
            send(&mut socket, event(state("off", "12")));
            send(&mut socket, event(alarm("armed_home")));
            let call = read(&mut socket);
            assert_eq!(call["type"], "call_service");
            assert_eq!(call["service"], "alarm_disarm");
            send(&mut socket, json!({"id":call["id"],"type":"result","success":true,"result":null}));
            socket.close(None).unwrap();
        });
        let (settings, config) = mpsc::channel();
        let (call_tx, call_rx) = mpsc::channel();
        let (engine, events) = mpsc::channel();
        let client = std::thread::spawn(move || {
            let cfg = Connection { epoch: 4, url: format!("http://{address}"), token: "fixture".into() };
            assert!(session(&cfg, &config, &call_rx, &engine).is_err());
        });
        match events.recv_timeout(Duration::from_secs(8)).unwrap() {
            EngineMsg::HomeAssistant { epoch: 4, update: Update::Snapshot(states) } => {
                assert_eq!(states["binary_sensor.door"].state, "on");
                assert_eq!(AlarmState::parse(&states["alarm_control_panel.house"].state), Some(AlarmState::Disarmed));
            }
            _ => panic!("expected connection snapshot"),
        }
        match events.recv_timeout(Duration::from_secs(8)).unwrap() {
            EngineMsg::HomeAssistant { epoch: 4, update: Update::State { id, entity: Some(entity) } } => {
                assert_eq!(id, "binary_sensor.door");
                assert_eq!(entity.state, "off");
            }
            _ => panic!("expected live state change"),
        }
        match events.recv_timeout(Duration::from_secs(8)).unwrap() {
            EngineMsg::HomeAssistant { epoch: 4, update: Update::State { id, entity: Some(entity) } } => {
                assert_eq!(id, "alarm_control_panel.house");
                assert_eq!(AlarmState::parse(&entity.state), Some(AlarmState::Stay));
            }
            _ => panic!("expected physical panel update"),
        }
        call_tx
            .send(ServiceCall {
                epoch: 4,
                entity_id: "alarm_control_panel.house".into(),
                kind: CallKind::Alarm { action: AlarmAction::Disarm, code: Some("1234".into()) },
            })
            .unwrap();
        match events.recv_timeout(Duration::from_secs(8)).unwrap() {
            EngineMsg::HomeAssistant { epoch: 4, update: Update::ServiceResult { action: AlarmAction::Disarm, result: Ok(()) } } => {}
            _ => panic!("expected service call ack"),
        }
        client.join().unwrap();
        server.join().unwrap();
        drop(settings);
    }
}
