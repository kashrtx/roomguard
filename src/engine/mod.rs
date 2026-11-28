//! The engine thread. It owns the panel and the operator, runs them on a
//! steady clock whether or not the window is visible, routes their output to
//! sound, the camera and your phone, turns the computer's own condition into
//! panel troubles, and publishes a snapshot of everything for the interface.

pub mod bus;
pub mod eeprom;
mod kp1550;
mod kp1832;
pub mod kp_vista;
pub mod operator;
pub mod panel;
pub mod types;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::audio::{AudioCmd, AudioHandle, BellPattern, KeypadVoice};
use crate::config::AppConfig;
use crate::notify::{self, NotifyHandle, NotifyJob, NtfyCfg};
use crate::power::Power;
use crate::util;
use crate::vision::{CamCmd, CameraHandle, VisionEvent};
use operator::{DialInfo, OpOut, Operator};
use panel::{Out, Panel, PanelSave};
use types::*;
use crate::home_assistant as ha;

/// Trouble names Home Assistant sees in RoomGuard's status.
const TROUBLE_NAMES: [(u32, &str); 11] = [
    (panel::tb::LOW_BATT, "Low battery"),
    (panel::tb::AC, "AC power"),
    (panel::tb::BELL, "Siren"),
    (panel::tb::FTC, "Failure to communicate"),
    (panel::tb::TLM, "Phone line"),
    (panel::tb::ZONE_FAULT, "Zone fault"),
    (panel::tb::ZONE_TAMPER, "Zone tamper"),
    (panel::tb::CLOCK, "Clock not set"),
    (panel::tb::FIRE_LOOP, "Fire zone"),
    (panel::tb::HOME_SENSOR, "Home Assistant sensor"),
    (panel::tb::HOME_PANEL, "Home Assistant panel"),
];

pub enum OpInput {
    Answer,
    Decline,
    Passcode(String),
    Dispatch(bool),
    Clear,
}

/// A direct change to the panel's programming from Settings, writing the same
/// EEPROM sections and panel state as keying it in at the keypad.
#[derive(Clone, Debug)]
pub enum PanelEdit {
    SetSection { key: String, value: String },
    SetToggle { key: String, opt: u8, on: bool },
    SetCode { slot: String, code: String },
    SetChime(bool),
    SetQuickArm(bool),
    SetAutoArm { enabled: bool, hm: Option<(u8, u8)> },
    SetKeypadLevels { brightness: u8, contrast: u8, buzzer: u8 },
    SetVoiceLevel(u8),
    SetInstallerLockout(bool),
    /// Sets a single zone's definition code (e.g., "01", "03", "00" on PC1832/Vista, or "00".."09" on PC1550).
    SetZoneType { zone: u8, code: String },
    /// Disables unused zones (setting them to Null / Not Used) so only real/configured zones appear.
    DisableZones(Vec<u8>),
    FactoryDefault,
}

pub enum EngineMsg {
    Key { key: Key, down: bool },
    SimZone { zone: u8, open: bool },
    Vision(VisionEvent),
    Config(Box<AppConfig>),
    Op(OpInput),
    NotifyResult { ok: bool, test: bool, err: String },
    Memo(crate::audio::memo::Event),
    TestMicrophone(Option<String>),
    NetStatus(bool),
    SetMasterCode(String),
    EditPanel(PanelEdit),
    /// Checks a code without pressing keys, used before closing while armed.
    VerifyCode { code: String, reply: std::sync::mpsc::Sender<bool> },
    UnlockInstaller { model: Model, code: String, reply: Sender<Result<(), &'static str>> },
    TestNotify,
    ClearEvents,
    HomeAssistant { epoch: u64, update: ha::Update },
    HomePreview(String),
    TestHaOutput { entity_id: String, on: bool, auto_off_secs: Option<u64> },
    SilenceHaSiren,
    ResetLearnedHaTimers,
    Shutdown,
}

const MAX_PANEL_ZONES: usize = Model::Vista20p.zone_count();

#[derive(Clone, Default)]
pub struct HomeView {
    pub state: ha::ConnectionState,
    pub status: String,
    pub online: bool,
    pub entities: Arc<ha::Entities>,
    pub esphome_services: Vec<String>,
    pub detected_esphome: Option<String>,
    pub detected_bypass_entity: Option<String>,
    pub learned_exit_away: Option<u64>,
    pub learned_exit_stay: Option<u64>,
    pub learned_entry: Option<u64>,
    pub preview: Option<(String, Result<Arc<crate::vision::frame::Rgb>, String>)>,
    pub service_notice: Option<(bool, String)>,
    pub output_notice: Option<(bool, String)>,
}

#[derive(Clone)]
pub struct View {
    pub seq: u64,
    pub model: Model,
    pub leds: [Led; 11],
    pub lcd: [String; 2],
    pub lcd_cursor: Option<(usize, usize)>,
    pub lcd_leds: [Led; 4],
    pub backlight: f32,
    pub contrast: f32,
    pub all_on: bool,
    pub status: StatusKind,
    pub status_title: String,
    pub status_detail: String,
    pub zones: Vec<ZoneView>,
    pub bell: bool,
    pub ha_external_siren: bool,
    pub siren: SirenModel,
    pub hold: Option<(Key, f32)>,
    pub troubles: Vec<String>,
    pub events: Arc<Vec<EventRow>>,
    pub chat: Arc<Vec<ChatMsg>>,
    pub call: Option<CallView>,
    pub armed: bool,
    pub following_panel: bool,
    pub panel_link: ha::PanelLink,
    pub physical_unavailable: bool,
    pub programming: bool,
    pub notify_status: String,
    pub mic_test_status: String,
    pub mic_test_running: bool,
    pub notify_test_status: String,
    pub sim: [bool; MAX_PANEL_ZONES],
    pub home: HomeView,
    pub home_zones: [bool; MAX_PANEL_ZONES],
    /// Plain help about what the keypad is waiting for.
    pub hint: String,
    /// The panel's programming, for the installer view in Settings.
    pub prog: Arc<ProgView>,
}

/// What the installer view in Settings shows: the panel's programming as it
/// stands right now. It's rebuilt only when something in it changes.
#[derive(Clone)]
pub struct ProgView {
    pub model: Model,
    pub sections: std::collections::BTreeMap<String, String>,
    pub installer_default: bool,
    pub master_default: bool,
    pub codes: Vec<String>,
    pub chime: bool,
    pub quick_arm: bool,
    pub auto_arm: bool,
    pub auto_arm_hm: Option<(u8, u8)>,
    pub installer_lockout: bool,
    pub brightness: u8,
    pub contrast: u8,
    pub buzzer: u8,
    pub voice_level: u8,
}

impl Default for ProgView {
    fn default() -> Self {
        ProgView {
            model: Model::Pc1832,
            sections: std::collections::BTreeMap::new(),
            installer_default: true,
            master_default: true,
            codes: Vec::new(),
            chime: false,
            quick_arm: false,
            auto_arm: false,
            auto_arm_hm: None,
            installer_lockout: false,
            brightness: 7,
            contrast: 5,
            buzzer: 12,
            voice_level: 5,
        }
    }
}

fn prog_fingerprint(p: &Panel) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.model.file_key().hash(&mut h);
    for (k, v) in &p.ee.sections {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    for (k, c) in &p.ee.codes {
        k.hash(&mut h);
        c.hash.hash(&mut h);
        c.attrs.hash(&mut h);
    }
    p.chime.hash(&mut h);
    p.quick_arm.hash(&mut h);
    p.auto_arm.hash(&mut h);
    p.auto_arm_hm.hash(&mut h);
    p.ee.installer_lockout.hash(&mut h);
    p.brightness.hash(&mut h);
    p.contrast.hash(&mut h);
    p.buzzer.hash(&mut h);
    p.vista_voice_level.hash(&mut h);
    h.finish()
}

fn prog_view(p: &Panel) -> ProgView {
    ProgView {
        model: p.model,
        sections: p.ee.sections.clone(),
        installer_default: p.ee.code_matches("inst", p.model.installer_default()),
        master_default: p.ee.code_matches(master_slot(p.model), "1234"),
        codes: p.ee.codes.iter().filter(|(_, c)| !c.hash.is_empty()).map(|(k, _)| k.clone()).collect(),
        chime: p.chime,
        quick_arm: p.quick_arm,
        auto_arm: p.auto_arm,
        auto_arm_hm: p.auto_arm_hm,
        installer_lockout: p.ee.installer_lockout,
        brightness: p.brightness,
        contrast: p.contrast,
        buzzer: p.buzzer,
        voice_level: p.vista_voice_level,
    }
}

impl Default for View {
    fn default() -> Self {
        View {
            seq: 0,
            model: Model::Pc1832,
            leds: [Led::Off; 11],
            lcd: [String::new(), String::new()],
            lcd_cursor: None,
            lcd_leds: [Led::Off; 4],
            backlight: 0.8,
            contrast: 0.5,
            all_on: false,
            status: StatusKind::Ready,
            status_title: "Starting".to_string(),
            status_detail: String::new(),
            zones: Vec::new(),
            bell: false,
            ha_external_siren: false,
            siren: SirenModel::Wave2F,
            hold: None,
            troubles: Vec::new(),
            events: Arc::new(Vec::new()),
            chat: Arc::new(Vec::new()),
            call: None,
            armed: false,
            following_panel: false,
            panel_link: ha::PanelLink::Independent,
            physical_unavailable: false,
            programming: false,
            notify_status: String::new(),
            mic_test_status: String::new(),
            mic_test_running: false,
            notify_test_status: String::new(),
            sim: [false; MAX_PANEL_ZONES],
            home: HomeView::default(),
            home_zones: [false; MAX_PANEL_ZONES],
            hint: String::new(),
            prog: Arc::new(ProgView::default()),
        }
    }
}

pub struct EngineHandle {
    pub tx: Sender<EngineMsg>,
    view: Arc<Mutex<Arc<View>>>,
    done: Mutex<Option<Receiver<()>>>,
}

impl EngineHandle {
    pub fn send(&self, m: EngineMsg) {
        let _ = self.tx.send(m);
    }

    pub fn view(&self) -> Arc<View> {
        self.view
            .lock()
            .map(|v| v.clone())
            .unwrap_or_else(|_| Arc::new(View::default()))
    }

    /// Saves everything and waits (briefly) for the engine to finish.
    pub fn shutdown(&self) {
        self.send(EngineMsg::Shutdown);
        if let Ok(mut g) = self.done.lock() {
            if let Some(rx) = g.take() {
                let _ = rx.recv_timeout(Duration::from_secs(8));
            }
        }
    }
}

pub fn start(
    cfg: AppConfig,
    tx: Sender<EngineMsg>,
    rx: Receiver<EngineMsg>,
    audio: AudioHandle,
    cam: CameraHandle,
    notify: NotifyHandle,
    ctx: egui::Context,
) -> EngineHandle {
    let view = Arc::new(Mutex::new(Arc::new(View::default())));
    let v2 = view.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let memo_events = tx.clone();
    let memo = crate::audio::memo::start(move |event| {
        let _ = memo_events.send(EngineMsg::Memo(event));
    });
    let hub = ha::start(tx.clone());
    let spawned = std::thread::Builder::new().name("engine".into()).spawn(move || {
        // The panel's clock and beeps come first, ahead of camera work.
        util::set_thread_priority(util::prio::PANEL);
        let mut e = Engine::new(cfg, audio, cam, notify, ctx, v2, memo, hub);
        e.run(rx);
        let _ = done_tx.send(());
    });
    if let Err(err) = spawned {
        util::log(&format!("could not start the engine thread: {}", err));
    }
    EngineHandle { tx, view, done: Mutex::new(Some(done_rx)) }
}

fn panel_path(model: Model) -> PathBuf {
    util::data_dir().join(format!("panel-{}.json", model.file_key()))
}

fn master_slot(model: Model) -> &'static str {
    match model {
        Model::Pc1550 => "1",
        Model::Pc1832 => "40",
        Model::Vista20p => "02",
    }
}

fn voice_for(model: Model) -> KeypadVoice {
    match model {
        Model::Pc1550 => KeypadVoice::Classic,
        Model::Pc1832 => KeypadVoice::Pk5500,
        Model::Vista20p => KeypadVoice::Vista6160,
    }
}

fn load_panel(model: Model, cfg: &AppConfig, now: Instant) -> Panel {
    let save = util::load_json::<PanelSave>(&panel_path(model)).unwrap_or_else(|| PanelSave::fresh(model));
    let link = cfg.home_assistant.panel_link();
    let mut p = Panel::new(model, save, cfg.labels(model).clone(), cfg.display_name(), link != ha::PanelLink::Independent, now);
    p.panel_link = link;
    p.following_panel = link == ha::PanelLink::FollowOnly;
    p.ha_zone_mirror_only = link != ha::PanelLink::Independent && !cfg.home_assistant.ha_zones_trigger_alarm;
    p.ha_learned_exit_away = cfg.home_assistant.learned_exit_away;
    p.ha_learned_exit_stay = cfg.home_assistant.learned_exit_stay;
    p.ha_learned_entry = cfg.home_assistant.learned_entry;
    p
}

struct Engine {
    cfg: AppConfig,
    panel: Panel,
    op: Operator,
    audio: AudioHandle,
    memo: Sender<crate::audio::memo::Command>,
    cam: CameraHandle,
    notify: NotifyHandle,
    ctx: egui::Context,
    view: Arc<Mutex<Arc<View>>>,
    power: Power,
    last_power: Option<Instant>,
    last_cam_check: Instant,
    grace_until: Instant,
    save_due: Option<Instant>,
    saved_events: u64,
    last_publish: Option<Instant>,
    seq: u64,
    cam_motion: [bool; 8],
    cam_contact_open: [bool; 8],
    cam_online: bool,
    cam_covered: bool,
    sim: [bool; MAX_PANEL_ZONES],
    events_cache: (u64, Arc<Vec<EventRow>>),
    chat_cache: (u64, Arc<Vec<ChatMsg>>),
    notify_status: String,
    mic_test_status: String,
    mic_test_running: bool,
    notify_test_status: String,
    pending_photos: Vec<(String, NotifyJob, Instant)>,
    alert_sequence: u64,
    entry_evidence: Option<(u16, Instant)>,
    last_face_log: Option<Instant>,
    last_stranger: Option<Instant>,
    last_buzzer: u8,
    awake: crate::power::KeepAwake,
    /// What each zone's sensor says right now (open, tamper, fault).
    zone_want: [(bool, bool, bool); MAX_PANEL_ZONES],
    /// A change waiting out the zone's loop response time.
    zone_pend: [Option<(bool, Instant)>; MAX_PANEL_ZONES],
    prog_cache: (u64, Arc<ProgView>),
    /// Since when the sound output has been missing.
    audio_bad_since: Option<Instant>,
    /// Internal errors recovered from in a row.
    faults: u32,
    hub: ha::Handle,
    home: HomeView,
    ha_companions: ha::PanelCompanions,
    hub_epoch: u64,
    home_pulses: std::collections::BTreeMap<String, Instant>,
    ha_bell_active: bool,
    /// When each Home Assistant zone's sensor went missing (supervision).
    home_lost_at: [Option<Instant>; MAX_PANEL_ZONES],
    home_lost_check: Option<Instant>,
    /// What RoomGuard last told Home Assistant about itself.
    ha_status_sent: Option<String>,
    ha_status_at: Option<Instant>,
    ha_status_check: Option<Instant>,
    ha_status_online: bool,
    ha_state_name: Option<&'static str>,
    ha_alarm_notified: bool,
    ha_bell_cmd_until: Option<Instant>,
    ha_bell_cmd_target: Option<bool>,
    ha_output_tests: std::collections::BTreeMap<String, Instant>,
    /// If the saved `HA panel PIN` fails with `invalid_code`, retry once with the code just typed on the keypad.
    ha_retry_cmd: Option<(panel::HaPanelCmd, String, Instant)>,
}

impl Engine {
    fn new(cfg: AppConfig, audio: AudioHandle, cam: CameraHandle, notify: NotifyHandle, ctx: egui::Context, view: Arc<Mutex<Arc<View>>>, memo: Sender<crate::audio::memo::Command>, hub: ha::Handle) -> Engine {
        let now = Instant::now();
        let panel = load_panel(cfg.model, &cfg, now);
        let op = Operator::load(cfg.display_name());
        let last_buzzer = panel.buzzer;
        let learned_exit_away = panel.ha_learned_exit_away;
        let learned_exit_stay = panel.ha_learned_exit_stay;
        let learned_entry = panel.ha_learned_entry;
        let mut e = Engine {
            cfg,
            panel,
            op,
            audio,
            memo,
            cam,
            notify,
            ctx,
            view,
            power: Power::new(),
            last_power: None,
            last_cam_check: now,
            grace_until: now + Duration::from_secs(12),
            save_due: None,
            saved_events: 0,
            last_publish: None,
            seq: 0,
            cam_motion: [false; 8],
            cam_contact_open: [false; 8],
            cam_online: true,
            cam_covered: false,
            sim: [false; MAX_PANEL_ZONES],
            events_cache: (0, Arc::new(Vec::new())),
            chat_cache: (0, Arc::new(Vec::new())),
            notify_status: String::new(),
            mic_test_status: String::new(),
            mic_test_running: false,
            notify_test_status: String::new(),
            pending_photos: Vec::new(),
            alert_sequence: 0,
            entry_evidence: None,
            last_face_log: None,
            last_stranger: None,
            last_buzzer,
            awake: crate::power::KeepAwake::new(),
            zone_want: [(false, false, false); MAX_PANEL_ZONES],
            zone_pend: [None; MAX_PANEL_ZONES],
            prog_cache: (0, Arc::new(ProgView::default())),
            audio_bad_since: None,
            faults: 0,
            hub,
            home: HomeView {
                learned_exit_away,
                learned_exit_stay,
                learned_entry,
                ..HomeView::default()
            },
            ha_companions: ha::PanelCompanions::default(),
            hub_epoch: 0,
            home_pulses: Default::default(),
            ha_bell_active: false,
            home_lost_at: [None; MAX_PANEL_ZONES],
            home_lost_check: None,
            ha_status_sent: None,
            ha_status_at: None,
            ha_status_check: None,
            ha_status_online: false,
            ha_state_name: None,
            ha_alarm_notified: false,
            ha_bell_cmd_until: None,
            ha_bell_cmd_target: None,
            ha_output_tests: Default::default(),
            ha_retry_cmd: None,
        };
        e.saved_events = e.panel.events_seq;
        e.push_settings();
        if e.cfg.home_assistant.enabled {
            e.home.state = ha::ConnectionState::Connecting;
            e.home.status = "Connecting to Home Assistant...".into();
        }
        e.hub.configure(&e.cfg.home_assistant, e.hub_epoch);
        let armed = e.panel.is_on();
        e.cam.send(CamCmd::Armed(armed));
        e.sync_zones(now);
        e
    }

    fn push_volumes(&mut self) {
        let s = &self.cfg.sound;
        self.audio.send(AudioCmd::BuzzerLevel(if self.panel.model == Model::Pc1832 { self.panel.buzzer } else { 20 }));
        self.audio.send(AudioCmd::VoiceLevel(self.panel.vista_voice_level));
        self.audio.send(AudioCmd::Volumes {
            master: s.master,
            siren: s.siren,
            keypad: s.keypad,
            line: if s.line_audio { 0.35 } else { 0.0 },
        });
    }

    fn push_settings(&mut self) {
        self.audio.configure(self.cfg.sound.keypad_output.clone(), self.cfg.sound.siren_output.clone());
        let _ = self.memo.send(crate::audio::memo::Command::Device(self.cfg.sound.microphone.clone()));
        self.push_volumes();
        self.audio.send(AudioCmd::Siren { model: self.cfg.sound.siren_model, warble: self.cfg.sound.wave2_warble });
        self.audio.send(AudioCmd::Voice(voice_for(self.panel.model)));
        self.cam.send(CamCmd::Config(Box::new(self.cfg.camera.clone())));
        self.notify.config(&self.cfg.operator);
        self.cam.send(CamCmd::AlertClipsEnabled(self.cfg.operator.motion_gifs_enabled()));
        let name = self.cfg.display_name();
        self.panel.labels = self.cfg.labels(self.panel.model).clone();
        self.panel.owner = name.clone();
        self.op.owner = name;
        self.panel.face_disarm = self.cfg.camera.faces && self.cfg.camera.face_disarm;
        self.panel.inputs.cam_enabled = self.cfg.camera.enabled;
        self.panel.inputs.cam_online = self.cam_online;
        self.panel.inputs.cam_covered = self.cam_covered;
        self.panel.inputs.net_watch = self.cfg.operator.phone_alerts;
        self.panel.quiet_intrusion = self.cfg.operator.quiet_intrusion;
        self.panel.camera_zone = self.cfg.camera.camera_mask();
        self.panel.dirty = true;
    }

    fn idle(&self) -> bool {
        self.panel.held.is_none()
            && self.panel.exit.is_none()
            && self.panel.entry.is_none()
            && self.panel.alarms.is_empty()
            && self.panel.ha_external_siren.is_none()
            && !matches!(self.panel.physical_state, Some(ha::AlarmState::Arming | ha::AlarmState::Pending))
            && !self.panel.walk_test
            && self.panel.bell_test_until.is_none()
            && self.panel.util_until.is_none()
            && self.panel.cmd_out_active.is_none()
            && self.panel.temp_msg.is_none()
            && self.panel.vista_pending_status.is_none()
            && self.panel.vista_code_disarm.is_none()
            && self.panel.vista_msg_recording.is_none()
            && matches!(self.panel.vista_mode, kp_vista::VistaKp::Normal)
            && self.save_due.is_none()
            && self.zone_pend.iter().all(|p| p.is_none())
            && self.ha_output_tests.is_empty()
            && self.op.call_view().is_none()
    }

    fn run(&mut self, rx: Receiver<EngineMsg>) {
        loop {
            let wait = if self.idle() { Duration::from_millis(100) } else { Duration::from_millis(30) };
            match rx.recv_timeout(wait) {
                Ok(msg) => {
                    if !self.handle_safely(msg) {
                        self.shutdown();
                        return;
                    }
                    while let Ok(m) = rx.try_recv() {
                        if !self.handle_safely(m) {
                            self.shutdown();
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.shutdown();
                    return;
                }
            }
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.tick())).is_ok();
            if ok {
                self.faults = 0;
            } else {
                self.recover();
            }
        }
    }

    /// A bug in one key sequence must never stop the whole panel: without
    /// this, the engine thread would end and the keypad would go silent and
    /// freeze while the window kept drawing.
    fn handle_safely(&mut self, msg: EngineMsg) -> bool {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(msg))) {
            Ok(keep_going) => keep_going,
            Err(_) => {
                self.recover();
                true
            }
        }
    }

    fn recover(&mut self) {
        self.faults += 1;
        if self.faults <= 3 || self.faults % 100 == 0 {
            util::log("the panel recovered from an internal error (the panic line above says where)");
        }
        self.panel.kp = panel::KpMode::Normal;
        self.panel.held = None;
        self.panel.buf.clear();
        self.panel.dirty = true;
        // If it keeps happening, go back to the last saved state.
        if self.faults == 3 {
            let now = Instant::now();
            self.panel = load_panel(self.cfg.model, &self.cfg, now);
            self.last_buzzer = self.panel.buzzer;
            self.saved_events = self.panel.events_seq;
            self.zone_pend = [None; MAX_PANEL_ZONES];
            self.push_settings();
            self.sync_physical_panel(false, now);
            self.sync_zones(now);
            util::log("reloaded the panel from its last save");
        }
    }

    fn handle(&mut self, msg: EngineMsg) -> bool {
        let now = Instant::now();
        match msg {
            EngineMsg::Key { key, down } => {
                self.panel.key(key, down, now);
                // Show the keypad's reaction right away, in step with its beep.
                self.last_publish = None;
            }
            EngineMsg::SimZone { zone, open } => {
                let z = zone as usize;
                if (1..=self.panel.n()).contains(&z) {
                    self.sim[z - 1] = open;
                    self.sync_zones(now);
                }
            }
            EngineMsg::Vision(ev) => self.vision(ev, now),
            EngineMsg::Memo(event) => {
                use crate::audio::memo::Event;
                match event {
                    Event::Memo { id, result } => self.panel.vista_memo_result(id, result, now),
                    Event::Testing(status) => self.mic_test_status = status,
                    Event::Test(result) => {
                        self.mic_test_running = false;
                        match result {
                            Ok(clip) => {
                                self.audio.send(AudioCmd::PlayTest(clip));
                                self.mic_test_status = if self.cfg.sound.master == 0.0 || self.cfg.sound.keypad == 0.0 {
                                    "Microphone captured audio. Turn up Keypad and Everything to hear playback."
                                } else if self.audio.keypad_ok() {
                                    "Microphone captured audio. Listen for playback through Keypad & voice."
                                } else {
                                    "Microphone captured audio. Check the Keypad & voice output to hear playback."
                                }.into();
                            }
                            Err(error) => self.mic_test_status = error,
                        }
                    }
                }
                self.panel.dirty = true;
            }
            EngineMsg::TestMicrophone(device) => {
                if !self.mic_test_running {
                    self.mic_test_running = self.memo.send(crate::audio::memo::Command::Test(device)).is_ok();
                    self.mic_test_status = if self.mic_test_running { "Opening microphone…" } else { "Microphone worker unavailable" }.into();
                    self.panel.dirty = true;
                }
            }
            EngineMsg::Config(c) => self.apply_config(*c, now),
            EngineMsg::Op(inp) => {
                let outs = self.op.input(inp, now, &self.cfg.operator);
                self.handle_op(outs);
                self.save_due = Some(now + Duration::from_secs(1));
            }
            EngineMsg::NotifyResult { ok, test, err } => {
                if test {
                    self.notify_test_status = if ok {
                        "Test accepted. Check your phone.".into()
                    } else {
                        format!("Test failed: {}", err)
                    };
                } else {
                    self.panel.inputs.ftc = !ok;
                    self.notify_status = if ok {
                        "Last alert accepted by ntfy".into()
                    } else {
                        format!("Phone alert failed: {}", err)
                    };
                }
                self.panel.dirty = true;
            }
            EngineMsg::NetStatus(ok) => self.panel.inputs.net_ok = ok,
            EngineMsg::SetMasterCode(code) => {
                if self.cfg.setup_done || self.panel.is_on() {
                    return true;
                }
                let model = self.panel.model;
                self.panel.ee.set_code(master_slot(model), &code);
                self.panel.log(EvKind::Program, "Master code set during setup");
                for other in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
                    if other == model {
                        continue;
                    }
                    let mut save = util::load_json::<PanelSave>(&panel_path(other)).unwrap_or_else(|| PanelSave::fresh(other));
                    save.ee.set_code(master_slot(other), &code);
                    util::save_json(&panel_path(other), &save);
                }
                self.save(false);
            }
            EngineMsg::EditPanel(edit) => self.apply_panel_edit(edit, now),
            EngineMsg::VerifyCode { code, reply } => {
                let ok = self.panel.ee.find_code(&code).map_or(false, |s| s != "inst");
                if !ok {
                    // Wrong guesses count toward the keypad lockout, photo included.
                    self.panel.bad_code(now);
                }
                let _ = reply.send(ok);
            }
            EngineMsg::UnlockInstaller { model, code, reply } => {
                let _ = reply.send(self.panel.verify_installer_master(model, &code, now));
                self.ctx.request_repaint();
            }
            EngineMsg::TestNotify => {
                self.notify.send(NotifyJob {
                    title: "RoomGuard test".to_string(),
                    message: format!("This is a test from RoomGuard, {}. If you can read it on your phone, pairing worked.", self.cfg.display_name()),
                    priority: 3,
                    tags: "white_check_mark".to_string(),
                    photo: None,
                    test: true,
                });
                self.notify_test_status = "Sending a test…".to_string();
                self.panel.dirty = true;
            }
            EngineMsg::HomeAssistant { epoch, update } => {
                if epoch == self.hub_epoch && self.cfg.home_assistant.enabled {
                    self.home_update(update, now);
                }
            }
            EngineMsg::HomePreview(id) => {
                if self.home.online && self.home.entities.get(&id).is_some_and(ha::Entity::camera) {
                    self.home.preview = Some((id.clone(), Err("Loading snapshot…".into())));
                    if !self.hub.preview(&self.cfg.home_assistant, self.hub_epoch, id.clone()) {
                        self.home.preview = Some((id, Err("Camera preview is busy. Try again shortly.".into())));
                    }
                    self.panel.dirty = true;
                }
            }
            EngineMsg::TestHaOutput { entity_id, on, auto_off_secs } => {
                if self.home.online {
                    let writable = self
                        .home
                        .entities
                        .get(&entity_id)
                        .map_or(true, ha::Entity::is_writable_output);
                    if writable {
                        self.hub.set_output(self.hub_epoch, entity_id.clone(), on);
                    }
                    self.ha_bell_cmd_until = Some(now + Duration::from_secs(2));
                    self.ha_bell_cmd_target = Some(on);
                    if on {
                        let ent = self.home.entities.get(&entity_id);
                        let pat = if ent.is_some_and(ha::Entity::is_fire_output) {
                            BellPattern::Temporal3
                        } else {
                            BellPattern::Steady
                        };
                        self.panel.ha_external_siren = Some(pat);
                        self.panel.ha_external_siren_label = Some(
                            ent.map(|e| e.name().to_string()).unwrap_or_else(|| entity_id.clone()),
                        );
                        if let Some(secs) = auto_off_secs {
                            self.ha_output_tests.insert(entity_id.clone(), now + Duration::from_secs(secs));
                        } else {
                            self.ha_output_tests.remove(&entity_id);
                        }
                    } else {
                        self.ha_output_tests.remove(&entity_id);
                        if self.ha_output_tests.is_empty() {
                            self.panel.ha_external_siren = None;
                            self.panel.ha_external_siren_label = None;
                        }
                    }
                    self.panel.dirty = true;
                }
            }
            EngineMsg::SilenceHaSiren => {
                self.ha_output_tests.clear();
                self.panel.ha_external_siren = None;
                self.panel.ha_external_siren_label = None;
                self.ha_bell_cmd_until = Some(now + Duration::from_secs(2));
                self.ha_bell_cmd_target = Some(false);
                if self.home.online {
                    for entity_id in &self.cfg.home_assistant.siren_outputs {
                        if self.home.entities.get(entity_id).map_or(true, ha::Entity::is_writable_output) {
                            self.hub.set_output(self.hub_epoch, entity_id.clone(), false);
                        }
                    }
                }
                self.panel.log(EvKind::Info, "External Home Assistant siren silenced");
                self.panel.dirty = true;
            }
            EngineMsg::ResetLearnedHaTimers => {
                self.panel.ha_learned_exit_away = None;
                self.panel.ha_learned_exit_stay = None;
                self.panel.ha_learned_entry = None;
                self.cfg.home_assistant.learned_exit_away = None;
                self.cfg.home_assistant.learned_exit_stay = None;
                self.cfg.home_assistant.learned_entry = None;
                self.home.learned_exit_away = None;
                self.home.learned_exit_stay = None;
                self.home.learned_entry = None;
                self.cfg.save();
                self.panel.dirty = true;
            }
            EngineMsg::ClearEvents => {
                self.panel.events.clear();
                self.panel.events_seq += 1;
                self.save_due = Some(now);
            }
            EngineMsg::Shutdown => return false,
        }
        true
    }

    fn apply_panel_edit(&mut self, edit: PanelEdit, now: Instant) {
        let busy = self.panel.is_on();
        let m = self.panel.model;
        match edit {
            PanelEdit::SetKeypadLevels { brightness, contrast, buzzer } => {
                self.panel.brightness = brightness.min(9);
                self.panel.contrast = contrast.min(9);
                self.panel.buzzer = buzzer.min(20);
                if self.panel.buzzer != self.last_buzzer {
                    self.last_buzzer = self.panel.buzzer;
                    self.push_volumes();
                }
            }
            PanelEdit::SetVoiceLevel(level) => {
                self.panel.vista_voice_level = level.min(7);
                self.audio.send(AudioCmd::VoiceLevel(self.panel.vista_voice_level));
            }
            PanelEdit::SetChime(on) => {
                self.panel.chime = on;
                self.panel.log(EvKind::Program, if on { "Door chime enabled (Settings)" } else { "Door chime disabled (Settings)" });
            }
            PanelEdit::SetQuickArm(on) => {
                if busy {
                    return;
                }
                if m == Model::Pc1550 {
                    self.panel.quick_arm = on;
                } else if m == Model::Vista20p {
                    self.panel.ee.sections.insert("21".to_string(), if on { "1" } else { "0" }.to_string());
                } else {
                    let mut t = self.panel.ee.toggles(m, "015");
                    t[3] = on;
                    self.panel.ee.set_toggles("015", &t[..8]);
                }
                self.panel.log(EvKind::Program, if on { "Quick arming enabled (Settings)" } else { "Quick arming disabled (Settings)" });
            }
            PanelEdit::SetAutoArm { enabled, hm } => {
                if busy {
                    return;
                }
                self.panel.auto_arm = enabled;
                self.panel.auto_arm_hm = hm;
                if m == Model::Pc1550 {
                    let mut raw = self.panel.ee.raw(m, "19");
                    if raw.len() < 8 {
                        raw = "99999999".to_string();
                    }
                    let head = match hm {
                        Some((h, mi)) => format!("{:02}{:02}", h.min(23), mi.min(59)),
                        None => "9999".to_string(),
                    };
                    let tail = raw.get(4..8).unwrap_or("9999");
                    self.panel.ee.sections.insert("19".to_string(), format!("{}{}", head, tail));
                }
                self.panel.log(EvKind::Program, "Auto-arm schedule updated (Settings)");
            }
            PanelEdit::SetSection { key, value } => {
                if busy {
                    return;
                }
                let Some(sp) = eeprom::spec(m, &key) else {
                    return;
                };
                if matches!(sp.kind, eeprom::Kind::Action | eeprom::Kind::Sub | eeprom::Kind::Code(_)) {
                    return;
                }
                let clean: String = value
                    .trim()
                    .to_ascii_uppercase()
                    .chars()
                    .filter(|c| c.is_ascii_hexdigit())
                    .collect();
                let final_val = match sp.kind {
                    eeprom::Kind::Data { entries, width } => {
                        let need = entries as usize * width as usize;
                        let mut v = clean;
                        while v.len() < need {
                            v.push('0');
                        }
                        v.truncate(need);
                        v
                    }
                    eeprom::Kind::Phone(max) => {
                        let mut v = clean;
                        v.truncate(max as usize);
                        v
                    }
                    eeprom::Kind::Toggle(n) => {
                        let mut v = String::new();
                        for i in 1..=n {
                            if let Some(c) = std::char::from_digit(i as u32, 10) {
                                if clean.contains(c) {
                                    v.push(c);
                                }
                            }
                        }
                        v
                    }
                    _ => return,
                };
                if m == Model::Pc1832 {
                    self.panel.ee.default_changed_pc1832_zone_attrs(&key, &final_val);
                }
                self.panel.ee.sections.insert(key.clone(), final_val);
                self.panel.log(EvKind::Program, format!("Section [{}] updated (Settings)", key));
            }
            PanelEdit::SetToggle { key, opt, on } => {
                if busy || !(1..=9).contains(&opt) {
                    return;
                }
                let Some(sp) = eeprom::spec(m, &key) else {
                    return;
                };
                let eeprom::Kind::Toggle(n) = sp.kind else {
                    return;
                };
                if opt > n {
                    return;
                }
                let mut t = self.panel.ee.toggles(m, &key);
                t[opt as usize - 1] = on;
                self.panel.ee.set_toggles(&key, &t[..n as usize]);
                self.panel.log(EvKind::Program, format!("Section [{}] option {} {} (Settings)", key, opt, if on { "on" } else { "off" }));
            }
            PanelEdit::SetCode { slot, code } => {
                if busy {
                    return;
                }
                let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
                if digits.is_empty() {
                    if slot != "inst" && slot != master_slot(m) {
                        self.panel.ee.clear_code(&slot);
                        self.panel.log(EvKind::Program, format!("Code slot {} cleared (Settings)", slot));
                    }
                } else if digits.len() == 4 {
                    self.panel.ee.set_code(&slot, &digits);
                    self.panel.log(EvKind::Program, format!("Code slot {} updated (Settings)", slot));
                } else {
                    return;
                }
            }
            PanelEdit::SetInstallerLockout(on) => {
                if busy {
                    return;
                }
                self.panel.ee.installer_lockout = on;
                self.panel.log(EvKind::Program, if on { "Installer lockout enabled (Settings)" } else { "Installer lockout disabled (Settings)" });
            }
            PanelEdit::SetZoneType { zone, code } => {
                if busy || zone == 0 || usize::from(zone) > m.zone_count() {
                    return;
                }
                self.set_zone_type_code(usize::from(zone), &code);
            }
            PanelEdit::DisableZones(zones) => {
                if busy {
                    return;
                }
                let null_code = match m {
                    Model::Pc1550 => "09",
                    Model::Pc1832 | Model::Vista20p => "00",
                };
                let mut disabled = 0usize;
                for z in zones {
                    let zi = usize::from(z);
                    if zi >= 1 && zi <= m.zone_count() && self.panel.cfg(zi).kind != ZoneKind::Null {
                        self.set_zone_type_code(zi, null_code);
                        self.sim[zi - 1] = false;
                        disabled += 1;
                    }
                }
                if disabled > 0 {
                    self.sync_zones(now);
                    self.panel.log(EvKind::Program, format!("Disabled {} unused zone(s)", disabled));
                }
            }
            PanelEdit::FactoryDefault => {
                if busy {
                    return;
                }
                let inst_keep = if self.panel.ee.installer_lockout {
                    self.panel.ee.codes.get("inst").cloned()
                } else {
                    None
                };
                let lock = self.panel.ee.installer_lockout;
                self.panel.ee = eeprom::Eeprom::factory(m);
                self.panel.ee.installer_lockout = lock;
                if let Some(c) = inst_keep {
                    self.panel.ee.codes.insert("inst".to_string(), c);
                }
                self.panel.chime = false;
                self.panel.quick_arm = true;
                self.panel.auto_arm = false;
                self.panel.auto_arm_hm = None;
                self.panel.log(EvKind::Program, "Panel programming reset to defaults (Settings)");
            }
        }
        self.panel.dirty = true;
        self.save_due = Some(now + Duration::from_millis(400));
        self.last_publish = None;
    }

    fn set_zone_type_code(&mut self, zone: usize, code: &str) {
        let m = self.panel.model;
        let clean: String = code.trim().to_ascii_uppercase().chars().filter(|c| c.is_ascii_hexdigit()).take(2).collect();
        if clean.len() != 2 {
            return;
        }
        let (sec, entries, idx) = match m {
            Model::Pc1550 => ("11", 6usize, zone - 1),
            Model::Pc1832 => {
                let (s, i) = eeprom::pc1832_zone_section(zone);
                (s, 16usize, i)
            }
            Model::Vista20p => ("56", m.zone_count(), zone - 1),
        };
        let mut raw = self.panel.ee.raw(m, sec);
        let need = entries * 2;
        while raw.len() < need {
            raw.push('0');
        }
        raw.truncate(need);
        let start = idx * 2;
        if start + 2 <= raw.len() {
            raw.replace_range(start..start + 2, &clean);
            if m == Model::Pc1832 {
                self.panel.ee.default_changed_pc1832_zone_attrs(sec, &raw);
            }
            self.panel.ee.sections.insert(sec.to_string(), raw);
            self.panel.log(EvKind::Program, format!("Zone {:02} type set to {} (Settings)", zone, clean));
        }
    }

    fn vision(&mut self, ev: VisionEvent, now: Instant) {
        if !self.cfg.camera.enabled {
            return;
        }
        match ev {
            VisionEvent::Motion { zone, open } => {
                if (1..=8).contains(&zone) {
                    self.cam_motion[zone as usize - 1] = open;
                    self.sync_zones(now);
                }
            }
            VisionEvent::Camera { online: _, covered } => {
                if covered != self.cam_covered {
                    self.cam_covered = covered;
                    self.panel.inputs.cam_covered = covered;
                    self.panel.log(EvKind::Camera, if covered { "Camera view blocked" } else { "Camera view clear again" });
                    self.sync_zones(now);
                }
            }
            VisionEvent::DoorOpen { zone, open } => {
                if (1..=8).contains(&zone) {
                    let i = zone as usize - 1;
                    if self.cam_contact_open[i] != open {
                        self.cam_contact_open[i] = open;
                        let lbl = self.panel.label(zone as usize);
                        self.panel.log(
                            EvKind::Camera,
                            if open {
                                format!("{} opened (seen by the camera)", lbl)
                            } else {
                                format!("{} closed (seen by the camera)", lbl)
                            },
                        );
                        self.sync_zones(now);
                    }
                }
            }
            VisionEvent::Recognized { name } => {
                let important = self.panel.entry.is_some() || self.panel.alarm_sounding(now);
                if important || self.last_face_log.map_or(true, |t| now.saturating_duration_since(t).as_secs() > 300) {
                    self.last_face_log = Some(now);
                    self.panel.log(EvKind::Camera, format!("Recognized {}", name));
                }
                self.panel.face_recognized(&name, now);
            }
            VisionEvent::Stranger => {
                let armed = self.panel.armed.is_some();
                if armed && self.last_stranger.map_or(true, |t| now.saturating_duration_since(t).as_secs() > 60) {
                    self.last_stranger = Some(now);
                    self.panel.log(EvKind::Camera, "Unknown person seen while armed");
                    self.cam.send(CamCmd::Snapshot("unknown-person".to_string()));
                    self.panel.report(ReportKind::Stranger, 0, 0, "Unknown person".to_string(), 0);
                }
            }
            VisionEvent::Snapshot { path, reason } => {
                if reason.starts_with("phone-alert-") {
                    if let Some(index) = self.pending_photos.iter().position(|(id, _, _)| id == &reason) {
                        let (_, mut job, deadline) = self.pending_photos.swap_remove(index);
                        if now <= deadline && self.cfg.operator.send_photos {
                            job.photo = Some(path);
                        }
                        self.notify.send(job);
                    }
                } else {
                    self.op.photo(path, now);
                }
            }
        }
    }

    fn home_update(&mut self, update: ha::Update, now: Instant) {
        match update {
            ha::Update::Preview { id, result } => {
                if self.home.online && self.home.preview.as_ref().is_some_and(|(selected, _)| selected == &id) {
                    self.home.preview = Some((id, result));
                    self.panel.dirty = true;
                }
                return;
            }
            ha::Update::Connection(conn) => {
                self.home.online = matches!(conn, ha::ConnectionState::Connected);
                self.home.status = match &conn {
                    ha::ConnectionState::Disconnected => "Disconnected".into(),
                    ha::ConnectionState::Connecting => "Connecting to Home Assistant...".into(),
                    ha::ConnectionState::Connected => "Connected".into(),
                    ha::ConnectionState::Error(err) => err.clone(),
                };
                self.home.state = conn;
                if !self.home.online {
                    self.home.preview = None;
                    self.home.esphome_services.clear();
                    self.home.detected_esphome = None;
                    self.home.detected_bypass_entity = None;
                    self.ha_companions = ha::PanelCompanions::default();
                    self.home_pulses.clear();
                    self.sync_physical_panel(false, now);
                    self.sync_ha_sirens(now);
                }
            }
            ha::Update::Services(services) => {
                self.home.esphome_services = services;
                self.refresh_esphome_detection();
                self.sync_physical_panel(false, now);
                self.panel.dirty = true;
                return;
            }
            ha::Update::KeypressResult { service, result } => {
                if let Err(err) = result {
                    let msg = format!("ESPHome keypress ({service}) failed: {err}");
                    self.home.service_notice = Some((false, msg.clone()));
                    self.panel.log(EvKind::Trouble, msg);
                    self.panel.ha_bypass_outbound_until = None;
                    self.sync_physical_panel(false, now);
                }
                self.panel.dirty = true;
                return;
            }
            ha::Update::ServiceResult { action, result } => {
                match result {
                    Ok(()) => {
                        self.ha_retry_cmd = None;
                        self.home.service_notice = Some((true, format!("Sent {} to Home Assistant panel", action.label())));
                    }
                    Err(err) => {
                        let code_err = err.to_ascii_lowercase().contains("code");
                        if code_err {
                            if let Some((retry_cmd, fallback_code, at)) = self.ha_retry_cmd.take() {
                                if now.saturating_duration_since(at) < Duration::from_secs(6) {
                                    self.panel.log(
                                        EvKind::Info,
                                        "Saved HA panel PIN was rejected; retrying with the keypad code you typed",
                                    );
                                    self.panel.ha_outbound_until = Some(now + Duration::from_secs(3));
                                    self.send_ha_panel_cmd_with_override(retry_cmd, Some(fallback_code));
                                    self.panel.dirty = true;
                                    return;
                                }
                            }
                        }
                        self.ha_retry_cmd = None;
                        let msg = if code_err {
                            format!(
                                "HA rejected {} (invalid PIN). Update 'HA panel PIN' in Settings -> Home Assistant (or leave blank to use your keypad code).",
                                action.label()
                            )
                        } else {
                            format!("Home Assistant panel rejected {}: {}", action.label(), err)
                        };
                        self.home.service_notice = Some((false, msg.clone()));
                        self.panel.log(EvKind::Trouble, msg);
                        if code_err {
                            match self.panel.model {
                                Model::Pc1832 => self.panel.temp_at("HA PIN Invalid", "Check Settings", 4000, now),
                                Model::Vista20p => self.panel.temp_at("HA PIN INVALID", "CHECK SETTINGS", 4000, now),
                                Model::Pc1550 => {}
                            }
                            self.panel.error_tone();
                        }
                        // Clear outbound suppression and immediately re-sync to HA's actual state.
                        self.panel.ha_outbound_until = None;
                        self.panel.ha_expect = None;
                        self.sync_physical_panel(false, now);
                    }
                }
                self.panel.dirty = true;
                return;
            }
            ha::Update::OutputResult { entity_id, on, result } => {
                let name = self
                    .home
                    .entities
                    .get(&entity_id)
                    .map(|e| e.name().to_string())
                    .unwrap_or(entity_id);
                match result {
                    Ok(()) => {
                        self.home.output_notice = Some((
                            true,
                            if on {
                                format!("Turned ON {name} in Home Assistant")
                            } else {
                                format!("Turned OFF {name} in Home Assistant")
                            },
                        ));
                    }
                    Err(err) => {
                        let msg = format!("Home Assistant output {name} failed: {err}");
                        self.home.output_notice = Some((false, msg.clone()));
                        self.panel.log(EvKind::Trouble, msg);
                    }
                }
                self.panel.dirty = true;
                return;
            }
            ha::Update::Snapshot(entities) => {
                self.home.entities = Arc::new(entities);
                self.home.online = true;
                self.home.state = ha::ConnectionState::Connected;
                self.home.status = "Connected".into();
                self.home_pulses.clear();
                self.refresh_esphome_detection();
                self.sync_home_baseline(now);
                // A changed state on reconnect is new to this running session.
                self.sync_physical_panel(self.panel.physical_state.is_some(), now);
                self.sync_ha_sirens(now);
                self.panel.dirty = true;
                return;
            }
            ha::Update::Refresh(entities) => {
                self.home.entities = Arc::new(entities);
                self.home.online = true;
                self.home.state = ha::ConnectionState::Connected;
                self.home.status = "Connected".into();
                self.refresh_esphome_detection();
                self.sync_zones(now);
                self.sync_physical_panel(true, now);
                self.sync_ha_sirens(now);
                if self.home.service_notice.as_ref().is_some_and(|(ok, _)| *ok)
                    && self.panel.ha_outbound_until.is_none()
                {
                    self.home.service_notice = None;
                }
                self.panel.dirty = true;
                return;
            }
            ha::Update::State { id, entity } => {
                if !self.home.online {
                    return;
                }
                let is_bound_zone = self
                    .cfg
                    .home_assistant
                    .bindings
                    .iter()
                    .any(|b| b.model == self.panel.model && b.entity == id);
                let is_siren_output = self.cfg.home_assistant.siren_outputs.iter().any(|s| s == &id);
                let changed = is_bound_zone
                    && self.home.entities.get(&id).zip(entity.as_ref()).is_some_and(|(old, new)| {
                        old.state != new.state
                            && (ha::Rule::Changes.active(&old.state, "").is_some()
                                || (id.starts_with("event.") && old.state == "unknown"))
                            && ha::Rule::Changes.active(&new.state, "").is_some()
                    });
                if changed {
                    let response = self.cfg.home_assistant.bindings.iter()
                        .filter(|binding| binding.model == self.panel.model && binding.entity == id && binding.rule == ha::Rule::Changes)
                        .map(|binding| self.panel.loop_response_ms(usize::from(binding.zone)))
                        .max();
                    if let Some(ms) = response {
                        self.home_pulses.insert(id.clone(), now + Duration::from_millis(ms + 1500));
                    }
                }
                let follow_id = self.cfg.home_assistant.followed_panel().map(str::to_owned);
                let follow_changed = follow_id.as_deref() == Some(id.as_str());
                let existed = self.home.entities.contains_key(&id);
                let entities = Arc::make_mut(&mut self.home.entities);
                let membership_changed = match entity {
                    Some(entity) => {
                        entities.insert(id.clone(), entity);
                        !existed
                    }
                    None => {
                        entities.remove(&id);
                        existed
                    }
                };
                if membership_changed {
                    self.refresh_esphome_detection();
                }
                let is_companion = is_bound_zone
                    || self.ha_companions.zone_status.as_deref() == Some(id.as_str())
                    || self.ha_companions.lab_bypasses.as_deref() == Some(id.as_str())
                    || self.ha_companions.bypass_flag.as_deref() == Some(id.as_str())
                    || self.ha_companions.instant_flag.as_deref() == Some(id.as_str())
                    || self.ha_companions.fire_flag.as_deref() == Some(id.as_str())
                    || self.ha_companions.alarm_flag.as_deref() == Some(id.as_str());
                if is_bound_zone {
                    self.sync_zones(now);
                }
                if follow_changed || is_companion {
                    if follow_changed && self.home.service_notice.as_ref().is_some_and(|(ok, _)| *ok) {
                        self.home.service_notice = None;
                    }
                    self.sync_physical_panel(true, now);
                }
                if is_siren_output {
                    self.sync_ha_sirens(now);
                }
                if is_bound_zone || follow_changed || is_companion || is_siren_output {
                    self.panel.dirty = true;
                }
                return;
            }
        }
        self.sync_zones(now);
        self.panel.dirty = true;
    }

    fn refresh_esphome_detection(&mut self) {
        let Some(fp) = self.cfg.home_assistant.followed_panel() else {
            self.home.detected_esphome = None;
            self.home.detected_bypass_entity = None;
            self.ha_companions = ha::PanelCompanions::default();
            return;
        };
        let pfx = ha::detect_esphome_prefix(fp, &self.home.esphome_services);
        let comp = ha::find_panel_companions(fp, &self.home.entities, pfx.as_deref());
        self.home.detected_esphome = pfx;
        self.home.detected_bypass_entity = comp
            .zone_status
            .clone()
            .or_else(|| comp.lab_bypasses.clone())
            .or_else(|| comp.bypass_flag.clone());
        self.ha_companions = comp;
    }

    fn sync_physical_panel(&mut self, live: bool, now: Instant) {
        let link = self.cfg.home_assistant.panel_link();
        self.panel.set_panel_link(link, now);
        self.panel.ha_zone_mirror_only = link != ha::PanelLink::Independent && !self.cfg.home_assistant.ha_zones_trigger_alarm;
        let id = self.cfg.home_assistant.followed_panel().map(str::to_owned);
        let mut instant_active = false;
        if let Some(ref panel_id) = id {
            if self.home.online {
                let comp = self.ha_companions.clone();
                let max_z = self.panel.n() as u8;
                let mut remote_bypasses: Vec<u8> = Vec::new();
                let mut remote_alarms: Vec<u8> = Vec::new();
                let mut has_bypass_source = false;
                if let Some(zs_id) = &comp.zone_status {
                    if let Some(ent) = self.home.entities.get(zs_id) {
                        let snap = ha::parse_zone_status(&ent.state, max_z, false);
                        has_bypass_source = true;
                        remote_bypasses.extend(snap.bypassed);
                        remote_alarms.extend(snap.alarmed);
                    }
                }
                if let Some(lab_id) = &comp.lab_bypasses {
                    if let Some(ent) = self.home.entities.get(lab_id) {
                        let snap = ha::parse_zone_status(&ent.state, max_z, true);
                        has_bypass_source = true;
                        for z in snap.bypassed {
                            if !remote_bypasses.contains(&z) {
                                remote_bypasses.push(z);
                            }
                        }
                    }
                }
                if let Some(panel_ent) = self.home.entities.get(panel_id) {
                    let attr_byps = panel_ent.attribute_bypassed_zones(max_z);
                    if !attr_byps.is_empty() {
                        has_bypass_source = true;
                        for z in attr_byps {
                            if !remote_bypasses.contains(&z) {
                                remote_bypasses.push(z);
                            }
                        }
                    }
                }
                for binding in self.cfg.home_assistant.bindings.iter().filter(|b| b.model == self.panel.model) {
                    if let Some(ent) = self.home.entities.get(&binding.entity) {
                        if ent.is_zone_bypassed() {
                            has_bypass_source = true;
                            if !remote_bypasses.contains(&binding.zone) {
                                remote_bypasses.push(binding.zone);
                            }
                        }
                    }
                }
                if !has_bypass_source {
                    if let Some(byp_id) = &comp.bypass_flag {
                        if let Some(ent) = self.home.entities.get(byp_id) {
                            if ent.state.eq_ignore_ascii_case("off") {
                                has_bypass_source = true;
                            }
                        }
                    }
                }
                let fire_active = comp
                    .fire_flag
                    .as_deref()
                    .and_then(|fid| self.home.entities.get(fid))
                    .is_some_and(|e| e.state.eq_ignore_ascii_case("on"));
                instant_active = comp
                    .instant_flag
                    .as_deref()
                    .and_then(|iid| self.home.entities.get(iid))
                    .is_some_and(|e| e.state.eq_ignore_ascii_case("on"));
                // Sync real-panel bypasses, alarmed zones, and fire flag BEFORE checking
                // armed/ready state so bypassed open zones on the real panel are honored!
                if self.cfg.home_assistant.sync_bypasses {
                    self.panel.sync_ha_bypasses(&remote_bypasses, &remote_alarms, fire_active, has_bypass_source, now);
                } else {
                    self.panel.sync_ha_bypasses(&[], &remote_alarms, fire_active, false, now);
                }
            }
        }
        let panel_ent = id
            .as_deref()
            .filter(|_| self.home.online)
            .and_then(|id| self.home.entities.get(id));
        let mut state = panel_ent.and_then(|entity| ha::AlarmState::parse(&entity.state));
        if state == Some(ha::AlarmState::Stay) && instant_active {
            state = Some(ha::AlarmState::StayInstant);
        }
        let next_state = panel_ent.and_then(ha::Entity::alarm_next_state);
        let delay_secs = panel_ent.and_then(ha::Entity::alarm_delay_secs);
        let elapsed_secs = panel_ent.and_then(ha::Entity::elapsed_since_last_changed_secs);
        self.panel.sync_physical_panel_meta(state, next_state, delay_secs, elapsed_secs, live, now);
        if self.panel.ha_timers_dirty {
            self.panel.ha_timers_dirty = false;
            self.cfg.home_assistant.learned_exit_away = self.panel.ha_learned_exit_away;
            self.cfg.home_assistant.learned_exit_stay = self.panel.ha_learned_exit_stay;
            self.cfg.home_assistant.learned_entry = self.panel.ha_learned_entry;
            self.home.learned_exit_away = self.panel.ha_learned_exit_away;
            self.home.learned_exit_stay = self.panel.ha_learned_exit_stay;
            self.home.learned_entry = self.panel.ha_learned_entry;
            self.cfg.save();
        }
    }

    fn sync_ha_sirens(&mut self, now: Instant) {
        if !self.home.online {
            if self.ha_output_tests.is_empty() && self.panel.ha_external_siren.is_some() {
                self.panel.ha_external_siren = None;
                self.panel.ha_external_siren_label = None;
                self.panel.dirty = true;
            }
            return;
        }
        // Clear outbound command guard early as soon as HA echoes the expected state,
        // so flipping the HA siren switch back ON immediately activates RoomGuard's siren!
        if let (Some(until), Some(target)) = (self.ha_bell_cmd_until, self.ha_bell_cmd_target) {
            if now >= until {
                self.ha_bell_cmd_until = None;
                self.ha_bell_cmd_target = None;
            } else {
                let mut any_writable = false;
                let mut all_match = true;
                for entity_id in &self.cfg.home_assistant.siren_outputs {
                    if let Some(ent) = self.home.entities.get(entity_id) {
                        if ent.is_writable_output() {
                            any_writable = true;
                            if ent.is_output_on() != target {
                                all_match = false;
                                break;
                            }
                        }
                    }
                }
                if any_writable && all_match {
                    self.ha_bell_cmd_until = None;
                    self.ha_bell_cmd_target = None;
                }
            }
        }
        if !self.ha_bell_active && self.ha_output_tests.is_empty() && self.ha_bell_cmd_until.is_none() {
            let mut ext_pat: Option<BellPattern> = None;
            let mut ext_label: Option<String> = None;
            for entity_id in &self.cfg.home_assistant.siren_outputs {
                if let Some(ent) = self.home.entities.get(entity_id) {
                    if ent.is_output_on() {
                        if ent.is_fire_output() {
                            ext_pat = Some(BellPattern::Temporal3);
                            ext_label = Some(ent.name().to_string());
                            break;
                        }
                        if ext_pat.is_none() {
                            ext_pat = Some(BellPattern::Steady);
                            ext_label = Some(ent.name().to_string());
                        }
                    }
                }
            }
            if self.panel.ha_external_siren != ext_pat || self.panel.ha_external_siren_label != ext_label {
                let was_on = self.panel.ha_external_siren.is_some();
                let now_on = ext_pat.is_some();
                self.panel.ha_external_siren = ext_pat;
                self.panel.ha_external_siren_label = ext_label.clone();
                if now_on && !was_on {
                    let who = ext_label.as_deref().unwrap_or("Home Assistant siren");
                    self.panel.log(EvKind::Alarm, format!("Home Assistant siren active: {who}"));
                } else if !now_on && was_on {
                    self.panel.log(EvKind::Info, "Home Assistant siren turned off");
                }
                self.panel.dirty = true;
            }
        }
    }

    fn home_active(&self, binding: &ha::Binding) -> Option<bool> {
        if !self.home.online {
            return None;
        }
        let state = &self.home.entities.get(&binding.entity)?.state;
        let result = binding.rule.active(state, &binding.operand)?;
        Some(result || (binding.rule == ha::Rule::Changes && self.home_pulses.contains_key(&binding.entity)))
    }

    fn sync_home_baseline(&mut self, now: Instant) {
        for i in 0..self.panel.n() {
            if let Some(binding) = self.cfg.home_assistant.binding(self.panel.model, (i + 1) as u8) {
                if let Some(open) = self.home_active(binding) {
                    self.panel.zones[i].open = open;
                }
                self.zone_pend[i] = None;
            }
        }
        self.sync_zones(now);
    }

    /// Camera conditions become zone states. The PowerSeries knows zone
    /// tamper and fault; the PC1550 doesn't, so on it a covered or dead camera
    /// simply shows as the zone being open, the same as a cut wire.
    fn sync_zones(&mut self, now: Instant) {
        let mask = self.cfg.camera.camera_mask();
        let n = self.panel.n();
        self.panel.camera_zone = mask;
        if self.panel.home_zone.len() != n {
            self.panel.home_zone.resize(n, false);
        }
        self.panel.home_sensor_unavailable = false;
        for i in 0..n {
            self.zone_want[i] = if let Some(binding) = self.cfg.home_assistant.binding(self.panel.model, (i + 1) as u8) {
                self.panel.home_zone[i] = true;
                let active = self.home_active(binding);
                if active.is_some() {
                    self.home_lost_at[i] = None;
                } else if self.home_lost_at[i].is_none() {
                    self.home_lost_at[i] = Some(now);
                }
                // A sensor, or the whole link, has to stay gone for the
                // supervision window before it's a trouble, like a wireless
                // zone missing its check-ins. Until then the zone keeps its
                // last known state, so a Home Assistant restart is silent.
                let missing = self.home_lost_at[i].is_some_and(|since| now.saturating_duration_since(since) >= ha::SUPERVISION_GRACE);
                if let Some(camera) = self.panel.camera_zone.get_mut(i) {
                    *camera = false;
                }
                self.panel.home_sensor_unavailable |= missing;
                // Transport failure is a supervision fault, never a motion edge.
                (active.unwrap_or(self.panel.zones[i].open), false, missing)
            } else if !mask.get(i).copied().unwrap_or(false) {
                self.panel.home_zone[i] = false;
                (self.sim[i], false, false)
            } else {
                self.panel.home_zone[i] = false;
                let active = self.cam_motion[i] || self.cam_contact_open[i];
                match self.panel.model {
                    Model::Pc1832 | Model::Vista20p => (active, self.cam_covered, !self.cam_online),
                    Model::Pc1550 => (active || self.cam_covered || !self.cam_online, false, false),
                }
            };
        }
        self.settle_zones(now);
    }

    /// Hands zone changes to the panel the way its zone inputs would: a zone
    /// has to stay open, or closed, for the programmed loop response time
    /// before the panel believes it (PC1550 500 ms from section [17], or 10
    /// ms for fast zones; PC1832 400 ms, or 36 ms for fast zones in [030]).
    /// Delay zones settle ahead of Interior/Follower zones so entering through
    /// a doorway never races into a false instant alarm on Room Motion.
    fn settle_zones(&mut self, now: Instant) {
        let n = self.panel.n();
        for pass in 0..2 {
            for i in 0..n {
                let z = i + 1;
                let (open, tamper, fault) = self.zone_want[i];
                let cur = self.panel.zones[i];
                if (cur.open, cur.tamper, cur.fault) == (open, tamper, fault) && self.zone_pend[i].is_none() {
                    continue;
                }
                let kind = self.panel.cfg(z).kind;
                let is_delay = kind.is_delay();
                if (pass == 0 && !is_delay) || (pass == 1 && is_delay) {
                    continue;
                }
                if cur.tamper != tamper || cur.fault != fault {
                    self.panel.set_zone(z, cur.open, tamper, fault, now);
                }
                if cur.open == open {
                    self.zone_pend[i] = None;
                    continue;
                }
                let is_follower = matches!(
                    kind,
                    ZoneKind::Interior | ZoneKind::StayAway | ZoneKind::InteriorDelay | ZoneKind::NightZone
                );
                if open && is_follower && self.panel.armed.is_some() && self.panel.entry.is_none() {
                    let delay_opening = (0..n).any(|d| {
                        d != i
                            && self.panel.cfg(d + 1).kind.is_delay()
                            && !self.panel.zones[d].bypassed
                            && self.zone_want[d].0
                            && !self.panel.zones[d].open
                    });
                    if delay_opening {
                        if self.zone_pend[i].is_none() {
                            self.zone_pend[i] = Some((open, now));
                        }
                        continue;
                    }
                }
                match self.zone_pend[i] {
                    Some((want, since)) if want == open => {
                        if now.saturating_duration_since(since).as_millis() as u64 >= self.panel.loop_response_ms(z) {
                            self.zone_pend[i] = None;
                            self.panel.set_zone(z, open, tamper, fault, now);
                        }
                    }
                    _ => self.zone_pend[i] = Some((open, now)),
                }
            }
        }
    }

    fn check_camera(&mut self, now: Instant) {
        if !self.cfg.camera.enabled {
            if !self.cam_online {
                self.cam_online = true;
                self.panel.inputs.cam_online = true;
                self.sync_zones(now);
            }
            return;
        }
        let last = self.cam.shared.lock().map(|s| s.last_frame_ms).unwrap_or(0);
        let fresh = util::now_ms() - last < 6000;
        let online = fresh || now < self.grace_until;
        if online != self.cam_online {
            self.cam_online = online;
            self.panel.inputs.cam_online = online;
            self.panel.log(EvKind::Camera, if online { "Camera back online" } else { "Camera stopped sending pictures" });
            if !online {
                self.cam_motion = [false; 8];
                self.cam_contact_open = [false; 8];
            }
            self.sync_zones(now);
        }
    }

    /// Keeps Home Assistant told what RoomGuard is doing: the
    /// `sensor.roomguard_alarm` status, `roomguard_state` and
    /// `roomguard_alarm` events, and alarm notifications. Codes are never
    /// sent. It republishes as soon as the link comes back, so Home Assistant
    /// is never left with a stale state, and it notifies an alarm that began
    /// while the link was down once it returns.
    fn publish_ha_status(&mut self, now: Instant) {
        let ha_cfg = &self.cfg.home_assistant;
        let (publish, alerts) = (ha_cfg.publish_status, ha_cfg.alarm_notifications);
        let notify = ha::notify_target(&ha_cfg.notify_service);
        if !ha_cfg.enabled || !self.home.online || !(publish || alerts || notify.is_some()) {
            self.ha_status_online = false;
            return;
        }
        if self.ha_status_online && self.ha_status_check.is_some_and(|at| now.saturating_duration_since(at) < Duration::from_millis(250)) {
            return;
        }
        self.ha_status_check = Some(now);
        let reconnected = !self.ha_status_online;
        self.ha_status_online = true;

        let p = &self.panel;
        let live: Vec<(types::AlarmKind, u8)> = p.alarms.iter().filter(|a| !a.silenced).map(|a| (a.kind, a.zone)).collect();
        let triggered = !live.is_empty();
        let armed = p.armed.map(|a| (matches!(a.mode, types::ArmMode::Away), p.vista_night));
        let state = ha::alarm_state_name(triggered, p.entry.is_some(), p.exit.is_some(), armed);
        let model = p.model.panel_name();
        let alarm_zones: Vec<u8> = live.iter().map(|&(_, z)| z).filter(|&z| z != 0).collect();
        let alarm_names: Vec<String> = alarm_zones
            .iter()
            .map(|&z| if z as usize <= p.n() { p.label(z as usize) } else { format!("Zone {z}") })
            .collect();
        let kind = live.first().map(|(k, _)| k.label());
        let pick = |test: fn(&panel::ZoneRt) -> bool| -> Vec<u8> {
            (1..=p.n()).filter(|&z| test(&p.zones[z - 1])).map(|z| z as u8).collect()
        };
        let troubles: Vec<&str> = TROUBLE_NAMES.iter().filter(|(bit, _)| p.troubles & bit != 0).map(|&(_, name)| name).collect();
        let icon = if triggered {
            "mdi:shield-alert"
        } else if armed.is_some() || p.exit.is_some() {
            "mdi:shield-lock"
        } else {
            "mdi:shield-off-outline"
        };
        let open_zones = pick(|z| z.open);
        let bypassed_zones = pick(|z| z.bypassed);
        let (ready, entry, exit, trouble, ac_power) = (p.ready(), p.entry.is_some(), p.exit.is_some(), p.troubles != 0, p.inputs.ac_ok);
        let attributes = serde_json::json!({
            "friendly_name": "RoomGuard",
            "icon": icon,
            "panel": model,
            "ready": ready,
            "alarm": triggered,
            "alarm_kind": kind,
            "alarm_zones": alarm_zones,
            "alarm_zone_names": alarm_names,
            "entry_delay": entry,
            "exit_delay": exit,
            "trouble": trouble,
            "troubles": troubles,
            "ac_power": ac_power,
            "open_zones": open_zones,
            "bypassed_zones": bypassed_zones,
        });
        let place = if alarm_names.is_empty() { String::new() } else { format!(": {}", alarm_names.join(", ")) };
        let message = format!("{} alarm on the {}{}", kind.unwrap_or("Burglary"), model, place);

        let snapshot = format!("{state}{attributes}");
        let heartbeat = self.ha_status_at.map_or(true, |at| now.saturating_duration_since(at) >= Duration::from_secs(600));
        if publish && (reconnected || heartbeat || self.ha_status_sent.as_deref() != Some(snapshot.as_str())) {
            self.hub.publish_state(&self.cfg.home_assistant, self.hub_epoch, state.to_string(), attributes);
            self.ha_status_at = Some(now);
        }
        self.ha_status_sent = Some(snapshot);
        if publish && self.ha_state_name.is_some_and(|previous| previous != state) {
            self.hub.fire_event(
                self.hub_epoch,
                "roomguard_state",
                serde_json::json!({ "state": state, "previous": self.ha_state_name, "panel": model }),
            );
        }
        self.ha_state_name = Some(state);
        if triggered && !self.ha_alarm_notified {
            self.ha_alarm_notified = true;
            if publish {
                self.hub.fire_event(
                    self.hub_epoch,
                    "roomguard_alarm",
                    serde_json::json!({ "kind": kind, "zones": alarm_zones, "zone_names": alarm_names, "panel": model, "message": message }),
                );
            }
            if alerts {
                self.hub.call_service(
                    self.hub_epoch,
                    "persistent_notification",
                    "create",
                    serde_json::json!({ "notification_id": "roomguard_alarm", "title": "RoomGuard alarm", "message": message }),
                );
            }
            if let Some((domain, service)) = &notify {
                self.hub.call_service(self.hub_epoch, domain, service, serde_json::json!({ "title": "RoomGuard alarm", "message": message }));
            }
        } else if !triggered && self.ha_alarm_notified {
            self.ha_alarm_notified = false;
            if alerts {
                self.hub.call_service(
                    self.hub_epoch,
                    "persistent_notification",
                    "dismiss",
                    serde_json::json!({ "notification_id": "roomguard_alarm" }),
                );
            }
        }
    }

    fn apply_config(&mut self, mut new: AppConfig, now: Instant) {
        let busy = self.panel.is_on();
        if new.model != self.panel.model && busy {
            new.model = self.panel.model;
        }
        if busy {
            // Keep HA panel PIN, siren outputs, and sync preferences editable even while armed
            // so a wrong PIN can never trap the user out of disarming!
            let pin = new.home_assistant.panel_code;
            let sirens = new.home_assistant.siren_outputs;
            let zones_trigger = new.home_assistant.ha_zones_trigger_alarm;
            let sync_bypasses = new.home_assistant.sync_bypasses;
            new.home_assistant = self.cfg.home_assistant.clone();
            new.home_assistant.panel_code = pin;
            new.home_assistant.siren_outputs = sirens;
            new.home_assistant.ha_zones_trigger_alarm = zones_trigger;
            new.home_assistant.sync_bypasses = sync_bypasses;
        }
        let hub_changed = new.home_assistant.enabled != self.cfg.home_assistant.enabled
            || new.home_assistant.url != self.cfg.home_assistant.url
            || new.home_assistant.token != self.cfg.home_assistant.token;
        let bindings_changed = new.home_assistant.bindings != self.cfg.home_assistant.bindings;
        let followed_entity_changed = new.home_assistant.followed_panel() != self.cfg.home_assistant.followed_panel();
        let followed_panel_changed = followed_entity_changed
            || new.home_assistant.panel_sync != self.cfg.home_assistant.panel_sync
            || new.home_assistant.ha_zones_trigger_alarm != self.cfg.home_assistant.ha_zones_trigger_alarm;
        let sirens_changed = new.home_assistant.siren_outputs != self.cfg.home_assistant.siren_outputs;
        let sync_bypasses_changed = new.home_assistant.sync_bypasses != self.cfg.home_assistant.sync_bypasses;
        // Preserve Engine-learned panel timers unless the user switched to a different linked panel.
        if followed_entity_changed {
            new.home_assistant.learned_exit_away = None;
            new.home_assistant.learned_exit_stay = None;
            new.home_assistant.learned_entry = None;
        } else {
            new.home_assistant.learned_exit_away = self.panel.ha_learned_exit_away.or(new.home_assistant.learned_exit_away);
            new.home_assistant.learned_exit_stay = self.panel.ha_learned_exit_stay.or(new.home_assistant.learned_exit_stay);
            new.home_assistant.learned_entry = self.panel.ha_learned_entry.or(new.home_assistant.learned_entry);
        }
        let model_changed = new.model != self.panel.model;
        let cam_changed = new.camera != self.cfg.camera;
        let webcam_toggled = new.camera.enabled != self.cfg.camera.enabled;
        let sound_changed = new.sound != self.cfg.sound;
        let microphone_changed = new.sound.microphone != self.cfg.sound.microphone;
        let outputs_changed = new.sound.keypad_output != self.cfg.sound.keypad_output || new.sound.siren_output != self.cfg.sound.siren_output;
        let op_changed = new.operator != self.cfg.operator;
        let old_phone = NtfyCfg::from(&self.cfg.operator);
        let new_phone = NtfyCfg::from(&new.operator);
        let destination_changed = (old_phone.enabled && !new_phone.enabled) || old_phone.topic_url() != new_phone.topic_url() || old_phone.token != new_phone.token;
        self.cfg = new;
        if model_changed {
            self.save(true);
            if self.home.online {
                if self.ha_bell_active {
                    for entity_id in &self.cfg.home_assistant.siren_outputs {
                        self.hub.set_output(self.hub_epoch, entity_id.clone(), false);
                    }
                }
                for entity_id in self.ha_output_tests.keys() {
                    self.hub.set_output(self.hub_epoch, entity_id.clone(), false);
                }
            }
            self.ha_bell_active = false;
            self.ha_output_tests.clear();
            self.audio.send(AudioCmd::Silence);
            let _ = self.memo.send(crate::audio::memo::Command::Cancel);
            self.zone_pend = [None; MAX_PANEL_ZONES];
            self.panel = load_panel(self.cfg.model, &self.cfg, now);
            // Switching keypads is only permitted while disarmed; clear any stale saved armed
            // state from the other keypad's save file so a previous session never resurrects Armed.
            if self.panel.armed.is_some() || self.panel.exit.is_some() {
                self.panel.armed = None;
                self.panel.exit = None;
                self.panel.entry = None;
                self.panel.alarms.clear();
                self.panel.vista_night = false;
                for z in &mut self.panel.zones {
                    z.bypassed = false;
                    z.auto_bypass = false;
                    z.alarm = false;
                }
                self.save_due = Some(now);
            }
            self.last_buzzer = self.panel.buzzer;
            self.last_power = None;
            self.saved_events = self.panel.events_seq;
            let text = format!("Switched to the {} with its {} keypad", self.panel.model.panel_name(), self.panel.model.keypad_name());
            self.panel.log(EvKind::Info, text);
        }
        if cam_changed {
            self.entry_evidence = None;
            self.grace_until = now + Duration::from_secs(10);
            if webcam_toggled {
                self.cam_online = true;
                self.cam_covered = false;
                if !self.cfg.camera.enabled && !destination_changed {
                    for (_, job, _) in self.pending_photos.drain(..) {
                        self.notify.send(job);
                    }
                }
            }
            self.cam.send(CamCmd::Config(Box::new(self.cfg.camera.clone())));
        }
        if microphone_changed {
            let _ = self.memo.send(crate::audio::memo::Command::Device(self.cfg.sound.microphone.clone()));
            self.mic_test_status.clear();
        }
        if outputs_changed {
            self.audio.configure(self.cfg.sound.keypad_output.clone(), self.cfg.sound.siren_output.clone());
        }
        if sound_changed || model_changed {
            self.push_volumes();
            self.audio.send(AudioCmd::Siren { model: self.cfg.sound.siren_model, warble: self.cfg.sound.wave2_warble });
            self.audio.send(AudioCmd::Voice(voice_for(self.panel.model)));
        }
        if destination_changed {
            self.pending_photos.clear();
            self.entry_evidence = None;
            self.notify_status.clear();
            self.notify_test_status.clear();
        }
        if op_changed {
            self.notify.config(&self.cfg.operator);
            self.cam.send(CamCmd::AlertClipsEnabled(self.cfg.operator.motion_gifs_enabled()));
            if self.cfg.operator.quiet_intrusion {
                self.audio.send(AudioCmd::Ring(false));
            }
        }
        let name = self.cfg.display_name();
        self.panel.labels = self.cfg.labels(self.panel.model).clone();
        self.panel.owner = name.clone();
        self.op.owner = name;
        self.panel.face_disarm = self.cfg.camera.faces && self.cfg.camera.face_disarm;
        self.panel.inputs.cam_enabled = self.cfg.camera.enabled;
        self.panel.inputs.cam_online = self.cam_online;
        self.panel.inputs.cam_covered = self.cam_covered;
        self.panel.inputs.net_watch = self.cfg.operator.phone_alerts;
        self.panel.quiet_intrusion = self.cfg.operator.quiet_intrusion;
        let mask = self.cfg.camera.camera_mask();
        for i in 0..8 {
            if !mask[i] {
                self.cam_motion[i] = false;
                self.cam_contact_open[i] = false;
            }
        }
        self.panel.camera_zone = std::array::from_fn(|i| {
            mask[i] && self.cfg.home_assistant.binding(self.panel.model, (i + 1) as u8).is_none()
        });
        self.panel.dirty = true;
        if hub_changed {
            self.hub_epoch += 1;
            self.home = HomeView {
                learned_exit_away: self.panel.ha_learned_exit_away,
                learned_exit_stay: self.panel.ha_learned_exit_stay,
                learned_entry: self.panel.ha_learned_entry,
                ..HomeView::default()
            };
            self.ha_companions = ha::PanelCompanions::default();
            if self.cfg.home_assistant.enabled {
                self.home.state = ha::ConnectionState::Connecting;
                self.home.status = "Connecting to Home Assistant...".into();
            } else {
                self.home.state = ha::ConnectionState::Disconnected;
                self.home.status = "Disconnected".into();
            }
            self.home_pulses.clear();
            self.hub.configure(&self.cfg.home_assistant, self.hub_epoch);
        }
        if hub_changed || bindings_changed || model_changed {
            self.home_pulses.clear();
            self.sync_home_baseline(now);
        } else if cam_changed {
            self.sync_zones(now);
        }
        if followed_panel_changed || hub_changed || model_changed {
            if followed_entity_changed {
                self.panel.ha_learned_exit_away = None;
                self.panel.ha_learned_exit_stay = None;
                self.panel.ha_learned_entry = None;
                self.home.learned_exit_away = None;
                self.home.learned_exit_stay = None;
                self.home.learned_entry = None;
            }
            if followed_panel_changed {
                self.panel.set_following_panel(false, now);
            }
            self.refresh_esphome_detection();
            self.sync_physical_panel(false, now);
        } else if sync_bypasses_changed {
            self.sync_physical_panel(false, now);
        }
        if sirens_changed {
            self.sync_ha_sirens(now);
        }
    }

    fn dial_info(&self, r: &Report) -> DialInfo {
        let ee = &self.panel.ee;
        let m = self.panel.model;
        let (phone, account) = match m {
            Model::Pc1550 => (ee.raw(m, "01"), ee.raw(m, "02")),
            Model::Pc1832 => (ee.raw(m, "301"), ee.raw(m, "310")),
            Model::Vista20p => (ee.raw(m, "41"), ee.raw(m, "43")),
        };
        let code42 = if m == Model::Pc1550 && r.zone_or_user > 0 && matches!(r.kind, ReportKind::Alarm(_)) {
            ee.entry_digits(m, "05", r.zone_or_user as usize - 1)
        } else {
            format!("{:02}", r.cid % 100)
        };
        DialInfo { phone, account, contact_id: m != Model::Pc1550, code42 }
    }

    fn handle_op(&mut self, outs: Vec<OpOut>) {
        for o in outs {
            match o {
                OpOut::Audio(AudioCmd::Ring(true) | AudioCmd::Line(_)) if self.cfg.operator.quiet_intrusion => {}
                OpOut::Audio(c) => self.audio.send(c),
            }
        }
    }

    fn phone_report(&mut self, report: &Report, now: Instant) {
        let gifs_enabled = self.cfg.operator.motion_gifs_enabled();
        if gifs_enabled && self.cfg.camera.enabled && matches!(report.kind, ReportKind::EntryStarted | ReportKind::PhysicalEntry) {
            self.entry_evidence = Some((report.zone_or_user, now));
            self.cam.send(CamCmd::MarkEntryForClip(now));
        }
        let Some((job, snapshot)) = notify::for_report(report, &self.cfg.operator) else {
            return;
        };
        if snapshot && self.cfg.operator.send_photos && self.cfg.camera.enabled && self.cam_online && !self.cam_covered {
            self.alert_sequence += 1;
            let reason = format!("phone-alert-{}", self.alert_sequence);
            let intrusion = matches!(report.kind, ReportKind::PhysicalAlarm | ReportKind::Alarm(AlarmKind::Burglary));
            let motion_gif = gifs_enabled
                && (intrusion || matches!(report.kind, ReportKind::EntryStarted | ReportKind::PhysicalEntry | ReportKind::Stranger));
            let deadline = now + Duration::from_millis(if motion_gif { 3000 } else { 1200 });
            if motion_gif {
                let event_at = if intrusion {
                    self.entry_evidence.take()
                        .filter(|(zone, at)| *zone == report.zone_or_user && now.saturating_duration_since(*at) < Duration::from_secs(300))
                        .map_or(now, |(_, at)| at)
                } else {
                    now
                };
                self.cam.send(CamCmd::AlertClip { reason: reason.clone(), event_at, deadline });
            } else {
                self.cam.send(CamCmd::AlertSnapshot { reason: reason.clone(), deadline });
            }
            self.pending_photos.push((reason, job, deadline));
        } else {
            self.notify.send(job);
        }
    }

    fn route(&mut self, now: Instant) {
        let outs = std::mem::take(&mut self.panel.out);
        for o in outs {
            match o {
                Out::Audio(c) => self.audio.send(c),
                Out::Memo(cmd) => {
                    let _ = self.memo.send(cmd);
                }
                Out::PhoneReport(r) => self.phone_report(&r, now),
                Out::Report(r) => {
                    self.phone_report(&r, now);
                    let dial = self.dial_info(&r);
                    let res = self.op.report(r, now, dial, &self.cfg.operator);
                    self.handle_op(res);
                }
                Out::Snapshot(reason) if self.cfg.camera.enabled => self.cam.send(CamCmd::Snapshot(reason)),
                Out::Save => self.save_due = Some(now + Duration::from_millis(400)),
                Out::LearnFace if self.cfg.camera.enabled => self.cam.send(CamCmd::LearnOwner),
                Out::Snapshot(_) | Out::LearnFace => {}
                Out::Disarmed { after_alarm } => {
                    self.entry_evidence = None;
                    if self.home.online && (self.ha_bell_active || self.panel.ha_external_siren.is_some()) {
                        self.ha_bell_active = false;
                        self.ha_bell_cmd_until = Some(now + Duration::from_secs(2));
                        self.ha_bell_cmd_target = Some(false);
                        self.ha_output_tests.clear();
                        self.panel.ha_external_siren = None;
                        self.panel.ha_external_siren_label = None;
                        for entity_id in &self.cfg.home_assistant.siren_outputs {
                            if self.home.entities.get(entity_id).map_or(true, ha::Entity::is_writable_output) {
                                self.hub.set_output(self.hub_epoch, entity_id.clone(), false);
                            }
                        }
                    }
                    let res = self.op.disarmed(after_alarm, now, &self.cfg.operator);
                    self.handle_op(res);
                }
                Out::Armed(a) => self.cam.send(CamCmd::Armed(a)),
                Out::HaPanelCmd(cmd) => self.send_ha_panel_cmd(cmd),
                Out::HaBypassCmd(cmd) => self.send_ha_bypass_cmd(cmd),
            }
        }
    }

    fn send_ha_bypass_cmd(&mut self, cmd: panel::HaBypassCmd) {
        if self.cfg.home_assistant.panel_link() != ha::PanelLink::TwoWay
            || !self.cfg.home_assistant.sync_bypasses
            || !self.home.online
        {
            return;
        }
        let Some(followed) = self.cfg.home_assistant.followed_panel().map(str::to_owned) else {
            return;
        };
        // 1) Home Assistant Test Lab companion input_text (two-way bypass testing without hardware).
        if let Some(lab_id) = self.ha_companions.lab_bypasses.clone() {
            let manual: Vec<u8> = (1..=self.panel.n())
                .filter(|&z| self.panel.zones[z - 1].bypassed && !self.panel.zones[z - 1].auto_bypass)
                .map(|z| z as u8)
                .collect();
            self.hub.set_input_text(self.hub_epoch, lab_id, ha::format_bypass_list(&manual));
        }
        // 2) Real ESPHome DSC Keybus / Vista ECP keypress service (auto-discovered via get_services).
        let Some(prefix) = self.home.detected_esphome.clone() else {
            return;
        };
        let part_svc = format!("{prefix}_alarm_keypress_partition");
        let base_svc = format!("{prefix}_alarm_keypress");
        let (svc, part) = if self.home.esphome_services.iter().any(|s| s == &part_svc) {
            (part_svc, Some(ha::detect_partition(&followed)))
        } else if self.home.esphome_services.iter().any(|s| s == &base_svc) {
            (base_svc, None)
        } else {
            return;
        };
        let pick_code = |entered: &Option<String>| -> String {
            let cfg_pin = self.cfg.home_assistant.panel_code.trim();
            if !cfg_pin.is_empty() {
                cfg_pin.to_string()
            } else {
                entered.as_deref().unwrap_or("").trim().to_string()
            }
        };
        let keys = match self.panel.model {
            Model::Pc1832 => {
                let need_code = self.panel.ee.toggle(Model::Pc1832, "015", 5)
                    || self.panel.ee.toggle(Model::Pc1832, "022", 1);
                match &cmd {
                    panel::HaBypassCmd::SetZone { zone, code, .. } => {
                        let c = if need_code { pick_code(code) } else { String::new() };
                        format!("*1{c}{:02}#", zone)
                    }
                    panel::HaBypassCmd::ClearAll { code } => {
                        let c = if need_code { pick_code(code) } else { String::new() };
                        format!("*1{c}00#")
                    }
                    panel::HaBypassCmd::RecallGroup { code_num, code, .. } => {
                        let c = if need_code { pick_code(code) } else { String::new() };
                        format!("*1{c}{:02}#", code_num)
                    }
                    panel::HaBypassCmd::QuickBypass { zones, code } => {
                        let c = if need_code { pick_code(code) } else { String::new() };
                        let zstr: String = zones.iter().map(|z| format!("{:02}", z)).collect();
                        format!("*1{c}{zstr}#")
                    }
                }
            }
            Model::Pc1550 => {
                let need_code = self.panel.ee.toggle(Model::Pc1550, "14", 1);
                match &cmd {
                    panel::HaBypassCmd::SetZone { zone, code, .. } if (1..=6).contains(zone) => {
                        let c = if need_code { pick_code(code) } else { String::new() };
                        format!("*1{c}{zone}#")
                    }
                    panel::HaBypassCmd::RecallGroup { code, .. } => {
                        let c = if need_code { pick_code(code) } else { String::new() };
                        format!("*1{c}9#")
                    }
                    _ => String::new(),
                }
            }
            Model::Vista20p => match &cmd {
                panel::HaBypassCmd::SetZone { zone, bypassed: true, code } => {
                    let c = pick_code(code);
                    if c.len() == 4 { format!("{c}6{:02}", zone) } else { String::new() }
                }
                panel::HaBypassCmd::QuickBypass { code, .. } => {
                    let c = pick_code(code);
                    if c.len() == 4 { format!("{c}6#") } else { String::new() }
                }
                panel::HaBypassCmd::ClearAll { code } => {
                    let c = pick_code(code);
                    if c.len() == 4 { format!("{c}600") } else { String::new() }
                }
                _ => String::new(),
            },
        };
        if !keys.is_empty() {
            self.hub.call_esphome_keypress(self.hub_epoch, svc, keys, part);
        }
    }

    fn send_ha_panel_cmd(&mut self, cmd: panel::HaPanelCmd) {
        self.send_ha_panel_cmd_with_override(cmd, None);
    }

    fn send_ha_panel_cmd_with_override(&mut self, cmd: panel::HaPanelCmd, code_override: Option<String>) {
        if self.cfg.home_assistant.panel_link() != ha::PanelLink::TwoWay || !self.home.online {
            return;
        }
        let Some(entity_id) = self.cfg.home_assistant.followed_panel().map(str::to_owned) else {
            return;
        };
        let entity = self.home.entities.get(&entity_id);
        if matches!(cmd, panel::HaPanelCmd::Trigger) {
            // Only forward alarm_trigger to HA panels that explicitly advertise TRIGGER (e.g. Lab/Manual/Alarmo).
            // Real ESPHome house panels do not advertise TRIGGER, keeping monitored house alarms safe.
            if entity.is_some_and(ha::Entity::supports_trigger) {
                self.hub.call_alarm(self.hub_epoch, entity_id, ha::AlarmAction::Trigger, None);
            }
            return;
        }
        let configured_code = self.cfg.home_assistant.panel_code.trim();
        let (action, entered_code, esphome_state) = match &cmd {
            panel::HaPanelCmd::Arm { mode, night, no_entry, code } => {
                let act = if *night {
                    if entity.map_or(true, ha::Entity::supports_night) {
                        ha::AlarmAction::ArmNight
                    } else {
                        ha::AlarmAction::ArmHome
                    }
                } else {
                    match mode {
                        ArmMode::Stay => ha::AlarmAction::ArmHome,
                        ArmMode::Away => ha::AlarmAction::ArmAway,
                    }
                };
                let esp = if *night {
                    "N"
                } else if *no_entry && *mode == ArmMode::Stay && self.panel.model == Model::Vista20p {
                    "I"
                } else if *mode == ArmMode::Stay {
                    "S"
                } else {
                    "A"
                };
                (act, code.clone(), esp)
            }
            panel::HaPanelCmd::Disarm { code } => (ha::AlarmAction::Disarm, code.clone(), "D"),
            panel::HaPanelCmd::Trigger => return,
        };
        // Pick the code to send: override wins (retry path); next explicit HA panel PIN;
        // otherwise forward the code typed on RoomGuard's keypad directly to HA / ESPHome
        // so the user never has to store their real alarm PIN in plaintext config or YAML.
        let is_alarm_entity = entity.is_some_and(ha::Entity::supports_alarm_services);
        let forward_typed = matches!(action, ha::AlarmAction::Disarm)
            || !is_alarm_entity
            || entity.map_or(true, |e| e.needs_code() || e.code_arm_required());
        let typed = entered_code.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());
        let code = if let Some(ov) = code_override {
            self.ha_retry_cmd = None;
            Some(ov)
        } else if !configured_code.is_empty() {
            if let Some(ref kp_code) = typed {
                if kp_code != configured_code {
                    self.ha_retry_cmd = Some((cmd.clone(), kp_code.clone(), Instant::now()));
                } else {
                    self.ha_retry_cmd = None;
                }
            } else {
                self.ha_retry_cmd = None;
            }
            Some(configured_code.to_owned())
        } else if forward_typed {
            self.ha_retry_cmd = None;
            typed
        } else {
            self.ha_retry_cmd = None;
            None
        };
        if is_alarm_entity {
            self.hub.call_alarm(self.hub_epoch, entity_id, action, code);
        } else if let Some(prefix) = self.home.detected_esphome.clone() {
            // Direct ESPHome DSC/Vista control even when linked straight to sensor.*_status_partition_1!
            let set_svc = format!("{prefix}_set_alarm_state");
            if self.home.esphome_services.iter().any(|s| s == &set_svc) {
                let part = ha::detect_partition(&entity_id);
                self.hub.call_esphome_alarm_state(
                    self.hub_epoch,
                    set_svc,
                    esphome_state.to_string(),
                    code.unwrap_or_default(),
                    part,
                    action,
                );
            }
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let wall = util::now_ms();
        let pulses = self.home_pulses.len();
        self.home_pulses.retain(|_, until| now < *until);
        if self.home_pulses.len() != pulses {
            self.sync_zones(now);
        }
        // When an outbound Two-Way HA command window expires, verify HA's real state
        // immediately so RoomGuard and Home Assistant never stay out of sync (e.g. wrong PIN).
        if self.panel.ha_outbound_until.is_some_and(|until| now >= until) {
            let expected = self.panel.ha_expect.take();
            self.panel.ha_outbound_until = None;
            let mut mismatch = false;
            if let Some(expect) = expected {
                if let Some(id) = self.cfg.home_assistant.followed_panel() {
                    if let Some(st) = self.home.entities.get(id).and_then(|e| ha::AlarmState::parse(&e.state)) {
                        if !expect.matches(st) {
                            mismatch = true;
                            let msg = if expect == panel::HaExpect::Disarmed {
                                format!("Home Assistant panel is still {} (check your HA panel PIN)", st.label())
                            } else {
                                format!("Home Assistant panel stayed {} (check your HA panel PIN/settings)", st.label())
                            };
                            self.home.service_notice = Some((false, msg.clone()));
                            self.panel.log(EvKind::Trouble, msg);
                        }
                    }
                }
            }
            // Re-sync with live=false on mismatch so we snap to HA's actual state without looping Disarm.
            self.sync_physical_panel(!mismatch, now);
        }
        self.pending_photos.retain(|(_, job, deadline)| {
            if now < *deadline {
                true
            } else {
                self.notify.send(job.clone());
                false
            }
        });
        // Reading the cached battery state is instant, so it can be checked often.
        if self.last_power.map_or(true, |t| now.saturating_duration_since(t).as_millis() >= 1000) {
            self.last_power = Some(now);
            let st = self.power.read();
            self.panel.inputs.ac_ok = st.on_ac;
            self.panel.inputs.battery_low = st.has_battery && !st.on_ac && st.percent < 0.20;
        }
        // A sound output that reconnects within a few seconds (Windows does
        // that when a device wakes up or changes) isn't a bell trouble.
        if self.audio.is_ok() {
            self.audio_bad_since = None;
        } else if self.audio_bad_since.is_none() {
            self.audio_bad_since = Some(now);
        }
        let audio_gone = self.audio_bad_since.map_or(false, |t| now.saturating_duration_since(t).as_secs() >= 4);
        self.panel.inputs.audio_ok = !audio_gone || now < self.grace_until;
        if now.saturating_duration_since(self.last_cam_check).as_millis() >= 1000 {
            self.last_cam_check = now;
            self.check_camera(now);
        }
        // Supervision windows run out without any new Home Assistant message.
        if self.home_lost_at.iter().any(Option::is_some)
            && self.home_lost_check.map_or(true, |at| now.saturating_duration_since(at).as_millis() >= 1000)
        {
            self.home_lost_check = Some(now);
            self.sync_zones(now);
        }
        self.settle_zones(now);
        // Two-way Home Assistant siren/bell sync:
        // 1) HA -> RoomGuard: if any linked HA siren/bell/alarm entity (including ESPHome al_1/alm_1/fa_1/fire_1
        //    or Lab siren toggle) is turned ON from Home Assistant, sound RoomGuard's PC siren live!
        self.sync_ha_sirens(now);
        self.panel.tick(now, wall);
        self.publish_ha_status(now);
        // 2) RoomGuard -> HA: drive linked writable Home Assistant sirens/switches/lights automatically
        //    whenever RoomGuard's OWN panel bell/siren sounds (and turn them OFF on disarm/silence).
        let local_bell = self.panel.local_bell_active(now);
        if local_bell != self.ha_bell_active {
            self.ha_bell_active = local_bell;
            self.ha_bell_cmd_until = Some(now + Duration::from_secs(2));
            self.ha_bell_cmd_target = Some(local_bell);
            if self.home.online {
                for entity_id in &self.cfg.home_assistant.siren_outputs {
                    let writable = self
                        .home
                        .entities
                        .get(entity_id)
                        .map_or(true, ha::Entity::is_writable_output);
                    if writable {
                        self.hub.set_output(self.hub_epoch, entity_id.clone(), local_bell);
                    }
                }
            }
            if !local_bell {
                self.ha_output_tests.clear();
                self.panel.ha_external_siren = None;
                self.panel.ha_external_siren_label = None;
            }
        }
        // Auto-turn-off timed HA siren test bursts (e.g. "Test (3s)").
        if !self.ha_output_tests.is_empty() {
            let expired: Vec<String> = self
                .ha_output_tests
                .iter()
                .filter(|(_, until)| now >= **until)
                .map(|(id, _)| id.clone())
                .collect();
            for id in expired {
                self.ha_output_tests.remove(&id);
                if self.home.online && !self.ha_bell_active {
                    let writable = self
                        .home
                        .entities
                        .get(&id)
                        .map_or(true, ha::Entity::is_writable_output);
                    if writable {
                        self.hub.set_output(self.hub_epoch, id, false);
                    }
                }
            }
            if self.ha_output_tests.is_empty() {
                self.ha_bell_cmd_until = Some(now + Duration::from_secs(2));
                self.ha_bell_cmd_target = Some(false);
                self.panel.ha_external_siren = None;
                self.panel.ha_external_siren_label = None;
                self.panel.dirty = true;
            }
        }
        if self.panel.buzzer != self.last_buzzer {
            self.last_buzzer = self.panel.buzzer;
            self.push_volumes();
        }
        self.route(now);
        let want_awake = self.cfg.keep_awake && self.panel.is_on();
        self.awake.set(want_awake);
        let ops = self.op.tick(now, &self.cfg.operator, self.cfg.sound.line_audio);
        self.handle_op(ops);
        if self.panel.events_seq != self.saved_events && self.save_due.is_none() {
            self.save_due = Some(now + Duration::from_secs(2));
        }
        if self.save_due.map_or(false, |t| now >= t) {
            self.save(false);
        }
        let gap = if self.panel.dirty { 33 } else { 250 };
        if self.last_publish.map_or(true, |t| now.saturating_duration_since(t).as_millis() >= gap) {
            self.publish(now, wall);
        }
    }

    fn save(&mut self, clean: bool) {
        self.save_due = None;
        self.saved_events = self.panel.events_seq;
        let s = self.panel.to_save(clean);
        util::save_json(&panel_path(self.panel.model), &s);
        self.op.save();
    }

    fn shutdown(&mut self) {
        for (_, job, _) in self.pending_photos.drain(..) {
            self.notify.send(job);
        }
        let armed = self.panel.is_on();
        if armed {
            self.panel.log(EvKind::Info, "RoomGuard was closed while armed.");
            let cfg = NtfyCfg::from(&self.cfg.operator);
            if cfg.enabled {
                let job = NotifyJob {
                    title: "RoomGuard closed while armed".to_string(),
                    message: format!(
                        "The app was closed at {} while the system was armed.",
                        util::fmt_clock(util::now_ms())
                    ),
                    priority: 4,
                    tags: "warning".to_string(),
                    photo: None,
                    test: false,
                };
                if let Err(e) = notify::send_now_timeout(&cfg, &job, Duration::from_secs(3)) {
                    util::log(&format!("could not send the shutdown alert: {}", e));
                }
            }
            if self.cfg.operator.enabled {
                let name = self.cfg.display_name();
                self.op.note_shutdown(&name);
            }
        }
        if self.home.online {
            if self.ha_bell_active {
                for entity_id in &self.cfg.home_assistant.siren_outputs {
                    self.hub.set_output(self.hub_epoch, entity_id.clone(), false);
                }
            }
            for entity_id in self.ha_output_tests.keys() {
                self.hub.set_output(self.hub_epoch, entity_id.clone(), false);
            }
        }
        self.audio.send(AudioCmd::Silence);
        self.awake.set(false);
        self.save(true);
        self.cam.send(CamCmd::Shutdown);
        util::log("engine stopped cleanly");
    }

    fn publish(&mut self, now: Instant, wall: i64) {
        self.last_publish = Some(now);
        self.panel.dirty = false;
        self.seq += 1;
        if self.events_cache.0 != self.panel.events_seq {
            self.events_cache = (self.panel.events_seq, Arc::new(self.panel.events.clone()));
        }
        if self.chat_cache.0 != self.op.seq {
            self.chat_cache = (self.op.seq, Arc::new(self.op.chat.clone()));
        }
        let fp = prog_fingerprint(&self.panel);
        if self.prog_cache.0 != fp {
            self.prog_cache = (fp, Arc::new(prog_view(&self.panel)));
        }
        let p = &self.panel;
        let (status, title, detail) = p.status(now);
        let detail = if p.physical_unavailable_since.is_some() {
            "Physical panel unavailable · Showing last known state".into()
        } else {
            detail
        };
        let (lcd, lcd_cursor) = match p.model {
            Model::Pc1832 => {
                let l = p.lcd_1832(now, wall);
                (l.lines, l.cursor)
            }
            Model::Vista20p => {
                let l = p.lcd_vista(now, wall);
                (l.lines, l.cursor)
            }
            Model::Pc1550 => ([String::new(), String::new()], None),
        };
        let lcd_leds = match p.model {
            Model::Pc1832 => p.leds_5500(now),
            Model::Vista20p => p.leds_vista(),
            Model::Pc1550 => [Led::Off; 4],
        };
        let backlight = match p.model {
            Model::Pc1832 if !p.ee.toggle(Model::Pc1832, "016", 5) || p.is_blanked_1832(now) => 0.0,
            Model::Vista20p => {
                if p.vista_backlight(now) { 1.0 } else { 0.30 }
            }
            _ => p.brightness as f32 / 9.0,
        };
        let view = View {
            seq: self.seq,
            model: p.model,
            leds: if p.model == Model::Pc1550 { p.leds_1550(now) } else { [Led::Off; 11] },
            lcd,
            lcd_cursor,
            lcd_leds,
            backlight,
            contrast: p.contrast as f32 / 9.0,
            all_on: p.all_on(now),
            status,
            status_title: title,
            status_detail: detail,
            zones: p.zone_views().into_iter().map(|mut zone| {
                if self.cfg.home_assistant.binding(p.model, zone.num).is_some() && zone.fault {
                    zone.note = "Home Assistant sensor unavailable".into();
                }
                zone
            }).collect(),
            bell: p.bell_sent != BellPattern::Off,
            ha_external_siren: p.ha_external_siren.is_some(),
            siren: self.cfg.sound.siren_model,
            hold: p.hold_progress(now),
            troubles: p.trouble_list(),
            events: self.events_cache.1.clone(),
            chat: self.chat_cache.1.clone(),
            call: self.op.call_view(),
            armed: p.is_on(),
            following_panel: p.following_panel,
            panel_link: p.panel_link,
            physical_unavailable: p.physical_unavailable_since.is_some(),
            programming: p.is_programming(),
            notify_status: self.notify_status.clone(),
            mic_test_status: self.mic_test_status.clone(),
            mic_test_running: self.mic_test_running,
            notify_test_status: self.notify_test_status.clone(),
            sim: self.sim,
            home: self.home.clone(),
            home_zones: std::array::from_fn(|index| self.cfg.home_assistant.binding(p.model, (index + 1) as u8).is_some()),
            hint: if p.following_panel { "Arm or disarm at the physical keypad".into() } else { p.hint() },
            prog: self.prog_cache.1.clone(),
        };
        if let Ok(mut v) = self.view.lock() {
            *v = Arc::new(view);
        }
        self.ctx.request_repaint();
    }
}
