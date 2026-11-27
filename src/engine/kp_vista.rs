//! VistaGuard 20P (VISTA-20P compatible) + VG6160V Alpha Display Voice Keypad.
//!
//! Operates on the authentic [Code] + [Action Key] firmware philosophy:
//!   - [Code] + 1 (OFF): Disarm / Silence (two-step: 1st silences & latches
//!     alarm memory, 2nd clears alarm memory, resets smoke detectors, & flashes Alarm Canceled)
//!   - [Code] + 2 (AWAY): Arm Away with Exit Delay + Auto-Stay (*84)
//!   - [Code] + 3 (STAY): Arm Stay (press 3 again within 5 s for NIGHT-STAY)
//!   - [Code] + 4 (MAX): Arm Away with NO Entry Delay
//!   - [Code] + 5 (TEST) + 0 / 1: Walk Test / Dialer Test (PHONE OKAY)
//!   - [Code] + 6 (BYPASS) + [01-48] or [#]: Manual or Quick Bypass (*23)
//!   - [Code] + 7 (INSTANT): Arm Stay with NO Entry Delay
//!   - [Master] + 8 + [02-49]: User codes & attributes (#0 delete, #1 authority 0-4, #2 group, #3 part, #5 pager)
//!   - [Code] + 9 (CHIME): Toggle Chime Mode (*26 Chime by Zone supported)
//!   - [Master/Inst] + # + 60: 100-Event History Log Viewer (* older, # newer)
//!   - [Master/Inst] + # + 63: View & Set Real-Time Clock (* to edit)
//!   - [Master/Inst] + # + 64: Scheduling Mode (01-16 user, 17-32 installer)
//!   - [Master/Inst] + # + 66: Macro Key Programming (Macros 1-4)
//!   - [Code] + # + 7 / 8 + [01-18]: Activate / Deactivate Output Devices
//!   - [Code] + * + [0-3]: GoTo Partition; [Code] + 0 + [1/2/3/4/7]: Multi-Partition command
//!   - [*] (READY): View faulted zones / [#] + 0 + [*] voice status (press [*] again within 10 s for up to 3 faults)
//!   - [#] + [2/3/4/7]: Quick Arm when *21 = 1
//!   - [#] + 0 + 1 / [1] / [#] + 0 + 3: 6160V Voice Message Center (Record / Stop / Play)
//!   - [#] + 0 + 2: 6160V Voice Volume control (3 = Up, 6 = Down, 4 = Voice On/Off, # = Save)
//!   - [Installer 4112] + 8 + 0 + 0: Installer Programming (*field / #field / *97 / *98 / *99)
//!   - [Installer 4112] + 8 + 0 + 2 + [4-digit]: Set Master Code (single entry)
//!   - [Installer 4112] + # + 0 + [0/1/2]: Periodic Test Schedule (24h / 7d / 28d)
//!   - Hold A (Fire 95), B (Police 99), C (Medical 96), D (Away / Programmable *57 including 02 Time/Date, 07 Output, 09-12 Macros)

use std::time::{Duration, Instant};

use chrono::Datelike;

use super::eeprom::{self, Kind};
use super::kp1832::{pad, LcdOut};
use super::panel::*;
use super::types::*;
use crate::audio::voice;
use crate::audio::AudioCmd;
use crate::util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedStep {
    SelectNum,
    Event,
    Device,
    StartTime,
    Days,
    Repeat,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum ZoneListStep {
    #[default]
    Select,
    Add,
    DeleteList,
    DeleteZones,
    Remove,
}

#[derive(Clone, Debug, Default)]
pub struct ZoneListMenu {
    step: ZoneListStep,
    list: u8,
    digits: String,
}

#[derive(Clone, Debug, Default)]
pub enum VistaKp {
    #[default]
    Normal,
    /// Waiting for action digit (1-9, #, *, 0) after a valid 4-digit code or '#' quick-arm.
    Command { slot: String, user: u8, quick: bool },
    /// [Code] + #: waiting for 60 (Event Log), 63 (Time/Date), 64 (Schedules), 66 (Macros),
    /// 7 (Output On), 8 (Output Off), or 00/01/02 (Installer periodic test).
    HashCmd { slot: String, user: u8, buf: String },
    /// [Code] + 6: waiting for a 2-digit zone number (or '#' for quick bypass).
    /// With no digits, the bypassed zones are shown after a moment.
    Bypass { buf: String, at: Instant },
    /// [Code] + 5 (TEST): 0 starts the walk test, 1 the dialer test.
    TestWait { user: u8 },
    /// [Master/Inst] + 8 + [2-digit user] + [4-digit code], or # 0/1/2/3/5 attributes.
    /// Master's own code (user 02) is entered twice by Master, once by Installer.
    UserCode { slot: String, buf: String, first: String, by_inst: bool, caller_is_master02: bool },
    /// Installer programming: [Installer] + 8 + 0 + 0.
    Program { field: Option<String>, buf: String, review: bool },
    ZoneList(ZoneListMenu),
    /// [#] + 0 + 2: voice volume (3 up, 6 down, 4 voice on or off, # done).
    Volume,
    /// Just armed STAY: pressing STAY again makes it NIGHT-STAY.
    NightWait { at: Instant },
    /// [Master] + # + 63 (view/edit) or *57 option 02 (view-only) Real-Time Clock.
    TimeDate { can_edit: bool, editing: bool, digits: String, pos: usize, at: Instant },
    /// [Master/Inst] + # + 60: 100-event history log viewer (* older, # newer).
    EventLog { idx: usize, at: Instant },
    /// [Master/Inst] + # + 64: user (01-16) / installer (17-32) scheduling mode.
    Schedule {
        is_inst: bool,
        step: SchedStep,
        input: String,
        num: u8,
        event: u8,
        device: u8,
        start_buf: String,
        days_buf: String,
        pos: usize,
        at: Instant,
    },
    /// [Master/Inst] + # + 66: Macro 1-4 programming.
    MacroPgm { slot: Option<usize>, viewing: bool, buf: String, at: Instant },
    /// Holding a *57 Macro key (09-12) prompts for a valid 4-digit user code within 30 s.
    MacroCode { slot: usize, buf: String, at: Instant },
    /// [Code] + # + 7 (activate) or # + 8 (deactivate) + [01-18] output device.
    OutputCmd { activate: bool, user: u8, buf: String },
    /// [Code] + * + [0-3]: GoTo Partition.
    GoToPart,
    /// [Code] + 0 + [1/2/3/4/7]: Multi-Partition global command.
    MultiPart { slot: String, user: u8 },
}

fn plain(a: &str, b: &str) -> LcdOut {
    LcdOut { lines: [pad(a), pad(b)], cursor: None }
}

fn with_cursor(a: &str, b: &str, row: usize, col: usize) -> LcdOut {
    LcdOut { lines: [pad(a), pad(b)], cursor: Some((row, col.min(15))) }
}

/// Cursor LCD columns on Line 2 for `#63` edit format `04:04P2026/11/26` (11 editable digits).
const TIME_COLS: [usize; 11] = [0, 1, 3, 4, 5, 8, 9, 11, 12, 14, 15];

impl Panel {
    /// Speaks a sequence of 6160V voice ROM tokens unconditionally (alarms, volume test).
    pub fn vista_speak(&mut self, tokens: Vec<String>) {
        if self.model == Model::Vista20p && !tokens.is_empty() {
            self.audio(AudioCmd::Speak(tokens));
        }
    }

    /// The 6160's arming confirmation, filmed on a real VISTA-20P: three low
    /// "tu-tu-tu" beeps at 1019 Hz (the disarm beep's sound), each a little
    /// longer than the last (about 150, 175 and 240 ms, with 50 and 75 ms
    /// gaps), starting about 0.55 s after the key. The voice follows about
    /// half a second after the last one, and the exit delay counts down quietly.
    pub fn vista_arm_beeps(&mut self) {
        self.audio(AudioCmd::ToneSeq { delay_ms: 550, tones: vec![(1019, 150, 50), (1019, 175, 75), (1019, 240, 500)] });
    }

    /// Speaks a status/chime/arming phrase only when 6160V voice status is ON (#+0+2+4).
    pub fn vista_speak_if_on(&mut self, tokens: Vec<String>) {
        if self.vista_voice_on {
            self.vista_speak(tokens);
        }
    }

    /// Speaks an alarm announcement using the 6160V ROM whitelist.
    pub fn vista_speak_alarm(&mut self, kind: AlarmKind, zone: u8) {
        if self.mute_alarm_kind(kind) {
            return;
        }
        if self.model != Model::Vista20p {
            return;
        }
        let mut toks = Vec::new();
        match kind {
            AlarmKind::Fire => {
                toks.push("fire".into());
                if zone == 95 {
                    toks.push("zone".into());
                    toks.extend(voice::number_tokens(95));
                } else if zone > 0 {
                    let words = voice::filter_zone_words(&self.label(zone as usize));
                    if words.is_empty() {
                        toks.push("zone".into());
                        toks.extend(voice::number_tokens(zone));
                    } else {
                        toks.extend(words);
                    }
                }
            }
            AlarmKind::Panic => {
                toks.push("alarm".into());
                toks.push("police".into());
                toks.push("panic".into());
            }
            AlarmKind::Medical => {
                toks.push("alarm".into());
                toks.push("medical".into());
                toks.push("panic".into());
            }
            _ => {
                toks.push("alarm".into());
                if zone > 0 && (zone as usize) <= self.n() {
                    // Authentic 6160V behavior: only pronounce words in the ROM!
                    // Words like "PANEL" or "TAMPER" are skipped completely.
                    let words = voice::filter_zone_words(&self.label(zone as usize));
                    toks.extend(words);
                }
            }
        }
        self.vista_speak(toks);
    }

    /// Returns the authentic 6160V spoken tokens for a fully armed state.
    /// Appends "zone bypassed" ONLY when at least one zone was manually bypassed
    /// (never for Stay mode's automatic interior zone bypass).
    pub fn vista_armed_tokens(&self, mode: ArmMode, no_entry: bool) -> Vec<String> {
        let mut toks = vec!["armed".into()];
        match (mode, no_entry) {
            (ArmMode::Away, true) => toks.push("maximum".into()),
            (ArmMode::Away, false) => toks.push("away".into()),
            (ArmMode::Stay, true) => {
                toks.push("stay".into());
                toks.push("instant".into());
            }
            (ArmMode::Stay, false) if self.vista_night => {
                toks.push("night".into());
                toks.push("stay".into());
            }
            (ArmMode::Stay, false) => toks.push("stay".into()),
        }
        if self.manual_bypass_any() {
            toks.push("zone".into());
            toks.push("bypassed".into());
        }
        toks
    }

    /// Authentic 6160V spoken tokens during an exit delay.
    fn vista_exit_tokens(&self, how: ArmHow) -> Vec<String> {
        let mut t = vec!["armed".into()];
        let needs_exit = match how {
            ArmHow::MaxKey => {
                t.push("maximum".into());
                true
            }
            ArmHow::StayKey if self.vista_night => {
                t.push("night".into());
                t.push("stay".into());
                false
            }
            ArmHow::StayKey => {
                t.push("stay".into());
                false
            }
            ArmHow::NoEntry => {
                t.push("stay".into());
                t.push("instant".into());
                false
            }
            _ => {
                t.push("away".into());
                true
            }
        };
        if self.manual_bypass_any() {
            t.push("zone".into());
            t.push("bypassed".into());
        }
        if needs_exit {
            t.push("exit_now".into());
        }
        t
    }

    /// Announces the final armed mode on the 6160V once the exit delay finishes.
    pub fn vista_speak_armed(&mut self, mode: ArmMode, no_entry: bool) {
        let toks = self.vista_armed_tokens(mode, no_entry);
        self.vista_speak_if_on(toks);
    }

    /// Speaks current system status (triggered by # + 0 + *, # + *, or after disarm).
    /// Includes "bypassed" / "zone bypassed" when manual bypasses are active,
    /// without reading individual bypassed zone names.
    pub fn vista_speak_status(&mut self) {
        if self.model != Model::Vista20p || !self.vista_voice_on {
            return;
        }
        let toks = if let Some(a) = &self.armed {
            self.vista_armed_tokens(a.mode, a.no_entry)
        } else if let Some(ex) = &self.exit {
            self.vista_exit_tokens(ex.how)
        } else {
            let mut t = vec!["disarmed".into()];
            if self.manual_bypass_any() {
                t.push("bypassed".into());
            }
            if self.ready() {
                t.push("ready_to_arm".into());
            } else {
                t.push("not_ready_to_arm".into());
            }
            t
        };
        self.vista_speak(toks);
    }

    /// Displays a temporary 6160V alpha message anchored to `now` (deterministic in tests).
    fn vista_temp(&mut self, l1: &str, l2: &str, ms: u64, now: Instant) {
        self.temp_msg = Some((l1.to_string(), l2.to_string(), now + Duration::from_millis(ms)));
        self.dirty = true;
    }

    /// 6160V double-STATUS (`# + 0 + *` followed by `*` within 10 s): speaks up to 3 faulted zones.
    fn vista_speak_faults_up_to_3(&mut self, now: Instant) {
        let open: Vec<u8> = self.open_zones().into_iter().take(3).collect();
        if open.is_empty() {
            return;
        }
        let mut toks = Vec::new();
        for z in &open {
            toks.push("fault".into());
            let w = voice::filter_zone_words(&self.label(*z as usize));
            if w.is_empty() {
                toks.push("zone".into());
                toks.extend(voice::number_tokens(*z));
            } else {
                toks.extend(w);
            }
        }
        let first = open[0];
        let lbl = self.label(first as usize).to_ascii_uppercase();
        let (l1, l2) = split_zone_lines("FAULT", first, &lbl);
        self.vista_temp(&l1, &l2, 4000, now);
        self.vista_speak_if_on(toks);
    }

    /// Single crisp 6160V confirmation beep (~340 ms at 1019 Hz) followed by a
    /// 750 ms breath before the voice chip speaks.
    pub fn vista_confirm_beep(&mut self) {
        self.audio(AudioCmd::Tone { hz: 1019, on_ms: 340, off_ms: 750, delay_ms: 0 });
    }

    /// After [Code] + OFF the same beep comes about 1.1 s after the key, the
    /// panel's answer, as two videos of a real 6160 show.
    pub fn vista_off_beep(&mut self) {
        self.audio(AudioCmd::Tone { hz: 1019, on_ms: 340, off_ms: 750, delay_ms: 1100 });
    }

    /// Opens the 6160V Time/Date display (30 s timeout). Used by `[Master]+#+63`
    /// (`can_edit = true`) and `*57` function key option `02` (`can_edit = false`).
    pub(crate) fn vista_open_time_date(&mut self, can_edit: bool, now: Instant) {
        self.temp_msg = None;
        self.vista_mode = VistaKp::TimeDate {
            can_edit,
            editing: false,
            digits: String::new(),
            pos: 0,
            at: now,
        };
        self.dirty = true;
    }

    /// Function key `*57` option `07`: toggles Output Device 01.
    pub(crate) fn vista_fn_output_toggle(&mut self, now: Instant) {
        let next = !self.vista_outputs[0];
        self.vista_outputs[0] = next;
        self.vista_confirm_beep();
        let state = if next { "ON" } else { "OFF" };
        self.vista_temp(&format!("DEVICE 01 {}", state), "", 3000, now);
        self.log_cid(EvKind::Info, format!("Output device 01 {}", state), 400, if next { 'E' } else { 'R' }, 1, false);
        self.out.push(Out::Save);
    }

    /// Function key `*57` options `09..=12`: starts Macro 1..=4 user-code prompt (30 s).
    pub(crate) fn vista_start_macro(&mut self, slot: usize, now: Instant) {
        if slot >= 4 {
            return;
        }
        self.temp_msg = None;
        self.vista_mode = VistaKp::MacroCode { slot, buf: String::new(), at: now };
        self.dirty = true;
    }

    /// Returns up to 100 VISTA-20P `#60` event log entries (newest first),
    /// filtered by field `*90` Event Log Enables bitmask.
    pub fn vista_event_log_entries(&self) -> Vec<&EventRow> {
        let mask = self.ee.entry_or(Model::Vista20p, "90", 0, 15);
        if mask == 0 {
            return Vec::new();
        }
        self.events
            .iter()
            .rev()
            .filter(|ev| {
                let is_byp = ev.cid == 570 || ev.text.to_ascii_lowercase().contains("bypass");
                let is_alarm = ev.kind == EvKind::Alarm || (100..=199).contains(&ev.cid);
                let is_trouble = matches!(ev.kind, EvKind::Trouble | EvKind::Restore) || (300..=399).contains(&ev.cid);
                if is_byp {
                    (mask & 4) != 0
                } else if is_alarm {
                    (mask & 1) != 0
                } else if is_trouble {
                    (mask & 2) != 0
                } else {
                    (mask & 8) != 0
                }
            })
            .take(100)
            .collect()
    }

    fn vista_has_master_priv(&self, slot: &str) -> bool {
        slot == "02" || slot == "inst" || (self.ee.attrs(slot) & 0x07) == 4
    }

    pub(crate) fn press_vista(&mut self, key: Key, now: Instant) {
        // Cancel any pending post-OFF status announcement and clear any stale
        // temporary confirmation banner so new keypresses/menus appear immediately.
        self.vista_pending_status = None;
        self.vista_reminder_until = None;
        self.temp_msg = None;
        // "When entering codes and commands, sequential key depressions must be
        // made within 4-5 seconds of one another. If 4-5 seconds elapse without a
        // key depression, the entry will be aborted."
        // (Long-lived interactive displays — Time/Date, Event Log, Schedules,
        // Macros — have a 30-second timeout handled below and in tick_vista.)
        let elapsed = now.saturating_duration_since(self.vista_prev_key).as_secs_f32();
        self.vista_prev_key = now;
        if elapsed > 5.0 {
            self.buf.clear();
            if matches!(
                self.vista_mode,
                VistaKp::Command { .. }
                    | VistaKp::HashCmd { .. }
                    | VistaKp::Bypass { .. }
                    | VistaKp::UserCode { .. }
                    | VistaKp::TestWait { .. }
                    | VistaKp::NightWait { .. }
                    | VistaKp::OutputCmd { .. }
                    | VistaKp::GoToPart
                    | VistaKp::MultiPart { .. }
            ) {
                self.vista_mode = VistaKp::Normal;
            }
        }
        if elapsed > 30.0
            && matches!(
                self.vista_mode,
                VistaKp::TimeDate { .. }
                    | VistaKp::EventLog { .. }
                    | VistaKp::Schedule { .. }
                    | VistaKp::MacroPgm { .. }
                    | VistaKp::MacroCode { .. }
            )
        {
            self.vista_mode = VistaKp::Normal;
        }
        // "To silence the beeping for these conditions, press any key."
        self.ack_troubles();
        // VISTA-20P Installation Manual p.70: "Pressing any key will silence the
        // keypad sounder for 10 seconds. Disarming the system will silence both
        // keypad and external sounders."
        if self.alarms.iter().any(|a| !a.silenced && !a.kind.pulsed() && (a.audible || a.buzzer)) {
            let ten_s = now + Duration::from_secs(10);
            if self.vista_sounder_mute_until.map_or(true, |t| t < ten_s) {
                self.vista_sounder_mute_until = Some(ten_s);
            }
        }

        let mode = std::mem::replace(&mut self.vista_mode, VistaKp::Normal);
        let next = match mode {
            VistaKp::Normal => self.kv_normal(key, now),
            VistaKp::Command { slot, user, quick } => self.kv_command(slot, user, quick, key, now),
            VistaKp::HashCmd { slot, user, buf } => self.kv_hash_cmd(slot, user, buf, key, now),
            VistaKp::Bypass { buf, at } => self.kv_bypass(buf, at, key, now),
            VistaKp::TestWait { user } => self.kv_test_wait(user, key, now),
            VistaKp::UserCode { slot, buf, first, by_inst, caller_is_master02 } => {
                self.kv_user_code(slot, buf, first, by_inst, caller_is_master02, key)
            }
            VistaKp::Program { field, buf, review } => self.kv_program(field, buf, review, key, now),
            VistaKp::ZoneList(menu) => self.kv_zone_list(menu, key),
            VistaKp::Volume => self.kv_volume(key),
            VistaKp::NightWait { at } => {
                let fresh = now.saturating_duration_since(at).as_secs_f32() <= 5.0;
                let stay = self.armed.map_or(true, |a| a.mode == ArmMode::Stay);
                if fresh && stay && key == Key::Digit(3) && !self.vista_night {
                    // [Code] + STAY + STAY: NIGHT-STAY.
                    self.key_beep();
                    self.vista_set_night();
                    VistaKp::Normal
                } else {
                    self.kv_normal(key, now)
                }
            }
            VistaKp::TimeDate { can_edit, editing, digits, pos, .. } => {
                self.kv_time_date(can_edit, editing, digits, pos, key, now)
            }
            VistaKp::EventLog { idx, .. } => self.kv_event_log(idx, key, now),
            VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, .. } => {
                self.kv_schedule(is_inst, step, input, num, event, device, start_buf, days_buf, pos, key, now)
            }
            VistaKp::MacroPgm { slot, viewing, buf, .. } => self.kv_macro_pgm(slot, viewing, buf, key, now),
            VistaKp::MacroCode { slot, buf, .. } => self.kv_macro_code(slot, buf, key, now),
            VistaKp::OutputCmd { activate, user, buf } => self.kv_output_cmd(activate, user, buf, key, now),
            VistaKp::GoToPart => self.kv_goto_part(key, now),
            VistaKp::MultiPart { slot, user } => self.kv_multi_part(slot, user, key, now),
        };
        self.vista_mode = next;
        self.dirty = true;
    }

    fn kv_normal(&mut self, key: Key, now: Instant) -> VistaKp {
        match key {
            Key::Digit(d) => {
                self.key_beep();
                // 6160V Voice Message Center: pressing [1] while recording stops & saves memo.
                if d == 1 && self.buf.is_empty() && self.vista_msg_recording.is_some() {
                    self.vista_stop_memo();
                    return VistaKp::Normal;
                }
                // Fire and CO alarms are silenced by OFF alone: "the security code
                // is not needed to silence fire alarms". The digit still counts
                // toward a code typed after it.
                if d == 1 && self.buf.is_empty() && self.alarms.iter().any(|a| !a.silenced && a.kind.pulsed()) {
                    let mem = self.alarms.iter().find(|a| a.kind.pulsed()).map(|a| (a.zone, a.kind));
                    for a in self.alarms.iter_mut() {
                        if a.kind.pulsed() {
                            a.silenced = true;
                        }
                    }
                    if mem.is_some() {
                        self.vista_alarm_mem_zone = mem;
                    }
                    self.log(EvKind::Disarm, "Fire alarm silenced with OFF");
                }
                // # + 0 + 1: 6160V Voice Message Center — Record memo (up to 2.5 min, 1 stops).
                if self.buf == "#0" && d == 1 {
                    self.buf.clear();
                    if self.exit.is_some() || self.vista_msg_recording.is_some() || self.vista_msg_saving.is_some()
                        || self.vista_msg_play_until.is_some_and(|until| now < until)
                    {
                        self.error_tone();
                        return VistaKp::Normal;
                    }
                    self.vista_msg_recording = Some(now);
                    self.out.push(Out::Memo(crate::audio::memo::Command::Record(now)));
                    self.vista_confirm_beep();
                    self.vista_temp("RECORDING MSG", "Press 1 to stop", 3000, now);
                    return VistaKp::Normal;
                }
                // # + 0 + 2: 6160V voice volume.
                if self.buf == "#0" && d == 2 {
                    self.buf.clear();
                    if self.vista_msg_play_until.is_some_and(|until| now < until) {
                        return VistaKp::Normal;
                    }
                    self.vista_confirm_beep();
                    self.vista_speak(vec!["system".into()]);
                    return VistaKp::Volume;
                }
                // # + 0 + 3: 6160V Voice Message Center — Play memo & clear MESSAGE LED.
                if self.buf == "#0" && d == 3 {
                    self.buf.clear();
                    self.vista_confirm_beep();
                    if let Some(clip) = self.vista_memo.clone() {
                        self.vista_msg_waiting = false;
                        let duration = Duration::from_secs_f64(clip.samples.len() as f64 / f64::from(clip.rate));
                        self.vista_msg_play_until = Some(now + duration);
                        self.vista_temp("PLAYING MESSAGE", "", 3000, now);
                        self.audio(AudioCmd::PlayMemo(clip));
                        self.log(EvKind::Info, "6160V voice message played");
                    } else {
                        self.vista_temp("NO MESSAGES", "", 2500, now);
                    }
                    return VistaKp::Normal;
                }
                if self.buf == "#" && d == 0 {
                    self.buf.push('0');
                    return VistaKp::Normal;
                }
                if self.buf.starts_with('#') {
                    self.buf.clear();
                }
                self.buf.push(char::from(b'0' + d));
                if self.buf.len() >= 4 {
                    let code = std::mem::take(&mut self.buf);
                    let urgent = self.entry.is_some() || self.alarms.iter().any(|a| !a.silenced);
                    if urgent {
                        self.vista_sounder_mute_until = Some(now + Duration::from_secs(15));
                    }
                    if let Some(slot) = self.ee.find_code(&code) {
                        self.bad_codes = 0;
                        let user = eeprom::slot_user(&slot);
                        if urgent {
                            self.vista_code_disarm = Some((now + Duration::from_secs(15), user, slot.clone()));
                        }
                        return VistaKp::Command { slot, user, quick: false };
                    }
                    self.bad_code(now);
                }
                VistaKp::Normal
            }
            Key::Hash => {
                self.key_beep();
                if self.buf == "#" {
                    self.buf.clear();
                    return VistaKp::Normal;
                }
                self.buf = "#".to_string();
                VistaKp::Command { slot: String::new(), user: 0, quick: true }
            }
            Key::Star => {
                self.key_beep();
                if self.buf == "#0" {
                    self.buf.clear();
                    let repeat = self.vista_status_at.map_or(false, |t| now.saturating_duration_since(t).as_secs() <= 10);
                    if repeat && self.armed.is_none() && self.exit.is_none() && !self.open_zones().is_empty() {
                        self.vista_status_at = None;
                        self.vista_speak_faults_up_to_3(now);
                    } else {
                        self.vista_status_at = Some(now);
                        self.vista_speak_status();
                    }
                    return VistaKp::Normal;
                }
                self.buf.clear();
                let open = self.open_zones();
                let recent_status = self.vista_status_at.map_or(false, |t| now.saturating_duration_since(t).as_secs() <= 10);
                if recent_status && !open.is_empty() && self.armed.is_none() && self.exit.is_none() {
                    self.vista_status_at = None;
                    self.vista_speak_faults_up_to_3(now);
                } else if !open.is_empty() && self.armed.is_none() && self.exit.is_none() {
                    let idx = self.vista_fault_idx % open.len();
                    let z = open[idx] as usize;
                    self.vista_fault_idx = (idx + 1) % open.len();
                    let lbl = self.label(z).to_ascii_uppercase();
                    let (l1, l2) = split_zone_lines("FAULT", z as u8, &lbl);
                    self.vista_temp(&l1, &l2, 4000, now);
                    let mut toks = vec!["fault".into()];
                    let w = voice::filter_zone_words(&lbl);
                    if w.is_empty() {
                        toks.push("zone".into());
                        toks.extend(voice::number_tokens(z as u8));
                    } else {
                        toks.extend(w);
                    }
                    self.vista_speak_if_on(toks);
                } else if self.armed.map_or(false, |a| a.mode == ArmMode::Stay)
                    && (self.ee.entry_or(Model::Vista20p, "91", 0, 8) & 8) != 0
                {
                    // *91 exit delay restart, armed STAY or INSTANT.
                    let secs = self.exit_secs();
                    if let Some(a) = self.armed.as_mut() {
                        a.quick_exit = Some(now + Duration::from_secs(secs));
                    }
                    self.log(EvKind::Arm, "Exit delay restarted");
                } else if self.armed.is_none() && self.exit.is_none() {
                    // The voice keypad's STATUS key.
                    self.vista_status_at = Some(now);
                    self.vista_speak_status();
                }
                VistaKp::Normal
            }
            _ => VistaKp::Normal,
        }
    }

    fn kv_command(&mut self, slot: String, user: u8, quick: bool, key: Key, now: Instant) -> VistaKp {
        if quick {
            match key {
                Key::Star => {
                    self.key_beep();
                    self.buf.clear();
                    let repeat = self.vista_status_at.map_or(false, |t| now.saturating_duration_since(t).as_secs() <= 10);
                    if repeat && self.armed.is_none() && self.exit.is_none() && !self.open_zones().is_empty() {
                        self.vista_status_at = None;
                        self.vista_speak_faults_up_to_3(now);
                    } else {
                        self.vista_status_at = Some(now);
                        self.vista_speak_status();
                    }
                    return VistaKp::Normal;
                }
                Key::Digit(0) => {
                    self.key_beep();
                    self.buf = "#0".to_string();
                    return VistaKp::Normal;
                }
                Key::Digit(d @ (2 | 3 | 4 | 7)) => {
                    self.key_beep();
                    self.buf.clear();
                    // Quick arm (*21, off from the factory) works in every mode
                    // except NIGHT-STAY.
                    if self.ee.entry_or(Model::Vista20p, "21", 0, 0) == 0 {
                        self.error_tone();
                        return VistaKp::Normal;
                    }
                    return self.kv_arm_action(d, "", 0, true, now);
                }
                _ => {
                    self.buf.clear();
                    return self.kv_normal(key, now);
                }
            }
        }

        // Installer programming entry: [Installer 4112] + 8 + 0 + 0
        // Installer Master Code set:   [Installer 4112] + 8 + 0 + 2 + [4-digit] (single entry!)
        if slot == "inst" && self.buf.is_empty() && key == Key::Digit(8) && self.armed.is_none() && self.exit.is_none() {
            self.key_beep();
            self.buf = "INST8".to_string();
            return VistaKp::Command { slot, user, quick: false };
        }
        if slot == "inst" && self.buf == "INST8" && key == Key::Digit(0) {
            self.key_beep();
            self.buf = "INST80".to_string();
            return VistaKp::Command { slot, user, quick: false };
        }
        if slot == "inst" && self.buf == "INST80" && key == Key::Digit(0) {
            self.buf.clear();
            if self.ee.installer_lockout {
                self.error_tone();
                self.vista_temp("INSTALLER LOCK", "OUT ENABLED", 3000, now);
                return VistaKp::Normal;
            }
            self.beeps(3);
            self.log_cid(EvKind::Program, "Entered Vista installer programming", 627, 'E', 0, true);
            return VistaKp::Program { field: None, buf: String::new(), review: false };
        }
        if slot == "inst" && self.buf == "INST80" && key == Key::Digit(2) {
            self.key_beep();
            self.buf.clear();
            return VistaKp::UserCode {
                slot: "02".to_string(),
                buf: String::new(),
                first: String::new(),
                by_inst: true,
                caller_is_master02: true,
            };
        }
        self.buf.clear();

        match key {
            // 0 = Multi-Partition global command ([Code] + 0 + [1/2/3/4/7])
            Key::Digit(0) => {
                self.key_beep();
                VistaKp::MultiPart { slot, user }
            }
            // 1 = OFF
            Key::Digit(1) => {
                self.key_beep();
                self.vista_off(&slot, user, now);
                VistaKp::Normal
            }
            // 2 = AWAY, 3 = STAY (twice for NIGHT-STAY), 4 = MAXIMUM, 7 = INSTANT
            Key::Digit(d @ (2 | 3 | 4 | 7)) => {
                self.key_beep();
                self.vista_code_disarm = None;
                self.kv_arm_action(d, &slot, user, false, now)
            }
            // 5 = TEST, then 0 (walk) or 1 (dial)
            Key::Digit(5) => {
                self.key_beep();
                if self.armed.is_some() || self.exit.is_some() {
                    self.error_tone();
                    VistaKp::Normal
                } else {
                    VistaKp::TestWait { user }
                }
            }
            // 6 = BYPASS ("the system must be disarmed first")
            Key::Digit(6) => {
                self.key_beep();
                if self.armed.is_some() || self.exit.is_some() {
                    self.error_tone();
                    VistaKp::Normal
                } else {
                    VistaKp::Bypass { buf: String::new(), at: now }
                }
            }
            // 8 = CODE (Master or Partition Master user code programming)
            Key::Digit(8) => {
                self.key_beep();
                if self.vista_has_master_priv(&slot) && slot != "inst" && self.armed.is_none() && self.exit.is_none() {
                    VistaKp::UserCode {
                        slot: String::new(),
                        buf: String::new(),
                        first: String::new(),
                        by_inst: false,
                        caller_is_master02: slot == "02",
                    }
                } else {
                    self.error_tone();
                    VistaKp::Normal
                }
            }
            // 9 = CHIME
            Key::Digit(9) => {
                self.key_beep();
                if self.armed.is_some() || self.exit.is_some() {
                    self.error_tone();
                    return VistaKp::Normal;
                }
                self.chime = !self.chime;
                self.vista_confirm_beep();
                self.log(EvKind::Program, if self.chime { "Door chime on" } else { "Door chime off" });
                self.out.push(Out::Save);
                if self.chime {
                    self.vista_speak_if_on(vec!["chime".into()]);
                }
                VistaKp::Normal
            }
            // # = Advanced functions (#60 Event Log, #63 Time/Date, #64 Schedules, #66 Macros, #7/#8 Outputs, #0 test)
            Key::Hash => {
                self.key_beep();
                VistaKp::HashCmd { slot, user, buf: String::new() }
            }
            // * = GoTo Partition ([Code] + * + [0-3])
            Key::Star => {
                self.key_beep();
                VistaKp::GoToPart
            }
            _ => {
                self.key_beep();
                VistaKp::Normal
            }
        }
    }

    fn kv_hash_cmd(&mut self, slot: String, user: u8, mut buf: String, key: Key, now: Instant) -> VistaKp {
        let Key::Digit(d) = key else {
            self.error_tone();
            return VistaKp::Normal;
        };
        self.key_beep();
        if buf.is_empty() {
            match d {
                7 => return VistaKp::OutputCmd { activate: true, user, buf: String::new() },
                8 => return VistaKp::OutputCmd { activate: false, user, buf: String::new() },
                6 => {
                    buf.push('6');
                    return VistaKp::HashCmd { slot, user, buf };
                }
                0 if slot == "inst" => {
                    buf.push('0');
                    return VistaKp::HashCmd { slot, user, buf };
                }
                _ => {
                    self.error_tone();
                    return VistaKp::Normal;
                }
            }
        }
        if buf == "0" && slot == "inst" {
            // [Installer] + # + 0 + 0/1/2: periodic test schedule (24h / 7d / 28d).
            let days = match d {
                0 => 1,
                1 => 7,
                2 => 28,
                _ => {
                    self.error_tone();
                    return VistaKp::Normal;
                }
            };
            let first_ms = self.vista_wall_ms + self.clock_offset_ms + 12 * 60 * 60 * 1000;
            let first = util::parts(first_ms);
            let weekday = util::weekday_idx(first_ms);
            let first_day = util::local(first_ms).num_days_from_ce();
            self.vista_schedules.retain(|schedule| schedule.num != 32);
            self.vista_schedules.push(VistaSched {
                num: 32,
                event: 11,
                hour: first.hour as u8,
                minute: first.minute as u8,
                days: if days == 1 { 0x7f } else { 1 << weekday },
                repeat: if days == 28 { 4 } else { 1 },
                start_day: Some(first_day),
                first_run_week: Some(first_day - i32::from(weekday)),
                ..Default::default()
            });
            self.out.push(Out::Save);
            self.vista_confirm_beep();
            let lbl = if days == 1 { "24 HOUR" } else if days == 7 { "WEEKLY" } else { "28 DAY" };
            self.vista_temp("PERIODIC TEST", lbl, 3000, now);
            self.log_cid(EvKind::Program, format!("Periodic test schedule set: {}", lbl), 602, 'E', 0, true);
            return VistaKp::Normal;
        }
        if buf == "6" {
            if !self.vista_has_master_priv(&slot) {
                self.error_tone();
                return VistaKp::Normal;
            }
            match d {
                // #60: 100-Event History Log Viewer
                0 => return VistaKp::EventLog { idx: 0, at: now },
                // #63: Real-Time Clock Viewing & Setting
                3 => {
                    return VistaKp::TimeDate {
                        can_edit: true,
                        editing: false,
                        digits: String::new(),
                        pos: 0,
                        at: now,
                    };
                }
                // #64: Scheduling Mode
                4 => {
                    if self.armed.is_some() || self.exit.is_some() {
                        self.error_tone();
                        return VistaKp::Normal;
                    }
                    return VistaKp::Schedule {
                        is_inst: slot == "inst",
                        step: SchedStep::SelectNum,
                        input: String::new(),
                        num: 0,
                        event: 0,
                        device: 1,
                        start_buf: "12002".to_string(),
                        days_buf: "1111111".to_string(),
                        pos: 0,
                        at: now,
                    };
                }
                // #66: Macro Key Programming
                6 => {
                    if self.armed.is_some() || self.exit.is_some() {
                        self.error_tone();
                        return VistaKp::Normal;
                    }
                    return VistaKp::MacroPgm { slot: None, viewing: false, buf: String::new(), at: now };
                }
                _ => {
                    self.error_tone();
                    return VistaKp::Normal;
                }
            }
        }
        self.error_tone();
        VistaKp::Normal
    }

    fn kv_arm_action(&mut self, digit: u8, slot: &str, user: u8, _quick: bool, now: Instant) -> VistaKp {
        let how = match digit {
            2 => ArmHow::AwayKey,
            3 => ArmHow::StayKey,
            4 => ArmHow::MaxKey,
            7 => ArmHow::NoEntry,
            _ => return VistaKp::Normal,
        };
        if self.armed.is_some() || self.exit.is_some() {
            self.error_tone();
            return VistaKp::Normal;
        }
        if !slot.is_empty() && self.is_duress(slot) {
            self.trigger(AlarmKind::Duress, 0, false, false, now);
        }
        if !self.arm(how, user, now) {
            if let Some(&z) = self.open_zones().first() {
                let lbl = self.label(z as usize).to_ascii_uppercase();
                let (l1, l2) = split_zone_lines("FAULT", z, &lbl);
                self.vista_temp(&l1, &l2, 4000, now);
            }
            self.vista_speak_if_on(vec!["not_ready_to_arm".into()]);
            return VistaKp::Normal;
        }
        self.vista_armed_by_inst = slot == "inst";
        self.vista_armed_by_slot = if slot.is_empty() { None } else { Some(slot.to_string()) };
        self.vista_exit_voice_done = false;
        self.vista_alarm_mem_zone = None;
        self.vista_night = false;
        let exit_warning = self.ee.entry_or(Model::Vista20p, "37", 0, 1) != 0;
        match digit {
            4 => {
                if !exit_warning {
                    self.beeps(2);
                    let toks = self.vista_exit_tokens(ArmHow::MaxKey);
                    self.vista_speak_if_on(toks);
                    self.vista_exit_voice_done = true;
                }
                VistaKp::Normal
            }
            2 => {
                if !exit_warning {
                    self.beeps(2);
                    let toks = self.vista_exit_tokens(ArmHow::AwayKey);
                    self.vista_speak_if_on(toks);
                    self.vista_exit_voice_done = true;
                }
                VistaKp::Normal
            }
            3 => {
                self.vista_arm_beeps();
                let toks = self.vista_exit_tokens(ArmHow::StayKey);
                self.vista_speak_if_on(toks);
                // Allow STAY -> STAY within 5 s for NIGHT-STAY (including #33 Quick-Arm Night-Stay)
                VistaKp::NightWait { at: now }
            }
            _ => {
                self.vista_arm_beeps();
                let toks = self.vista_exit_tokens(ArmHow::NoEntry);
                self.vista_speak_if_on(toks);
                VistaKp::Normal
            }
        }
    }

    fn kv_bypass(&mut self, mut buf: String, _at: Instant, key: Key, now: Instant) -> VistaKp {
        match key {
            Key::Hash if buf.is_empty() => {
                self.key_beep();
                // Quick bypass (*23, off from the factory): code + 6 + #.
                if self.ee.entry_or(Model::Vista20p, "23", 0, 0) == 0 {
                    self.error_tone();
                    return VistaKp::Normal;
                }
                let mut list = Vec::new();
                for z in 1..=self.n() {
                    if self.violated(z) && self.cfg(z).bypassable {
                        self.zones[z - 1].bypassed = true;
                        list.push(z as u8);
                    }
                }
                if list.is_empty() {
                    self.error_tone();
                } else {
                    // Authentic 6160V: electronic confirmation beep ONLY during bypass entry (NO speech!).
                    self.vista_show_bypassed(&list, now);
                    let first = list[0] as u16;
                    self.log_cid(EvKind::Info, format!("Quick bypassed {} open zone(s)", list.len()), 570, 'E', first, false);
                }
                VistaKp::Normal
            }
            Key::Digit(d) => {
                self.key_beep();
                buf.push(char::from(b'0' + d));
                if buf.len() < 2 {
                    return VistaKp::Bypass { buf, at: now };
                }
                let z = buf.parse::<usize>().unwrap_or(0);
                if z == 0 {
                    // Authentic VISTA-20P [Code] + 6 + 00 clears all active bypasses
                    let had = self.zones.iter().any(|zr| zr.bypassed);
                    for zr in self.zones.iter_mut() {
                        zr.bypassed = false;
                        zr.auto_bypass = false;
                    }
                    self.vista_confirm_beep();
                    if had {
                        self.vista_temp("BYPASS CLEARED", "", 2500, now);
                        self.log_cid(EvKind::Info, "All bypasses cleared ([Code]+6+00)", 570, 'R', 0, false);
                    }
                } else if (1..=self.n()).contains(&z) && self.cfg(z).bypassable {
                    let was = self.zones[z - 1].bypassed;
                    self.zones[z - 1].bypassed = true;
                    // Authentic 6160V: electronic confirmation beep ONLY during bypass entry (NO speech!).
                    self.vista_show_bypassed(&[z as u8], now);
                    if !was {
                        self.log_cid(EvKind::Info, format!("Zone {:02} bypassed", z), 570, 'E', z as u16, false);
                    }
                } else {
                    self.error_tone();
                }
                VistaKp::Normal
            }
            _ => VistaKp::Normal,
        }
    }

    fn kv_test_wait(&mut self, user: u8, key: Key, now: Instant) -> VistaKp {
        match key {
            Key::Digit(0) => {
                self.key_beep();
                self.walk_test = true;
                self.vista_test_since = Some(now);
                self.walk_burst_until = Some(now + Duration::from_secs(1));
                self.log_cid(EvKind::Info, "Walk test started (code + OFF ends it)", 607, 'E', user as u16, true);
                self.vista_speak(vec!["system".into()]);
            }
            Key::Digit(1) => {
                // Authentic VISTA-20P Installation Guide p.4-2 Dialer Test:
                // sends CID 601, displays PHONE OKAY + 2 beeps (or COMM. FAILURE if FTC).
                self.key_beep();
                if self.comms_enabled() && !self.inputs.ftc {
                    self.beeps(2);
                    self.log_cid(EvKind::Info, "Dialer test report sent (Contact ID 601)", 601, 'E', user as u16, true);
                    self.report(ReportKind::Restore, 601, user as u16, "Manual dialer test".to_string(), 0);
                    self.vista_temp("PHONE OKAY", "", 3500, now);
                } else {
                    self.error_tone();
                    self.log_cid(EvKind::Trouble, "Dialer test failed (COMM. FAILURE)", 354, 'E', 0, false);
                    self.vista_temp("COMM. FAILURE", "", 3500, now);
                }
            }
            _ => self.error_tone(),
        }
        VistaKp::Normal
    }

    fn kv_user_code(
        &mut self,
        mut slot: String,
        mut buf: String,
        mut first: String,
        by_inst: bool,
        caller_is_master02: bool,
        key: Key,
    ) -> VistaKp {
        match key {
            Key::Digit(d) => {
                self.key_beep();
                if slot.len() < 2 {
                    slot.push(char::from(b'0' + d));
                    if slot.len() == 2 {
                        let u = slot.parse::<u8>().unwrap_or(0);
                        if !(2..=49).contains(&u) || (u == 2 && !caller_is_master02 && !by_inst) {
                            self.error_tone();
                            return VistaKp::Normal;
                        }
                    }
                    return VistaKp::UserCode { slot, buf, first, by_inst, caller_is_master02 };
                }
                let u = slot.parse::<u8>().unwrap_or(0);
                // User attribute sub-commands (#0 delete, #1 authority 0-4, #2 group 0-8, #3 partition, #5 pager)
                if buf.starts_with('#') {
                    if !(3..=49).contains(&u) {
                        self.error_tone();
                        return VistaKp::Normal;
                    }
                    if buf == "#" {
                        match d {
                            0 => {
                                self.ee.clear_code(&slot);
                                self.vista_confirm_beep();
                                self.log(EvKind::Program, format!("User {:02} deleted", u));
                                self.out.push(Out::Save);
                                return VistaKp::Normal;
                            }
                            1 | 2 | 3 | 5 => {
                                buf.push(char::from(b'0' + d));
                                return VistaKp::UserCode { slot, buf, first, by_inst, caller_is_master02 };
                            }
                            _ => {
                                self.error_tone();
                                return VistaKp::Normal;
                            }
                        }
                    }
                    if !self.ee.has_code(&slot) {
                        self.error_tone();
                        return VistaKp::Normal;
                    }
                    match buf.as_str() {
                        "#1" if d <= 4 => {
                            let attrs = (self.ee.attrs(&slot) & !0x07) | d;
                            self.ee.set_attrs(&slot, attrs);
                            let name = match d {
                                1 => "Arm Only",
                                2 => "Guest",
                                3 => "Duress",
                                4 => "Partition Master",
                                _ => "Standard",
                            };
                            self.vista_confirm_beep();
                            self.log(EvKind::Program, format!("User {:02} authority: {} ({})", u, d, name));
                            self.out.push(Out::Save);
                            return VistaKp::Normal;
                        }
                        "#2" if d <= 8 => {
                            let attrs = (self.ee.attrs(&slot) & !0x78) | ((d & 0x0F) << 3);
                            self.ee.set_attrs(&slot, attrs);
                            self.vista_confirm_beep();
                            self.log(EvKind::Program, format!("User {:02} access group set to {}", u, d));
                            self.out.push(Out::Save);
                            return VistaKp::Normal;
                        }
                        "#3" => {
                            // Accepts #3 + 0 + [1-3] + # (or #3 + [1-3] + #)
                            buf.push(char::from(b'0' + d));
                            return VistaKp::UserCode { slot, buf, first, by_inst, caller_is_master02 };
                        }
                        s if s.starts_with("#3") && s.len() < 5 => {
                            buf.push(char::from(b'0' + d));
                            return VistaKp::UserCode { slot, buf, first, by_inst, caller_is_master02 };
                        }
                        "#5" if d <= 1 => {
                            let attrs = (self.ee.attrs(&slot) & !0x80) | (d << 7);
                            self.ee.set_attrs(&slot, attrs);
                            self.vista_confirm_beep();
                            self.log(EvKind::Program, format!("User {:02} paging {}", u, if d == 1 { "on" } else { "off" }));
                            self.out.push(Out::Save);
                            return VistaKp::Normal;
                        }
                        _ => {
                            self.error_tone();
                            return VistaKp::Normal;
                        }
                    }
                }
                buf.push(char::from(b'0' + d));
                if buf.len() < 4 {
                    return VistaKp::UserCode { slot, buf, first, by_inst, caller_is_master02 };
                }
                if !(2..=49).contains(&u) {
                    self.error_tone();
                    return VistaKp::Normal;
                }
                // Master changing own code 02 requires double entry; Installer changing 02 is single entry!
                if u == 2 && !by_inst && first.is_empty() {
                    first = buf;
                    return VistaKp::UserCode { slot, buf: String::new(), first, by_inst, caller_is_master02 };
                }
                if u == 2 && !by_inst && first != buf {
                    self.error_tone();
                    return VistaKp::Normal;
                }
                self.ee.set_code(&slot, &buf);
                self.vista_confirm_beep();
                self.log(EvKind::Program, format!("User {:02} code set", u));
                self.out.push(Out::Save);
                VistaKp::Normal
            }
            Key::Hash if slot.len() == 2 && buf.is_empty() && !by_inst => {
                self.key_beep();
                VistaKp::UserCode { slot, buf: "#".to_string(), first, by_inst, caller_is_master02 }
            }
            Key::Hash if buf.starts_with("#3") && buf.len() >= 3 => {
                // Confirm #3 partition assignment with trailing '#'
                let u = slot.parse::<u8>().unwrap_or(0);
                self.vista_confirm_beep();
                self.log(EvKind::Program, format!("User {:02} partition updated ({})", u, &buf[2..]));
                self.out.push(Out::Save);
                VistaKp::Normal
            }
            _ => VistaKp::Normal,
        }
    }

    fn kv_time_date(&mut self, can_edit: bool, editing: bool, mut digits: String, mut pos: usize, key: Key, now: Instant) -> VistaKp {
        if !editing {
            self.key_beep();
            if key == Key::Star && can_edit && self.armed.is_none() && self.exit.is_none() {
                // Pre-populate edit buffer from current panel clock: HH MM P(1/2) YY MO DD
                let p = util::parts(self.vista_wall_ms + self.clock_offset_ms);
                let (h12, pm_digit) = match p.hour {
                    0 => (12, '2'),
                    1..=11 => (p.hour, '2'),
                    12 => (12, '1'),
                    _ => (p.hour - 12, '1'),
                };
                let init = format!("{:02}{:02}{}{:02}{:02}{:02}", h12, p.minute, pm_digit, p.year.rem_euclid(100), p.month, p.day);
                return VistaKp::TimeDate { can_edit: true, editing: true, digits: init, pos: 0, at: now };
            }
            return VistaKp::Normal;
        }

        // Editing mode (`04:04P2026/11/26` with flashing cursor under `pos`)
        match key {
            Key::Digit(d) => {
                self.key_beep();
                if pos == 4 && d != 1 && d != 2 {
                    self.error_tone();
                    return VistaKp::TimeDate { can_edit, editing: true, digits, pos, at: now };
                }
                let mut chars: Vec<char> = digits.chars().collect();
                if chars.len() == 11 && pos < 11 {
                    chars[pos] = char::from(b'0' + d);
                    digits = chars.into_iter().collect();
                    if pos < 10 {
                        pos += 1;
                    }
                }
                VistaKp::TimeDate { can_edit, editing: true, digits, pos, at: now }
            }
            Key::Star => {
                self.key_beep();
                if pos < 10 {
                    pos += 1;
                    return VistaKp::TimeDate { can_edit, editing: true, digits, pos, at: now };
                }
                // Cursor is at the last digit (pos == 10): validate & save!
                let h12: u32 = digits.get(0..2).and_then(|s| s.parse().ok()).unwrap_or(0);
                let min: u32 = digits.get(2..4).and_then(|s| s.parse().ok()).unwrap_or(99);
                let is_pm = digits.as_bytes().get(4).copied() == Some(b'1');
                let yy: i32 = digits.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(0);
                let mo: u32 = digits.get(7..9).and_then(|s| s.parse().ok()).unwrap_or(0);
                let dd: u32 = digits.get(9..11).and_then(|s| s.parse().ok()).unwrap_or(0);
                if (1..=12).contains(&h12) && min < 60 && (1..=12).contains(&mo) && (1..=31).contains(&dd) {
                    let hour24 = (if h12 == 12 { 0 } else { h12 }) + if is_pm { 12 } else { 0 };
                    if let Some(off) = util::offset_to_datetime(self.vista_wall_ms, 2000 + yy, mo, dd, hour24, min) {
                        self.clock_offset_ms = off;
                        self.vista_confirm_beep();
                        self.log_cid(EvKind::Program, "Real-Time Clock was Changed", 625, 'E', 2, true);
                        self.out.push(Out::Save);
                        return VistaKp::Normal;
                    }
                }
                self.error_tone();
                VistaKp::TimeDate { can_edit, editing: true, digits, pos: 0, at: now }
            }
            Key::Hash => {
                self.key_beep();
                pos = pos.saturating_sub(1);
                VistaKp::TimeDate { can_edit, editing: true, digits, pos, at: now }
            }
            _ => VistaKp::Normal,
        }
    }

    fn kv_event_log(&mut self, idx: usize, key: Key, now: Instant) -> VistaKp {
        let len = self.vista_event_log_entries().len();
        match key {
            Key::Star => {
                self.key_beep();
                if len == 0 {
                    return VistaKp::Normal;
                }
                let next = if idx + 1 < len { idx + 1 } else { idx };
                if next == idx {
                    self.error_tone();
                }
                VistaKp::EventLog { idx: next, at: now }
            }
            Key::Hash => {
                self.key_beep();
                if len == 0 {
                    return VistaKp::Normal;
                }
                let next = idx.saturating_sub(1);
                if next == idx {
                    self.error_tone();
                }
                VistaKp::EventLog { idx: next, at: now }
            }
            _ => {
                self.key_beep();
                VistaKp::Normal
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn kv_schedule(
        &mut self,
        is_inst: bool,
        step: SchedStep,
        mut input: String,
        mut num: u8,
        mut event: u8,
        mut device: u8,
        mut start_buf: String,
        mut days_buf: String,
        mut pos: usize,
        key: Key,
        now: Instant,
    ) -> VistaKp {
        let max_num: u8 = if is_inst { 32 } else { 16 };
        match step {
            SchedStep::SelectNum => match key {
                Key::Digit(d) => {
                    self.key_beep();
                    input.push(char::from(b'0' + d));
                    if input.len() < 2 {
                        return VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    let n = input.parse::<u8>().unwrap_or(0);
                    input.clear();
                    if n == 0 {
                        self.vista_confirm_beep();
                        return VistaKp::Normal;
                    }
                    if n > max_num {
                        self.error_tone();
                        return VistaKp::Schedule { is_inst, step: SchedStep::SelectNum, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    num = n;
                    if let Some(ex) = self.vista_schedules.iter().find(|s| s.num == num).cloned() {
                        event = ex.event;
                        device = ex.device.max(1);
                        let (h12, pm) = match ex.hour {
                            0 => (12, '2'),
                            1..=11 => (ex.hour, '2'),
                            12 => (12, '1'),
                            _ => (ex.hour - 12, '1'),
                        };
                        start_buf = format!("{:02}{:02}{}", h12, ex.minute, pm);
                        days_buf = (0..7).map(|b| if (ex.days & (1 << b)) != 0 { '1' } else { '0' }).collect();
                    } else {
                        event = 0;
                        device = 1;
                        start_buf = "12002".to_string();
                        days_buf = "1111111".to_string();
                    }
                    VistaKp::Schedule { is_inst, step: SchedStep::Event, input, num, event, device, start_buf, days_buf, pos: 0, at: now }
                }
                Key::Star => {
                    self.key_beep();
                    VistaKp::Normal
                }
                _ => VistaKp::Normal,
            },
            SchedStep::Event => match key {
                Key::Digit(d) => {
                    self.key_beep();
                    input.push(char::from(b'0' + d));
                    if input.len() < 2 {
                        return VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    let ev = input.parse::<u8>().unwrap_or(99);
                    input.clear();
                    if ev == 0 {
                        self.vista_schedules.retain(|s| s.num != num);
                        self.vista_confirm_beep();
                        self.log(EvKind::Program, format!("Schedule {:02} cleared", num));
                        self.out.push(Out::Save);
                        return VistaKp::Schedule { is_inst, step: SchedStep::SelectNum, input, num: 0, event: 0, device: 1, start_buf, days_buf, pos: 0, at: now };
                    }
                    let valid = matches!(ev, 1 | 4 | 5 | 6 | 7) || (is_inst && ev == 11);
                    if !valid {
                        self.error_tone();
                        return VistaKp::Schedule { is_inst, step: SchedStep::Event, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    event = ev;
                    let next_step = if event == 1 { SchedStep::Device } else { SchedStep::StartTime };
                    VistaKp::Schedule { is_inst, step: next_step, input, num, event, device, start_buf, days_buf, pos: 0, at: now }
                }
                Key::Hash => {
                    self.key_beep();
                    VistaKp::Schedule { is_inst, step: SchedStep::SelectNum, input: String::new(), num: 0, event, device, start_buf, days_buf, pos: 0, at: now }
                }
                _ => VistaKp::Normal,
            },
            SchedStep::Device => match key {
                Key::Digit(d) => {
                    self.key_beep();
                    input.push(char::from(b'0' + d));
                    if input.len() < 2 {
                        return VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    let dev = input.parse::<u8>().unwrap_or(0);
                    input.clear();
                    if !(1..=18).contains(&dev) {
                        self.error_tone();
                        return VistaKp::Schedule { is_inst, step: SchedStep::Device, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    device = dev;
                    VistaKp::Schedule { is_inst, step: SchedStep::StartTime, input, num, event, device, start_buf, days_buf, pos: 0, at: now }
                }
                Key::Hash => {
                    self.key_beep();
                    VistaKp::Schedule { is_inst, step: SchedStep::Event, input: String::new(), num, event, device, start_buf, days_buf, pos: 0, at: now }
                }
                _ => VistaKp::Normal,
            },
            SchedStep::StartTime => match key {
                Key::Digit(d) => {
                    self.key_beep();
                    if pos == 4 && d != 1 && d != 2 {
                        self.error_tone();
                        return VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    let mut chars: Vec<char> = start_buf.chars().collect();
                    if chars.len() == 5 && pos < 5 {
                        chars[pos] = char::from(b'0' + d);
                        start_buf = chars.into_iter().collect();
                        if pos < 4 {
                            pos += 1;
                        }
                    }
                    VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now }
                }
                Key::Star => {
                    self.key_beep();
                    if pos < 4 {
                        pos += 1;
                        return VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    let h12: u8 = start_buf.get(0..2).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let mm: u8 = start_buf.get(2..4).and_then(|s| s.parse().ok()).unwrap_or(99);
                    if (1..=12).contains(&h12) && mm < 60 {
                        VistaKp::Schedule { is_inst, step: SchedStep::Days, input, num, event, device, start_buf, days_buf, pos: 0, at: now }
                    } else {
                        self.error_tone();
                        VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos: 0, at: now }
                    }
                }
                Key::Hash => {
                    self.key_beep();
                    if pos > 0 {
                        pos -= 1;
                        VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now }
                    } else {
                        VistaKp::Schedule { is_inst, step: SchedStep::Event, input: String::new(), num, event, device, start_buf, days_buf, pos: 0, at: now }
                    }
                }
                _ => VistaKp::Normal,
            },
            SchedStep::Days => match key {
                Key::Digit(d) if d <= 1 => {
                    self.key_beep();
                    let mut chars: Vec<char> = days_buf.chars().collect();
                    if chars.len() == 7 && pos < 7 {
                        chars[pos] = char::from(b'0' + d);
                        days_buf = chars.into_iter().collect();
                        if pos < 6 {
                            pos += 1;
                        }
                    }
                    VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now }
                }
                Key::Star => {
                    self.key_beep();
                    if pos < 6 {
                        pos += 1;
                        return VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now };
                    }
                    VistaKp::Schedule { is_inst, step: SchedStep::Repeat, input, num, event, device, start_buf, days_buf, pos: 0, at: now }
                }
                Key::Hash => {
                    self.key_beep();
                    if pos > 0 {
                        pos -= 1;
                        VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now }
                    } else {
                        VistaKp::Schedule { is_inst, step: SchedStep::StartTime, input, num, event, device, start_buf, days_buf, pos: 0, at: now }
                    }
                }
                _ => {
                    self.error_tone();
                    VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now }
                }
            },
            SchedStep::Repeat => match key {
                Key::Digit(d) if d <= 4 => {
                    self.key_beep();
                    let h12: u8 = start_buf.get(0..2).and_then(|s| s.parse().ok()).unwrap_or(12);
                    let minute: u8 = start_buf.get(2..4).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let is_pm = start_buf.as_bytes().get(4).copied() == Some(b'1');
                    let hour = (if h12 == 12 { 0 } else { h12 }) + if is_pm { 12 } else { 0 };
                    let mut days = 0u8;
                    for (b, c) in days_buf.chars().take(7).enumerate() {
                        if c == '1' {
                            days |= 1 << b;
                        }
                    }
                    self.vista_schedules.retain(|s| s.num != num);
                    self.vista_schedules.push(VistaSched {
                        num,
                        event,
                        device,
                        hour,
                        minute,
                        days,
                        repeat: d,
                        start_day: None,
                        first_run_week: None,
                        last_run_day: None,
                    });
                    self.vista_schedules.sort_by_key(|s| s.num);
                    self.vista_confirm_beep();
                    self.log(EvKind::Program, format!("Schedule {:02} saved (event {:02} at {:02}:{:02})", num, event, hour, minute));
                    self.out.push(Out::Save);
                    VistaKp::Schedule {
                        is_inst,
                        step: SchedStep::SelectNum,
                        input: String::new(),
                        num: 0,
                        event: 0,
                        device: 1,
                        start_buf: "12002".to_string(),
                        days_buf: "1111111".to_string(),
                        pos: 0,
                        at: now,
                    }
                }
                Key::Hash => {
                    self.key_beep();
                    VistaKp::Schedule { is_inst, step: SchedStep::Days, input, num, event, device, start_buf, days_buf, pos: 0, at: now }
                }
                _ => {
                    self.error_tone();
                    VistaKp::Schedule { is_inst, step, input, num, event, device, start_buf, days_buf, pos, at: now }
                }
            },
        }
    }

    fn kv_macro_pgm(&mut self, slot: Option<usize>, viewing: bool, mut buf: String, key: Key, now: Instant) -> VistaKp {
        let Some(s) = slot else {
            return match key {
                Key::Digit(d @ 1..=4) => {
                    self.key_beep();
                    let idx = (d - 1) as usize;
                    let cur = self.vista_macros[idx].clone();
                    VistaKp::MacroPgm { slot: Some(idx), viewing: true, buf: cur, at: now }
                }
                Key::Digit(0) | Key::Hash | Key::Star => {
                    self.key_beep();
                    VistaKp::Normal
                }
                _ => {
                    self.error_tone();
                    VistaKp::MacroPgm { slot: None, viewing: false, buf: String::new(), at: now }
                }
            };
        };
        if viewing {
            match key {
                Key::Star => {
                    self.key_beep();
                    VistaKp::MacroPgm { slot: Some(s), viewing: false, buf: String::new(), at: now }
                }
                Key::Digit(0) | Key::Hash => {
                    self.key_beep();
                    VistaKp::Normal
                }
                _ => VistaKp::MacroPgm { slot: Some(s), viewing: true, buf, at: now },
            }
        } else {
            // Editing macro keystrokes (max 16 chars). Key D (`Key::Away`) inserts 'F'
            // command separator; a second Key D (or Key D on empty buffer) saves & exits.
            match key {
                Key::Away => {
                    self.key_beep();
                    if buf.is_empty() || buf.ends_with('F') {
                        let cleaned = buf.trim_end_matches('F').to_string();
                        self.vista_macros[s] = cleaned.clone();
                        self.vista_confirm_beep();
                        self.log(EvKind::Program, format!("Macro {} saved ({})", s + 1, if cleaned.is_empty() { "cleared" } else { &cleaned }));
                        self.out.push(Out::Save);
                        VistaKp::Normal
                    } else if buf.len() < 16 {
                        buf.push('F');
                        VistaKp::MacroPgm { slot: Some(s), viewing: false, buf, at: now }
                    } else {
                        self.vista_macros[s] = buf.clone();
                        self.vista_confirm_beep();
                        self.log(EvKind::Program, format!("Macro {} saved ({})", s + 1, buf));
                        self.out.push(Out::Save);
                        VistaKp::Normal
                    }
                }
                Key::Digit(d) => {
                    self.key_beep();
                    if buf.len() < 16 {
                        buf.push(char::from(b'0' + d));
                    } else {
                        self.error_tone();
                    }
                    VistaKp::MacroPgm { slot: Some(s), viewing: false, buf, at: now }
                }
                Key::Star => {
                    self.key_beep();
                    if buf.len() < 16 {
                        buf.push('*');
                    } else {
                        self.error_tone();
                    }
                    VistaKp::MacroPgm { slot: Some(s), viewing: false, buf, at: now }
                }
                Key::Hash => {
                    self.key_beep();
                    if buf.len() < 16 {
                        buf.push('#');
                    } else {
                        self.error_tone();
                    }
                    VistaKp::MacroPgm { slot: Some(s), viewing: false, buf, at: now }
                }
                _ => VistaKp::MacroPgm { slot: Some(s), viewing: false, buf, at: now },
            }
        }
    }

    fn kv_macro_code(&mut self, slot: usize, mut buf: String, key: Key, now: Instant) -> VistaKp {
        let Key::Digit(d) = key else {
            self.key_beep();
            return VistaKp::Normal;
        };
        self.key_beep();
        buf.push(char::from(b'0' + d));
        if buf.len() < 4 {
            return VistaKp::MacroCode { slot, buf, at: now };
        }
        let code = buf;
        if self.ee.find_code(&code).is_none() {
            self.bad_code(now);
            return VistaKp::Normal;
        }
        let macro_str = self.vista_macros.get(slot).cloned().unwrap_or_default();
        if macro_str.is_empty() {
            self.error_tone();
            return VistaKp::Normal;
        }
        self.log(EvKind::Info, format!("Executed Macro {}", slot + 1));
        for seg in macro_str.split('F') {
            if seg.is_empty() {
                continue;
            }
            // Reset keypad state cleanly before each macro command segment
            self.vista_mode = VistaKp::Normal;
            self.buf.clear();
            let raw_prefix = seg.starts_with("#0");
            if !raw_prefix {
                for ch in code.chars() {
                    if let Some(dig) = ch.to_digit(10) {
                        self.press_vista(Key::Digit(dig as u8), now);
                    }
                }
            }
            for ch in seg.chars() {
                let k = match ch {
                    '0'..='9' => Key::Digit(ch as u8 - b'0'),
                    '*' => Key::Star,
                    '#' => Key::Hash,
                    _ => continue,
                };
                self.press_vista(k, now);
            }
        }
        VistaKp::Normal
    }

    fn kv_output_cmd(&mut self, activate: bool, user: u8, mut buf: String, key: Key, now: Instant) -> VistaKp {
        let Key::Digit(d) = key else {
            self.error_tone();
            return VistaKp::Normal;
        };
        self.key_beep();
        buf.push(char::from(b'0' + d));
        if buf.len() < 2 {
            return VistaKp::OutputCmd { activate, user, buf };
        }
        let dev = buf.parse::<usize>().unwrap_or(0);
        if (1..=18).contains(&dev) {
            self.vista_outputs[dev - 1] = activate;
            self.vista_confirm_beep();
            let st = if activate { "ON" } else { "OFF" };
            self.vista_temp(&format!("DEVICE {:02} {}", dev, st), "", 3000, now);
            self.log_cid(
                EvKind::Info,
                format!("Output device {:02} {} by {}", dev, st, self.who(user)),
                400,
                if activate { 'E' } else { 'R' },
                dev as u16,
                false,
            );
            self.out.push(Out::Save);
        } else {
            self.error_tone();
        }
        VistaKp::Normal
    }

    fn kv_goto_part(&mut self, key: Key, now: Instant) -> VistaKp {
        let Key::Digit(d) = key else {
            self.error_tone();
            return VistaKp::Normal;
        };
        self.key_beep();
        if d <= 3 {
            let p = if d == 0 { 1 } else { d };
            self.vista_partition = p;
            self.vista_partition_at = if p == 1 { None } else { Some(now) };
            self.vista_confirm_beep();
            self.vista_temp(&format!("PARTITION {}", p), "", 2500, now);
            self.log(EvKind::Info, format!("Switched keypad to Partition {}", p));
        } else {
            self.error_tone();
        }
        VistaKp::Normal
    }

    fn kv_multi_part(&mut self, slot: String, user: u8, key: Key, now: Instant) -> VistaKp {
        match key {
            Key::Digit(1) => {
                self.key_beep();
                self.vista_off(&slot, user, now);
                VistaKp::Normal
            }
            Key::Digit(d @ (2 | 3 | 4 | 7)) => {
                self.key_beep();
                self.vista_code_disarm = None;
                self.kv_arm_action(d, &slot, user, false, now)
            }
            _ => {
                self.error_tone();
                VistaKp::Normal
            }
        }
    }

    fn kv_volume(&mut self, key: Key) -> VistaKp {
        match key {
            Key::Digit(3) => {
                self.vista_voice_level = (self.vista_voice_level + 1).min(7);
                self.audio(AudioCmd::VoiceLevel(self.vista_voice_level));
                self.vista_confirm_beep();
                self.vista_speak(vec!["system".into()]);
                self.out.push(Out::Save);
                VistaKp::Volume
            }
            Key::Digit(6) => {
                self.vista_voice_level = self.vista_voice_level.saturating_sub(1);
                self.audio(AudioCmd::VoiceLevel(self.vista_voice_level));
                self.vista_confirm_beep();
                if self.vista_voice_level > 0 {
                    self.vista_speak(vec!["system".into()]);
                }
                self.out.push(Out::Save);
                VistaKp::Volume
            }
            Key::Digit(4) => {
                self.vista_voice_on = !self.vista_voice_on;
                self.vista_confirm_beep();
                if self.vista_voice_on {
                    self.vista_speak(vec!["chime".into()]);
                }
                self.log(EvKind::Program, if self.vista_voice_on { "Voice status on" } else { "Voice status off" });
                self.out.push(Out::Save);
                VistaKp::Normal
            }
            Key::Hash | Key::Star => {
                self.vista_confirm_beep();
                VistaKp::Normal
            }
            _ => VistaKp::Volume,
        }
    }

    fn kv_program(&mut self, mut field: Option<String>, mut buf: String, review: bool, key: Key, _now: Instant) -> VistaKp {
        match key {
            Key::Star => {
                self.key_beep();
                VistaKp::Program { field: None, buf: "*".to_string(), review: false }
            }
            Key::Hash => {
                self.key_beep();
                if let Some(f) = &field {
                    if let Some(sp) = eeprom::spec(Model::Vista20p, f) {
                        if matches!(sp.kind, Kind::Phone(_)) && !review {
                            self.ee.sections.insert(f.clone(), buf);
                            self.beeps(3);
                            self.out.push(Out::Save);
                            return VistaKp::Program { field: next_field(f), buf: String::new(), review: false };
                        }
                    }
                }
                VistaKp::Program { field: None, buf: "#".to_string(), review: true }
            }
            Key::Digit(d) => {
                self.key_beep();
                let ch = char::from(b'0' + d);
                if field.is_none() {
                    if buf.is_empty() || !matches!(buf.as_bytes()[0], b'*' | b'#') {
                        self.error_tone();
                        return VistaKp::Program { field: None, buf: String::new(), review: false };
                    }
                    buf.push(ch);
                    let prefix = buf.chars().next().unwrap_or('*');
                    let digits = &buf[1..];
                    // Handle 2-digit and 3-digit (197, 198) fields
                    if digits == "19" {
                        return VistaKp::Program { field: None, buf, review };
                    }
                    if digits.len() >= 2 && (digits != "19" || digits.len() == 3) {
                        if prefix == '*' {
                            match digits {
                                "81" => return VistaKp::ZoneList(ZoneListMenu::default()),
                                "99" => {
                                    self.beeps(3);
                                    self.log_cid(EvKind::Program, "Exited Vista installer programming (*99)", 628, 'E', 0, true);
                                    self.out.push(Out::Save);
                                    return VistaKp::Normal;
                                }
                                "98" => {
                                    self.ee.installer_lockout = true;
                                    self.beeps(3);
                                    self.log_cid(EvKind::Program, "Exited Vista programming with lockout (*98)", 628, 'E', 0, true);
                                    self.out.push(Out::Save);
                                    return VistaKp::Normal;
                                }
                                "97" => {
                                    let inst = if self.ee.installer_lockout { self.ee.codes.get("inst").cloned() } else { None };
                                    let lock = self.ee.installer_lockout;
                                    self.ee = eeprom::Eeprom::factory(Model::Vista20p);
                                    self.ee.installer_lockout = lock;
                                    if let Some(c) = inst {
                                        self.ee.codes.insert("inst".into(), c);
                                    }
                                    self.beeps(3);
                                    self.log(EvKind::Program, "Vista factory defaults restored (*97)");
                                    self.out.push(Out::Save);
                                    return VistaKp::Program { field: None, buf: String::new(), review: false };
                                }
                                _ => {}
                            }
                        }
                        if eeprom::spec(Model::Vista20p, digits).is_some() {
                            return VistaKp::Program {
                                field: Some(digits.to_string()),
                                buf: String::new(),
                                review: prefix == '#',
                            };
                        } else {
                            self.error_tone();
                            self.vista_temp("ENTRY ERROR", "", 2000, _now);
                            return VistaKp::Program { field: None, buf: String::new(), review: false };
                        }
                    }
                    VistaKp::Program { field: None, buf, review }
                } else {
                    let f = field.take().unwrap();
                    if review {
                        return VistaKp::Program { field: Some(f), buf, review: true };
                    }
                    buf.push(ch);
                    if let Some(sp) = eeprom::spec(Model::Vista20p, &f) {
                        let need = eeprom::data_len(sp.kind);
                        if matches!(sp.kind, Kind::Code(_)) {
                            if buf.len() >= 4 {
                                if let Kind::Code(slot) = sp.kind {
                                    self.ee.set_code(slot, &buf[..4]);
                                }
                                self.beeps(3);
                                self.log(EvKind::Program, format!("Vista field *{} updated", f));
                                self.out.push(Out::Save);
                                return VistaKp::Program { field: next_field(&f), buf: String::new(), review: false };
                            }
                        } else if !matches!(sp.kind, Kind::Phone(_)) && need > 0 && buf.len() >= need {
                            self.ee.sections.insert(f.clone(), buf[..need].to_string());
                            self.beeps(3);
                            self.log(EvKind::Program, format!("Vista field *{} = {}", f, &buf[..need]));
                            self.out.push(Out::Save);
                            return VistaKp::Program { field: next_field(&f), buf: String::new(), review: false };
                        }
                    }
                    VistaKp::Program { field: Some(f), buf, review: false }
                }
            }
            _ => VistaKp::Program { field, buf, review },
        }
    }

    fn kv_zone_list(&mut self, mut menu: ZoneListMenu, key: Key) -> VistaKp {
        use ZoneListStep::*;
        self.key_beep();
        match (menu.step, key) {
            (DeleteList, Key::Digit(0)) => menu.step = DeleteZones,
            (DeleteList, Key::Digit(1)) => {
                self.save_vista_zone_list(menu.list, 0);
                menu = ZoneListMenu::default();
            }
            (DeleteZones, Key::Digit(0)) => menu = ZoneListMenu::default(),
            (DeleteZones, Key::Digit(1)) => menu.step = Remove,
            (Select | Add | Remove, Key::Digit(d)) if menu.digits.len() < 2 => {
                menu.digits.push(char::from(b'0' + d));
            }
            (Select | Add | Remove, Key::Hash) => {
                menu.digits.pop();
            }
            (Select | Add | Remove, Key::Star) if menu.digits.len() == 2 => {
                let number = std::mem::take(&mut menu.digits).parse::<u8>().unwrap_or(0);
                match (menu.step, number) {
                    (Select, 0) => return VistaKp::Program { field: None, buf: String::new(), review: false },
                    (Select, 1..=12) => {
                        menu.list = number;
                        menu.step = Add;
                    }
                    (Add, 0) => menu.step = DeleteList,
                    (Remove, 0) => menu = ZoneListMenu::default(),
                    (Add | Remove, 1..=64) => {
                        let bit = 1u64 << (number - 1);
                        let current = self.vista_zone_list(menu.list);
                        let updated = if matches!(menu.step, Add) { current | bit } else { current & !bit };
                        self.save_vista_zone_list(menu.list, updated);
                    }
                    _ => self.error_tone(),
                }
            }
            _ => self.error_tone(),
        }
        VistaKp::ZoneList(menu)
    }

    pub(crate) fn vista_zone_list(&self, list: u8) -> u64 {
        match list {
            3 | 5 => {
                let field = if list == 3 { "chime_list" } else { "81" };
                self.ee.raw(self.model, field).bytes().take(8).enumerate().fold(0, |mask, (i, value)| {
                    mask | (u64::from(value == b'1') << i)
                }) | self.ee.sections.get(&format!("zone_list_{:02}", list))
                    .and_then(|value| value.parse::<u64>().ok()).unwrap_or(0) & !0xff
            }
            _ => self.ee.sections.get(&format!("zone_list_{:02}", list)).and_then(|value| value.parse().ok()).unwrap_or(0),
        }
    }

    fn save_vista_zone_list(&mut self, list: u8, zones: u64) {
        if matches!(list, 3 | 5) {
            let field = if list == 3 { "chime_list" } else { "81" };
            let value = (0..8).map(|zone| if zones & (1 << zone) == 0 { '0' } else { '1' }).collect();
            self.ee.sections.insert(field.into(), value);
        }
        self.ee.sections.insert(format!("zone_list_{:02}", list), zones.to_string());
        self.out.push(Out::Save);
    }

    fn vista_stop_memo(&mut self) {
        if let Some(id) = self.vista_msg_recording.take() {
            self.vista_msg_saving = Some(id);
            self.out.push(Out::Memo(crate::audio::memo::Command::Stop(id)));
            self.dirty = true;
        }
    }

    pub(crate) fn vista_memo_result(&mut self, id: Instant, result: crate::audio::memo::Result, now: Instant) {
        if self.vista_msg_recording != Some(id) && self.vista_msg_saving != Some(id) {
            return;
        }
        self.vista_msg_recording = None;
        self.vista_msg_saving = None;
        match result {
            Ok(clip) => {
                self.vista_memo = Some(clip);
                self.vista_msg_waiting = true;
                self.beeps(2);
                self.vista_temp("MESSAGE SAVED", "#03 to play", 3000, now);
                self.log(EvKind::Info, "6160V voice message recorded");
            }
            Err(error) => {
                self.out.push(Out::Memo(crate::audio::memo::Command::Cancel));
                self.error_tone();
                self.vista_temp("MICROPHONE ERROR", "Review event log", 3000, now);
                self.log(EvKind::Info, format!("Could not record keypad message: {}", error));
            }
        }
        self.dirty = true;
    }

    /// Periodic Vista-specific timers (voice during exit delay, periodic voice
    /// during burglary alarm, 30 s mode timeouts, voice message center auto-stop,
    /// `#64` schedules, and GoTo Partition auto-revert).
    pub(crate) fn tick_vista(&mut self, now: Instant, wall_ms: i64) {
        if self.model != Model::Vista20p {
            return;
        }
        // 30-second timeout for interactive Time/Date, Event Log, Schedules, and Macro modes
        let timed_out_30 = match &self.vista_mode {
            VistaKp::TimeDate { at, .. }
            | VistaKp::EventLog { at, .. }
            | VistaKp::Schedule { at, .. }
            | VistaKp::MacroPgm { at, .. }
            | VistaKp::MacroCode { at, .. } => now.saturating_duration_since(*at).as_secs() >= 30,
            _ => false,
        };
        if timed_out_30 {
            self.vista_mode = VistaKp::Normal;
            self.dirty = true;
        }
        // Auto-revert GoTo Partition back to Partition 1 after 120 s idle
        if self.vista_partition != 1 && self.vista_partition_at.map_or(false, |t| now.saturating_duration_since(t).as_secs() >= 120) {
            self.vista_partition = 1;
            self.vista_partition_at = None;
            self.dirty = true;
        }
        // Voice Message Center (# + 0 + 1) auto-stops after 2.5 minutes (150 s)
        if self.vista_msg_recording.is_some_and(|t| now.saturating_duration_since(t).as_secs() >= crate::audio::memo::MAX_SECONDS) {
            self.vista_stop_memo();
        }
        // Clear expired reminder display
        if self.vista_reminder_until.map_or(false, |t| now >= t) {
            self.vista_reminder_until = None;
            self.dirty = true;
        }
        // A code entered during entry delay or an alarm disarms about 15
        // seconds later even if OFF isn't pressed.
        if self.vista_code_disarm.as_ref().map_or(false, |d| now >= d.0) {
            if let Some((_, user, slot)) = self.vista_code_disarm.take() {
                let busy = self.armed.is_some() || self.exit.is_some() || self.entry.is_some() || self.alarms.iter().any(|a| !a.silenced);
                if busy {
                    self.vista_mode = VistaKp::Normal;
                    self.vista_off(&slot, user, now);
                }
            }
        }
        // NIGHT-STAY keeps its zone list 05 zones armed after the exit delay.
        if self.vista_night {
            if self.armed.is_some() {
                self.vista_unbypass_night_zones();
            } else if self.exit.is_none() {
                self.vista_night = false;
            }
        }
        // Walk test auto-ends after 4 hours.
        if self.walk_test && self.vista_test_since.map_or(false, |t| now.saturating_duration_since(t).as_secs() >= 4 * 3600) {
            self.walk_test = false;
            self.vista_test_since = None;
            self.log(EvKind::Info, "Walk test ended after 4 hours");
        }
        // Deferred 6160V status voice after OFF.
        if self.vista_pending_status.map_or(false, |t| now >= t) {
            self.vista_pending_status = None;
            self.vista_speak_status();
        }
        // Code + BYPASS with no zone number for 5 seconds shows the bypassed zones.
        let show = matches!(&self.vista_mode, VistaKp::Bypass { buf, at } if buf.is_empty() && now.saturating_duration_since(*at).as_secs_f32() >= 5.0);
        if show {
            self.vista_mode = VistaKp::Normal;
            let list: Vec<u8> = (1..=self.n()).filter(|&z| self.zones[z - 1].bypassed).map(|z| z as u8).collect();
            if !list.is_empty() {
                self.vista_show_bypassed(&list, now);
            }
        }
        // After ~2.5 s of Away or Maximum exit delay, announce "Armed away/maximum [, zone bypassed], exit now"
        if let Some(ex) = &self.exit {
            if !self.vista_exit_voice_done
                && self.buf.is_empty()
                && matches!(self.vista_mode, VistaKp::Normal)
                && matches!(ex.how, ArmHow::AwayKey | ArmHow::MaxKey)
                && now.saturating_duration_since(ex.started).as_millis() >= 2500
            {
                self.vista_exit_voice_done = true;
                let toks = self.vista_exit_tokens(ex.how);
                self.vista_speak_if_on(toks);
            }
        }
        // During active burglary alarm, periodically (~14 s) speak "Alarm [zone]"
        let active_burg = self
            .alarms
            .iter()
            .find(|a| !a.silenced && now < a.bell_until && !a.kind.pulsed() && a.audible)
            .map(|a| (a.kind, a.zone));
        if let Some((kind, zone)) = active_burg {
            let due = self
                .vista_last_voice_alarm
                .map_or(true, |t| now.saturating_duration_since(t).as_secs() >= 14);
            if due {
                self.vista_last_voice_alarm = Some(now);
                self.vista_speak_alarm(kind, zone);
            }
        } else {
            self.vista_last_voice_alarm = None;
        }
        // Execute `#64` schedules matching current panel clock
        self.tick_vista_schedules(now, wall_ms);
    }

    fn tick_vista_schedules(&mut self, now: Instant, wall_ms: i64) {
        if self.vista_schedules.is_empty() {
            return;
        }
        let panel_ms = wall_ms + self.clock_offset_ms;
        let clock = util::parts(panel_ms);
        let weekday = util::weekday_idx(panel_ms);
        let day = util::local(panel_ms).num_days_from_ce();
        let week = day - i32::from(weekday);
        let mut to_fire = Vec::new();
        for schedule in &mut self.vista_schedules {
            if (self.following_panel && (4..=6).contains(&schedule.event))
                || u32::from(schedule.hour) != clock.hour
                || u32::from(schedule.minute) != clock.minute
                || schedule.days & (1 << weekday) == 0
                || schedule.start_day.is_some_and(|start| day < start)
                || schedule.last_run_day == Some(day)
            {
                continue;
            }
            let first_week = *schedule.first_run_week.get_or_insert(week);
            let weeks_elapsed = (week - first_week) / 7;
            if weeks_elapsed < 0 || weeks_elapsed % i32::from(schedule.repeat.max(1)) != 0 {
                continue;
            }
            schedule.last_run_day = Some(day);
            to_fire.push((schedule.num, schedule.event, schedule.device, schedule.repeat));
        }
        for (num, event, device, repeat) in to_fire {
            match event {
                1 => {
                    let idx = (device as usize).saturating_sub(1);
                    if idx < self.vista_outputs.len() {
                        self.vista_outputs[idx] = true;
                        self.log_cid(EvKind::Info, format!("Schedule {:02}: output {:02} activated", num, device), 400, 'E', device as u16, false);
                    }
                }
                4 => {
                    if self.armed.is_none() && self.exit.is_none() {
                        // Forced Stay arm bypasses open bypassable zones first
                        for z in 1..=self.n() {
                            if self.violated(z) && self.cfg(z).bypassable {
                                self.zones[z - 1].bypassed = true;
                            }
                        }
                        self.log(EvKind::Arm, format!("Schedule {:02}: forced Stay arm", num));
                        let _ = self.kv_arm_action(3, "", 0, true, now);
                    }
                }
                5 => {
                    if self.armed.is_none() && self.exit.is_none() {
                        for z in 1..=self.n() {
                            if self.violated(z) && self.cfg(z).bypassable {
                                self.zones[z - 1].bypassed = true;
                            }
                        }
                        self.log(EvKind::Arm, format!("Schedule {:02}: forced Away arm", num));
                        let _ = self.kv_arm_action(2, "", 0, true, now);
                    }
                }
                6 => {
                    if self.armed.is_some() || self.exit.is_some() {
                        self.log(EvKind::Disarm, format!("Schedule {:02}: auto disarm", num));
                        self.vista_off("02", 2, now);
                    }
                }
                7 => {
                    self.vista_reminder_until = Some(now + Duration::from_secs(3600));
                    self.beeps(3);
                    self.log(EvKind::Info, format!("Schedule {:02}: REMINDER", num));
                }
                11 => {
                    self.log_cid(EvKind::Info, format!("Schedule {:02}: periodic test report (CID 602)", num), 602, 'E', 0, false);
                    self.report(ReportKind::Restore, 602, 0, "Periodic test report".to_string(), 0);
                }
                _ => {}
            }
            if repeat == 0 {
                self.vista_schedules.retain(|s| s.num != num);
            }
            self.out.push(Out::Save);
            self.dirty = true;
        }
    }

    /// Authentic 6160V ECP `0xF7` backlight flag (`cbuf[12] & 0x80`): stays lit on
    /// recent keypress (<30 s), active menus, alarms, delays, open zones, or unacked trouble.
    pub fn vista_backlight(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_key_at).as_secs() < 30
            || self.alarm_sounding(now)
            || self.entry.is_some()
            || self.exit.is_some()
            || !self.open_zones().is_empty()
            || !matches!(self.vista_mode, VistaKp::Normal)
            || self.vista_msg_recording.is_some()
            || (self.troubles & !self.trouble_ack) != 0
    }

    /// Authentic 6160V 2x16 Alpha Display strings (with optional HD44780 underline cursor).
    pub fn lcd_vista(&self, now: Instant, wall_ms: i64) -> LcdOut {
        if let Some((l1, l2, until)) = &self.temp_msg {
            if now < *until {
                return plain(l1, l2);
            }
        }
        if let Some(until) = self.lockout_until {
            if now < until {
                return plain("KEYPAD LOCKOUT", "TRY AGAIN LATER");
            }
        }
        match &self.vista_mode {
            VistaKp::Program { field, buf, review } => {
                return match field {
                    None => with_cursor("Installer Code", "              20", 1, 14),
                    Some(f) => {
                        let sp = eeprom::spec(Model::Vista20p, f);
                        let title = sp.map(|s| s.name).unwrap_or("Field");
                        if *review {
                            plain(title, &self.ee.raw(Model::Vista20p, f))
                        } else if buf.is_empty() {
                            let shown = format!("*{} [{}]", f, self.ee.raw(Model::Vista20p, f));
                            with_cursor(title, &shown, 1, (f.len() + 3).min(15))
                        } else {
                            let shown = format!("*{} {}", f, buf);
                            with_cursor(title, &shown, 1, shown.len().min(15))
                        }
                    }
                };
            }
            VistaKp::ZoneList(menu) => {
                use ZoneListStep::*;
                let (title, line) = match menu.step {
                    Select => ("Zone List No.", format!("00=Quit       {}", menu.digits)),
                    Add => ("Enter Zn Num.", format!("{:02} List     {}", menu.list, menu.digits)),
                    DeleteList => ("Del Zn List?", "0=No 1=Yes".into()),
                    DeleteZones => ("Delete Zone?", "0=No 1=Yes".into()),
                    Remove => ("Zn to Delete?", format!("00=Quit       {}", menu.digits)),
                };
                return with_cursor(title, &line, 1, line.len());
            }
            VistaKp::UserCode { slot, buf, first, .. } => {
                let (s, col) = if slot.len() < 2 {
                    (format!("USER: {}_", slot), 6 + slot.len())
                } else if buf == "#" {
                    (format!("U{} ATTR: #0/1/2/5", slot), 15)
                } else if buf == "#1" {
                    (format!("U{} AUTH (0-4):_", slot), 15)
                } else if buf == "#2" {
                    (format!("U{} GROUP (0-8):_", slot), 15)
                } else if buf.starts_with("#3") {
                    let line = format!("U{} PART: {}#", slot, &buf[2..]);
                    let c = line.len().saturating_sub(1);
                    (line, c)
                } else if buf == "#5" {
                    (format!("U{} PAGE (0/1):_", slot), 15)
                } else if !first.is_empty() {
                    let line = format!("U{} AGAIN: {}", slot, "*".repeat(buf.len()));
                    let c = line.len();
                    (line, c)
                } else {
                    let line = format!("U{} CODE: {}", slot, "*".repeat(buf.len()));
                    let c = line.len();
                    (line, c)
                };
                return with_cursor("USER CODE PROG", &s, 1, col.min(15));
            }
            VistaKp::Volume => {
                let l2 = if self.vista_voice_level == 0 {
                    "MUTED (3+ 6-)".to_string()
                } else {
                    format!("VOL {} (3+ 6-)", self.vista_voice_level)
                };
                return plain("VOICE VOLUME", &l2);
            }
            VistaKp::Bypass { buf, .. } => {
                return if buf.is_empty() {
                    plain("BYPASS ZONE", "Enter Zn (01-48)")
                } else {
                    with_cursor("BYPASS ZONE", &format!("Zone: {}_", buf), 1, (6 + buf.len()).min(15))
                };
            }
            VistaKp::TestWait { .. } => {
                return plain("1=DIAL, 0=WALK", "");
            }
            VistaKp::TimeDate { editing, digits, pos, .. } => {
                let panel_ms = wall_ms + self.clock_offset_ms;
                let (l1, l2_view) = util::fmt_vista_time_date(panel_ms);
                if !editing {
                    return plain(&l1, &l2_view);
                }
                let b: Vec<char> = digits.chars().collect();
                if b.len() == 11 {
                    let ap = if b[4] == '1' { 'P' } else { 'A' };
                    let l2 = format!(
                        "{}{}:{}{}{}20{}{}/{}{}/{}{}",
                        b[0], b[1], b[2], b[3], ap, b[5], b[6], b[7], b[8], b[9], b[10]
                    );
                    let col = TIME_COLS[(*pos).min(10)];
                    return with_cursor(&l1, &l2, 1, col);
                }
                return plain(&l1, &l2_view);
            }
            VistaKp::EventLog { idx, .. } => {
                let entries = self.vista_event_log_entries();
                if entries.is_empty() {
                    return plain("EVENT LOG", "NO EVENTS");
                }
                let i = (*idx).min(entries.len() - 1);
                let ev = entries[i];
                let (cid, qual, zu, is_user) = vista_event_cid(ev);
                let zu_ch = if is_user { 'U' } else { 'Z' };
                let l1 = format!("{:03} {}{:03} {}{:03} P1", i + 1, qual, cid, zu_ch, zu.min(999));
                let l2 = util::fmt_vista_event_time(ev.ts + self.clock_offset_ms);
                return plain(&l1, &l2);
            }
            VistaKp::Schedule { step, input, num, start_buf, days_buf, pos, .. } => {
                return match step {
                    SchedStep::SelectNum => plain("ENTER SCHED NO.", &format!("(00=QUIT)    {:2}", if input.is_empty() { "00" } else { input })),
                    SchedStep::Event => plain(&format!("SCHED {:02} EVENT", num), &format!("00,01,04-07  {:2}", input)),
                    SchedStep::Device => plain("DEVICE NUMBER", &format!("01-18        {:2}", input)),
                    SchedStep::StartTime => {
                        let b: Vec<char> = start_buf.chars().collect();
                        let ap = if b.get(4) == Some(&'1') { 'P' } else { 'A' };
                        let l2 = if b.len() == 5 {
                            format!("{}{}:{}{} {}  *=NEXT", b[0], b[1], b[2], b[3], ap)
                        } else {
                            "12:00 A  *=NEXT".to_string()
                        };
                        let col = [0usize, 1, 3, 4, 6][(*pos).min(4)];
                        with_cursor("START TIME 1P/2A", &l2, 1, col)
                    }
                    SchedStep::Days => with_cursor("DAYS  SMTWTFS", &format!("      {} *", days_buf), 1, 6 + (*pos).min(6)),
                    SchedStep::Repeat => plain("REPEAT OPTION", "0-4 (0=ONCE)   _"),
                };
            }
            VistaKp::MacroPgm { slot, viewing, buf, .. } => {
                return match slot {
                    None => plain("SELECT MACRO", "1-4            0"),
                    Some(s) if *viewing => {
                        let shown = if buf.is_empty() { "(EMPTY) *=EDIT" } else { buf.as_str() };
                        plain(&format!("MACRO {} DISPLAY", s + 1), shown)
                    }
                    Some(s) => with_cursor(&format!("MACRO {} PGM", s + 1), buf, 1, buf.len().min(15)),
                };
            }
            VistaKp::MacroCode { buf, .. } => {
                let stars = format!("{}{}", "*".repeat(buf.len()), "_".repeat(4 - buf.len().min(4)));
                return with_cursor("ENTER USER CODE", &stars, 1, buf.len().min(3));
            }
            VistaKp::OutputCmd { activate, buf, .. } => {
                let l1 = if *activate { "DEVICE TO ACT." } else { "DEVICE TO DEACT." };
                return with_cursor(l1, &format!("ENTER 01-18: {:2}", buf), 1, (13 + buf.len()).min(15));
            }
            VistaKp::GoToPart => {
                return plain("GOTO PARTITION", "ENTER 0-3");
            }
            VistaKp::MultiPart { .. } => {
                return plain("GLOBAL COMMAND", "1=OFF 2=AWY 3=ST");
            }
            _ => {}
        }

        // Active live alarm
        if let Some(a) = self.live_alarms(now).next() {
            if a.kind.pulsed() {
                if a.zone == 95 || a.zone == 0 {
                    let (l1, l2) = split_zone_lines("FIRE", 95, "FIRE PANIC");
                    return plain(&l1, &l2);
                }
                let lbl = self.label(a.zone as usize).to_ascii_uppercase();
                let (l1, l2) = split_zone_lines("FIRE", a.zone, &lbl);
                return plain(&l1, &l2);
            }
            if a.zone == 99 {
                return plain("ALARM 99 POLICE", "PANIC");
            }
            if a.zone == 96 {
                return plain("ALARM 96 MEDICAL", "PANIC");
            }
            let lbl = self.label(a.zone as usize).to_ascii_uppercase();
            let (l1, l2) = split_zone_lines("ALARM", a.zone, &lbl);
            return plain(&l1, &l2);
        }

        if let Some((top, bottom)) = self.physical_lcd() {
            return plain(top, bottom);
        }
        // Entry delay (alternates EXIT ALARM when triggered by an exit-error fault)
        if let Some(e) = &self.entry {
            if e.exit_fault && (now.saturating_duration_since(e.started).as_millis() / 2000) % 2 == 1 {
                return plain("EXIT ALARM", "DISARM SYSTEM");
            }
            return plain("DISARM SYSTEM", "or alarm occurs");
        }

        // Exit delay
        if let Some(ex) = &self.exit {
            let left = (ex.started + ex.dur).saturating_duration_since(now).as_secs();
            let head = match ex.how {
                ArmHow::StayKey if self.vista_night => "ARMED NIGHT-STAY",
                ArmHow::StayKey => "ARMED ***STAY***",
                ArmHow::MaxKey => "ARMED *MAXIMUM*",
                ArmHow::NoEntry => "ARMED *INSTANT*",
                _ => "ARMED ***AWAY***",
            };
            // *197 Exit Time Display Interval: 0 = "You may exit now",
            // 1..=5 = countdown seconds updated in multiples of that interval!
            let iv = self.ee.entry_or(Model::Vista20p, "197", 0, 0).min(5) as u64;
            let l2 = if iv > 0 {
                let shown = if iv == 1 { left } else { ((left + iv - 1) / iv) * iv };
                format!("May Exit Now {:02}", shown.min(99))
            } else {
                "You may exit now".to_string()
            };
            return plain(head, &l2);
        }

        // Armed idle
        if let Some(a) = &self.armed {
            let head = match (a.mode, a.no_entry) {
                (ArmMode::Away, true) => "ARMED *MAXIMUM*",
                (ArmMode::Away, false) => "ARMED ***AWAY***",
                (ArmMode::Stay, true) => "ARMED *INSTANT*",
                (ArmMode::Stay, false) if self.vista_night => "ARMED NIGHT-STAY",
                (ArmMode::Stay, false) => "ARMED ***STAY***",
            };
            let byp = self.manual_bypass_any();
            let restart = a.quick_exit.map_or(false, |t| now < t);
            let l2 = if restart {
                "You may exit now"
            } else if byp {
                "ZONE BYPASSED"
            } else {
                "** ALL SECURE **"
            };
            return plain(head, l2);
        }

        // Active 6160V Voice Message Center recording
        if self.vista_msg_recording.is_some() {
            return plain("RECORDING MSG", "Press 1 to stop");
        }

        // Walk test
        if self.walk_test {
            return plain("TEST IN PROGRESS", "Code+1 to exit");
        }

        // Latched Alarm Memory after 1st [Code] + 1 disarm:
        // Authentic 0xF7 ECP display alternates ALARM XX <ZONE> and DISARMED every 2.5 s
        // until the 2nd [Code] + 1 clears memory (phase 0 starts on ALARM XX).
        let cycle = (now.saturating_duration_since(self.last_key_at).as_millis() / 2500) as usize;
        if let Some((z, k)) = self.vista_alarm_mem_zone {
            if cycle % 2 == 0 {
                let prefix = if k.pulsed() { "FIRE" } else { "ALARM" };
                let lbl = if z == 95 {
                    "FIRE PANIC".to_string()
                } else if z == 99 {
                    "POLICE PANIC".to_string()
                } else if z == 96 {
                    "MEDICAL PANIC".to_string()
                } else if z > 0 {
                    self.label(z as usize).to_ascii_uppercase()
                } else {
                    "MEMORY".to_string()
                };
                let (l1, l2) = split_zone_lines(prefix, z, &lbl);
                return plain(&l1, &l2);
            }
        }

        // Disarmed: check *198 Display Partition Number prefix
        let show_part = self.ee.entry_or(Model::Vista20p, "198", 0, 0) != 0;
        let line1 = if show_part {
            if self.chime {
                format!("{} DISARMED CHIME", self.vista_partition)
            } else if self.manual_bypass_any() {
                format!("{} DISARMED BYPAS", self.vista_partition)
            } else {
                format!("{} **DISARMED**", self.vista_partition)
            }
        } else if self.chime {
            "DISARMED CHIME".to_string()
        } else if self.manual_bypass_any() {
            "DISARMED BYPASS".to_string()
        } else {
            "****DISARMED****".to_string()
        };

        let open = self.open_zones();
        let ready = if open.is_empty() { "  Ready to Arm" } else { "Hit * for faults" };
        // Collect all active trouble/CHECK conditions so multiple simultaneous troubles
        // cycle cleanly every 2.5 s on the 6160V just like real VISTA-20P 0xF7 packets.
        let mut trouble_screens: Vec<(String, String)> = Vec::new();
        if (self.troubles & (tb::ZONE_FAULT | tb::ZONE_TAMPER | tb::HOME_SENSOR)) != 0 {
            for z in 1..=self.n() {
                if self.camera_zone.get(z - 1).copied().unwrap_or(false) && self.violated(z) {
                    let lbl = self.label(z).to_ascii_uppercase();
                    trouble_screens.push(split_zone_lines("CHECK", z as u8, &lbl));
                }
            }
        }
        if !open.is_empty() {
            if !trouble_screens.is_empty() && cycle % (trouble_screens.len() + 1) < trouble_screens.len() {
                let (l1, l2) = &trouble_screens[cycle % (trouble_screens.len() + 1)];
                return plain(l1, l2);
            }
            return plain(&line1, "Hit * for faults");
        }
        if self.vista_reminder_until.map_or(false, |t| now < t) {
            return plain(&line1, "REMINDER");
        }
        if (self.troubles & tb::AC) != 0 {
            trouble_screens.push(("AC LOSS".to_string(), ready.to_string()));
        }
        if (self.troubles & tb::LOW_BATT) != 0 {
            trouble_screens.push(("SYSTEM LO BAT".to_string(), ready.to_string()));
        }
        if (self.troubles & tb::BELL) != 0 {
            trouble_screens.push(("BELL FAILURE".to_string(), ready.to_string()));
        }
        if (self.troubles & tb::FTC) != 0 {
            trouble_screens.push(("COMM. FAILURE".to_string(), ready.to_string()));
        }
        if !trouble_screens.is_empty() {
            let (l1, l2) = &trouble_screens[cycle % trouble_screens.len()];
            return plain(l1, l2);
        }
        plain(&line1, ready)
    }

    /// [Code] + OFF: disarms and silences; a second one clears the memory of
    /// alarm display (and resets smoke detectors after fire). With nothing to
    /// disarm it removes any bypasses. Enforces VISTA-20P authority levels.
    pub(crate) fn vista_off(&mut self, slot: &str, user: u8, now: Instant) {
        self.vista_code_disarm = None;
        self.vista_sounder_mute_until = None;
        let is_armed = self.armed.is_some() || self.exit.is_some();
        let has_live_alarm = self.alarms.iter().any(|a| !a.silenced);

        // Enforce VISTA-20P authority levels (User Guide p.28):
        //   - Installer code can only disarm if it was used to arm.
        //   - Authority 1 (Arm-Only) cannot disarm or silence.
        //   - Authority 2 (Guest/Babysitter) can only disarm if used to arm.
        //   - Authority 3 (Duress) acts normally and silently sends CID 121 Duress.
        if slot == "inst" && is_armed && !self.vista_armed_by_inst {
            self.error_tone();
            return;
        }
        let auth = if slot == "02" || slot == "inst" || slot.is_empty() {
            0
        } else {
            self.ee.attrs(slot) & 0x07
        };
        if auth == 1 && (is_armed || has_live_alarm || self.entry.is_some()) {
            self.error_tone();
            return;
        }
        if auth == 2 && is_armed && self.vista_armed_by_slot.as_deref() != Some(slot) {
            self.error_tone();
            return;
        }
        if auth == 3 {
            self.trigger(AlarmKind::Duress, 0, false, false, now);
        }

        let status_at = now + Duration::from_millis(2190);
        if self.walk_test {
            self.walk_test = false;
            self.vista_test_since = None;
            self.vista_off_beep();
            self.log(EvKind::Info, "Walk test ended");
            self.vista_pending_status = Some(status_at);
            return;
        }
        if is_armed || has_live_alarm {
            let alarmed_info = self.alarms.iter().find(|a| a.kind != AlarmKind::Duress).map(|a| (a.zone, a.kind));
            if is_armed {
                self.disarm(user, now);
            } else {
                self.silence(user);
            }
            self.vista_off_beep();
            self.vista_armed_by_inst = false;
            self.vista_armed_by_slot = None;
            self.vista_night = false;
            if let Some((z, k)) = alarmed_info {
                self.vista_alarm_mem_zone = Some((z, k));
                self.vista_pending_status = None;
                self.vista_speak(vec!["disarmed".into()]);
            } else {
                self.vista_pending_status = Some(status_at);
            }
        } else if self.vista_alarm_mem_zone.is_some() || self.memory_flag {
            let was_fire = self.vista_alarm_mem_zone.map_or(false, |(_, k)| k.pulsed());
            self.vista_alarm_mem_zone = None;
            self.memory_flag = false;
            for z in self.zones.iter_mut() {
                z.alarm = false;
            }
            if was_fire {
                // User Guide p.17: second [Code] + OFF resets smoke detectors (5 s power reset)
                self.fire_reset_until = Some(now + Duration::from_secs(5));
            }
            if self.ee.entry_or(Model::Vista20p, "86", 0, 1) != 0 {
                self.temp_msg = Some(("Alarm Canceled".into(), String::new(), now + Duration::from_millis(1500)));
            }
            self.vista_off_beep();
            self.log_cid(EvKind::Disarm, if was_fire { "Fire alarm memory cleared (smoke detectors reset)" } else { "Alarm memory cleared" }, 406, 'E', user as u16, true);
            self.vista_pending_status = Some(status_at);
        } else {
            let had = self.zones.iter().any(|z| z.bypassed);
            for z in self.zones.iter_mut() {
                z.bypassed = false;
                z.auto_bypass = false;
            }
            if had {
                self.log_cid(EvKind::Info, "Bypasses removed", 570, 'R', 0, false);
            }
            self.vista_off_beep();
            self.vista_pending_status = Some(status_at);
        }
    }

    /// NIGHT-STAY arms the perimeter plus the zones in zone list 05 (field *81).
    pub(crate) fn vista_set_night(&mut self) {
        self.vista_night = true;
        self.vista_unbypass_night_zones();
        self.log(EvKind::Arm, "Armed NIGHT-STAY");
        self.out.push(Out::Save);
        self.vista_arm_beeps();
        let toks = self.vista_exit_tokens(ArmHow::StayKey);
        self.vista_speak_if_on(toks);
    }

    fn vista_night_zone(&self, z: usize) -> bool {
        self.vista_zone_list(5) & (1u64 << z.saturating_sub(1)) != 0
    }

    fn vista_unbypass_night_zones(&mut self) {
        for z in 1..=self.n() {
            if self.zones[z - 1].auto_bypass && self.vista_night_zone(z) {
                self.zones[z - 1].bypassed = false;
                self.zones[z - 1].auto_bypass = false;
            }
        }
    }

    /// "BYPAS 05 FRONT DOOR", with one beep for each zone shown.
    fn vista_show_bypassed(&mut self, zones: &[u8], now: Instant) {
        let Some(&first) = zones.first() else {
            return;
        };
        let (l1, l2) = if zones.len() == 1 {
            let lbl = self.label(first as usize).to_ascii_uppercase();
            split_zone_lines("BYPAS", first, &lbl)
        } else {
            ("BYPAS".to_string(), zones.iter().map(|z| format!("{:02}", z)).collect::<Vec<_>>().join(" "))
        };
        self.temp_msg = Some((l1, l2, now + Duration::from_millis(3000)));
        self.dirty = true;
        self.audio(AudioCmd::Beeps { count: zones.len().min(8) as u8, on_ms: 100, off_ms: 300 });
    }

    /// Test mode: a faulted zone shows on the display, and the voice keypad says it.
    pub(crate) fn vista_test_fault(&mut self, z: u8, label: &str) {
        let (l1, l2) = split_zone_lines("FAULT", z, label);
        self.temp(&l1, &l2, 3000);
        if self.vista_voice_on {
            let mut toks = vec!["fault".to_string()];
            let w = voice::filter_zone_words(label);
            if w.is_empty() {
                toks.push("zone".into());
                toks.extend(voice::number_tokens(z));
            } else {
                toks.extend(w);
            }
            self.vista_speak(toks);
        }
    }

    /// Single button arming from a function key: no code needed, and no
    /// second STAY press for NIGHT-STAY.
    pub(crate) fn vista_arm_key(&mut self, digit: u8, now: Instant) {
        let _ = self.kv_arm_action(digit, "", 0, true, now);
    }

    /// Step arming's third press: from NIGHT-STAY up to AWAY.
    pub(crate) fn vista_step_to_away(&mut self) {
        if let Some(a) = self.armed.as_mut() {
            a.mode = ArmMode::Away;
        }
        let exiting = if let Some(ex) = self.exit.as_mut() {
            ex.how = ArmHow::AwayKey;
            ex.silent = false;
            true
        } else {
            false
        };
        self.vista_night = false;
        for z in self.zones.iter_mut() {
            if z.auto_bypass {
                z.bypassed = false;
                z.auto_bypass = false;
            }
        }
        self.beeps(2);
        self.log(EvKind::Arm, "Armed AWAY (step arming)");
        self.out.push(Out::Save);
        if exiting {
            self.vista_exit_voice_done = true;
            let toks = self.vista_exit_tokens(ArmHow::AwayKey);
            self.vista_speak_if_on(toks);
        } else {
            let toks = self.vista_armed_tokens(ArmMode::Away, false);
            self.vista_speak_if_on(toks);
        }
    }

    /// Function code 08: communication test (Contact ID 601).
    pub(crate) fn vista_comm_test(&mut self) {
        self.vista_confirm_beep();
        self.log_cid(EvKind::Info, "Communication test sent (Contact ID 601)", 601, 'E', 0, false);
        self.report(ReportKind::Restore, 601, 0, "Communication test".to_string(), 0);
        self.temp("PHONE OKAY", "", 3000);
    }

    /// Function code 00 on key D, or 01: single button paging, "999-9999".
    pub(crate) fn vista_page(&mut self) {
        self.vista_confirm_beep();
        self.log(EvKind::Info, "Pager message 999-9999 sent");
        self.temp("PAGING", "999-9999", 3000);
    }

    /// 6160V status LEDs: [READY (green), ARMED (red), MESSAGE (red), AC/MIC].
    pub fn leds_vista(&self) -> [Led; 4] {
        let ready = if !self.is_on() && self.ready() {
            Led::On
        } else {
            Led::Off
        };
        let armed = if self.is_on() {
            Led::On
        } else if self.vista_alarm_mem_zone.is_some() {
            Led::Flash
        } else {
            Led::Off
        };
        // 6160V MESSAGE LED: steady while recording (#+0+1), flashing when a voice
        // memo is waiting. Playing it clears only the indicator.
        let msg = if self.vista_msg_recording.is_some() {
            Led::On
        } else if self.vista_msg_waiting {
            Led::Flash
        } else {
            Led::Off
        };
        let ac = if self.inputs.ac_ok { Led::On } else { Led::Off };
        [ready, armed, msg, ac]
    }
}

/// Derives authentic Contact ID metadata for `#60` Event Log rows (falls back
/// gracefully for legacy saved events that predate Contact ID stamping).
fn vista_event_cid(ev: &EventRow) -> (u16, char, u16, bool) {
    if ev.cid > 0 {
        let q = if ev.qual == 'R' { 'R' } else { 'E' };
        return (ev.cid, q, ev.zu, ev.is_user);
    }
    let low = ev.text.to_ascii_lowercase();
    if low.contains("bypass") {
        return (570, if low.contains("remov") { 'R' } else { 'E' }, 0, false);
    }
    match ev.kind {
        EvKind::Alarm => (130, 'E', 0, false),
        EvKind::Arm => {
            let cid = if low.contains("stay") || low.contains("instant") { 441 } else { 401 };
            (cid, 'R', 2, true)
        }
        EvKind::Disarm => (401, 'E', 2, true),
        EvKind::Trouble => (300, 'E', 0, false),
        EvKind::Restore => (300, 'R', 0, false),
        EvKind::Program => (627, 'E', 0, true),
        EvKind::Info | EvKind::Camera => (601, 'E', 0, false),
    }
}

/// The data field after `f`, the way the real panel steps on to the next
/// field once an entry is complete.
fn next_field(f: &str) -> Option<String> {
    let mut keys: Vec<&'static str> = eeprom::VISTA20P
        .iter()
        .filter(|s| !matches!(s.kind, Kind::Action) && !matches!(s.key, "02" | "81") && s.key.chars().all(|c| c.is_ascii_digit()))
        .map(|s| s.key)
        .collect();
    keys.sort_by_key(|k| k.parse::<u32>().unwrap_or(0));
    let i = keys.iter().position(|k| *k == f)?;
    keys.get(i + 1).map(|k| k.to_string())
}

/// A 6160 starts a zone's description at a fixed column, whatever comes
/// before it, as filmed on a real keypad: "ALARM 95 POLICE" / "PANIC",
/// "FIRE 99  FIRE" / "PANIC", "BYPAS 09 MASTER" / "BEDROOM MOTION".
fn split_zone_lines(prefix: &str, zone: u8, label: &str) -> (String, String) {
    let mut l1 = if zone > 0 { format!("{} {:02}", prefix, zone) } else { prefix.to_string() };
    let col = l1.len().max(8) + 1;
    while l1.len() < col {
        l1.push(' ');
    }
    let mut l2 = String::new();
    let mut first_line = true;
    for w in label.split_whitespace() {
        if first_line {
            let sep = usize::from(!l1.ends_with(' '));
            if l1.len() + sep + w.len() <= 16 {
                if sep == 1 {
                    l1.push(' ');
                }
                l1.push_str(w);
                continue;
            }
            first_line = false;
        }
        if !l2.is_empty() {
            l2.push(' ');
        }
        l2.push_str(w);
    }
    (l1.trim_end().to_string(), l2)
}
