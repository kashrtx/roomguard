//! The panel engine shared by both models: zones, arming, entry and exit
//! delays, alarms and the bell, troubles, the keypad sounder, codes and the
//! programming framework. Model specific keypad behaviour lives in
//! kp1550.rs and kp1832.rs.
//!
//! The engine is pure logic. It never touches audio, the network or the
//! screen directly; it queues `Out` items that the engine thread routes.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::eeprom::{self, Eeprom, Kind, ZoneCfg};
use super::types::*;
use crate::audio::{AudioCmd, BellPattern, ChimeKind, Pattern};
use crate::util;

pub const MAX_EVENTS: usize = 500;

pub mod tb {
    pub const LOW_BATT: u32 = 1;
    pub const AC: u32 = 2;
    pub const BELL: u32 = 4;
    pub const FTC: u32 = 8;
    pub const TLM: u32 = 16;
    pub const ZONE_FAULT: u32 = 32;
    pub const ZONE_TAMPER: u32 = 64;
    pub const CLOCK: u32 = 128;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ZoneRt {
    pub open: bool,
    pub tamper: bool,
    pub fault: bool,
    pub bypassed: bool,
    pub auto_bypass: bool,
    pub alarm: bool,
    pub trips: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmHow {
    Code,
    StayKey,
    AwayKey,
    NoEntry,
    Quick,
    Auto,
}

#[derive(Clone, Copy, Debug)]
pub struct ExitRt {
    pub started: Instant,
    pub dur: Duration,
    pub how: ArmHow,
    pub user: u8,
    pub delay_seen: bool,
    pub silent: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ArmedRt {
    pub mode: ArmMode,
    pub no_entry: bool,
    pub user: u8,
    pub since: Instant,
    pub quick_exit: Option<Instant>,
    pub shunt_until: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct EntryRt {
    pub started: Instant,
    pub dur: Duration,
    pub zone: u8,
    pub exit_fault: bool,
    pub followers: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct AlarmRt {
    pub kind: AlarmKind,
    pub zone: u8,
    pub audible: bool,
    pub buzzer: bool,
    pub silenced: bool,
    pub bell_until: Instant,
    pub started: Instant,
    pub report_id: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct HeldKey {
    pub key: Key,
    pub since: Instant,
    pub fired: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeFor {
    UserCodes,
    UserFns,
    Installer,
    Keypad,
    NoEntry,
    Bypass,
    Arm(ArmHow),
}

#[derive(Clone, Debug)]
pub struct Prog {
    pub keypad: bool,
    pub section: Option<String>,
    pub sel: String,
    pub pos: usize,
    pub work: Vec<char>,
    pub hex: bool,
    pub sub_for: Option<String>,
    pub reset_stage: u8,
}

impl Prog {
    pub fn new(keypad: bool) -> Prog {
        Prog { keypad, section: None, sel: String::new(), pos: 0, work: Vec::new(), hex: false, sub_for: None, reset_stage: 0 }
    }
}

#[derive(Clone, Debug)]
pub enum KpMode {
    Normal,
    Star { idx: usize },
    NeedCode { purpose: CodeFor, buf: String },
    Bypass { buf: String, idx: usize },
    Troubles { idx: usize, sub: bool, mem: bool },
    Memory { level: usize, idx: usize },
    UserCodes { slot: Option<String>, buf: String, idx: usize, attrs: bool },
    UserFn { item: Option<u8>, buf: String, idx: usize },
    Program(Prog),
    Events { idx: usize, detail: bool },
    Level { which: u8 },
    CmdOut { idx: usize },
}

pub enum Out {
    Audio(AudioCmd),
    Report(Report),
    Snapshot(String),
    Save,
    LearnFace,
    Disarmed { after_alarm: bool },
    Armed(bool),
}

#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub ac_ok: bool,
    pub battery_low: bool,
    pub audio_ok: bool,
    pub net_ok: bool,
    pub net_watch: bool,
    pub ftc: bool,
    pub cam_enabled: bool,
    pub cam_online: bool,
    pub cam_covered: bool,
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs {
            ac_ok: true,
            battery_low: false,
            audio_ok: true,
            net_ok: true,
            net_watch: false,
            ftc: false,
            cam_enabled: true,
            cam_online: true,
            cam_covered: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SavedArm {
    pub mode: ArmMode,
    pub no_entry: bool,
    pub user: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSave {
    pub ee: Eeprom,
    pub chime: bool,
    pub quick_arm: bool,
    pub auto_arm: bool,
    pub auto_arm_hm: Option<(u8, u8)>,
    pub armed: Option<SavedArm>,
    pub memory: Vec<Vec<u8>>,
    pub memory_flag: bool,
    pub events: Vec<EventRow>,
    pub last_bypass: Vec<u8>,
    pub clock_offset_ms: i64,
    pub brightness: u8,
    pub contrast: u8,
    pub buzzer: u8,
    pub clean_exit: bool,
}

impl Default for PanelSave {
    fn default() -> Self {
        PanelSave {
            ee: Eeprom::default(),
            chime: false,
            quick_arm: true,
            auto_arm: false,
            auto_arm_hm: None,
            armed: None,
            memory: vec![Vec::new(), Vec::new(), Vec::new()],
            memory_flag: false,
            events: Vec::new(),
            last_bypass: Vec::new(),
            clock_offset_ms: 0,
            brightness: 7,
            contrast: 5,
            buzzer: 12,
            clean_exit: true,
        }
    }
}

impl PanelSave {
    pub fn fresh(model: Model) -> PanelSave {
        PanelSave { ee: Eeprom::factory(model), ..Default::default() }
    }
}

pub struct Panel {
    pub model: Model,
    pub ee: Eeprom,
    pub zones: Vec<ZoneRt>,
    pub labels: Vec<String>,
    pub camera_zone: [bool; 8],
    pub exit: Option<ExitRt>,
    pub armed: Option<ArmedRt>,
    pub entry: Option<EntryRt>,
    pub alarms: Vec<AlarmRt>,
    pub memory: Vec<Vec<u8>>,
    pub memory_flag: bool,
    pub mem_flash_until: Option<Instant>,
    pub troubles: u32,
    pub trouble_ack: u32,
    pub trouble_mem: u32,
    pub kp: KpMode,
    pub pending_kp: Option<KpMode>,
    pub buf: String,
    pub buf_at: Instant,
    pub last_key_at: Instant,
    pub held: Option<HeldKey>,
    pub chime: bool,
    pub quick_arm: bool,
    pub auto_arm: bool,
    pub auto_arm_hm: Option<(u8, u8)>,
    pub auto_pre: Option<Instant>,
    pub auto_last_squawk: Option<Instant>,
    pub auto_done_day: Option<u32>,
    pub walk_test: bool,
    pub walk_burst_until: Option<Instant>,
    pub bad_codes: u8,
    pub lockout_until: Option<Instant>,
    pub events: Vec<EventRow>,
    pub events_seq: u64,
    pub out: Vec<Out>,
    pub sounder_sent: Pattern,
    pub bell_sent: BellPattern,
    pub bell_test_until: Option<Instant>,
    pub util_until: Option<Instant>,
    pub temp_msg: Option<(String, String, Instant)>,
    pub lcd_scroll: i32,
    pub last_bypass: Vec<u8>,
    pub clock_offset_ms: i64,
    pub brightness: u8,
    pub contrast: u8,
    pub buzzer: u8,
    pub inputs: Inputs,
    pub ac_lost_at: Option<Instant>,
    pub ac_reported: bool,
    pub dirty: bool,
    pub next_report: u64,
    pub fire_reset_until: Option<Instant>,
    pub owner: String,
    pub face_disarm: bool,
}

impl Panel {
    pub fn new(model: Model, save: PanelSave, labels: Vec<String>, owner: String, now: Instant) -> Panel {
        let mut ee = save.ee;
        let master = match model {
            Model::Pc1550 => "1",
            Model::Pc1832 => "40",
        };
        if !ee.has_code(master) {
            ee.set_code(master, "1234");
        }
        if !ee.has_code("inst") {
            ee.set_code("inst", model.installer_default());
        }
        let mut memory = save.memory;
        while memory.len() < 3 {
            memory.push(Vec::new());
        }
        let mut p = Panel {
            model,
            ee,
            zones: vec![ZoneRt::default(); model.zone_count()],
            labels,
            camera_zone: [true, true, false, false, false, false, false, false],
            exit: None,
            armed: None,
            entry: None,
            alarms: Vec::new(),
            memory,
            memory_flag: save.memory_flag,
            mem_flash_until: None,
            troubles: 0,
            trouble_ack: 0,
            trouble_mem: 0,
            kp: KpMode::Normal,
            pending_kp: None,
            buf: String::new(),
            buf_at: now,
            last_key_at: now,
            held: None,
            chime: save.chime,
            quick_arm: save.quick_arm,
            auto_arm: save.auto_arm,
            auto_arm_hm: save.auto_arm_hm,
            auto_pre: None,
            auto_last_squawk: None,
            auto_done_day: None,
            walk_test: false,
            walk_burst_until: None,
            bad_codes: 0,
            lockout_until: None,
            events: save.events,
            events_seq: 1,
            out: Vec::new(),
            sounder_sent: Pattern::Off,
            bell_sent: BellPattern::Off,
            bell_test_until: None,
            util_until: None,
            temp_msg: None,
            lcd_scroll: 0,
            last_bypass: save.last_bypass,
            clock_offset_ms: save.clock_offset_ms,
            brightness: save.brightness.min(9),
            contrast: save.contrast.min(9),
            buzzer: save.buzzer.min(20),
            inputs: Inputs::default(),
            ac_lost_at: None,
            ac_reported: false,
            dirty: true,
            next_report: 0,
            fire_reset_until: None,
            owner,
            face_disarm: true,
        };
        // Like a real panel after a power cut, come back armed if it was armed.
        if let Some(a) = save.armed {
            let shunt = match model {
                Model::Pc1550 => p.ee.toggle(model, "32", 6),
                Model::Pc1832 => true,
            };
            p.armed = Some(ArmedRt {
                mode: a.mode,
                no_entry: a.no_entry,
                user: a.user,
                since: now,
                quick_exit: None,
                shunt_until: if shunt { Some(now + Duration::from_secs(60)) } else { None },
            });
            if a.mode == ArmMode::Stay {
                for z in 1..=p.zones.len() {
                    if p.cfg(z).kind.is_stay_away() {
                        p.zones[z - 1].bypassed = true;
                        p.zones[z - 1].auto_bypass = true;
                    }
                }
            }
            let how = if save.clean_exit { "after a shutdown" } else { "after an unexpected shutdown" };
            p.log(EvKind::Arm, format!("Power restored while armed ({}), still armed", how));
            p.report(ReportKind::PowerUp, 305, 0, format!("System restarted while armed {}", how), 0);
        }
        p
    }

    pub fn to_save(&self, clean_exit: bool) -> PanelSave {
        PanelSave {
            ee: self.ee.clone(),
            chime: self.chime,
            quick_arm: self.quick_arm,
            auto_arm: self.auto_arm,
            auto_arm_hm: self.auto_arm_hm,
            armed: self.armed.map(|a| SavedArm { mode: a.mode, no_entry: a.no_entry, user: a.user }).or_else(|| {
                self.exit.map(|e| SavedArm {
                    mode: if e.how == ArmHow::StayKey || e.how == ArmHow::NoEntry { ArmMode::Stay } else { ArmMode::Away },
                    no_entry: e.how == ArmHow::NoEntry,
                    user: e.user,
                })
            }),
            memory: self.memory.clone(),
            memory_flag: self.memory_flag,
            events: self.events.clone(),
            last_bypass: self.last_bypass.clone(),
            clock_offset_ms: self.clock_offset_ms,
            brightness: self.brightness,
            contrast: self.contrast,
            buzzer: self.buzzer,
            clean_exit,
        }
    }

    pub fn n(&self) -> usize {
        self.zones.len()
    }

    pub fn cfg(&self, z: usize) -> ZoneCfg {
        self.ee.zone_cfg(self.model, z)
    }

    pub fn label(&self, z: usize) -> String {
        if z == 0 {
            return "Keypad".to_string();
        }
        self.labels.get(z - 1).cloned().unwrap_or_else(|| format!("Zone {}", z))
    }

    pub fn who(&self, user: u8) -> String {
        let master = match self.model {
            Model::Pc1550 => user == 1,
            Model::Pc1832 => user == 40,
        };
        if user == 0 {
            "quick arm".to_string()
        } else if master {
            format!("master code ({})", self.owner)
        } else {
            format!("user {:02}", user)
        }
    }

    // ----- sounds and records -----

    pub fn audio(&mut self, c: AudioCmd) {
        self.out.push(Out::Audio(c));
    }
    pub fn key_beep(&mut self) {
        self.audio(AudioCmd::Beeps { count: 1, on_ms: 55, off_ms: 0 });
    }
    pub fn beeps(&mut self, n: u8) {
        self.audio(AudioCmd::Beeps { count: n, on_ms: 70, off_ms: 70 });
    }
    pub fn long_beep(&mut self) {
        self.audio(AudioCmd::Beeps { count: 1, on_ms: 900, off_ms: 0 });
    }
    pub fn error_tone(&mut self) {
        let ms = match self.model {
            Model::Pc1550 => 1500,
            Model::Pc1832 => 2000,
        };
        self.audio(AudioCmd::Beeps { count: 1, on_ms: ms, off_ms: 0 });
    }
    pub fn squawk(&mut self, n: u8) {
        self.audio(AudioCmd::Squawk(n));
    }

    pub fn log(&mut self, kind: EvKind, text: impl Into<String>) {
        let text = text.into();
        util::log(&format!("[{}] {}", self.model.panel_name(), text));
        self.events.push(EventRow { ts: util::now_ms(), kind, text });
        if self.events.len() > MAX_EVENTS {
            let extra = self.events.len() - MAX_EVENTS;
            self.events.drain(0..extra);
        }
        self.events_seq += 1;
        self.dirty = true;
    }

    pub fn report(&mut self, kind: ReportKind, cid: u16, zu: u16, text: String, delay_s: u32) -> u64 {
        self.next_report += 1;
        let zone_label = if zu > 0 && matches!(kind, ReportKind::Alarm(_) | ReportKind::EntryStarted) {
            self.label(zu as usize)
        } else {
            String::new()
        };
        let qualifier = if matches!(kind, ReportKind::Restore | ReportKind::Closing) { 'R' } else { 'E' };
        self.out.push(Out::Report(Report {
            id: self.next_report,
            kind,
            cid,
            qualifier,
            zone_or_user: zu,
            zone_label,
            text,
            delay_s,
            ts: util::now_ms(),
        }));
        self.next_report
    }

    pub fn temp(&mut self, l1: &str, l2: &str, ms: u64) {
        self.temp_msg = Some((l1.to_string(), l2.to_string(), Instant::now() + Duration::from_millis(ms)));
        self.dirty = true;
    }

    // ----- state queries -----

    pub fn is_programming(&self) -> bool {
        matches!(self.kp, KpMode::Program(_))
    }

    pub fn violated(&self, z: usize) -> bool {
        self.zones.get(z.wrapping_sub(1)).map_or(false, |r| r.open || r.tamper || r.fault)
    }

    pub fn ready(&self) -> bool {
        for z in 1..=self.n() {
            let c = self.cfg(z);
            if c.kind == ZoneKind::Null || c.kind.is_24h() || self.zones[z - 1].bypassed {
                continue;
            }
            if c.force_arm && self.zones[z - 1].open {
                continue;
            }
            if self.violated(z) {
                return false;
            }
        }
        true
    }

    pub fn open_zones(&self) -> Vec<u8> {
        (1..=self.n())
            .filter(|&z| self.violated(z) && self.cfg(z).kind != ZoneKind::Null && !self.zones[z - 1].bypassed)
            .map(|z| z as u8)
            .collect()
    }

    pub fn bypass_any(&self) -> bool {
        self.zones.iter().any(|z| z.bypassed)
    }

    pub fn live_alarms(&self, now: Instant) -> impl Iterator<Item = &AlarmRt> {
        self.alarms.iter().filter(move |a| !a.silenced && now < a.bell_until)
    }

    pub fn alarm_sounding(&self, now: Instant) -> bool {
        self.live_alarms(now).any(|a| a.audible || a.buzzer)
    }

    // ----- programmed times and options -----

    pub fn entry_secs(&self, kind: ZoneKind) -> u64 {
        let m = self.model;
        let v = match m {
            Model::Pc1550 => {
                if kind == ZoneKind::Delay2 {
                    self.ee.entry_or(m, "18", 0, 45)
                } else {
                    self.ee.entry_or(m, "17", 0, 30)
                }
            }
            Model::Pc1832 => {
                if kind == ZoneKind::Delay2 {
                    self.ee.entry_or(m, "005.01", 1, 45)
                } else {
                    self.ee.entry_or(m, "005.01", 0, 30)
                }
            }
        };
        v.max(1) as u64
    }

    pub fn exit_secs(&self) -> u64 {
        let m = self.model;
        let v = match m {
            Model::Pc1550 => self.ee.entry_or(m, "17", 1, 120),
            Model::Pc1832 => self.ee.entry_or(m, "005.01", 2, 120),
        };
        v.max(1) as u64
    }

    pub fn bell_minutes(&self) -> u64 {
        let m = self.model;
        let v = match m {
            Model::Pc1550 => self.ee.entry_or(m, "17", 2, 4),
            Model::Pc1832 => self.ee.entry_or(m, "005.09", 0, 4),
        };
        v.max(1) as u64
    }

    pub fn swinger_limit(&self) -> u8 {
        match self.model {
            Model::Pc1550 => self.ee.entry_or(self.model, "15", 0, 3).min(99) as u8,
            Model::Pc1832 => 3,
        }
    }

    pub fn tx_delay(&self) -> u32 {
        match self.model {
            Model::Pc1550 => self.ee.entry_or(self.model, "15", 1, 0),
            Model::Pc1832 => 30,
        }
    }

    pub fn lockout_params(&self) -> (u32, u32) {
        let m = self.model;
        match m {
            Model::Pc1550 => (self.ee.entry_or(m, "36", 0, 0), self.ee.entry_or(m, "36", 1, 0)),
            Model::Pc1832 => (self.ee.entry_or(m, "012", 0, 0), self.ee.entry_or(m, "012", 1, 0)),
        }
    }

    pub fn squawk_on(&self) -> bool {
        match self.model {
            Model::Pc1550 => self.ee.toggle(self.model, "13", 3),
            Model::Pc1832 => self.ee.toggle(self.model, "014", 1),
        }
    }

    pub fn quick_arm_enabled(&self) -> bool {
        match self.model {
            Model::Pc1550 => self.quick_arm,
            Model::Pc1832 => self.ee.toggle(self.model, "015", 4),
        }
    }

    pub fn quick_exit_enabled(&self) -> bool {
        match self.model {
            Model::Pc1550 => self.ee.toggle(self.model, "32", 4),
            Model::Pc1832 => self.ee.toggle(self.model, "015", 3),
        }
    }

    // ----- arming -----

    pub fn arm(&mut self, how: ArmHow, user: u8, now: Instant) -> bool {
        if self.armed.is_some() || self.exit.is_some() {
            return false;
        }
        if !self.ready() {
            self.error_tone();
            if self.model == Model::Pc1832 {
                self.temp("Secure System", "Before Arming <>", 2500);
            }
            self.log(EvKind::Info, "Arming failed because a zone is open");
            return false;
        }
        let silent = self.model == Model::Pc1832 && matches!(how, ArmHow::StayKey | ArmHow::NoEntry);
        let mut secs = self.exit_secs();
        if silent {
            secs *= 2;
        }
        if how == ArmHow::Auto {
            secs = 0;
        }
        self.shift_memory();
        self.mem_flash_until = None;
        self.memory_flag = false;
        for z in self.zones.iter_mut() {
            z.alarm = false;
            z.trips = 0;
        }
        self.alarms.clear();
        self.entry = None;
        self.kp = KpMode::Normal;
        self.exit = Some(ExitRt { started: now, dur: Duration::from_secs(secs), how, user, delay_seen: false, silent });
        if self.model == Model::Pc1550 && how != ArmHow::Auto {
            self.beeps(6);
        }
        if self.squawk_on() {
            self.squawk(1);
        }
        let what = match how {
            ArmHow::StayKey => "Stay arming",
            ArmHow::AwayKey => "Away arming",
            ArmHow::NoEntry => "Arming with no entry delay",
            ArmHow::Auto => "Auto arming",
            _ => "Arming",
        };
        self.log(EvKind::Arm, format!("{} by {}", what, self.who(user)));
        self.report(ReportKind::Closing, 401, user as u16, format!("Closing by {}", self.who(user)), 0);
        self.out.push(Out::Armed(true));
        self.out.push(Out::Save);
        self.dirty = true;
        if secs == 0 {
            self.finish_exit(now);
        }
        true
    }

    fn finish_exit(&mut self, now: Instant) {
        let Some(ex) = self.exit.take() else {
            return;
        };
        let mode = match ex.how {
            ArmHow::AwayKey | ArmHow::Auto => ArmMode::Away,
            ArmHow::StayKey | ArmHow::NoEntry => ArmMode::Stay,
            _ => {
                if ex.delay_seen {
                    ArmMode::Away
                } else {
                    ArmMode::Stay
                }
            }
        };
        if mode == ArmMode::Stay {
            for z in 1..=self.n() {
                if self.cfg(z).kind.is_stay_away() && !self.zones[z - 1].bypassed {
                    self.zones[z - 1].bypassed = true;
                    self.zones[z - 1].auto_bypass = true;
                }
            }
        }
        self.armed = Some(ArmedRt {
            mode,
            no_entry: ex.how == ArmHow::NoEntry,
            user: ex.user,
            since: now,
            quick_exit: None,
            shunt_until: None,
        });
        let text = match (mode, ex.how == ArmHow::NoEntry) {
            (ArmMode::Stay, true) => "Armed, no entry delay",
            (ArmMode::Stay, false) => "Armed stay, interior zones bypassed",
            (ArmMode::Away, _) => "Armed away",
        };
        self.log(EvKind::Arm, text);
        self.out.push(Out::Save);
        self.dirty = true;
        let exit_fault = match self.model {
            Model::Pc1550 => self.ee.toggle(self.model, "34", 3),
            Model::Pc1832 => self.ee.toggle(self.model, "013", 6),
        };
        for z in 1..=self.n() {
            if !self.violated(z) || self.zones[z - 1].bypassed {
                continue;
            }
            let kind = self.cfg(z).kind;
            if kind.is_24h() || kind == ZoneKind::Null {
                continue;
            }
            if kind.is_delay() && self.entry.is_none() {
                if exit_fault {
                    self.log(EvKind::Alarm, format!("Audible exit fault, {} left open", self.label(z)));
                }
                self.start_entry(z as u8, now, exit_fault);
            } else {
                self.zone_tripped(z, now);
            }
        }
    }

    pub fn disarm(&mut self, user: u8, now: Instant) {
        let was_on = self.armed.is_some() || self.exit.is_some();
        let had_alarm = self.zones.iter().any(|z| z.alarm) || self.alarms.iter().any(|a| a.kind != AlarmKind::Duress);
        self.exit = None;
        self.armed = None;
        self.entry = None;
        self.alarms.clear();
        self.auto_pre = None;
        for z in self.zones.iter_mut() {
            z.bypassed = false;
            z.auto_bypass = false;
            z.trips = 0;
        }
        if had_alarm {
            self.memory_flag = true;
            match self.model {
                Model::Pc1550 => self.mem_flash_until = Some(now + Duration::from_secs(120)),
                Model::Pc1832 => {
                    self.temp("Alarm Occurred", "While Armed  <>", 4000);
                    self.pending_kp = Some(KpMode::Memory { level: 0, idx: 0 });
                }
            }
        }
        if was_on && self.squawk_on() {
            self.squawk(2);
        }
        self.log(EvKind::Disarm, format!("Disarmed by {}", self.who(user)));
        self.report(ReportKind::Opening, 401, user as u16, format!("Opening by {}", self.who(user)), 0);
        self.out.push(Out::Disarmed { after_alarm: had_alarm });
        self.out.push(Out::Armed(false));
        self.out.push(Out::Save);
        self.dirty = true;
    }

    fn start_entry(&mut self, zone: u8, now: Instant, exit_fault: bool) {
        if self.entry.is_some() {
            return;
        }
        let kind = self.cfg(zone as usize).kind;
        let secs = self.entry_secs(kind);
        self.entry = Some(EntryRt { started: now, dur: Duration::from_secs(secs), zone, exit_fault, followers: Vec::new() });
        self.log(EvKind::Alarm, format!("Entry delay started by zone {} ({})", zone, self.label(zone as usize)));
        self.out.push(Out::Snapshot("entry".to_string()));
        self.report(ReportKind::EntryStarted, 0, zone as u16, format!("Entry delay, zone {}", zone), 0);
        if self.model == Model::Pc1550 && self.ee.toggle(self.model, "34", 5) {
            self.squawk(1);
        }
        self.dirty = true;
    }

    pub fn trigger(&mut self, kind: AlarmKind, zone: u8, audible: bool, buzzer: bool, now: Instant) {
        if zone > 0 && (zone as usize) <= self.n() {
            let limit = self.swinger_limit();
            let r = &mut self.zones[zone as usize - 1];
            if limit > 0 && r.trips >= limit && !matches!(kind, AlarmKind::Fire) {
                return;
            }
            r.trips = r.trips.saturating_add(1);
            r.alarm = true;
            if r.trips == limit && limit > 0 {
                let label = self.label(zone as usize);
                self.log(EvKind::Info, format!("Zone {} ({}) shut down after {} alarms (swinger)", zone, label, limit));
            }
        }
        if self.alarms.iter().any(|a| a.kind == kind && a.zone == zone && !a.silenced) {
            return;
        }
        let bell = Duration::from_secs(self.bell_minutes() * 60);
        let delay = if matches!(kind, AlarmKind::Burglary | AlarmKind::Tamper) && zone > 0 { self.tx_delay() } else { 0 };
        let where_ = if zone == 0 { "keypad".to_string() } else { format!("zone {} ({})", zone, self.label(zone as usize)) };
        let text = format!("{} alarm, {}", kind.label(), where_);
        let report_id = self.report(ReportKind::Alarm(kind), kind.cid(), zone as u16, text.clone(), delay);
        self.alarms.push(AlarmRt { kind, zone, audible, buzzer, silenced: false, bell_until: now + bell, started: now, report_id });
        if zone > 0 && !self.memory[0].contains(&zone) {
            self.memory[0].push(zone);
        }
        if kind != AlarmKind::Duress {
            self.memory_flag = true;
            self.out.push(Out::Snapshot(kind.label().to_lowercase()));
        }
        self.log(EvKind::Alarm, text);
        self.out.push(Out::Save);
        self.dirty = true;
    }

    /// Entering a code while a 24 hour alarm sounds on a disarmed system.
    pub fn silence(&mut self, user: u8) {
        for a in self.alarms.iter_mut() {
            a.silenced = true;
        }
        self.log(EvKind::Disarm, format!("Alarm silenced by {}", self.who(user)));
        self.out.push(Out::Disarmed { after_alarm: true });
        self.dirty = true;
    }

    fn shift_memory(&mut self) {
        match self.model {
            Model::Pc1550 => {
                if !self.memory[0].is_empty() {
                    self.memory.insert(0, Vec::new());
                    self.memory.truncate(3);
                }
            }
            Model::Pc1832 => self.memory[0].clear(),
        }
    }

    /// [*][1] while armed stay: bring the interior zones back.
    pub fn reactivate_stay(&mut self) {
        let mut any = false;
        for z in self.zones.iter_mut() {
            if z.auto_bypass {
                z.bypassed = false;
                z.auto_bypass = false;
                any = true;
            }
        }
        if let Some(a) = self.armed.as_mut() {
            a.mode = ArmMode::Away;
        }
        if any {
            self.beeps(3);
            self.log(EvKind::Arm, "Interior zones activated (night arming)");
        } else {
            self.key_beep();
        }
        self.dirty = true;
    }

    pub fn quick_exit(&mut self, now: Instant) {
        if self.armed.is_some() && self.quick_exit_enabled() {
            if let Some(a) = self.armed.as_mut() {
                a.quick_exit = Some(now + Duration::from_secs(120));
            }
            self.beeps(3);
            self.log(EvKind::Info, "Quick exit, two minutes to leave");
            if self.model == Model::Pc1832 {
                self.temp("Quick Exit in", "Progress", 2500);
            }
        } else {
            self.error_tone();
        }
    }

    pub fn toggle_chime(&mut self) {
        self.chime = !self.chime;
        if self.chime {
            self.beeps(3);
        } else {
            self.long_beep();
        }
        if self.model == Model::Pc1832 {
            let (a, b) = if self.chime { ("Door Chime", "Enabled") } else { ("Door Chime", "Disabled") };
            self.temp(a, b, 2000);
        }
        self.log(EvKind::Info, if self.chime { "Door chime on" } else { "Door chime off" });
        self.out.push(Out::Save);
    }

    pub fn sensor_reset(&mut self, now: Instant) {
        self.fire_reset_until = Some(now + Duration::from_secs(5));
        let fire_zones_clear = (1..=self.n()).all(|z| !(self.cfg(z).kind.is_fire() && self.violated(z)));
        if fire_zones_clear {
            self.alarms.retain(|a| !a.kind.pulsed());
        }
        self.beeps(1);
        self.log(EvKind::Info, "Sensor reset");
        self.dirty = true;
    }

    pub fn system_test(&mut self, now: Instant) {
        self.bell_test_until = Some(now + Duration::from_secs(2));
        self.log(EvKind::Info, "System test: bell, sounder and lights");
        self.dirty = true;
    }

    pub fn all_on(&self, now: Instant) -> bool {
        self.bell_test_until.map_or(false, |t| now < t)
    }

    pub fn ack_troubles(&mut self) {
        self.trouble_ack = self.troubles;
    }

    pub fn factory_reset(&mut self) {
        let keep_installer = if self.ee.installer_lockout { self.ee.codes.get("inst").cloned() } else { None };
        self.ee = Eeprom::factory(self.model);
        if let Some(c) = keep_installer {
            self.ee.codes.insert("inst".to_string(), c);
            self.ee.installer_lockout = true;
        }
        self.beeps(8);
        self.log(EvKind::Program, "EEPROM reset to factory defaults");
        self.out.push(Out::Save);
        self.dirty = true;
    }

    // ----- codes -----

    pub fn is_duress(&self, slot: &str) -> bool {
        self.model == Model::Pc1832 && (self.ee.attrs(slot) & 2) != 0
    }

    /// A 4 digit code typed in normal mode: disarm, silence or arm.
    pub fn normal_code(&mut self, code: &str, now: Instant) {
        let Some(slot) = self.ee.find_code(code) else {
            self.bad_code(now);
            return;
        };
        if slot == "inst" {
            self.bad_code(now);
            return;
        }
        self.bad_codes = 0;
        let user = eeprom::slot_user(&slot);
        if self.is_duress(&slot) {
            self.trigger(AlarmKind::Duress, 0, false, false, now);
        }
        if self.auto_pre.is_some() && self.model == Model::Pc1832 {
            self.auto_pre = None;
            self.beeps(3);
            self.log(EvKind::Info, format!("Auto arm cancelled by {}", self.who(user)));
            return;
        }
        if self.exit.is_some() || self.armed.is_some() {
            self.disarm(user, now);
            if self.entry.is_none() {
                self.out.push(Out::LearnFace);
            }
        } else if self.alarms.iter().any(|a| !a.silenced) {
            self.silence(user);
        } else {
            self.arm(ArmHow::Code, user, now);
        }
    }

    pub fn bad_code(&mut self, now: Instant) {
        self.error_tone();
        self.bad_codes = self.bad_codes.saturating_add(1);
        let (count, minutes) = self.lockout_params();
        if count > 0 && self.bad_codes as u32 >= count {
            self.bad_codes = 0;
            if minutes > 0 {
                self.lockout_until = Some(now + Duration::from_secs(minutes as u64 * 60));
            }
            self.log(EvKind::Alarm, format!("Keypad lockout after {} wrong codes", count));
            self.report(ReportKind::Lockout, 461, 0, format!("{} wrong codes entered at the keypad", count), 0);
            self.out.push(Out::Snapshot("keypad-lockout".to_string()));
        }
        self.dirty = true;
    }

    pub fn k_need_code(&mut self, purpose: CodeFor, mut buf: String, key: Key, now: Instant) -> KpMode {
        match key {
            Key::Digit(d) => {
                self.key_beep();
                buf.push(char::from(b'0' + d));
                if buf.len() < 4 {
                    return KpMode::NeedCode { purpose, buf };
                }
                self.code_for(purpose, &buf, now)
            }
            Key::Star => {
                self.key_beep();
                KpMode::NeedCode { purpose, buf: String::new() }
            }
            Key::Hash => {
                self.key_beep();
                KpMode::Normal
            }
            _ => KpMode::NeedCode { purpose, buf },
        }
    }

    fn code_for(&mut self, purpose: CodeFor, code: &str, now: Instant) -> KpMode {
        let slot = self.ee.find_code(code);
        match purpose {
            CodeFor::Installer | CodeFor::Keypad => {
                if slot.as_deref() == Some("inst") {
                    self.bad_codes = 0;
                    self.enter_programming(purpose == CodeFor::Keypad, now)
                } else {
                    self.bad_code(now);
                    KpMode::Normal
                }
            }
            CodeFor::UserCodes | CodeFor::UserFns => match slot {
                Some(s) if eeprom::is_master(self.model, &self.ee, &s) => {
                    self.bad_codes = 0;
                    self.beeps(2);
                    if purpose == CodeFor::UserCodes {
                        KpMode::UserCodes { slot: None, buf: String::new(), idx: 0, attrs: false }
                    } else {
                        KpMode::UserFn { item: None, buf: String::new(), idx: 0 }
                    }
                }
                _ => {
                    self.bad_code(now);
                    KpMode::Normal
                }
            },
            CodeFor::NoEntry | CodeFor::Bypass | CodeFor::Arm(_) => match slot {
                Some(s) if s != "inst" => {
                    self.bad_codes = 0;
                    let user = eeprom::slot_user(&s);
                    match purpose {
                        CodeFor::Bypass => KpMode::Bypass { buf: String::new(), idx: 0 },
                        CodeFor::NoEntry => {
                            self.arm(ArmHow::NoEntry, user, now);
                            KpMode::Normal
                        }
                        CodeFor::Arm(how) => {
                            self.arm(how, user, now);
                            KpMode::Normal
                        }
                        _ => KpMode::Normal,
                    }
                }
                _ => {
                    self.bad_code(now);
                    KpMode::Normal
                }
            },
        }
    }

    // ----- installer programming (shared by both keypads) -----

    pub fn enter_programming(&mut self, keypad: bool, _now: Instant) -> KpMode {
        if self.armed.is_some() || self.exit.is_some() {
            self.error_tone();
            return KpMode::Normal;
        }
        self.beeps(2);
        self.log(EvKind::Program, if keypad { "Keypad programming entered" } else { "Installer programming entered" });
        KpMode::Program(Prog::new(keypad))
    }

    fn exit_programming(&mut self) {
        self.key_beep();
        self.log(EvKind::Program, "Installer programming exited");
        self.out.push(Out::Save);
        self.dirty = true;
    }

    pub fn k_program(&mut self, mut p: Prog, key: Key, now: Instant) -> KpMode {
        if p.section.is_some() {
            return self.k_section(p, key);
        }
        let digits = match self.model {
            Model::Pc1550 => 2,
            Model::Pc1832 => 3,
        };
        match key {
            Key::Digit(d) => {
                self.key_beep();
                p.sel.push(char::from(b'0' + d));
                if p.reset_stage == 1 {
                    if p.sel.len() < 4 {
                        return KpMode::Program(p);
                    }
                    let code = std::mem::take(&mut p.sel);
                    if self.ee.code_matches("inst", &code) {
                        p.reset_stage = 2;
                    } else {
                        self.error_tone();
                        p.reset_stage = 0;
                    }
                    return KpMode::Program(p);
                }
                if p.reset_stage == 2 {
                    if p.sel.len() < 3 {
                        return KpMode::Program(p);
                    }
                    let confirm = std::mem::take(&mut p.sel);
                    p.reset_stage = 0;
                    if confirm == "999" {
                        self.factory_reset();
                        return KpMode::Normal;
                    }
                    self.error_tone();
                    return KpMode::Program(p);
                }
                if let Some(parent) = p.sub_for.clone() {
                    if p.sel.len() < 2 {
                        return KpMode::Program(p);
                    }
                    let key = format!("{}.{}", parent, p.sel);
                    p.sel.clear();
                    p.sub_for = None;
                    return self.open_section(p, key, now);
                }
                if p.sel.len() < digits {
                    return KpMode::Program(p);
                }
                let sel = std::mem::take(&mut p.sel);
                let key = if p.keypad { format!("kp{}", sel) } else { sel };
                self.open_section(p, key, now)
            }
            Key::Hash => {
                self.key_beep();
                if !p.sel.is_empty() || p.sub_for.is_some() || p.reset_stage > 0 {
                    p.sel.clear();
                    p.sub_for = None;
                    p.reset_stage = 0;
                    return KpMode::Program(p);
                }
                self.exit_programming();
                KpMode::Normal
            }
            Key::Star if self.model == Model::Pc1832 && p.sel.is_empty() && p.sub_for.is_none() && p.reset_stage == 0 => {
                // [*][8][installer code][*] switches to PK5500 keypad programming.
                self.beeps(2);
                p.keypad = !p.keypad;
                KpMode::Program(p)
            }
            _ => KpMode::Program(p),
        }
    }

    fn open_section(&mut self, mut p: Prog, key: String, _now: Instant) -> KpMode {
        let Some(sp) = eeprom::spec(self.model, &key) else {
            self.error_tone();
            return KpMode::Program(p);
        };
        match sp.kind {
            Kind::Sub => {
                self.beeps(2);
                p.sub_for = Some(key);
                KpMode::Program(p)
            }
            Kind::Action => {
                match key.as_str() {
                    "30" => {
                        self.factory_reset();
                        return KpMode::Normal;
                    }
                    "999" => {
                        self.beeps(2);
                        p.reset_stage = 1;
                    }
                    "90" => {
                        self.ee.installer_lockout = true;
                        self.beeps(3);
                        self.log(EvKind::Program, "Installer's lockout enabled");
                    }
                    "91" => {
                        self.ee.installer_lockout = false;
                        self.beeps(3);
                        self.log(EvKind::Program, "Installer's lockout disabled");
                    }
                    _ => self.error_tone(),
                }
                KpMode::Program(p)
            }
            kind => {
                self.beeps(3);
                let raw = self.ee.raw(self.model, &key);
                p.work = match kind {
                    Kind::Data { .. } => {
                        let len = eeprom::data_len(kind);
                        let mut v: Vec<char> = raw.chars().take(len).collect();
                        while v.len() < len {
                            v.push('0');
                        }
                        v
                    }
                    Kind::Toggle(_) => raw.chars().collect(),
                    _ => Vec::new(),
                };
                p.pos = 0;
                p.hex = false;
                p.section = Some(key);
                KpMode::Program(p)
            }
        }
    }

    fn commit(&mut self, key: &str, work: &[char], kind: Kind) {
        match kind {
            Kind::Toggle(_) => {
                let mut v: Vec<char> = work.to_vec();
                v.sort();
                v.dedup();
                self.ee.sections.insert(key.to_string(), v.into_iter().collect());
            }
            Kind::Data { .. } | Kind::Phone(_) => {
                self.ee.sections.insert(key.to_string(), work.iter().collect());
            }
            _ => {}
        }
        self.log(EvKind::Program, format!("Section [{}] programmed", key.trim_start_matches("kp")));
        self.dirty = true;
    }

    fn k_section(&mut self, mut p: Prog, key: Key) -> KpMode {
        let sec = p.section.clone().unwrap_or_default();
        let Some(sp) = eeprom::spec(self.model, &sec) else {
            p.section = None;
            return KpMode::Program(p);
        };
        let kind = sp.kind;
        let hex_digit = |d: u8, hex: bool| -> Option<char> {
            if hex {
                if (1..=6).contains(&d) {
                    Some((b'A' + d - 1) as char)
                } else {
                    None
                }
            } else {
                Some(char::from(b'0' + d))
            }
        };
        match kind {
            Kind::Toggle(n) => match key {
                Key::Digit(0) => {
                    self.key_beep();
                    p.work.clear();
                    KpMode::Program(p)
                }
                Key::Digit(d) if d <= n => {
                    self.key_beep();
                    let c = char::from(b'0' + d);
                    if let Some(i) = p.work.iter().position(|&x| x == c) {
                        p.work.remove(i);
                    } else {
                        p.work.push(c);
                    }
                    KpMode::Program(p)
                }
                Key::Hash => {
                    let work = p.work.clone();
                    self.commit(&sec, &work, kind);
                    self.beeps(3);
                    p.section = None;
                    KpMode::Program(p)
                }
                _ => {
                    self.error_tone();
                    KpMode::Program(p)
                }
            },
            Kind::Data { width, .. } => {
                let len = eeprom::data_len(kind);
                let w = width.max(1) as usize;
                match key {
                    Key::Digit(d) => {
                        let Some(c) = hex_digit(d, p.hex) else {
                            self.error_tone();
                            return KpMode::Program(p);
                        };
                        if p.pos < len {
                            p.work[p.pos] = c;
                            p.pos += 1;
                            self.key_beep();
                        }
                        if p.pos >= len {
                            let work = p.work.clone();
                            self.commit(&sec, &work, kind);
                            self.beeps(6);
                            p.section = None;
                        } else if p.pos % w == 0 {
                            self.beeps(2);
                        }
                        KpMode::Program(p)
                    }
                    Key::Fire | Key::Right => {
                        p.pos += 1;
                        if p.pos >= len {
                            let work = p.work.clone();
                            self.commit(&sec, &work, kind);
                            self.beeps(6);
                            p.section = None;
                        }
                        KpMode::Program(p)
                    }
                    Key::Left => {
                        p.pos = p.pos.saturating_sub(1);
                        KpMode::Program(p)
                    }
                    Key::Star => {
                        p.hex = !p.hex;
                        self.key_beep();
                        KpMode::Program(p)
                    }
                    Key::Hash => {
                        // Keep completed entries, put back a half typed one.
                        let keep = p.pos - p.pos % w;
                        let old: Vec<char> = self.ee.raw(self.model, &sec).chars().collect();
                        for i in keep..len {
                            if let Some(c) = old.get(i) {
                                p.work[i] = *c;
                            }
                        }
                        let work = p.work.clone();
                        if keep > 0 {
                            self.commit(&sec, &work, kind);
                        }
                        self.key_beep();
                        p.section = None;
                        KpMode::Program(p)
                    }
                    _ => KpMode::Program(p),
                }
            }
            Kind::Phone(max) => match key {
                Key::Digit(d) => {
                    if let Some(c) = hex_digit(d, p.hex) {
                        if p.work.len() < max as usize {
                            p.work.push(c);
                        }
                        self.key_beep();
                    } else {
                        self.error_tone();
                    }
                    KpMode::Program(p)
                }
                Key::Star => {
                    p.hex = !p.hex;
                    self.key_beep();
                    KpMode::Program(p)
                }
                Key::Hash => {
                    let work = p.work.clone();
                    self.commit(&sec, &work, kind);
                    self.beeps(3);
                    p.section = None;
                    KpMode::Program(p)
                }
                _ => KpMode::Program(p),
            },
            Kind::Code(slot) => match key {
                Key::Digit(d) => {
                    self.key_beep();
                    p.work.push(char::from(b'0' + d));
                    if p.work.len() >= 4 {
                        let code: String = p.work.iter().collect();
                        p.work.clear();
                        p.section = None;
                        let taken = self.ee.find_code(&code).map_or(false, |s| s != slot);
                        if taken {
                            self.error_tone();
                        } else {
                            self.ee.set_code(slot, &code);
                            self.beeps(3);
                            self.log(EvKind::Program, format!("Section [{}] code changed", sec));
                            self.out.push(Out::Save);
                        }
                    }
                    KpMode::Program(p)
                }
                Key::Hash => {
                    p.work.clear();
                    p.section = None;
                    self.key_beep();
                    KpMode::Program(p)
                }
                _ => KpMode::Program(p),
            },
            _ => {
                p.section = None;
                KpMode::Program(p)
            }
        }
    }

    // ----- zones -----

    pub fn set_zone(&mut self, z: usize, open: bool, tamper: bool, fault: bool, now: Instant) {
        if z == 0 || z > self.n() {
            return;
        }
        let before = self.violated(z);
        let was_open = self.zones[z - 1].open;
        {
            let r = &mut self.zones[z - 1];
            r.open = open;
            r.tamper = tamper;
            r.fault = fault;
        }
        let after = self.violated(z);
        self.dirty = true;
        if open != was_open {
            self.chime_check(z, open);
        }
        if after && !before {
            self.zone_tripped(z, now);
        }
        if !after && before {
            self.zone_restored(z, now);
        }
    }

    fn chime_check(&mut self, z: usize, opened: bool) {
        if !self.chime || self.armed.is_some() || self.exit.is_some() || self.is_programming() {
            return;
        }
        let c = self.cfg(z);
        if !c.chime || self.zones[z - 1].bypassed {
            return;
        }
        if !opened && self.camera_zone.get(z - 1).copied().unwrap_or(false) {
            return;
        }
        match self.model {
            Model::Pc1550 => self.audio(AudioCmd::Chime(ChimeKind::Beeps(5))),
            Model::Pc1832 => {
                let openings = self.ee.toggle(Model::Pc1832, "kp077", 1);
                let closings = self.ee.toggle(Model::Pc1832, "kp077", 2);
                if (opened && !openings) || (!opened && !closings) {
                    return;
                }
                let kind = match self.ee.chime_sound(z) {
                    2 => ChimeKind::BingBing,
                    3 => ChimeKind::DingDong,
                    4 => ChimeKind::AlarmTone,
                    _ => ChimeKind::Beeps(6),
                };
                self.audio(AudioCmd::Chime(kind));
            }
        }
    }

    fn zone_tripped(&mut self, z: usize, now: Instant) {
        let c = self.cfg(z);
        if c.kind == ZoneKind::Null {
            return;
        }
        let zr = self.zones[z - 1];
        if self.walk_test && self.armed.is_none() && self.exit.is_none() {
            self.log(EvKind::Info, format!("Walk test: zone {} ({})", z, self.label(z)));
            match self.model {
                Model::Pc1550 => self.walk_burst_until = Some(now + Duration::from_secs(2)),
                Model::Pc1832 => self.beeps(1),
            }
            return;
        }
        if c.kind.is_24h() {
            if zr.bypassed || (c.kind.is_fire() && self.fire_reset_until.map_or(false, |t| now < t)) {
                return;
            }
            let armed = self.armed.is_some();
            let (audible, buzzer) = match c.kind {
                ZoneKind::H24(H24::BellBuzzer) => (armed && c.audible, !armed),
                ZoneKind::H24(H24::Buzzer) | ZoneKind::H24(H24::SupervisoryBuzzer) => (false, true),
                ZoneKind::H24(H24::Supervisory) => (false, false),
                _ => (c.audible, false),
            };
            self.trigger(c.kind.alarm_kind(), z as u8, audible, buzzer, now);
            return;
        }
        if zr.bypassed {
            return;
        }
        if let Some(ex) = self.exit.as_mut() {
            if c.kind.is_delay() {
                ex.delay_seen = true;
            }
            return;
        }
        let Some(a) = self.armed else {
            return;
        };
        if a.shunt_until.map_or(false, |t| now < t) {
            return;
        }
        if c.kind.is_delay() && a.quick_exit.map_or(false, |t| now < t) {
            if let Some(am) = self.armed.as_mut() {
                am.quick_exit = None;
            }
            return;
        }
        if zr.tamper || zr.fault {
            self.trigger(AlarmKind::Tamper, z as u8, c.audible, false, now);
        }
        let home_away_delay = self.model == Model::Pc1550 && c.kind == ZoneKind::StayAway && self.ee.toggle(self.model, "32", 3);
        if c.kind.is_delay() || home_away_delay {
            if a.no_entry {
                self.trigger(AlarmKind::Burglary, z as u8, c.audible, false, now);
            } else if self.entry.is_none() && !self.alarms.iter().any(|al| !al.silenced && al.kind == AlarmKind::Burglary) {
                self.start_entry(z as u8, now, false);
            } else if self.entry.is_none() {
                self.trigger(AlarmKind::Burglary, z as u8, c.audible, false, now);
            }
            return;
        }
        match c.kind {
            ZoneKind::Instant => self.trigger(AlarmKind::Burglary, z as u8, c.audible, false, now),
            ZoneKind::Interior | ZoneKind::StayAway => {
                if let Some(e) = self.entry.as_mut() {
                    if !e.followers.contains(&(z as u8)) {
                        e.followers.push(z as u8);
                    }
                } else {
                    self.trigger(AlarmKind::Burglary, z as u8, c.audible, false, now);
                }
            }
            _ => {}
        }
    }

    fn zone_restored(&mut self, z: usize, now: Instant) {
        // PowerSeries exit delay termination: once you're out and the door
        // closes, the remaining exit time drops to five seconds.
        let is_delay = self.cfg(z).kind.is_delay();
        if self.model == Model::Pc1832 && self.ee.toggle(self.model, "014", 7) {
            if let Some(ex) = self.exit.as_mut() {
                if ex.delay_seen && ex.how != ArmHow::StayKey && is_delay {
                    let left = (ex.started + ex.dur).saturating_duration_since(now);
                    if left > Duration::from_secs(5) {
                        ex.dur = now.saturating_duration_since(ex.started) + Duration::from_secs(5);
                    }
                }
            }
        }
    }

    // ----- keys -----

    fn needs_hold(&self, key: Key) -> bool {
        if key.is_emergency() {
            return !(self.model == Model::Pc1550 && key == Key::Fire && self.is_programming());
        }
        key.is_function()
    }

    pub fn key(&mut self, key: Key, down: bool, now: Instant) {
        self.dirty = true;
        if !down {
            if let Some(h) = self.held {
                if h.key == key {
                    self.held = None;
                }
            }
            return;
        }
        self.last_key_at = now;
        if let Some(until) = self.lockout_until {
            if now < until {
                if !key.is_emergency() {
                    self.error_tone();
                    return;
                }
            } else {
                self.lockout_until = None;
            }
        }
        if self.needs_hold(key) {
            self.held = Some(HeldKey { key, since: now, fired: false });
            self.key_beep();
            return;
        }
        match self.model {
            Model::Pc1550 => self.press_1550(key, now),
            Model::Pc1832 => self.press_1832(key, now),
        }
    }

    fn hold_complete(&mut self, key: Key, now: Instant) {
        if key.is_emergency() {
            self.emergency(key, now);
            return;
        }
        if !matches!(self.kp, KpMode::Normal) {
            return;
        }
        match key {
            Key::Stay => self.fn_arm(ArmHow::StayKey, now),
            Key::Away => self.fn_arm(ArmHow::AwayKey, now),
            Key::Chime => self.toggle_chime(),
            Key::Reset => self.sensor_reset(now),
            Key::Exit => self.quick_exit(now),
            _ => {}
        }
    }

    fn fn_arm(&mut self, how: ArmHow, now: Instant) {
        if self.armed.is_some() || self.exit.is_some() {
            self.error_tone();
            return;
        }
        if self.quick_arm_enabled() {
            self.arm(how, 0, now);
        } else {
            self.kp = KpMode::NeedCode { purpose: CodeFor::Arm(how), buf: String::new() };
        }
    }

    fn emergency(&mut self, key: Key, now: Instant) {
        if self.is_programming() {
            return;
        }
        let m = self.model;
        match key {
            Key::Fire => {
                let enabled = match m {
                    Model::Pc1550 => !self.ee.toggle(m, "34", 6),
                    Model::Pc1832 => self.ee.toggle(m, "015", 1) && self.ee.toggle(m, "kp074", 1),
                };
                if !enabled {
                    self.error_tone();
                    return;
                }
                self.audio(AudioCmd::Beeps { count: 10, on_ms: 50, off_ms: 50 });
                self.trigger(AlarmKind::Fire, 0, true, false, now);
            }
            Key::Aux => {
                if m == Model::Pc1832 && !self.ee.toggle(m, "kp074", 2) {
                    self.error_tone();
                    return;
                }
                self.audio(AudioCmd::Beeps { count: 10, on_ms: 50, off_ms: 50 });
                self.trigger(AlarmKind::Medical, 0, false, false, now);
            }
            Key::Panic => {
                if m == Model::Pc1832 && !self.ee.toggle(m, "kp074", 3) {
                    self.error_tone();
                    return;
                }
                let audible = match m {
                    Model::Pc1550 => self.ee.toggle(m, "12", 6),
                    Model::Pc1832 => self.ee.toggle(m, "015", 2),
                };
                let feedback = match m {
                    Model::Pc1550 => self.ee.toggle(m, "14", 5),
                    Model::Pc1832 => audible,
                };
                if feedback {
                    self.beeps(3);
                }
                self.trigger(AlarmKind::Panic, 0, audible, false, now);
            }
            _ => {}
        }
    }

    /// The camera recognized an enrolled person.
    pub fn face_recognized(&mut self, name: &str, now: Instant) -> bool {
        if !self.face_disarm {
            return false;
        }
        let entering = self.entry.is_some() || (self.armed.is_some() && self.alarm_sounding(now));
        if !entering {
            return false;
        }
        let master = match self.model {
            Model::Pc1550 => 1,
            Model::Pc1832 => 40,
        };
        self.log(EvKind::Disarm, format!("Recognized {} at the camera", name));
        self.disarm(master, now);
        true
    }

    // ----- the clock tick -----

    pub fn tick(&mut self, now: Instant, wall_ms: i64) {
        if let Some(h) = self.held {
            if !h.fired && now.saturating_duration_since(h.since).as_millis() >= self.model.hold_ms() {
                self.held = Some(HeldKey { fired: true, ..h });
                self.hold_complete(h.key, now);
                self.dirty = true;
            }
        }
        let exit_done = self.exit.map_or(false, |e| now >= e.started + e.dur);
        if exit_done {
            self.finish_exit(now);
        }
        let entry_done = self.entry.as_ref().map(|e| (now >= e.started + e.dur, e.zone, e.followers.clone()));
        if let Some((true, zone, followers)) = entry_done {
            self.entry = None;
            let audible = self.cfg(zone as usize).audible;
            self.trigger(AlarmKind::Burglary, zone, audible, false, now);
            for f in followers {
                let a = self.cfg(f as usize).audible;
                self.trigger(AlarmKind::Burglary, f, a, false, now);
            }
        }
        if self.mem_flash_until.map_or(false, |t| now >= t) {
            self.mem_flash_until = None;
            self.dirty = true;
        }
        if self.temp_msg.as_ref().map_or(false, |m| now >= m.2) {
            self.temp_msg = None;
            self.dirty = true;
        }
        if !self.buf.is_empty() && now.saturating_duration_since(self.last_key_at).as_secs() >= 10 {
            self.buf.clear();
            self.dirty = true;
        }
        let idle = now.saturating_duration_since(self.last_key_at).as_secs();
        if idle >= 120 && !matches!(self.kp, KpMode::Normal) {
            if self.is_programming() {
                self.exit_programming();
            }
            self.kp = KpMode::Normal;
        }
        self.update_troubles(now);
        self.auto_arm_tick(now, wall_ms);
        if self.walk_test && self.armed.is_some() {
            self.walk_test = false;
        }
        let s = self.sounder_wanted(now);
        if s != self.sounder_sent {
            self.sounder_sent = s;
            self.audio(AudioCmd::Sounder(s));
        }
        let b = self.bell_wanted(now);
        if b != self.bell_sent {
            self.bell_sent = b;
            self.audio(AudioCmd::Bell(b));
            self.dirty = true;
        }
    }

    fn update_troubles(&mut self, now: Instant) {
        let i = self.inputs;
        let mut t = 0u32;
        if i.battery_low {
            t |= tb::LOW_BATT;
        }
        if !i.ac_ok {
            t |= tb::AC;
        }
        if !i.audio_ok {
            t |= tb::BELL;
        }
        if i.ftc {
            t |= tb::FTC;
        }
        if self.model == Model::Pc1832 && i.net_watch && !i.net_ok {
            t |= tb::TLM;
        }
        if self.model == Model::Pc1832 && i.cam_enabled {
            if !i.cam_online {
                t |= tb::ZONE_FAULT;
            }
            if i.cam_covered {
                t |= tb::ZONE_TAMPER;
            }
        }
        if t == self.troubles {
            if t & tb::AC != 0 && !self.ac_reported {
                let delay_min: u64 = match self.model {
                    Model::Pc1550 => self.ee.entry_or(self.model, "17", 3, 30) as u64,
                    Model::Pc1832 => 30,
                };
                if self.ac_lost_at.map_or(false, |at| now.saturating_duration_since(at).as_secs() >= delay_min * 60) {
                    self.ac_reported = true;
                    self.report(ReportKind::Trouble, 301, 0, "AC power failure".to_string(), 0);
                }
            }
            return;
        }
        let new = t & !self.troubles;
        let gone = self.troubles & !t;
        for (bit, name) in trouble_names(self.model) {
            if new & bit != 0 {
                self.log(EvKind::Trouble, format!("Trouble: {}", name));
                if bit != tb::AC {
                    self.report(ReportKind::Trouble, trouble_cid(bit), 0, name.to_string(), 0);
                }
            }
            if gone & bit != 0 {
                self.log(EvKind::Restore, format!("Trouble restored: {}", name));
            }
        }
        if new & tb::AC != 0 {
            self.ac_lost_at = Some(now);
            self.ac_reported = false;
        }
        if gone & tb::AC != 0 {
            if self.ac_reported {
                self.report(ReportKind::Restore, 301, 0, "AC power restored".to_string(), 0);
            }
            self.ac_lost_at = None;
            self.ac_reported = false;
        }
        // New troubles start the trouble beeps again. AC loss stays quiet,
        // like the real panels.
        self.trouble_ack &= !(new & !tb::AC);
        self.trouble_ack |= new & tb::AC;
        self.trouble_ack &= t;
        self.troubles = t;
        self.trouble_mem |= t;
        self.dirty = true;
    }

    pub fn trouble_list(&self) -> Vec<String> {
        trouble_names(self.model)
            .into_iter()
            .filter(|(bit, _)| self.troubles & bit != 0)
            .map(|(_, n)| n.to_string())
            .collect()
    }

    fn sounder_wanted(&self, now: Instant) -> Pattern {
        if self.lockout_until.map_or(false, |t| now < t) {
            return Pattern::Off;
        }
        if self.all_on(now) || self.util_until.map_or(false, |t| now < t) {
            return Pattern::Steady;
        }
        let mut fire = false;
        let mut burg = false;
        for a in self.live_alarms(now) {
            if !(a.audible || a.buzzer) {
                continue;
            }
            if a.kind.pulsed() {
                fire = true;
            } else {
                burg = true;
            }
        }
        if fire {
            return Pattern::Beeps { on_ms: 500, off_ms: 500, count: 1, period_ms: 1000 };
        }
        if burg {
            return Pattern::Steady;
        }
        if let Some(e) = &self.entry {
            let left = (e.started + e.dur).saturating_duration_since(now).as_secs_f32();
            let urgency = self.model == Model::Pc1550 && self.ee.toggle(self.model, "34", 4);
            if urgency && left <= 10.0 {
                return Pattern::Beeps { on_ms: 250, off_ms: 250, count: 1, period_ms: 500 };
            }
            return Pattern::Steady;
        }
        if let Some(ex) = &self.exit {
            let left = (ex.started + ex.dur).saturating_duration_since(now).as_secs_f32();
            match self.model {
                Model::Pc1550 => {
                    if self.ee.toggle(self.model, "34", 1) {
                        return if left <= 10.0 {
                            Pattern::Beeps { on_ms: 150, off_ms: 150, count: 1, period_ms: 300 }
                        } else {
                            Pattern::Beeps { on_ms: 300, off_ms: 700, count: 1, period_ms: 1000 }
                        };
                    }
                }
                Model::Pc1832 => {
                    if !ex.silent {
                        let urgency = self.ee.toggle(self.model, "014", 6);
                        if urgency && left <= 10.0 {
                            return Pattern::Beeps { on_ms: 90, off_ms: 0, count: 1, period_ms: 333 };
                        }
                        let away = ex.how == ArmHow::AwayKey || ex.delay_seen || ex.how != ArmHow::StayKey;
                        return if away {
                            Pattern::Beeps { on_ms: 120, off_ms: 0, count: 1, period_ms: 1000 }
                        } else {
                            Pattern::Beeps { on_ms: 120, off_ms: 0, count: 1, period_ms: 3000 }
                        };
                    }
                }
            }
        }
        if self.auto_pre.is_some() {
            return Pattern::Steady;
        }
        let unacked = self.troubles & !self.trouble_ack;
        if unacked != 0 && self.armed.is_none() && !self.is_programming() {
            return Pattern::Beeps { on_ms: 100, off_ms: 120, count: 2, period_ms: 10_000 };
        }
        Pattern::Off
    }

    fn bell_wanted(&self, now: Instant) -> BellPattern {
        if self.all_on(now) || self.walk_burst_until.map_or(false, |t| now < t) {
            return BellPattern::Steady;
        }
        let mut fire = false;
        let mut burg = false;
        for a in self.live_alarms(now) {
            if !a.audible {
                continue;
            }
            if a.kind.pulsed() {
                fire = true;
            } else {
                burg = true;
            }
        }
        if fire {
            if self.model == Model::Pc1832 && self.ee.toggle(self.model, "013", 8) {
                return BellPattern::Temporal3;
            }
            return BellPattern::Pulsed;
        }
        if burg {
            return BellPattern::Steady;
        }
        if self.entry.as_ref().map_or(false, |e| e.exit_fault) {
            return BellPattern::Steady;
        }
        BellPattern::Off
    }

    fn auto_arm_tick(&mut self, now: Instant, wall_ms: i64) {
        if self.armed.is_some() || self.exit.is_some() {
            self.auto_pre = None;
            return;
        }
        let time = match self.model {
            Model::Pc1550 => {
                let v = self.ee.entry_or(self.model, "19", 0, 9999);
                if v >= 2400 || v % 100 >= 60 {
                    None
                } else {
                    Some(((v / 100) as u8, (v % 100) as u8))
                }
            }
            Model::Pc1832 => self.auto_arm_hm,
        };
        let Some((hh, mm)) = time else {
            return;
        };
        if !self.auto_arm {
            return;
        }
        if let Some(pre) = self.auto_pre {
            let elapsed = now.saturating_duration_since(pre).as_secs();
            let bell_bursts = match self.model {
                Model::Pc1550 => !self.ee.toggle(self.model, "32", 2),
                Model::Pc1832 => self.ee.toggle(self.model, "014", 2),
            };
            if bell_bursts && self.auto_last_squawk.map_or(true, |t| now.saturating_duration_since(t).as_secs() >= 10) {
                self.auto_last_squawk = Some(now);
                self.squawk(1);
            }
            if elapsed >= 60 {
                self.auto_pre = None;
                if self.ready() {
                    self.arm(ArmHow::Auto, 0, now);
                } else {
                    self.log(EvKind::Info, "Auto arm cancelled because a zone is open");
                    if self.model == Model::Pc1832 {
                        self.temp("System Has", "Failed to Arm", 4000);
                    }
                }
            }
            return;
        }
        let lp = util::parts(wall_ms + self.clock_offset_ms);
        if lp.hour == hh as u32 && lp.minute == mm as u32 && self.auto_done_day != Some(lp.ordinal) {
            self.auto_done_day = Some(lp.ordinal);
            self.auto_pre = Some(now);
            self.auto_last_squawk = None;
            self.log(EvKind::Info, "Auto arm in one minute");
            self.dirty = true;
        }
    }

    // ----- what the interface shows -----

    pub fn hold_progress(&self, now: Instant) -> Option<(Key, f32)> {
        self.held.filter(|h| !h.fired).map(|h| {
            let p = now.saturating_duration_since(h.since).as_millis() as f32 / self.model.hold_ms() as f32;
            (h.key, p.min(1.0))
        })
    }

    pub fn zone_views(&self) -> Vec<ZoneView> {
        (1..=self.n())
            .map(|z| {
                let r = self.zones[z - 1];
                ZoneView {
                    num: z as u8,
                    label: self.label(z),
                    kind: self.cfg(z).kind,
                    open: r.open,
                    bypassed: r.bypassed,
                    alarm: r.alarm,
                    tamper: r.tamper,
                    fault: r.fault,
                    camera: self.camera_zone.get(z - 1).copied().unwrap_or(false),
                }
            })
            .collect()
    }

    pub fn status(&self, now: Instant) -> (StatusKind, String, String) {
        if self.lockout_until.map_or(false, |t| now < t) {
            let left = self.lockout_until.map(|t| t.saturating_duration_since(now).as_secs()).unwrap_or(0);
            return (StatusKind::Lockout, "Keypad locked".into(), format!("Too many wrong codes. {} min left", left / 60 + 1));
        }
        if let KpMode::Program(p) = &self.kp {
            let what = match &p.section {
                Some(s) => format!("Section [{}]", s.trim_start_matches("kp")),
                None => "Waiting for a section number".to_string(),
            };
            return (StatusKind::Program, "Installer programming".into(), what);
        }
        if let Some(a) = self.live_alarms(now).next() {
            let where_ = if a.zone == 0 { "Keypad".to_string() } else { format!("Zone {} {}", a.zone, self.label(a.zone as usize)) };
            return (StatusKind::Alarm, format!("{} alarm", a.kind.label()), where_);
        }
        if let Some(e) = &self.entry {
            let left = (e.started + e.dur).saturating_duration_since(now).as_secs();
            return (StatusKind::Entry, "Entry delay".into(), format!("{} s to enter a code. Zone {} {}", left, e.zone, self.label(e.zone as usize)));
        }
        if let Some(ex) = &self.exit {
            let left = (ex.started + ex.dur).saturating_duration_since(now).as_secs();
            return (StatusKind::Exit, "Exit delay".into(), format!("{}:{:02} to leave", left / 60, left % 60));
        }
        if let Some(a) = &self.armed {
            let title = match (a.mode, a.no_entry) {
                (_, true) => "Armed, no entry delay",
                (ArmMode::Stay, false) => "Armed stay",
                (ArmMode::Away, false) => "Armed away",
            };
            let detail = if self.memory_flag { "An alarm happened while armed" } else { "Watching the room" };
            return (StatusKind::Armed, title.into(), detail.into());
        }
        let open = self.open_zones();
        if !open.is_empty() {
            let z = open[0] as usize;
            let more = if open.len() > 1 { format!(" and {} more", open.len() - 1) } else { String::new() };
            return (StatusKind::NotReady, "Not ready".into(), format!("Zone {} {} is open{}", z, self.label(z), more));
        }
        let troubles = self.trouble_list();
        let detail = if !troubles.is_empty() {
            format!("Trouble: {}", troubles.join(", "))
        } else if self.memory_flag {
            "Alarm in memory".to_string()
        } else {
            "All zones closed".to_string()
        };
        (StatusKind::Ready, "Ready to arm".into(), detail)
    }
}

/// The trouble types, in the order each keypad numbers them.
pub fn trouble_names(model: Model) -> Vec<(u32, &'static str)> {
    match model {
        Model::Pc1550 => vec![
            (tb::LOW_BATT, "Low battery"),
            (tb::AC, "AC failure"),
            (tb::BELL, "Bell circuit fuse"),
            (tb::FTC, "Failure to communicate"),
            (tb::CLOCK, "Loss of time"),
        ],
        Model::Pc1832 => vec![
            (tb::LOW_BATT, "Low battery"),
            (tb::BELL, "Bell circuit"),
            (tb::AC, "AC failure"),
            (tb::TLM, "Telephone line"),
            (tb::FTC, "Failure to communicate"),
            (tb::ZONE_FAULT, "Zone fault"),
            (tb::ZONE_TAMPER, "Zone tamper"),
            (tb::CLOCK, "Loss of time"),
        ],
    }
}

fn trouble_cid(bit: u32) -> u16 {
    match bit {
        tb::LOW_BATT => 302,
        tb::AC => 301,
        tb::BELL => 321,
        tb::FTC => 354,
        tb::TLM => 351,
        tb::ZONE_FAULT => 380,
        tb::ZONE_TAMPER => 383,
        _ => 300,
    }
}
