//! Home Assistant entity discovery, live states, two-way alarm panel control,
//! and on-demand camera snapshots.

use std::collections::{BTreeMap, HashMap};
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

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub url: String,
    pub token: String,
    pub bindings: Vec<Binding>,
    pub follow_panel: Option<String>,
    pub panel_sync: PanelSyncMode,
    pub panel_code: String,
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
        self.follow_panel.as_deref().filter(|id| self.enabled && id.starts_with("alarm_control_panel."))
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
    Vacation,
    CustomBypass,
    Pending,
    Triggered,
    Disarming,
}

impl AlarmState {
    pub fn parse(state: &str) -> Option<Self> {
        Some(match state {
            "disarmed" => Self::Disarmed,
            "arming" => Self::Arming,
            "armed_home" => Self::Stay,
            "armed_away" => Self::Away,
            "armed_night" => Self::Night,
            "armed_vacation" => Self::Vacation,
            "armed_custom_bypass" => Self::CustomBypass,
            "pending" => Self::Pending,
            "triggered" => Self::Triggered,
            "disarming" => Self::Disarming,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Disarmed => "Disarmed",
            Self::Arming => "Exit delay",
            Self::Stay => "Armed stay",
            Self::Away => "Armed away",
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
}

impl AlarmAction {
    pub fn service(self) -> &'static str {
        match self {
            Self::Disarm => "alarm_disarm",
            Self::ArmHome => "alarm_arm_home",
            Self::ArmAway => "alarm_arm_away",
            Self::ArmNight => "alarm_arm_night",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Disarm => "Disarm",
            Self::ArmHome => "Arm Stay",
            Self::ArmAway => "Arm Away",
            Self::ArmNight => "Arm Night",
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
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct Attributes {
    friendly_name: Option<String>,
    unit_of_measurement: Option<String>,
    device_class: Option<String>,
    supported_features: Option<u32>,
    code_format: Option<String>,
    code_arm_required: Option<bool>,
}

impl Entity {
    pub fn name(&self) -> &str {
        self.attributes.friendly_name.as_deref().unwrap_or(&self.entity_id)
    }

    pub fn camera(&self) -> bool {
        self.entity_id.starts_with("camera.")
    }

    pub fn alarm_panel(&self) -> bool {
        self.entity_id.starts_with("alarm_control_panel.")
    }

    pub fn binary(&self) -> bool {
        self.entity_id.starts_with("binary_sensor.")
            || self.entity_id.starts_with("input_boolean.")
            || self.entity_id.starts_with("switch.")
    }

    pub fn supports_night(&self) -> bool {
        // HA AlarmControlPanelEntityFeature.ARM_NIGHT = 4
        self.attributes.supported_features.map_or(true, |f| (f & 4) != 0)
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

    pub fn description(&self) -> String {
        match &self.attributes.unit_of_measurement {
            Some(unit) => format!("{} {}", self.state, unit),
            None => self.state.clone(),
        }
    }
}

pub type Entities = BTreeMap<String, Entity>;

pub enum Update {
    Connection(ConnectionState),
    Snapshot(Entities),
    State { id: String, entity: Option<Entity> },
    ServiceResult { action: AlarmAction, result: Result<(), String> },
    Preview { id: String, result: Result<Arc<Rgb>, String> },
}

struct Connection {
    epoch: u64,
    url: String,
    token: String,
}

#[derive(Clone, Debug)]
struct ServiceCall {
    epoch: u64,
    entity_id: String,
    action: AlarmAction,
    code: Option<String>,
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
        let _ = self.calls.send(ServiceCall { epoch, entity_id, action, code });
    }

    pub fn preview(&self, cfg: &Config, epoch: u64, id: String) -> bool {
        self.previews.try_send((connection(cfg, epoch), id)).is_ok()
    }
}

fn connection(cfg: &Config, epoch: u64) -> Connection {
    Connection { epoch, url: cfg.url.trim().trim_end_matches('/').into(), token: cfg.token.trim().into() }
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
    let url = format!("{}://{}/api/websocket", prefix, cfg.url.split_once("://").unwrap().1);
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

fn service_call_payload(id: u64, call: &ServiceCall) -> Value {
    let mut service_data = serde_json::Map::new();
    service_data.insert("entity_id".into(), json!(call.entity_id));
    if let Some(code) = call.code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        service_data.insert("code".into(), json!(code));
    }
    json!({
        "id": id,
        "type": "call_service",
        "domain": "alarm_control_panel",
        "service": call.action.service(),
        "service_data": service_data,
    })
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
    let mut pending_calls: HashMap<u64, AlarmAction> = HashMap::new();
    let mut online = false;
    let mut next_id = 2u64;
    let mut active_ping: Option<(u64, Instant)> = None;
    let mut next_ping = Instant::now() + Duration::from_secs(6);
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
        // Send any outbound two-way alarm panel service calls once online.
        if online {
            while let Ok(call) = calls.try_recv() {
                if call.epoch != cfg.epoch {
                    continue;
                }
                next_id += 1;
                let payload = service_call_payload(next_id, &call);
                send(&mut socket, payload)?;
                pending_calls.insert(next_id, call.action);
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
        } else if message["type"] == "result" {
            if let Some(id) = message["id"].as_u64() {
                if let Some(action) = pending_calls.remove(&id) {
                    let result = if message["success"] == true {
                        Ok(())
                    } else {
                        let detail = message["error"]["message"]
                            .as_str()
                            .unwrap_or("Home Assistant rejected the panel command");
                        Err(detail.to_owned())
                    };
                    emit(engine, cfg.epoch, Update::ServiceResult { action, result });
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
                action: AlarmAction::Disarm,
                code: Some("1234".into()),
            },
        );
        assert_eq!(with_code["type"], "call_service");
        assert_eq!(with_code["domain"], "alarm_control_panel");
        assert_eq!(with_code["service"], "alarm_disarm");
        assert_eq!(with_code["service_data"]["code"], "1234");

        let without_code = service_call_payload(
            8,
            &ServiceCall {
                epoch: 1,
                entity_id: "alarm_control_panel.roomguard_test_panel".into(),
                action: AlarmAction::ArmAway,
                code: Some("   ".into()),
            },
        );
        assert_eq!(without_code["service"], "alarm_arm_away");
        assert!(without_code["service_data"].get("code").is_none());
    }

    #[test]
    fn entity_smart_defaults_cover_common_wireless_sensor_domains() {
        let mk = |entity_id: &str, state: &str| Entity {
            entity_id: entity_id.into(),
            state: state.into(),
            attributes: Attributes::default(),
            last_updated: String::new(),
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
                action: AlarmAction::Disarm,
                code: Some("1234".into()),
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
