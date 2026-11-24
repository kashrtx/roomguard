//! The engine thread. It owns the panel and the operator, runs them on a
//! steady clock whether or not the window is visible, routes their output to
//! sound, the camera and your phone, turns the computer's own condition into
//! panel troubles, and publishes a snapshot of everything for the interface.

pub mod eeprom;
mod kp1550;
mod kp1832;
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

pub enum OpInput {
    Answer,
    Decline,
    Passcode(String),
    Dispatch(bool),
    Clear,
}

pub enum EngineMsg {
    Key { key: Key, down: bool },
    SimZone { zone: u8, open: bool },
    Vision(VisionEvent),
    Config(Box<AppConfig>),
    Op(OpInput),
    NotifyResult { ok: bool, test: bool, err: String },
    NetStatus(bool),
    SetMasterCode(String),
    /// Checks a code without pressing keys, used before closing while armed.
    VerifyCode { code: String, reply: std::sync::mpsc::Sender<bool> },
    TestNotify,
    ClearEvents,
    Shutdown,
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
    pub programming: bool,
    pub notify_status: String,
    pub sim: [bool; 8],
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
            programming: false,
            notify_status: String::new(),
            sim: [false; 8],
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
    let spawned = std::thread::Builder::new().name("engine".into()).spawn(move || {
        let mut e = Engine::new(cfg, audio, cam, notify, ctx, v2);
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
    }
}

fn voice_for(model: Model) -> KeypadVoice {
    match model {
        Model::Pc1550 => KeypadVoice::Classic,
        Model::Pc1832 => KeypadVoice::Pk5500,
    }
}

fn load_panel(model: Model, cfg: &AppConfig, now: Instant) -> Panel {
    let save = util::load_json::<PanelSave>(&panel_path(model)).unwrap_or_else(|| PanelSave::fresh(model));
    Panel::new(model, save, cfg.labels(model).clone(), cfg.display_name(), now)
}

struct Engine {
    cfg: AppConfig,
    panel: Panel,
    op: Operator,
    audio: AudioHandle,
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
    cam_motion: [bool; 2],
    door_open: bool,
    cam_online: bool,
    cam_covered: bool,
    sim: [bool; 8],
    events_cache: (u64, Arc<Vec<EventRow>>),
    chat_cache: (u64, Arc<Vec<ChatMsg>>),
    notify_status: String,
    last_photo: Option<PathBuf>,
    last_face_log: Option<Instant>,
    last_stranger: Option<Instant>,
    last_buzzer: u8,
    awake: crate::power::KeepAwake,
}

impl Engine {
    fn new(cfg: AppConfig, audio: AudioHandle, cam: CameraHandle, notify: NotifyHandle, ctx: egui::Context, view: Arc<Mutex<Arc<View>>>) -> Engine {
        let now = Instant::now();
        let panel = load_panel(cfg.model, &cfg, now);
        let op = Operator::load(cfg.display_name());
        let last_buzzer = panel.buzzer;
        let mut e = Engine {
            cfg,
            panel,
            op,
            audio,
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
            cam_motion: [false; 2],
            door_open: false,
            cam_online: true,
            cam_covered: false,
            sim: [false; 8],
            events_cache: (0, Arc::new(Vec::new())),
            chat_cache: (0, Arc::new(Vec::new())),
            notify_status: String::new(),
            last_photo: None,
            last_face_log: None,
            last_stranger: None,
            last_buzzer,
            awake: crate::power::KeepAwake::new(),
        };
        e.saved_events = e.panel.events_seq;
        e.push_settings();
        let armed = e.panel.armed.is_some();
        e.cam.send(CamCmd::Armed(armed));
        e.sync_zones(now);
        e
    }

    fn push_volumes(&mut self) {
        let s = &self.cfg.sound;
        let buzzer = if self.panel.model == Model::Pc1832 { self.panel.buzzer as f32 / 20.0 } else { 1.0 };
        self.audio.send(AudioCmd::Volumes {
            master: s.master,
            siren: s.siren,
            keypad: s.keypad * buzzer,
            line: if s.line_audio { 0.35 } else { 0.0 },
        });
    }

    fn push_settings(&mut self) {
        self.push_volumes();
        self.audio.send(AudioCmd::Siren { model: self.cfg.sound.siren_model, warble: self.cfg.sound.wave2_warble });
        self.audio.send(AudioCmd::Voice(voice_for(self.panel.model)));
        self.cam.send(CamCmd::Config(Box::new(self.cfg.camera.clone())));
        self.notify.config(&self.cfg.operator);
        let name = self.cfg.display_name();
        self.panel.labels = self.cfg.labels(self.panel.model).clone();
        self.panel.owner = name.clone();
        self.op.owner = name;
        self.panel.face_disarm = self.cfg.camera.faces && self.cfg.camera.face_disarm;
        self.panel.inputs.cam_enabled = self.cfg.camera.enabled;
        self.panel.inputs.cam_online = self.cam_online;
        self.panel.inputs.cam_covered = self.cam_covered;
        self.panel.inputs.net_watch = self.cfg.operator.phone_alerts;
        let cam = self.cfg.camera.enabled;
        self.panel.camera_zone = [cam, cam && self.cfg.camera.doorway.is_some(), false, false, false, false, false, false];
        self.panel.dirty = true;
    }

    fn run(&mut self, rx: Receiver<EngineMsg>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(30)) {
                Ok(msg) => {
                    if !self.handle(msg) {
                        self.shutdown();
                        return;
                    }
                    while let Ok(m) = rx.try_recv() {
                        if !self.handle(m) {
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
            self.tick();
        }
    }

    fn handle(&mut self, msg: EngineMsg) -> bool {
        let now = Instant::now();
        match msg {
            EngineMsg::Key { key, down } => self.panel.key(key, down, now),
            EngineMsg::SimZone { zone, open } => {
                let z = zone as usize;
                if (1..=8).contains(&z) {
                    self.sim[z - 1] = open;
                    self.sync_zones(now);
                }
            }
            EngineMsg::Vision(ev) => self.vision(ev, now),
            EngineMsg::Config(c) => self.apply_config(*c, now),
            EngineMsg::Op(inp) => {
                let outs = self.op.input(inp, now, &self.cfg.operator);
                self.handle_op(outs);
                self.save_due = Some(now + Duration::from_secs(1));
            }
            EngineMsg::NotifyResult { ok, test, err } => {
                if ok {
                    self.panel.inputs.ftc = false;
                    self.notify_status = if test { "Test sent. Check your phone." } else { "Last alert delivered" }.to_string();
                } else {
                    self.notify_status = format!("Couldn't send: {}", err);
                    if !test {
                        self.panel.inputs.ftc = true;
                    }
                }
                self.panel.dirty = true;
            }
            EngineMsg::NetStatus(ok) => self.panel.inputs.net_ok = ok,
            EngineMsg::SetMasterCode(code) => {
                let model = self.panel.model;
                self.panel.ee.set_code(master_slot(model), &code);
                self.panel.log(EvKind::Program, "Master code set during setup");
                let other = match model {
                    Model::Pc1550 => Model::Pc1832,
                    Model::Pc1832 => Model::Pc1550,
                };
                let mut save = util::load_json::<PanelSave>(&panel_path(other)).unwrap_or_else(|| PanelSave::fresh(other));
                save.ee.set_code(master_slot(other), &code);
                util::save_json(&panel_path(other), &save);
                self.save(false);
            }
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
                    message: format!("Phone alerts are working, {}. This is what an alarm alert will look like.", self.cfg.display_name()),
                    priority: 3,
                    tags: "white_check_mark".to_string(),
                    photo: self.last_photo.clone(),
                    test: true,
                });
                self.notify_status = "Sending a test...".to_string();
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

    fn vision(&mut self, ev: VisionEvent, now: Instant) {
        match ev {
            VisionEvent::Motion { zone, open } => {
                if (1..=2).contains(&zone) {
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
            VisionEvent::DoorOpen(open) => {
                self.door_open = open;
                self.panel.log(EvKind::Camera, if open { "Door opened (seen by the camera)" } else { "Door closed (seen by the camera)" });
                self.sync_zones(now);
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
            VisionEvent::Snapshot { path, reason: _ } => {
                self.op.photo(path.clone(), now);
                self.last_photo = Some(path);
            }
        }
    }

    /// Camera conditions become zone states. The PowerSeries knows zone
    /// tamper and fault; the PC1550 doesn't, so on it a covered or dead camera
    /// simply shows as the zone being open, the same as a cut wire.
    fn sync_zones(&mut self, now: Instant) {
        let enabled = self.cfg.camera.enabled;
        let has_door = self.cfg.camera.doorway.is_some();
        for i in 0..2usize {
            let z = i + 1;
            let wired = enabled && (i == 0 || has_door);
            let (open, tamper, fault) = if !wired {
                (self.sim[i], false, false)
            } else {
                let motion = self.cam_motion[i] || (i == 0 && self.door_open);
                match self.panel.model {
                    Model::Pc1832 => (motion, self.cam_covered, !self.cam_online),
                    Model::Pc1550 => (motion || self.cam_covered || !self.cam_online, false, false),
                }
            };
            let cur = self.panel.zones[i];
            if cur.open != open || cur.tamper != tamper || cur.fault != fault {
                self.panel.set_zone(z, open, tamper, fault, now);
            }
        }
        for z in 3..=self.panel.n() {
            let open = self.sim[z - 1];
            if self.panel.zones[z - 1].open != open {
                self.panel.set_zone(z, open, false, false, now);
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
                self.cam_motion = [false; 2];
            }
            self.sync_zones(now);
        }
    }

    fn apply_config(&mut self, mut new: AppConfig, now: Instant) {
        let busy = self.panel.armed.is_some() || self.panel.exit.is_some();
        if new.model != self.panel.model && busy {
            new.model = self.panel.model;
        }
        let model_changed = new.model != self.panel.model;
        let cam_changed = new.camera != self.cfg.camera;
        self.cfg = new;
        if model_changed {
            self.save(true);
            self.panel = load_panel(self.cfg.model, &self.cfg, now);
            self.last_buzzer = self.panel.buzzer;
            self.last_power = None;
            self.saved_events = self.panel.events_seq;
            let text = format!("Switched to the {} with its {} keypad", self.panel.model.panel_name(), self.panel.model.keypad_name());
            self.panel.log(EvKind::Info, text);
        }
        if cam_changed {
            self.grace_until = now + Duration::from_secs(10);
        }
        self.push_settings();
        self.sync_zones(now);
    }

    fn dial_info(&self, r: &Report) -> DialInfo {
        let ee = &self.panel.ee;
        let m = self.panel.model;
        let (phone, account) = match m {
            Model::Pc1550 => (ee.raw(m, "01"), ee.raw(m, "02")),
            Model::Pc1832 => (ee.raw(m, "301"), ee.raw(m, "310")),
        };
        let code42 = if m == Model::Pc1550 && r.zone_or_user > 0 && matches!(r.kind, ReportKind::Alarm(_)) {
            ee.entry_digits(m, "05", r.zone_or_user as usize - 1)
        } else {
            format!("{:02}", r.cid % 100)
        };
        DialInfo { phone, account, contact_id: m == Model::Pc1832, code42 }
    }

    fn handle_op(&mut self, outs: Vec<OpOut>) {
        for o in outs {
            match o {
                OpOut::Notify(job) => self.notify.send(job),
                OpOut::Audio(c) => self.audio.send(c),
            }
        }
    }

    fn route(&mut self, now: Instant) {
        let outs = std::mem::take(&mut self.panel.out);
        for o in outs {
            match o {
                Out::Audio(c) => self.audio.send(c),
                Out::Report(r) => {
                    let dial = self.dial_info(&r);
                    let res = self.op.report(r, now, dial, &self.cfg.operator);
                    self.handle_op(res);
                }
                Out::Snapshot(reason) => self.cam.send(CamCmd::Snapshot(reason)),
                Out::Save => self.save_due = Some(now + Duration::from_millis(400)),
                Out::LearnFace => self.cam.send(CamCmd::LearnOwner),
                Out::Disarmed { after_alarm } => {
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
        if self.last_power.map_or(true, |t| now.saturating_duration_since(t).as_secs() >= 5) {
            self.last_power = Some(now);
            let st = self.power.read();
            self.panel.inputs.ac_ok = st.on_ac;
            self.panel.inputs.battery_low = st.has_battery && !st.on_ac && st.percent < 0.20;
        }
        self.panel.inputs.audio_ok = self.audio.is_ok() || now < self.grace_until;
        if now.saturating_duration_since(self.last_cam_check).as_millis() >= 1000 {
            self.last_cam_check = now;
            self.check_camera(now);
        }
        self.panel.tick(now, wall);
        if self.panel.buzzer != self.last_buzzer {
            self.last_buzzer = self.panel.buzzer;
            self.push_volumes();
        }
        self.route(now);
        let want_awake = self.cfg.keep_awake && (self.panel.armed.is_some() || self.panel.exit.is_some());
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
        let armed = self.panel.armed.is_some() || self.panel.exit.is_some();
        if armed {
            self.panel.log(EvKind::Info, "RoomGuard was closed while armed. It will come back armed when it opens again.");
            let cfg = NtfyCfg::from(&self.cfg.operator);
            if cfg.enabled {
                let job = NotifyJob {
                    title: "RoomGuard closed while armed".to_string(),
                    message: format!(
                        "The app was closed at {} while the system was armed. It will re-arm by itself when it opens again.",
                        util::fmt_clock(util::now_ms())
                    ),
                    priority: 4,
                    tags: "warning".to_string(),
                    photo: None,
                    test: false,
                };
                if let Err(e) = notify::send_now(&cfg, &job) {
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
        let p = &self.panel;
        let (status, title, detail) = p.status(now);
        let (lcd, lcd_cursor) = if p.model == Model::Pc1832 {
            let l = p.lcd_1832(now, wall);
            (l.lines, l.cursor)
        } else {
            ([String::new(), String::new()], None)
        };
        let view = View {
            seq: self.seq,
            model: p.model,
            leds: if p.model == Model::Pc1550 { p.leds_1550(now) } else { [Led::Off; 11] },
            lcd,
            lcd_cursor,
            lcd_leds: if p.model == Model::Pc1832 { p.leds_5500(now) } else { [Led::Off; 4] },
            backlight: p.brightness as f32 / 9.0,
            contrast: p.contrast as f32 / 9.0,
            all_on: p.all_on(now),
            status,
            status_title: title,
            status_detail: detail,
            zones: p.zone_views(),
            bell: p.bell_sent != BellPattern::Off,
            siren: self.cfg.sound.siren_model,
            hold: p.hold_progress(now),
            troubles: p.trouble_list(),
            events: self.events_cache.1.clone(),
            chat: self.chat_cache.1.clone(),
            call: self.op.call_view(),
            armed: p.armed.is_some() || p.exit.is_some(),
            programming: p.is_programming(),
            notify_status: self.notify_status.clone(),
            sim: self.sim,
        };
        if let Ok(mut v) = self.view.lock() {
            *v = Arc::new(view);
        }
        self.ctx.request_repaint();
    }
}
