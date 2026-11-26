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
use super::kp_vista::VistaKp;
use super::types::*;
use crate::audio::voice;
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
    pub const FIRE_LOOP: u32 = 256;
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
    pub delay_closed: bool,
    pub restarted: bool,
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
    /// [*][1] in Stay: when the Stay/Away zones come back.
    pub reactivate_at: Option<Instant>,
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
    Troubles,
    Memory,
    Download,
    CmdOut,
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
    pub confirm_action: Option<&'static str>,
}

impl Prog {
    pub fn new(keypad: bool) -> Prog {
        Prog {
            keypad,
            section: None,
            sel: String::new(),
            pos: 0,
            work: Vec::new(),
            hex: false,
            sub_for: None,
            reset_stage: 0,
            confirm_action: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum KpMode {
    Normal,
    Star { idx: usize },
    NeedCode { purpose: CodeFor, buf: String },
    Bypass { buf: String, idx: usize },
    Troubles { idx: usize, sub: u8, mem: bool },
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
    pub saved_bypass: Vec<u8>,
    pub clock_offset_ms: i64,
    pub brightness: u8,
    pub contrast: u8,
    pub buzzer: u8,
    pub voice_level: u8,
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
            saved_bypass: Vec::new(),
            clock_offset_ms: 0,
            brightness: 7,
            contrast: 5,
            buzzer: 12,
            voice_level: 5,
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
    pub saved_bypass: Vec<u8>,
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
    pub vista_mode: VistaKp,
    pub vista_armed_by_inst: bool,
    pub vista_sounder_mute_until: Option<Instant>,
    pub vista_last_voice_alarm: Option<Instant>,
    pub vista_exit_voice_done: bool,
    pub vista_fault_idx: usize,
    pub vista_voice_level: u8,
    pub vista_alarm_mem_zone: Option<(u8, AlarmKind)>,
    /// When the last key was pressed, for the 4 to 5 second entry timeout.
    pub vista_prev_key: Instant,
    /// A code entered during entry delay or an alarm disarms about 15
    /// seconds later even without OFF: (when, user, code slot).
    pub vista_code_disarm: Option<(Instant, u8, String)>,
    /// Armed NIGHT-STAY ([Code] + STAY + STAY).
    pub vista_night: bool,
    /// When test mode started; it ends itself after four hours.
    pub vista_test_since: Option<Instant>,
    /// 6160V voice status and voice chime (# + 0 + 2 + 4 turns them on and off).
    pub vista_voice_on: bool,
    /// Deferred 6160V status announcement (evaluated LIVE right when spoken after
    /// the 1.1 s panel answer + confirmation beep, so a closing door/motion zone
    /// never causes a stale "not ready to arm" announcement).
    pub vista_pending_status: Option<Instant>,
}

impl Panel {
    pub fn new(model: Model, save: PanelSave, labels: Vec<String>, owner: String, now: Instant) -> Panel {
        let mut ee = save.ee;
        let master = match model {
            Model::Pc1550 => "1",
            Model::Pc1832 => "40",
            Model::Vista20p => "02",
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
            saved_bypass: save.saved_bypass,
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
            vista_mode: VistaKp::Normal,
            vista_armed_by_inst: false,
            vista_sounder_mute_until: None,
            vista_last_voice_alarm: None,
            vista_exit_voice_done: false,
            vista_fault_idx: 0,
            vista_voice_level: save.voice_level.min(7),
            vista_alarm_mem_zone: None,
            vista_prev_key: now,
            vista_code_disarm: None,
            vista_night: false,
            vista_test_since: None,
            vista_voice_on: true,
            vista_pending_status: None,
        };
        // Like a real panel after a power cut, come back armed if it was armed.
        if let Some(a) = save.armed {
            p.armed = Some(ArmedRt {
                mode: a.mode,
                no_entry: a.no_entry,
                user: a.user,
                since: now,
                quick_exit: None,
                shunt_until: Some(now + Duration::from_secs(60)),
                reactivate_at: None,
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
        // Panels saved by an earlier RoomGuard get the manual's real factory
        // defaults for any section that was never changed by hand.
        let fixed = p.ee.migrate(model);
        if !fixed.is_empty() {
            let plural = if fixed.len() == 1 { "" } else { "s" };
            p.log(EvKind::Program, format!("Section{} {} updated to the manual's factory default{}", plural, fixed.join(", "), plural));
            p.out.push(Out::Save);
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
            saved_bypass: self.saved_bypass.clone(),
            clock_offset_ms: self.clock_offset_ms,
            brightness: self.brightness,
            contrast: self.contrast,
            buzzer: self.buzzer,
            voice_level: self.vista_voice_level,
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
            Model::Vista20p => user == 2,
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
        self.audio(AudioCmd::KeyBeep);
    }
    /// Quick confirmation beeps, about 80 ms on and 80 ms off like the real
    /// keypads ("beep beep beep" when a code or command is accepted).
    pub fn beeps(&mut self, n: u8) {
        // A real PowerSeries keypad: about 67 ms on and 50 ms off.
        let (on_ms, off_ms) = if self.model == Model::Pc1832 { (67, 50) } else { (80, 80) };
        self.audio(AudioCmd::Beeps { count: n, on_ms, off_ms });
    }
    /// A feature turning off. The PC1832 manual calls it "a steady 2-second
    /// tone"; the PC1550 gives "one long beep".
    pub fn long_beep(&mut self) {
        let ms = match self.model {
            Model::Pc1550 | Model::Vista20p => 1000,
            Model::Pc1832 => 2000,
        };
        self.audio(AudioCmd::Beeps { count: 1, on_ms: ms, off_ms: 0 });
    }
    pub fn error_tone(&mut self) {
        if self.model == Model::Vista20p {
            // The user guide's "one short beep" (as when arming with a zone
            // open): the 6160's low 1019 Hz single beep.
            self.audio(AudioCmd::Tone { hz: 1019, on_ms: 340, off_ms: 60, delay_ms: 0 });
            return;
        }
        let ms = match self.model {
            Model::Pc1550 => 1500,
            Model::Pc1832 => 2000,
            // The Vista has no long error tone. A refused command gets the
            // user guide's "one short beep" (as for arming with a zone open).
            Model::Vista20p => 150,
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

    pub fn comms_enabled(&self) -> bool {
        match self.model {
            Model::Pc1550 => !self.ee.toggle(self.model, "12", 1),
            Model::Pc1832 => self.ee.toggle(self.model, "380", 1),
            Model::Vista20p => !self.ee.raw(self.model, "41").is_empty(),
        }
    }

    pub fn report(&mut self, kind: ReportKind, cid: u16, zu: u16, text: String, delay_s: u32) -> u64 {
        self.next_report += 1;
        if !self.comms_enabled() {
            return self.next_report;
        }
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
        matches!(self.kp, KpMode::Program(_)) || matches!(self.vista_mode, VistaKp::Program { .. })
    }

    pub fn violated(&self, z: usize) -> bool {
        self.zones.get(z.wrapping_sub(1)).map_or(false, |r| r.open || r.tamper || r.fault)
    }

    pub fn ready(&self) -> bool {
        for z in 1..=self.n() {
            let c = self.cfg(z);
            if c.kind == ZoneKind::Null || c.kind == ZoneKind::NonAlarm || c.kind.is_24h() || self.zones[z - 1].bypassed {
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
            Model::Vista20p => {
                let raw = if kind == ZoneKind::Delay2 {
                    self.ee.entry_or(m, "36", 0, 30)
                } else {
                    self.ee.entry_or(m, "35", 0, 30)
                };
                match raw {
                    97 => 120,
                    98 => 180,
                    99 => 240,
                    other => other,
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
            Model::Vista20p => match self.ee.entry_or(m, "34", 0, 60) {
                97 => 120,
                other => other,
            },
        };
        v.max(1) as u64
    }

    pub fn bell_minutes(&self) -> u64 {
        let m = self.model;
        let v = match m {
            Model::Pc1550 => self.ee.entry_or(m, "17", 2, 4),
            Model::Pc1832 => self.ee.entry_or(m, "005.09", 0, 4),
            Model::Vista20p => match self.ee.entry_or(m, "33", 0, 1) {
                0 => 60,
                n => n * 4,
            },
        };
        v.max(1) as u64
    }

    pub fn swinger_limit(&self) -> u8 {
        match self.model {
            Model::Pc1550 => self.ee.entry_or(self.model, "15", 0, 3).min(99) as u8,
            Model::Pc1832 => self.ee.entry_or(self.model, "377", 0, 3).min(99) as u8,
            // *93: report pairs per zone in an armed period, 1 from the factory.
            Model::Vista20p => self.ee.entry_or(self.model, "93", 0, 1).min(99) as u8,
        }
    }

    pub fn tx_delay(&self) -> u32 {
        match self.model {
            Model::Pc1550 => self.ee.entry_or(self.model, "15", 1, 0),
            Model::Pc1832 => self.ee.entry_or(self.model, "377", 3, 30),
            // *50 burglary dialer delay: 0 none, 1 15 s, 2 30 s (factory), 3 45 s.
            Model::Vista20p => match self.ee.entry_or(self.model, "50", 0, 2) {
                0 => 0,
                1 => 15,
                3 => 45,
                _ => 30,
            },
        }
    }

    pub fn ac_delay_mins(&self) -> u64 {
        match self.model {
            Model::Pc1550 => self.ee.entry_or(self.model, "17", 3, 30) as u64,
            Model::Pc1832 => self.ee.entry_or(self.model, "377", 4, 30) as u64,
            Model::Vista20p => 15,
        }
    }

    pub fn lockout_params(&self) -> (u32, u32) {
        let m = self.model;
        match m {
            Model::Pc1550 => (self.ee.entry_or(m, "36", 0, 0), self.ee.entry_or(m, "36", 1, 0)),
            Model::Pc1832 => (self.ee.entry_or(m, "012", 0, 0), self.ee.entry_or(m, "012", 1, 0)),
            Model::Vista20p => (0, 0),
        }
    }

    pub fn squawk_on(&self) -> bool {
        match self.model {
            Model::Pc1550 => self.ee.toggle(self.model, "13", 3),
            Model::Pc1832 => self.ee.toggle(self.model, "014", 1),
            Model::Vista20p => self.ee.entry_or(self.model, "38", 0, 0) != 0,
        }
    }

    pub fn quick_arm_enabled(&self) -> bool {
        match self.model {
            Model::Pc1550 => self.quick_arm,
            Model::Pc1832 => self.ee.toggle(self.model, "015", 4),
            Model::Vista20p => self.ee.entry_or(self.model, "21", 0, 0) != 0,
        }
    }

    pub fn quick_exit_enabled(&self) -> bool {
        match self.model {
            Model::Pc1550 => self.ee.toggle(self.model, "32", 4),
            Model::Pc1832 => self.ee.toggle(self.model, "015", 3),
            Model::Vista20p => false,
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
        let audible_stay = self.model == Model::Pc1832 && self.ee.toggle(self.model, "022", 8);
        let silent = self.model == Model::Pc1832
            && !audible_stay
            && matches!(how, ArmHow::StayKey | ArmHow::NoEntry);
        let mut secs = self.exit_secs();
        // PC1832 manual: "If the Exit Delay is silent (Section 14, Option 6 or
        // Stay Function Key Arming), the exit delay must be twice the
        // programmed value. It cannot, however, exceed 255 seconds."
        let silent_exit = silent || (self.model == Model::Pc1832 && !self.ee.toggle(self.model, "014", 6));
        if silent_exit {
            secs = (secs * 2).min(255);
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
        self.exit = Some(ExitRt {
            started: now,
            dur: Duration::from_secs(secs),
            how,
            user,
            delay_seen: false,
            delay_closed: false,
            restarted: false,
            silent,
        });
        if self.model == Model::Pc1550 && how != ArmHow::Auto {
            self.beeps(6);
        } else if self.model == Model::Pc1832 && how != ArmHow::Auto {
            if self.ee.toggle(self.model, "381", 4) {
                self.beeps(8);
            } else {
                // Six quick beeps when a code arms it, as recorded on a real
                // PowerSeries keypad (and as the PC1550 manual says too).
                self.beeps(6);
            }
        }
        let user_squawk = self.model == Model::Pc1832
            && user > 0
            && (self.ee.attrs(&format!("{:02}", user)) & (1 << 6)) != 0;
        if self.squawk_on() || user_squawk {
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
            ArmHow::AwayKey => {
                // VISTA-20P Auto-Stay (*84 = 1): if armed Away from keypad and
                // no Entry/Exit door opened during exit delay, auto-convert to Stay.
                if self.model == Model::Vista20p
                    && self.ee.entry_or(Model::Vista20p, "84", 0, 1) != 0
                    && !ex.delay_seen
                {
                    ArmMode::Stay
                } else {
                    ArmMode::Away
                }
            }
            ArmHow::Auto => ArmMode::Away,
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
            reactivate_at: None,
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
            // Always active on the VISTA-20P; field *59 is only its report code.
            Model::Vista20p => true,
        };
        if self.model == Model::Vista20p && self.squawk_on() {
            self.squawk(1);
        }
        for z in 1..=self.n() {
            if !self.violated(z) || self.zones[z - 1].bypassed {
                continue;
            }
            let c = self.cfg(z);
            let kind = c.kind;
            if kind.is_24h() || kind == ZoneKind::Null {
                continue;
            }
            // A force armed zone that's still open is left out until it
            // closes, then it works normally.
            if c.force_arm && !kind.is_delay() {
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
                Model::Vista20p => {}
            }
        }
        let user_squawk = self.model == Model::Pc1832
            && user > 0
            && (self.ee.attrs(&format!("{:02}", user)) & (1 << 6)) != 0;
        if was_on && (self.squawk_on() || user_squawk) {
            self.squawk(2);
        }
        if had_alarm && self.model == Model::Pc1832 && self.ee.toggle(self.model, "381", 1) {
            self.beeps(8);
        } else if was_on {
            if self.model == Model::Vista20p {
                self.vista_confirm_beep();
            } else {
                self.beeps(3);
            }
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
        if self.model == Model::Vista20p {
            self.vista_speak(vec!["disarm_system_now".into()]);
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
        let mut bell = Duration::from_secs(self.bell_minutes() * 60);
        // PC1832 [014] option 8, "Residential Fire Bell is Continuous": a fire
        // alarm rings until it's silenced instead of stopping at bell cut-off.
        if kind.pulsed() && self.model == Model::Pc1832 && self.ee.toggle(self.model, "014", 8) {
            bell = Duration::from_secs(24 * 3600);
        }
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
        if self.model == Model::Vista20p && kind != AlarmKind::Duress {
            self.vista_last_voice_alarm = Some(now);
            self.vista_speak_alarm(kind, zone);
        }
        self.out.push(Out::Save);
        self.dirty = true;
    }

    /// Entering a code while a 24 hour alarm sounds on a disarmed system.
    pub fn silence(&mut self, user: u8) {
        for a in self.alarms.iter_mut() {
            a.silenced = true;
        }
        if self.model == Model::Vista20p {
            self.vista_confirm_beep();
        } else {
            self.beeps(3);
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
            Model::Pc1832 | Model::Vista20p => self.memory[0].clear(),
        }
    }

    /// [*][1] while armed stay. The PC1832 switches to Away and "will add the
    /// Stay/Away zones back into the system after the exit delay time
    /// expires." The PC1550 does the same when its Home-Away zones have delay
    /// (section [32] light 3, on from the factory: "the Bypass light will go
    /// out after the delay"), and right away otherwise. Night zones and zones
    /// you bypassed by hand stay bypassed.
    pub fn reactivate_stay(&mut self, now: Instant) {
        let any = (1..=self.n()).any(|z| self.zones[z - 1].auto_bypass && self.cfg(z).kind != ZoneKind::NightZone);
        let delayed = match self.model {
            Model::Pc1832 | Model::Vista20p => true,
            Model::Pc1550 => self.ee.toggle(self.model, "32", 3),
        };
        let secs = self.exit_secs();
        if let Some(a) = self.armed.as_mut() {
            a.mode = ArmMode::Away;
            if any && delayed {
                a.reactivate_at = Some(now + Duration::from_secs(secs));
            }
        }
        if !any {
            self.key_beep();
        } else if delayed {
            self.beeps(3);
            self.log(EvKind::Arm, format!("Interior zones come back in {} seconds (night arming)", secs));
        } else {
            self.beeps(3);
            self.finish_reactivate(now);
        }
        self.dirty = true;
    }

    fn finish_reactivate(&mut self, now: Instant) {
        let mut back = Vec::new();
        for z in 1..=self.n() {
            if self.zones[z - 1].auto_bypass && self.cfg(z).kind != ZoneKind::NightZone {
                self.zones[z - 1].bypassed = false;
                self.zones[z - 1].auto_bypass = false;
                back.push(z);
            }
        }
        if let Some(a) = self.armed.as_mut() {
            a.reactivate_at = None;
        }
        self.log(EvKind::Arm, "Interior zones active (night arming)");
        self.dirty = true;
        // A zone that's already open when it comes back counts right away.
        for z in back {
            if self.violated(z) {
                self.zone_tripped(z, now);
            }
        }
    }

    /// How long a zone has to stay open or closed before the panel believes
    /// it. PC1550: zone definition digit 1 slow (section [17], 500 ms from
    /// the factory) or fast (10 ms). PC1832: section [030], 400 ms normal or
    /// 36 ms fast.
    pub fn loop_response_ms(&self, z: usize) -> u64 {
        match self.model {
            Model::Pc1550 => {
                let raw: Vec<char> = self.ee.raw(self.model, "11").chars().collect();
                let d1 = raw.get(z.saturating_sub(1) * 2).and_then(|c| c.to_digit(16)).unwrap_or(0);
                if d1 == 2 || d1 == 3 {
                    10
                } else {
                    self.ee.entry_or(self.model, "17", 4, 50).max(1) as u64 * 10
                }
            }
            Model::Pc1832 => {
                if z <= 8 && self.ee.toggle(self.model, "030", z as u8) {
                    36
                } else {
                    400
                }
            }
            Model::Vista20p => {
                let raw = self.ee.raw(self.model, "resp");
                let ch = raw.chars().nth(z.saturating_sub(1)).unwrap_or('1');
                match ch {
                    '0' => 10,
                    '2' => 700,
                    '3' => 1200,
                    _ => 350,
                }
            }
        }
    }

    /// One line of plain help about what the keypad is waiting for, shown
    /// under it. Empty when there's nothing to explain.
    pub fn hint(&self) -> String {
        let lcd = self.model == Model::Pc1832;
        let text: &str = match &self.kp {
            KpMode::Bypass { buf, .. } => {
                if !lcd {
                    "Zone bypass: press a zone number from 1 to 6. Its light turns on while it's bypassed. 9 brings back the last group, # when you're done."
                } else if buf.is_empty() {
                    "Zone bypass: type the zone as two digits, 02 for zone 2, or scroll with < > and press *. B means bypassed. # when you're done."
                } else {
                    "Zone bypass: now type the second digit of the zone number."
                }
            }
            KpMode::Troubles { .. } => {
                if lcd {
                    "Trouble display: scroll with < > and press the number to see details. # goes back."
                } else {
                    "Trouble display: steady lights are troubles (1 battery, 2 AC power, 3 bell, 4 communication, 5 fire loop, 6 clock). A flashing light is a zone whose camera is blocked or offline. # goes back."
                }
            }
            KpMode::Memory { .. } => {
                if lcd {
                    "Alarm memory: scroll with < > to see which zones went into alarm. # goes back."
                } else {
                    "Alarm memory: flashing zone lights went into alarm. Any digit shows older history, # goes back."
                }
            }
            KpMode::NeedCode { .. } => "Enter your 4 digit code. # cancels.",
            KpMode::UserCodes { .. } => {
                if lcd {
                    "Access codes: type the user number as two digits (40 is the master), then the new 4 digit code. * erases. # when you're done."
                } else {
                    "Access codes: press 1 to 6 for the code, then the new 4 digit code. **** erases it (not the master). # when you're done."
                }
            }
            KpMode::UserFn { .. } => {
                if lcd {
                    "User functions: scroll with < > and press *, or press the number. # goes back."
                } else {
                    "User functions: 1 clock, 2 auto arm time, 4 quick arm, 5 auto arm, 6 door chime, 8 bell test, 0 installer test. # goes back."
                }
            }
            KpMode::Program(_) => {
                if lcd {
                    "Installer programming: type a 3 digit section. # backs out. Settings has a plain English list of every section."
                } else {
                    "Installer programming: type a 2 digit section. Zone lights 1 to 4 show each digit in binary. # backs out. Settings lists every section."
                }
            }
            KpMode::Star { .. } => {
                if lcd {
                    "Star menu: scroll with < > and press *, or press the command number. # goes back."
                } else {
                    "Star command: 1 bypass, 2 troubles, 3 memory, 5 codes, 6 user functions, 7 sensor reset, 8 installer, 9 no entry, 0 quick arm or exit."
                }
            }
            KpMode::Normal => {
                if self.entry.is_some() {
                    "Entry delay: enter your code to disarm."
                } else if self.exit.map_or(false, |e| e.how == ArmHow::StayKey || e.how == ArmHow::NoEntry) {
                    "Exit delay: arming in Stay mode."
                } else if self.exit.is_some() {
                    "Exit delay: leave through the doorway to arm Away, or stay in the room and it arms in Stay mode."
                } else {
                    ""
                }
            }
            _ => "Scroll with < >, # goes back.",
        };
        text.to_string()
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
        // PC1832 [021] Opt 1: Access code blocked during entry delay (unless disarmed or no entry)
        if self.model == Model::Pc1832
            && self.entry.is_some()
            && self.ee.toggle(self.model, "021", 1)
            && !eeprom::is_master(self.model, &self.ee, &slot)
        {
            self.error_tone();
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
            // PC1832 [008] Maintenance code can arm, but cannot disarm unless in alarm.
            if self.model == Model::Pc1832 && slot == "maint" && !self.alarm_sounding(now) {
                self.error_tone();
                self.temp("Code Not Valid", "To Disarm", 2000);
                return;
            }
            let one_time = self.model == Model::Pc1832 && (self.ee.attrs(&slot) & (1 << 7)) != 0;
            self.disarm(user, now);
            if one_time {
                self.ee.clear_code(&slot);
                self.log(EvKind::Program, format!("One-time code {} used and cleared", slot));
                self.out.push(Out::Save);
            }
            if self.entry.is_none() {
                self.out.push(Out::LearnFace);
            }
        } else if self.alarms.iter().any(|a| !a.silenced) {
            self.silence(user);
        } else {
            let armed = self.arm(ArmHow::Code, user, now);
            // PC1550 [13] Light 5: 6th user code is Maid's (one-time) code, erased after arming.
            if armed && self.model == Model::Pc1550 && slot == "6" && self.ee.toggle(self.model, "13", 5) {
                self.ee.clear_code("6");
                self.log(EvKind::Program, "One-time code 6 used to arm and cleared");
                self.out.push(Out::Save);
            }
        }
    }

    pub fn bad_code(&mut self, now: Instant) {
        // On the Vista a wrong code just does nothing (its key beeps only).
        if self.model != Model::Vista20p {
            self.error_tone();
        }
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
        } else if self.bad_codes == 3 {
            // RoomGuard's own touch, not the panel's: a photo of whoever is
            // guessing, even with the keypad lockout off the way it ships.
            self.out.push(Out::Snapshot("wrong-codes".to_string()));
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
            CodeFor::UserCodes => match slot {
                Some(s) if eeprom::is_master(self.model, &self.ee, &s) => {
                    self.bad_codes = 0;
                    self.beeps(2);
                    KpMode::UserCodes { slot: None, buf: String::new(), idx: 0, attrs: false }
                }
                _ => {
                    self.bad_code(now);
                    KpMode::Normal
                }
            },
            CodeFor::UserFns => {
                let allow_all = self.model == Model::Pc1832 && self.ee.toggle(self.model, "019", 7);
                match slot {
                    Some(s) if s != "inst" && (allow_all || eeprom::is_master(self.model, &self.ee, &s)) => {
                        self.bad_codes = 0;
                        self.beeps(2);
                        KpMode::UserFn { item: None, buf: String::new(), idx: 0 }
                    }
                    _ => {
                        self.bad_code(now);
                        KpMode::Normal
                    }
                }
            }
            CodeFor::NoEntry | CodeFor::Bypass | CodeFor::Troubles | CodeFor::Memory | CodeFor::Download | CodeFor::CmdOut | CodeFor::Arm(_) => match slot {
                Some(s) if s != "inst" => {
                    self.bad_codes = 0;
                    let user = eeprom::slot_user(&s);
                    match purpose {
                        CodeFor::Bypass => {
                            let allowed = self.model != Model::Pc1832
                                || eeprom::is_master(self.model, &self.ee, &s)
                                || (self.ee.attrs(&s) & (1 << 2)) != 0
                                || self.ee.attrs(&s) == 0;
                            if allowed {
                                KpMode::Bypass { buf: String::new(), idx: 0 }
                            } else {
                                self.error_tone();
                                KpMode::Normal
                            }
                        }
                        CodeFor::Troubles => {
                            self.ack_troubles();
                            KpMode::Troubles { idx: 0, sub: 0, mem: false }
                        }
                        CodeFor::Memory => KpMode::Memory { level: 0, idx: 0 },
                        CodeFor::Download => {
                            self.beeps(3);
                            self.log(EvKind::Info, "Downloading call requested, but no computer is set up to answer");
                            KpMode::Normal
                        }
                        CodeFor::CmdOut => {
                            self.util_until = Some(now + Duration::from_secs(5));
                            self.sensor_reset(now);
                            KpMode::Normal
                        }
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
            Model::Pc1550 | Model::Vista20p => 2,
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
                        if p.confirm_action == Some("989") {
                            p.reset_stage = 0;
                            p.confirm_action = None;
                            self.ee.set_code("40", "1234");
                            self.beeps(6);
                            self.temp("Master Code", "Reset to 1234", 2500);
                            self.log(EvKind::Program, "Master code defaulted to 1234 ([989])");
                            self.out.push(Out::Save);
                        } else {
                            p.reset_stage = 2;
                        }
                    } else {
                        self.error_tone();
                        p.reset_stage = 0;
                        p.confirm_action = None;
                    }
                    return KpMode::Program(p);
                }
                if p.reset_stage == 2 {
                    if p.sel.len() < 3 {
                        return KpMode::Program(p);
                    }
                    let confirm = std::mem::take(&mut p.sel);
                    let act = p.confirm_action.take();
                    p.reset_stage = 0;
                    if act == Some(confirm.as_str()) {
                        match confirm.as_str() {
                            "999" => {
                                self.factory_reset();
                                return KpMode::Normal;
                            }
                            "990" => {
                                self.ee.installer_lockout = true;
                                self.beeps(6);
                                self.temp("Inst Lockout", "Enabled", 2500);
                                self.log(EvKind::Program, "Installer's lockout enabled ([990])");
                                self.out.push(Out::Save);
                                return KpMode::Program(p);
                            }
                            "991" => {
                                self.ee.installer_lockout = false;
                                self.beeps(6);
                                self.temp("Inst Lockout", "Disabled", 2500);
                                self.log(EvKind::Program, "Installer's lockout disabled ([991])");
                                self.out.push(Out::Save);
                                return KpMode::Program(p);
                            }
                            _ => {}
                        }
                    }
                    self.error_tone();
                    return KpMode::Program(p);
                }
                if let Some(parent) = p.sub_for.clone() {
                    let need = if parent == "000" { 1 } else { 2 };
                    if p.sel.len() < need {
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
                    p.confirm_action = None;
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
                    "900" => {
                        self.beeps(3);
                        self.temp("Panel Version", "0460 (v4.60)", 3500);
                        self.log(EvKind::Program, "Panel version displayed: 0460 (v4.60)");
                    }
                    "901" => {
                        self.walk_test = !self.walk_test;
                        if self.walk_test {
                            self.beeps(3);
                            self.temp("Installer Walk", "Test Enabled", 2500);
                            self.log(EvKind::Program, "Installer walk test enabled ([901])");
                        } else {
                            self.long_beep();
                            self.temp("Installer Walk", "Test Disabled", 2500);
                            self.log(EvKind::Program, "Installer walk test disabled ([901])");
                        }
                    }
                    "902" => {
                        self.beeps(3);
                        self.temp("Module Superv.", "Reset Complete", 2500);
                        self.log(EvKind::Program, "Module supervision reset ([902])");
                    }
                    "903" => {
                        self.beeps(3);
                        self.temp("Supervision", "01 PK5500 Keypad", 3500);
                        self.log(EvKind::Program, "Supervision field viewed: Slot 01 PK5500");
                    }
                    "989" => {
                        self.beeps(2);
                        p.reset_stage = 1;
                        p.confirm_action = Some("989");
                    }
                    "990" => {
                        self.beeps(2);
                        p.reset_stage = 1;
                        p.confirm_action = Some("990");
                    }
                    "991" => {
                        self.beeps(2);
                        p.reset_stage = 1;
                        p.confirm_action = Some("991");
                    }
                    "999" => {
                        self.beeps(2);
                        p.reset_stage = 1;
                        p.confirm_action = Some("999");
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
        let prev = if self.model == Model::Pc1832 && key == "001" {
            Some(self.ee.raw(self.model, "001"))
        } else {
            None
        };
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
        // PC1832 manual: changing a zone's definition in [001] auto-defaults its [101]-[108] attributes.
        if let Some(old) = prev {
            let new_s: String = work.iter().collect();
            for z in 1..=8usize {
                let i = (z - 1) * 2;
                let o = old.get(i..i + 2);
                let n = new_s.get(i..i + 2);
                if o != n {
                    if let Some(code) = n.and_then(|s| s.parse::<u32>().ok()) {
                        self.ee.sections.insert(format!("{:03}", 100 + z), Eeprom::default_zone_attrs(code).to_string());
                    }
                }
            }
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
            Model::Vista20p => {
                if !opened {
                    return;
                }
                // "Three tones sound at the keypad whenever a perimeter door or
                // window is opened", and voice chime (if on) says which one.
                self.beeps(3);
                if self.vista_voice_on {
                    let words = voice::filter_zone_words(&self.label(z));
                    if words.is_empty() {
                        self.vista_speak(vec!["fault".into()]);
                    } else {
                        self.vista_speak(words);
                    }
                }
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
            if self.model == Model::Vista20p {
                // "Open each protected door and window in turn and listen for
                // three beeps from the keypad", with the zone on the display.
                self.beeps(3);
                let lbl = self.label(z).to_ascii_uppercase();
                self.vista_test_fault(z as u8, &lbl);
            } else {
                self.walk_burst_until = Some(now + Duration::from_secs(2));
            }
            return;
        }
        if c.kind == ZoneKind::NonAlarm {
            return;
        }
        if c.kind == ZoneKind::DayZone && self.armed.is_none() && self.exit.is_none() {
            self.beeps(3);
            self.log(EvKind::Trouble, format!("Day zone {} ({}) opened while disarmed", z, self.label(z)));
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
                ZoneKind::H24(H24::Supervisory) | ZoneKind::H24(H24::FireSupervisory) => (false, false),
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
                let restart_on = match self.model {
                    Model::Pc1832 => self.ee.toggle(self.model, "018", 7),
                    // *91 option 8, on from the factory: "reopening and closing the
                    // entry/exit door before exit delay time expires will reset the
                    // exit delay time once."
                    Model::Vista20p => (self.ee.entry_or(self.model, "91", 0, 8) & 8) != 0,
                    Model::Pc1550 => false,
                };
                if restart_on && ex.delay_closed && !ex.restarted {
                    let secs = self.exit_secs();
                    if let Some(ex2) = self.exit.as_mut() {
                        ex2.started = now;
                        ex2.dur = Duration::from_secs(secs);
                        ex2.restarted = true;
                        ex2.delay_closed = false;
                    }
                    self.log(EvKind::Arm, "Exit delay restarted");
                } else {
                    ex.delay_seen = true;
                }
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
        let interior_delay_stay = c.kind == ZoneKind::InteriorDelay && a.mode == ArmMode::Stay;
        if c.kind.is_delay() || home_away_delay || interior_delay_stay {
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
            ZoneKind::Instant | ZoneKind::InstantStayAway | ZoneKind::DayZone => {
                self.trigger(AlarmKind::Burglary, z as u8, c.audible, false, now);
            }
            ZoneKind::Interior | ZoneKind::StayAway | ZoneKind::InteriorDelay | ZoneKind::NightZone => {
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
        let is_delay = self.cfg(z).kind.is_delay();
        if let Some(ex) = self.exit.as_mut() {
            if is_delay && ex.delay_seen {
                ex.delay_closed = true;
            }
        }
        // GuardSeries exit delay termination: once you're out and the door
        // closes, the remaining exit time drops to five seconds.
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
            Model::Vista20p => self.press_vista(key, now),
        }
        // PowerSeries keypads: "These buttons can also be activated by
        // pressing and holding numbers [1] through [5] respectively for 2
        // seconds." Only the first digit of a code can start a hold.
        if self.model == Model::Pc1832 && matches!(key, Key::Digit(1..=5)) && matches!(self.kp, KpMode::Normal) && self.buf.len() == 1 {
            self.held = Some(HeldKey { key, since: now, fired: false });
        }
    }

    fn hold_complete(&mut self, key: Key, now: Instant) {
        if key.is_emergency() {
            self.emergency(key, now);
            // An emergency key cancels whatever menu was open.
            if !self.is_programming() {
                self.kp = KpMode::Normal;
                self.buf.clear();
            }
            return;
        }
        if self.model == Model::Vista20p {
            // Vista Function Key D (mapped to Key::Away): check field *57 entry 3
            if key == Key::Away {
                // *57 function codes. Single button arming needs no code.
                let code = self.ee.entry_or(Model::Vista20p, "57", 3, 0);
                match code {
                    3 => self.vista_arm_key(2, now),
                    4 => self.vista_arm_key(3, now),
                    5 => {
                        self.vista_arm_key(3, now);
                        if self.exit.is_some() || self.armed.is_some() {
                            self.vista_set_night();
                        }
                    }
                    6 => {
                        // Step arming: STAY, then NIGHT-STAY, then AWAY.
                        if self.armed.is_none() && self.exit.is_none() {
                            self.vista_arm_key(3, now);
                        } else if !self.vista_night {
                            self.vista_set_night();
                        } else {
                            self.vista_step_to_away();
                        }
                    }
                    8 => self.vista_comm_test(),
                    2 => self.vista_speak_status(),
                    _ => self.vista_page(),
                }
                return;
            }
            return;
        }
        // A held 1 to 5 on the PowerSeries is function key 1 to 5.
        let key = match key {
            Key::Digit(d) if self.model == Model::Pc1832 && (1..=5).contains(&d) => {
                self.buf.clear();
                match d {
                    1 => Key::Stay,
                    2 => Key::Away,
                    3 => Key::Chime,
                    4 => Key::Reset,
                    _ => Key::Exit,
                }
            }
            other => other,
        };
        // Function keys don't work inside a [*] menu; the keypad says so.
        if !matches!(self.kp, KpMode::Normal) {
            self.error_tone();
            return;
        }
        let (fn_idx, default_code) = match key {
            Key::Stay => (1u8, 3u32),
            Key::Away => (2, 4),
            Key::Chime => (3, 6),
            Key::Reset => (4, 14),
            Key::Exit => (5, 16),
            _ => return,
        };
        let code = if self.model == Model::Pc1832 {
            self.ee.entry_or(Model::Pc1832, &format!("000.{}", fn_idx), 0, default_code)
        } else {
            default_code
        };
        match code {
            0 => {} // Null function key
            3 => self.fn_arm(ArmHow::StayKey, now),
            4 => self.fn_arm(ArmHow::AwayKey, now),
            5 => {
                if self.armed.is_none() && self.exit.is_none() {
                    self.kp = KpMode::NeedCode { purpose: CodeFor::NoEntry, buf: String::new() };
                } else {
                    self.error_tone();
                }
            }
            6 => self.toggle_chime(),
            7 => self.system_test(now),
            8 => {
                if self.armed.is_none() && self.exit.is_none() {
                    self.kp = KpMode::Bypass { buf: String::new(), idx: 0 };
                } else {
                    self.error_tone();
                }
            }
            9 => {
                self.ack_troubles();
                self.kp = KpMode::Troubles { idx: 0, sub: 0, mem: false };
            }
            10 => {
                self.kp = KpMode::Memory { level: 0, idx: 0 };
            }
            14 => self.sensor_reset(now),
            16 => self.quick_exit(now),
            17 => {
                if self.armed.is_some() {
                    self.reactivate_stay(now);
                } else {
                    self.error_tone();
                }
            }
            _ => self.error_tone(),
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
                if m == Model::Vista20p {
                    self.beeps(2);
                    self.trigger(AlarmKind::Fire, 95, true, false, now);
                    return;
                }
                let enabled = match m {
                    Model::Pc1550 => !self.ee.toggle(m, "34", 6),
                    Model::Pc1832 => self.ee.toggle(m, "015", 1) && self.ee.toggle(m, "kp074", 1),
                    Model::Vista20p => true,
                };
                if !enabled {
                    self.error_tone();
                    return;
                }
                let beeps_only = m == Model::Pc1832 && self.ee.toggle(m, "023", 1);
                if beeps_only {
                    self.beeps(3);
                } else {
                    self.audio(AudioCmd::Beeps { count: 10, on_ms: 50, off_ms: 50 });
                }
                self.trigger(AlarmKind::Fire, 0, true, false, now);
            }
            Key::Aux => {
                if m == Model::Vista20p {
                    self.beeps(2);
                    // VISTA-20P Zone 96 (Type 08 Auxiliary): keypad alarm, external bell silent.
                    self.trigger(AlarmKind::Medical, 96, false, true, now);
                    return;
                }
                if m == Model::Pc1832 && !self.ee.toggle(m, "kp074", 2) {
                    self.error_tone();
                    return;
                }
                self.audio(AudioCmd::Beeps { count: 10, on_ms: 50, off_ms: 50 });
                self.trigger(AlarmKind::Medical, 0, false, false, now);
            }
            Key::Panic => {
                if m == Model::Vista20p {
                    self.beeps(2);
                    self.trigger(AlarmKind::Panic, 99, true, false, now);
                    return;
                }
                if m == Model::Pc1832 && !self.ee.toggle(m, "kp074", 3) {
                    self.error_tone();
                    return;
                }
                let audible = match m {
                    Model::Pc1550 => self.ee.toggle(m, "12", 6),
                    Model::Pc1832 | Model::Vista20p => self.ee.toggle(m, "015", 2),
                };
                let feedback = match m {
                    Model::Pc1550 => self.ee.toggle(m, "14", 5),
                    Model::Pc1832 | Model::Vista20p => audible,
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
            Model::Vista20p => 2,
        };
        self.log(EvKind::Disarm, format!("Recognized {} at the camera", name));
        self.disarm(master, now);
        true
    }

    // ----- the clock tick -----

    pub fn tick(&mut self, now: Instant, wall_ms: i64) {
        self.tick_vista(now);
        if self.armed.map_or(false, |a| a.reactivate_at.map_or(false, |t| now >= t)) {
            self.finish_reactivate(now);
        }
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
        if self.model == Model::Pc1550 {
            let fire_trouble = (1..=self.n()).any(|z| {
                self.cfg(z).kind.is_fire() && (self.zones[z - 1].fault || self.zones[z - 1].tamper)
            });
            if fire_trouble {
                t |= tb::FIRE_LOOP;
            }
        }
        if self.model == Model::Pc1832 && i.net_watch && !i.net_ok {
            t |= tb::TLM;
        }
        // Both panels report a camera that's offline or covered. On the
        // PowerSeries these are its real zone fault and tamper troubles. The
        // PC1550 has no such troubles, so RoomGuard adds them: the Trouble
        // light, the trouble beeps, and in [*][2] the camera's zone light
        // flashes while the six real trouble lights keep their meanings.
        if i.cam_enabled {
            if !i.cam_online {
                t |= tb::ZONE_FAULT;
            }
            if i.cam_covered {
                t |= tb::ZONE_TAMPER;
            }
        }
        if t == self.troubles {
            if t & tb::AC != 0 && !self.ac_reported {
                let delay_min = self.ac_delay_mins();
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
        // New troubles start the trouble beeps again. AC loss stays quiet unless
        // PC1832 [018] Opt 8 (AC Trouble Beeps) is enabled.
        let ac_beeps = self.model == Model::Pc1832 && self.ee.toggle(self.model, "018", 8);
        if ac_beeps {
            self.trouble_ack &= !new;
        } else {
            self.trouble_ack &= !(new & !tb::AC);
            self.trouble_ack |= new & tb::AC;
        }
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
        let mut buzzer_alarm = false;
        for a in self.live_alarms(now) {
            if a.kind.pulsed() && (a.audible || a.buzzer) {
                fire = true;
            } else if a.buzzer {
                buzzer_alarm = true;
            }
        }
        if fire {
            if self.model == Model::Vista20p {
                // 6160V Fire alarm: loud pulsing Temporal-3 on keypad speaker.
                return Pattern::Beeps { on_ms: 500, off_ms: 500, count: 3, period_ms: 4000 };
            }
            return Pattern::Beeps { on_ms: 500, off_ms: 500, count: 1, period_ms: 1000 };
        }
        // Vista 6160V burglary alarm speaker: continuous slow alternating
        // two-tone hi-lo "dee-doo dee-doo" (~1960 Hz <-> ~1040 Hz, ~420 ms per tone),
        // muted for 10s if any key is pressed while external bell keeps ringing.
        if self.model == Model::Vista20p {
            // "LOUD, CONTINUOUS" in the user guide means it never pauses. A
            // recording of a real 6160 shows the sound itself: 1360 Hz and
            // 1019 Hz taking turns every 505 ms with no gap. Entering a code
            // quiets the keypad.
            let muted = self.vista_sounder_mute_until.map_or(false, |t| now < t);
            let burg = self.live_alarms(now).any(|a| (a.audible || a.buzzer) && !a.kind.pulsed());
            if burg {
                return if muted { Pattern::Off } else { Pattern::HiLo { hi_hz: 1360, lo_hz: 1019, step_ms: 505 } };
            }
        }
        if buzzer_alarm {
            return Pattern::Steady;
        }
        if self.model == Model::Pc1832
            && self.ee.toggle(self.model, "018", 5)
            && self.bell_wanted(now) != BellPattern::Off
        {
            return Pattern::Steady;
        }
        if let Some(e) = &self.entry {
            let left = (e.started + e.dur).saturating_duration_since(now).as_secs_f32();
            if self.model == Model::Vista20p {
                // Slow beeping, then rapid beeping for the last ten seconds, as
                // a video of a real 6160 shows (the user guide only lists the
                // slow part). Coming home after an alarm beeps rapidly all the
                // way. The beeps stop once a code is in.
                if self.vista_sounder_mute_until.map_or(false, |t| now < t) {
                    return Pattern::Off;
                }
                if self.memory_flag || left <= 10.0 {
                    // Measured: 90 ms on, 70 ms off.
                    return Pattern::Beeps { on_ms: 90, off_ms: 70, count: 1, period_ms: 160 };
                }
                // The same slow beeping as the exit warning: 500 ms on, 510 off.
                return Pattern::Beeps { on_ms: 500, off_ms: 0, count: 1, period_ms: 1010 };
            }
            let urgency = match self.model {
                Model::Pc1550 => self.ee.toggle(self.model, "34", 4),
                Model::Pc1832 | Model::Vista20p => true,
            };
            if urgency && left <= 10.0 {
                // Both PC1550 (v4.1 [34] light 4) and PC1832 emit the pulsing
                // double-beep tone during the last 10 seconds: beep, beep, beeeep.
                return Pattern::ShortShortLong { short_ms: 94, gap_ms: 26, long_ms: 731 };
            }
            return Pattern::Steady;
        }
        if let Some(ex) = &self.exit {
            let left = (ex.started + ex.dur).saturating_duration_since(now).as_secs_f32();
            match self.model {
                Model::Pc1550 => {
                    if self.ee.toggle(self.model, "34", 1) {
                        return if left <= 10.0 {
                            Pattern::Beeps { on_ms: 70, off_ms: 50, count: 3, period_ms: 996 }
                        } else {
                            Pattern::Beeps { on_ms: 70, off_ms: 0, count: 1, period_ms: 996 }
                        };
                    }
                }
                Model::Pc1832 => {
                    // [014] option 6 "Audible Exit with Urgency" (on from the
                    // factory): once a second, then three times a second for
                    // the last ten. Off is "Silent Exit Delay". Stay key arming
                    // stays silent unless [022] option 8 is on.
                    if !ex.silent && self.ee.toggle(self.model, "014", 6) {
                        // As recorded on a real PowerSeries keypad: one 70 ms beep
                        // every 996 ms, and for the last ten seconds a burst of
                        // three quick beeps each second ("3 times a second").
                        if left <= 10.0 {
                            return Pattern::Beeps { on_ms: 70, off_ms: 50, count: 3, period_ms: 996 };
                        }
                        return Pattern::Beeps { on_ms: 70, off_ms: 0, count: 1, period_ms: 996 };
                    }
                }
                Model::Vista20p => {
                    // Vista *37 Audible Exit Warning: 1 Hz single beeps (150ms ON / 850ms OFF),
                    // ramping to ~4 Hz urgency (100ms ON / 150ms OFF) in final 10s.
                    if !ex.silent && self.ee.entry_or(self.model, "37", 0, 1) != 0 {
                        if left <= 10.0 {
                            // The same rapid beeping as the end of the entry delay.
                            return Pattern::Beeps { on_ms: 90, off_ms: 70, count: 1, period_ms: 160 };
                        }
                        // Measured from a real 6160: 500 ms on, 510 ms off.
                        return Pattern::Beeps { on_ms: 500, off_ms: 0, count: 1, period_ms: 1010 };
                    }
                }
            }
        }
        if self.auto_pre.is_some() {
            return Pattern::Steady;
        }
        if self.model == Model::Vista20p && self.walk_test {
            // Test mode: "ONE SHORT BEEP about every 30 seconds", then a double
            // beep every 30 seconds for the last five minutes before it ends
            // itself at four hours.
            let late = self.vista_test_since.map_or(false, |t| now.saturating_duration_since(t).as_secs() >= 4 * 3600 - 300);
            return Pattern::Beeps { on_ms: 100, off_ms: 120, count: if late { 2 } else { 1 }, period_ms: 30_000 };
        }
        let unacked = self.troubles & !self.trouble_ack;
        let silent_troubles = self.model == Model::Pc1832 && self.ee.toggle(self.model, "023", 7);
        if unacked != 0 && !silent_troubles && self.armed.is_none() && !self.is_programming() {
            if self.model == Model::Vista20p {
                // "RAPID BEEPING: trouble (CHECK), system low battery, bell
                // failure." Any key silences it; the display stays.
                return Pattern::Beeps { on_ms: 90, off_ms: 70, count: 1, period_ms: 160 };
            }
            return Pattern::Beeps { on_ms: 100, off_ms: 120, count: 2, period_ms: 10_000 };
        }
        Pattern::Off
    }

    fn bell_wanted(&self, now: Instant) -> BellPattern {
        if self.all_on(now) || self.walk_burst_until.map_or(false, |t| now < t) {
            return BellPattern::Steady;
        }
        let mut fire = false;
        let mut co = false;
        let mut burg = false;
        for a in self.live_alarms(now) {
            if !a.audible {
                continue;
            }
            if a.kind.pulsed() {
                fire = true;
            } else if a.kind.is_co() {
                co = true;
            } else {
                burg = true;
            }
        }
        if fire {
            return match self.model {
                Model::Pc1832 => {
                    // [013] option 8: Temporal Three, otherwise standard pulsed.
                    if self.ee.toggle(self.model, "013", 8) {
                        BellPattern::Temporal3
                    } else {
                        BellPattern::Pulsed
                    }
                }
                Model::Pc1550 => {
                    // PC1550 [32] Light 6 ON = Standard Pulsed Fire, OFF = Temporal 3 Fire.
                    if self.ee.toggle(self.model, "32", 6) {
                        BellPattern::Pulsed
                    } else {
                        BellPattern::Temporal3
                    }
                }
                Model::Vista20p => BellPattern::Temporal3,
            };
        }
        if co {
            return BellPattern::Temporal4;
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
            Model::Pc1832 | Model::Vista20p => self.auto_arm_hm,
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
                Model::Vista20p => false,
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

    /// When a camera zone is open because of the camera itself. A real PC1550
    /// has no zone tamper or fault trouble, so a covered camera shows on its
    /// keypad as an open zone, exactly like a cut wire. This says why.
    pub fn camera_note(&self, z: usize) -> &'static str {
        let cam = self.camera_zone.get(z.wrapping_sub(1)).copied().unwrap_or(false);
        if !cam || !self.inputs.cam_enabled {
            ""
        } else if self.inputs.cam_covered {
            "Camera blocked"
        } else if !self.inputs.cam_online {
            "Camera offline"
        } else {
            ""
        }
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
                    note: self.camera_note(z).to_string(),
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
        if let VistaKp::Program { field, .. } = &self.vista_mode {
            let what = match field {
                Some(f) => format!("Field *{}", f),
                None => "Enter *field or *99 to exit".to_string(),
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
            let detail = if self.memory_flag {
                "An alarm happened while armed"
            } else if self.inputs.cam_enabled && self.inputs.cam_covered {
                "The camera's view is blocked"
            } else if self.inputs.cam_enabled && !self.inputs.cam_online {
                "The camera stopped sending pictures"
            } else {
                "Watching the room"
            };
            return (StatusKind::Armed, title.into(), detail.into());
        }
        let open = self.open_zones();
        if !open.is_empty() {
            let z = open[0] as usize;
            let more = if open.len() > 1 { format!(" and {} more", open.len() - 1) } else { String::new() };
            let why = match self.camera_note(z) {
                "Camera blocked" => " because the camera's view is blocked",
                "Camera offline" => " because the camera stopped sending pictures",
                _ => "",
            };
            return (StatusKind::NotReady, "Not ready".into(), format!("Zone {} {} is open{}{}", z, self.label(z), why, more));
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
            (tb::FIRE_LOOP, "Fire circuit trouble"),
            (tb::CLOCK, "Loss of time"),
            (tb::ZONE_FAULT, "Camera offline"),
            (tb::ZONE_TAMPER, "Camera view blocked"),
        ],
        Model::Pc1832 | Model::Vista20p => vec![
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
        tb::FIRE_LOOP => 373,
        tb::ZONE_FAULT => 380,
        tb::ZONE_TAMPER => 383,
        _ => 300,
    }
}
