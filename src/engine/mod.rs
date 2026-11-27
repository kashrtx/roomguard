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
    TestNotify,
    ClearEvents,
    HomeAssistant { epoch: u64, update: ha::Update },
    HomePreview(String),
    Shutdown,
}

const MAX_PANEL_ZONES: usize = Model::Vista20p.zone_count();

#[derive(Clone, Default)]
pub struct HomeView {
    pub status: String,
    pub online: bool,
    pub entities: Arc<ha::Entities>,
    pub preview: Option<(String, Result<Arc<crate::vision::frame::Rgb>, String>)>,
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
    pub siren: SirenModel,
    pub hold: Option<(Key, f32)>,
    pub troubles: Vec<String>,
    pub events: Arc<Vec<EventRow>>,
    pub chat: Arc<Vec<ChatMsg>>,
    pub call: Option<CallView>,
    pub armed: bool,
    pub following_panel: bool,
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
            siren: SirenModel::Wave2F,
            hold: None,
            troubles: Vec::new(),
            events: Arc::new(Vec::new()),
            chat: Arc::new(Vec::new()),
            call: None,
            armed: false,
            following_panel: false,
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
    Panel::new(model, save, cfg.labels(model).clone(), cfg.display_name(), cfg.home_assistant.followed_panel().is_some(), now)
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
    hub_epoch: u64,
    home_pulses: std::collections::BTreeMap<String, Instant>,
}

impl Engine {
    fn new(cfg: AppConfig, audio: AudioHandle, cam: CameraHandle, notify: NotifyHandle, ctx: egui::Context, view: Arc<Mutex<Arc<View>>>, memo: Sender<crate::audio::memo::Command>, hub: ha::Handle) -> Engine {
        let now = Instant::now();
        let panel = load_panel(cfg.model, &cfg, now);
        let op = Operator::load(cfg.display_name());
        let last_buzzer = panel.buzzer;
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
            home: HomeView::default(),
            hub_epoch: 0,
            home_pulses: Default::default(),
        };
        e.saved_events = e.panel.events_seq;
        e.push_settings();
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
                    self.panel.ee.set_toggles("015", &t);
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
                if busy || !(1..=8).contains(&opt) {
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

    fn vision(&mut self, ev: VisionEvent, now: Instant) {
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
            ha::Update::Status(status) => {
                self.home.online = false;
                self.home.status = status;
                self.home.preview = None;
                self.home_pulses.clear();
                self.sync_physical_panel(false, now);
            }
            ha::Update::Snapshot(entities) => {
                self.home.entities = Arc::new(entities);
                self.home.online = true;
                self.home.status = "Connected".into();
                self.home_pulses.clear();
                self.sync_home_baseline(now);
                // A changed state on reconnect is new to this running session.
                self.sync_physical_panel(self.panel.physical_state.is_some(), now);
                self.panel.dirty = true;
                return;
            }
            ha::Update::State { id, entity } => {
                if !self.home.online {
                    return;
                }
                let changed = self.home.entities.get(&id).zip(entity.as_ref()).is_some_and(|(old, new)| {
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
                let follow_changed = self.cfg.home_assistant.followed_panel() == Some(id.as_str());
                let entities = Arc::make_mut(&mut self.home.entities);
                match entity {
                    Some(entity) => { entities.insert(id, entity); }
                    None => { entities.remove(&id); }
                }
                if follow_changed {
                    self.sync_physical_panel(true, now);
                }
            }
        }
        self.sync_zones(now);
        self.panel.dirty = true;
    }

    fn sync_physical_panel(&mut self, live: bool, now: Instant) {
        let id = self.cfg.home_assistant.followed_panel();
        self.panel.set_following_panel(id.is_some(), now);
        let state = id.filter(|_| self.home.online)
            .and_then(|id| self.home.entities.get(id))
            .and_then(|entity| ha::AlarmState::parse(&entity.state));
        self.panel.sync_physical_panel(state, live, now);
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
        self.panel.home_sensor_unavailable = false;
        for i in 0..n {
            self.zone_want[i] = if let Some(binding) = self.cfg.home_assistant.binding(self.panel.model, (i + 1) as u8) {
                let active = self.home_active(binding);
                let available = active.is_some();
                let zone = &mut self.panel.zones[i];
                // Transport failure is a supervision fault, never a motion edge.
                zone.fault = !available;
                zone.tamper = false;
                if let Some(camera) = self.panel.camera_zone.get_mut(i) {
                    *camera = false;
                }
                self.panel.home_sensor_unavailable |= !available;
                (active.unwrap_or(zone.open), false, !available)
            } else if !mask.get(i).copied().unwrap_or(false) {
                (self.sim[i], false, false)
            } else {
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

    fn apply_config(&mut self, mut new: AppConfig, now: Instant) {
        let busy = self.panel.is_on();
        if new.model != self.panel.model && busy {
            new.model = self.panel.model;
        }
        if busy {
            new.home_assistant = self.cfg.home_assistant.clone();
        }
        let hub_changed = new.home_assistant.enabled != self.cfg.home_assistant.enabled
            || new.home_assistant.url != self.cfg.home_assistant.url
            || new.home_assistant.token != self.cfg.home_assistant.token;
        let bindings_changed = new.home_assistant.bindings != self.cfg.home_assistant.bindings;
        let followed_panel_changed = new.home_assistant.followed_panel() != self.cfg.home_assistant.followed_panel();
        let model_changed = new.model != self.panel.model;
        let cam_changed = new.camera != self.cfg.camera;
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
            self.audio.send(AudioCmd::Silence);
            let _ = self.memo.send(crate::audio::memo::Command::Cancel);
            self.zone_pend = [None; MAX_PANEL_ZONES];
            self.panel = load_panel(self.cfg.model, &self.cfg, now);
            self.last_buzzer = self.panel.buzzer;
            self.last_power = None;
            self.saved_events = self.panel.events_seq;
            let text = format!("Switched to the {} with its {} keypad", self.panel.model.panel_name(), self.panel.model.keypad_name());
            self.panel.log(EvKind::Info, text);
        }
        if cam_changed {
            self.entry_evidence = None;
            self.grace_until = now + Duration::from_secs(10);
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
            self.home = HomeView::default();
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
            if followed_panel_changed {
                self.panel.set_following_panel(false, now);
            }
            self.sync_physical_panel(false, now);
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
        if gifs_enabled && matches!(report.kind, ReportKind::EntryStarted | ReportKind::PhysicalEntry) {
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
                Out::Snapshot(reason) => self.cam.send(CamCmd::Snapshot(reason)),
                Out::Save => self.save_due = Some(now + Duration::from_millis(400)),
                Out::LearnFace => self.cam.send(CamCmd::LearnOwner),
                Out::Disarmed { after_alarm } => {
                    self.entry_evidence = None;
                    let res = self.op.disarmed(after_alarm, now, &self.cfg.operator);
                    self.handle_op(res);
                }
                Out::Armed(a) => self.cam.send(CamCmd::Armed(a)),
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
        self.settle_zones(now);
        self.panel.tick(now, wall);
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
            siren: self.cfg.sound.siren_model,
            hold: p.hold_progress(now),
            troubles: p.trouble_list(),
            events: self.events_cache.1.clone(),
            chat: self.chat_cache.1.clone(),
            call: self.op.call_view(),
            armed: p.is_on(),
            following_panel: p.following_panel,
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
