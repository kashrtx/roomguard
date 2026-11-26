//! VistaGuard 20P (VISTA-20P compatible) + VG6160V Alpha Display Voice Keypad.
//!
//! Operates on the authentic [Code] + [Action Key] firmware philosophy:
//!   - [Code] + 1 (OFF): Disarm / Silence (two-step: 1st silences & latches
//!     alarm memory, 2nd clears alarm memory & flashes Alarm Canceled)
//!   - [Code] + 2 (AWAY): Arm Away with Exit Delay + Auto-Stay (*84)
//!   - [Code] + 3 (STAY): Arm Stay (bypasses interior zones)
//!   - [Code] + 4 (MAX): Arm Away with NO Entry Delay
//!   - [Code] + 5 (TEST) + 0: Walk Test mode
//!   - [Code] + 6 (BYPASS) + [01-08] or [#]: Manual or Quick Bypass (*23)
//!   - [Code] + 7 (INSTANT): Arm Stay with NO Entry Delay
//!   - [Master] + 8 + [03..49] + [4-digit]: User code management
//!   - [Code] + 9 (CHIME): Toggle Chime Mode
//!   - [*] (READY): View faulted zones / [#] + [*] speak voice status
//!   - [#] + [2/3/4/7]: Quick Arm when *21 = 1
//!   - [#] + 0 + 2: 6160V Voice Volume control (3 = Up, 6 = Down, # = Save)
//!   - [Installer 4112] + 8 + 0 + 0: Installer Programming (*field / #field / *97 / *98 / *99)
//!   - Hold A (Fire 95), B (Police 99), C (Medical 96), D (Away / Programmable *57)

use std::time::{Duration, Instant};

use super::eeprom::{self, Kind};
use super::kp1832::{pad, LcdOut};
use super::panel::*;
use super::types::*;
use crate::audio::voice;
use crate::audio::AudioCmd;

#[derive(Clone, Debug, Default)]
pub enum VistaKp {
    #[default]
    Normal,
    /// Waiting for action digit (1-9) after a valid 4-digit code or '#' quick-arm.
    Command { slot: String, user: u8, quick: bool, at: Instant },
    /// [Code] + 6: waiting for a 2-digit zone number (or '#' for quick bypass).
    /// With no digits, the bypassed zones are shown after a moment.
    Bypass { buf: String, at: Instant },
    /// [Code] + 5 (TEST): 0 starts the walk test, 1 the dialer test.
    TestWait,
    /// [Master] + 8 + [2-digit user] + [4-digit code], or # 0 to delete. The
    /// master's own code (user 02) is entered twice.
    UserCode { slot: String, buf: String, first: String },
    /// Installer programming: [Installer] + 8 + 0 + 0.
    Program { field: Option<String>, buf: String, review: bool },
    /// [#] + 0 + 2: voice volume (3 up, 6 down, 4 voice on or off, # done).
    Volume,
    /// Just armed STAY: pressing STAY again makes it NIGHT-STAY.
    NightWait { at: Instant },
}

fn plain(a: &str, b: &str) -> LcdOut {
    LcdOut { lines: [pad(a), pad(b)], cursor: None }
}

impl Panel {
    /// Speaks a sequence of 6160V voice ROM tokens.
    pub fn vista_speak(&mut self, tokens: Vec<String>) {
        if self.model == Model::Vista20p && !tokens.is_empty() {
            self.audio(AudioCmd::Speak(tokens));
        }
    }

    /// Speaks an alarm announcement using the 6160V ROM whitelist.
    pub fn vista_speak_alarm(&mut self, kind: AlarmKind, zone: u8) {
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

    /// Speaks current system status (triggered by # + * or after disarm).
    pub fn vista_speak_status(&mut self) {
        if self.model != Model::Vista20p || !self.vista_voice_on {
            return;
        }
        let mut toks = Vec::new();
        if let Some(a) = &self.armed {
            toks.push("armed".into());
            match (a.mode, a.no_entry) {
                (ArmMode::Away, true) => toks.push("maximum".into()),
                (ArmMode::Away, false) => toks.push("away".into()),
                (ArmMode::Stay, true) => toks.push("instant".into()),
                (ArmMode::Stay, false) => toks.push("stay".into()),
            }
        } else if self.exit.is_some() {
            toks.push("armed".into());
            toks.push("away".into());
            toks.push("exit_now".into());
        } else {
            toks.push("disarmed".into());
            if self.ready() {
                toks.push("ready_to_arm".into());
            } else {
                toks.push("not_ready_to_arm".into());
            }
        }
        self.vista_speak(toks);
    }

    /// Single crisp 6160V confirmation beep (~220 ms) followed by a short 80 ms
    /// breath before the voice chip speaks.
    pub fn vista_confirm_beep(&mut self) {
        // Measured from a recording of a real 6160V: one low 1019 Hz beep of
        // about 340 ms, and the voice starts about 750 ms after it ends.
        self.audio(AudioCmd::Tone { hz: 1019, on_ms: 340, off_ms: 750, delay_ms: 0 });
    }

    /// After [Code] + OFF the same beep comes about 1.1 s after the key, the
    /// panel's answer, as two videos of a real 6160 show.
    pub fn vista_off_beep(&mut self) {
        self.audio(AudioCmd::Tone { hz: 1019, on_ms: 340, off_ms: 750, delay_ms: 1100 });
    }

    pub(crate) fn press_vista(&mut self, key: Key, now: Instant) {
        // "When entering codes and commands, sequential key depressions must be
        // made within 4-5 seconds of one another. If 4-5 seconds elapse without a
        // key depression, the entry will be aborted."
        let stale = now.saturating_duration_since(self.vista_prev_key).as_secs_f32() > 5.0;
        self.vista_prev_key = now;
        if stale {
            self.buf.clear();
            if matches!(
                self.vista_mode,
                VistaKp::Command { .. } | VistaKp::Bypass { .. } | VistaKp::UserCode { .. } | VistaKp::TestWait | VistaKp::NightWait { .. }
            ) {
                self.vista_mode = VistaKp::Normal;
            }
        }
        // "To silence the beeping for these conditions, press any key."
        self.ack_troubles();

        let mode = std::mem::replace(&mut self.vista_mode, VistaKp::Normal);
        let next = match mode {
            VistaKp::Normal => self.kv_normal(key, now),
            VistaKp::Command { slot, user, quick, .. } => self.kv_command(slot, user, quick, key, now),
            VistaKp::Bypass { buf, at } => self.kv_bypass(buf, at, key, now),
            VistaKp::TestWait => self.kv_test_wait(key, now),
            VistaKp::UserCode { slot, buf, first } => self.kv_user_code(slot, buf, first, key),
            VistaKp::Program { field, buf, review } => self.kv_program(field, buf, review, key, now),
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
        };
        self.vista_mode = next;
        self.dirty = true;
    }

    fn kv_normal(&mut self, key: Key, now: Instant) -> VistaKp {
        match key {
            Key::Digit(d) => {
                self.key_beep();
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
                // # + 0 + 2: 6160V voice volume.
                if self.buf == "#0" && d == 2 {
                    self.buf.clear();
                    self.vista_confirm_beep();
                    self.vista_speak(vec!["system".into()]);
                    return VistaKp::Volume;
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
                    // During an entry delay or an alarm a code alone is enough. "The
                    // beeps stop when a code is entered, but will restart after about
                    // 15 seconds if an invalid code is entered", and a valid one
                    // disarms "in about 15 seconds" even without OFF.
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
                        return VistaKp::Command { slot, user, quick: false, at: now };
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
                // '#' is the quick arm prefix (when *21 allows it), and the voice
                // keypad's FUNCTION key (# + 0 + ...).
                VistaKp::Command { slot: String::new(), user: 0, quick: true, at: now }
            }
            Key::Star => {
                self.key_beep();
                // # + 0 + * (STATUS): the 6160V announces the system status.
                if self.buf == "#0" {
                    self.buf.clear();
                    self.vista_speak_status();
                    return VistaKp::Normal;
                }
                self.buf.clear();
                let open = self.open_zones();
                if !open.is_empty() && self.armed.is_none() && self.exit.is_none() {
                    let idx = self.vista_fault_idx % open.len();
                    let z = open[idx] as usize;
                    self.vista_fault_idx = (idx + 1) % open.len();
                    let lbl = self.label(z).to_ascii_uppercase();
                    let (l1, l2) = split_zone_lines("FAULT", z as u8, &lbl);
                    self.temp(&l1, &l2, 4000);
                    let mut toks = vec!["fault".into()];
                    let w = voice::filter_zone_words(&lbl);
                    if w.is_empty() {
                        toks.push("zone".into());
                        toks.extend(voice::number_tokens(z as u8));
                    } else {
                        toks.extend(w);
                    }
                    self.vista_speak(toks);
                } else if self.armed.map_or(false, |a| a.mode == ArmMode::Stay)
                    && (self.ee.entry_or(Model::Vista20p, "91", 0, 8) & 8) != 0
                {
                    // *91 exit delay restart, armed STAY or INSTANT: "simply press the
                    // [*] key, then let that person in (the perimeter zones remain
                    // armed). The system automatically rearms the entry/exit door
                    // when the exit delay expires."
                    let secs = self.exit_secs();
                    if let Some(a) = self.armed.as_mut() {
                        a.quick_exit = Some(now + Duration::from_secs(secs));
                    }
                    self.log(EvKind::Arm, "Exit delay restarted");
                } else if self.armed.is_none() && self.exit.is_none() {
                    // The voice keypad's STATUS key.
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
                    self.vista_speak_status();
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
                    return self.kv_arm_action(d, 0, false, true, now);
                }
                _ => {
                    self.buf.clear();
                    return self.kv_normal(key, now);
                }
            }
        }

        // Installer programming entry: [Installer 4112] + 8 + 0 + 0
        if slot == "inst" && key == Key::Digit(8) && self.armed.is_none() && self.exit.is_none() {
            self.key_beep();
            self.buf = "INST8".to_string();
            return VistaKp::Command { slot, user, quick: false, at: now };
        }
        if slot == "inst" && self.buf == "INST8" && key == Key::Digit(0) {
            self.key_beep();
            self.buf = "INST80".to_string();
            return VistaKp::Command { slot, user, quick: false, at: now };
        }
        if slot == "inst" && self.buf == "INST80" && key == Key::Digit(0) {
            self.buf.clear();
            if self.ee.installer_lockout {
                self.error_tone();
                self.temp("INSTALLER LOCK", "OUT ENABLED", 3000);
                return VistaKp::Normal;
            }
            self.beeps(3);
            self.log(EvKind::Program, "Entered Vista installer programming");
            return VistaKp::Program { field: None, buf: String::new(), review: false };
        }
        self.buf.clear();

        match key {
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
                self.kv_arm_action(d, user, slot == "inst", false, now)
            }
            // 5 = TEST, then 0 (walk) or 1 (dial)
            Key::Digit(5) => {
                self.key_beep();
                if self.armed.is_some() || self.exit.is_some() {
                    self.error_tone();
                    VistaKp::Normal
                } else {
                    VistaKp::TestWait
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
            // 8 = CODE (master user code programming)
            Key::Digit(8) => {
                self.key_beep();
                if slot == "02" && self.armed.is_none() && self.exit.is_none() {
                    VistaKp::UserCode { slot: String::new(), buf: String::new(), first: String::new() }
                } else {
                    self.error_tone();
                    VistaKp::Normal
                }
            }
            // 9 = CHIME
            Key::Digit(9) => {
                self.key_beep();
                // "Chime mode can be used only while the system is disarmed."
                if self.armed.is_some() || self.exit.is_some() {
                    self.error_tone();
                    return VistaKp::Normal;
                }
                self.chime = !self.chime;
                self.vista_confirm_beep();
                self.log(EvKind::Program, if self.chime { "Door chime on" } else { "Door chime off" });
                self.out.push(Out::Save);
                if self.chime {
                    self.vista_speak(vec!["chime".into()]);
                }
                VistaKp::Normal
            }
            _ => {
                self.key_beep();
                VistaKp::Normal
            }
        }
    }

    fn kv_arm_action(&mut self, digit: u8, user: u8, by_inst: bool, quick: bool, now: Instant) -> VistaKp {
        let how = match digit {
            2 => ArmHow::AwayKey,
            3 => ArmHow::StayKey,
            4 | 7 => ArmHow::NoEntry,
            _ => return VistaKp::Normal,
        };
        if self.armed.is_some() || self.exit.is_some() {
            self.error_tone();
            return VistaKp::Normal;
        }
        if !self.arm(how, user, now) {
            // arm() gives the "ONE SHORT BEEP: system arming attempt with an open
            // zone"; the open zone is shown with its descriptor.
            if let Some(&z) = self.open_zones().first() {
                let lbl = self.label(z as usize).to_ascii_uppercase();
                let (l1, l2) = split_zone_lines("FAULT", z, &lbl);
                self.temp(&l1, &l2, 4000);
            }
            self.vista_speak(vec!["not_ready_to_arm".into()]);
            return VistaKp::Normal;
        }
        self.vista_armed_by_inst = by_inst;
        self.vista_exit_voice_done = false;
        self.vista_alarm_mem_zone = None;
        self.vista_night = false;
        // AWAY and MAXIMUM give "two beeps, or, if programmed, beeping for
        // duration of exit delay" (*37 Audible Exit Warning, on from the factory).
        let exit_warning = self.ee.entry_or(Model::Vista20p, "37", 0, 1) != 0;
        match digit {
            4 => {
                // MAXIMUM arms Away (all zones) with no entry delay.
                if let Some(a) = self.armed.as_mut() {
                    a.mode = ArmMode::Away;
                    a.no_entry = true;
                }
                if let Some(ex) = self.exit.as_mut() {
                    ex.how = ArmHow::AwayKey;
                    ex.silent = false;
                }
                if !exit_warning {
                    self.beeps(2);
                }
                self.vista_speak(vec!["armed".into(), "maximum".into(), "exit_now".into()]);
                self.vista_exit_voice_done = true;
                VistaKp::Normal
            }
            2 => {
                if !exit_warning {
                    self.beeps(2);
                }
                // The 6160V says "Armed away, you may exit now" from tick_vista.
                VistaKp::Normal
            }
            3 => {
                // STAY, NIGHT-STAY and INSTANT: three beeps.
                self.beeps(3);
                self.vista_speak(vec!["armed".into(), "stay".into()]);
                if quick {
                    VistaKp::Normal
                } else {
                    VistaKp::NightWait { at: now }
                }
            }
            _ => {
                self.beeps(3);
                self.vista_speak(vec!["armed".into(), "instant".into()]);
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
                    self.vista_show_bypassed(&list, now);
                    self.vista_speak(vec!["bypassed".into()]);
                    self.log(EvKind::Info, format!("Quick bypassed {} open zone(s)", list.len()));
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
                // Bypassing only adds. Zones come back with the next code + OFF
                // ("all bypasses are removed when an OFF sequence is performed").
                if (1..=self.n()).contains(&z) && self.cfg(z).bypassable {
                    let was = self.zones[z - 1].bypassed;
                    self.zones[z - 1].bypassed = true;
                    self.vista_show_bypassed(&[z as u8], now);
                    if !was {
                        self.log(EvKind::Info, format!("Zone {:02} bypassed", z));
                        self.vista_speak(vec!["bypassed".into()]);
                    }
                } else {
                    self.error_tone();
                }
                VistaKp::Normal
            }
            _ => VistaKp::Normal,
        }
    }

    fn kv_test_wait(&mut self, key: Key, now: Instant) -> VistaKp {
        match key {
            Key::Digit(0) => {
                self.key_beep();
                self.walk_test = true;
                self.vista_test_since = Some(now);
                // "The external sounder should sound for about 1 second."
                self.walk_burst_until = Some(now + Duration::from_secs(1));
                self.log(EvKind::Info, "Walk test started (code + OFF ends it)");
                self.vista_speak(vec!["system".into()]);
            }
            Key::Digit(1) => {
                // The dialer test, meant for the installer.
                self.key_beep();
                self.vista_confirm_beep();
                self.log(EvKind::Info, "Dialer test report sent (Contact ID 601)");
                self.temp("DIAL TEST", "Report sent", 3000);
            }
            _ => self.error_tone(),
        }
        VistaKp::Normal
    }

    fn kv_user_code(&mut self, mut slot: String, mut buf: String, mut first: String, key: Key) -> VistaKp {
        match key {
            Key::Digit(d) => {
                self.key_beep();
                if slot.len() < 2 {
                    slot.push(char::from(b'0' + d));
                    return VistaKp::UserCode { slot, buf, first };
                }
                // Master code + 8 + user + # + 0 deletes that user.
                if buf == "#" {
                    let u = slot.parse::<u8>().unwrap_or(0);
                    if d == 0 && (3..=49).contains(&u) {
                        self.ee.clear_code(&slot);
                        self.vista_confirm_beep();
                        self.log(EvKind::Program, format!("User {:02} deleted", u));
                        self.out.push(Out::Save);
                    } else {
                        // # + 1 to 5 set authority, access group, partitions, RF
                        // key and paging on the real panel; RoomGuard keeps defaults.
                        self.error_tone();
                    }
                    return VistaKp::Normal;
                }
                buf.push(char::from(b'0' + d));
                if buf.len() < 4 {
                    return VistaKp::UserCode { slot, buf, first };
                }
                let u = slot.parse::<u8>().unwrap_or(0);
                if !(2..=49).contains(&u) {
                    self.error_tone();
                    return VistaKp::Normal;
                }
                // "Master code + [8] + 02 + new master code + new master code again"
                if u == 2 && first.is_empty() {
                    first = buf;
                    return VistaKp::UserCode { slot, buf: String::new(), first };
                }
                if u == 2 && first != buf {
                    self.error_tone();
                    return VistaKp::Normal;
                }
                self.ee.set_code(&slot, &buf);
                // "The keypad beeps once to confirm that new user was added."
                self.vista_confirm_beep();
                self.log(EvKind::Program, format!("User {:02} code set", u));
                self.out.push(Out::Save);
                VistaKp::Normal
            }
            Key::Hash if slot.len() == 2 && buf.is_empty() => {
                self.key_beep();
                VistaKp::UserCode { slot, buf: "#".to_string(), first }
            }
            _ => VistaKp::Normal,
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
                // # + 0 + 2 + 4 turns voice status and voice chime on or off.
                self.vista_voice_on = !self.vista_voice_on;
                self.vista_confirm_beep();
                if self.vista_voice_on {
                    self.vista_speak(vec!["chime".into()]);
                }
                self.log(EvKind::Program, if self.vista_voice_on { "Voice status on" } else { "Voice status off" });
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
                // Start selecting a field to edit: *XX
                VistaKp::Program { field: None, buf: "*".to_string(), review: false }
            }
            Key::Hash => {
                self.key_beep();
                if let Some(f) = &field {
                    // Finish Phone field early with #
                    if let Some(sp) = eeprom::spec(Model::Vista20p, f) {
                        if matches!(sp.kind, Kind::Phone(_)) && !review {
                            self.ee.sections.insert(f.clone(), buf);
                            self.beeps(3);
                            self.out.push(Out::Save);
                            return VistaKp::Program { field: next_field(f), buf: String::new(), review: false };
                        }
                    }
                }
                // Start reviewing a field read-only: #XX
                VistaKp::Program { field: None, buf: "#".to_string(), review: true }
            }
            Key::Digit(d) => {
                self.key_beep();
                let ch = char::from(b'0' + d);
                if field.is_none() {
                    buf.push(ch);
                    let prefix = buf.chars().next().unwrap_or('*');
                    let digits = &buf[1..];
                    // Handle 2-digit and 3-digit (197) fields
                    if digits == "19" {
                        return VistaKp::Program { field: None, buf, review };
                    }
                    if digits.len() >= 2 && (digits != "19" || digits.len() == 3) {
                        if prefix == '*' {
                            match digits {
                                "99" => {
                                    self.beeps(3);
                                    self.log(EvKind::Program, "Exited Vista installer programming (*99)");
                                    self.out.push(Out::Save);
                                    return VistaKp::Normal;
                                }
                                "98" => {
                                    self.ee.installer_lockout = true;
                                    self.beeps(3);
                                    self.log(EvKind::Program, "Exited Vista programming with lockout (*98)");
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
                            // "EE or ENTRY ERROR = invalid field number entered."
                            self.error_tone();
                            self.temp("ENTRY ERROR", "", 2000);
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
                                // "The keypad beeps three times and automatically displays
                                // the next data field in sequence."
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

    /// Periodic Vista-specific timers (voice during exit delay, periodic voice
    /// during burglary alarm).
    pub(crate) fn tick_vista(&mut self, now: Instant) {
        if self.model != Model::Vista20p {
            return;
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
        // "If the test mode is inadvertently left active, it automatically turns
        // off after 4 hours."
        if self.walk_test && self.vista_test_since.map_or(false, |t| now.saturating_duration_since(t).as_secs() >= 4 * 3600) {
            self.walk_test = false;
            self.vista_test_since = None;
            self.log(EvKind::Info, "Walk test ended after 4 hours");
        }
        // Deferred 6160V status voice after OFF: evaluate live ready() right
        // when speaking so a door/motion zone that just closed isn't announced
        // as "not ready to arm" while the display says "Ready to Arm".
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
        // After ~2.5s of Away exit delay (3 exit beeps), speak "Armed... away... exit... now."
        if let Some(ex) = &self.exit {
            if !self.vista_exit_voice_done
                && ex.how == ArmHow::AwayKey
                && now.saturating_duration_since(ex.started).as_millis() >= 2500
            {
                self.vista_exit_voice_done = true;
                self.vista_speak(vec!["armed".into(), "away".into(), "exit_now".into()]);
            }
        }
        // During active burglary alarm, periodically (~14s) speak "Alarm [zone]"
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
    }

    /// Authentic 6160V 2x16 Alpha Display strings.
    pub fn lcd_vista(&self, now: Instant, _wall_ms: i64) -> LcdOut {
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
                    None => plain("Installer Code", "              20"),
                    Some(f) => {
                        let sp = eeprom::spec(Model::Vista20p, f);
                        let title = sp.map(|s| s.name).unwrap_or("Field");
                        let shown = if *review {
                            self.ee.raw(Model::Vista20p, f)
                        } else if buf.is_empty() {
                            format!("*{} [{}]", f, self.ee.raw(Model::Vista20p, f))
                        } else {
                            format!("*{} {}", f, buf)
                        };
                        plain(title, &shown)
                    }
                };
            }
            VistaKp::UserCode { slot, buf, first } => {
                let s = if slot.len() < 2 {
                    format!("USER: {}_", slot)
                } else if buf.starts_with('#') {
                    format!("U{} DELETE: #_", slot)
                } else if !first.is_empty() {
                    format!("U{} AGAIN: {}", slot, "*".repeat(buf.len()))
                } else {
                    format!("U{} CODE: {}", slot, "*".repeat(buf.len()))
                };
                return plain("USER CODE PROG", &s);
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
                let s = if buf.is_empty() {
                    "Enter Zn (01-08)".to_string()
                } else {
                    format!("Zone: {}_", buf)
                };
                return plain("BYPASS ZONE", &s);
            }
            VistaKp::TestWait => {
                return plain("1=DIAL, 0=WALK", "");
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

        // Entry delay
        if self.entry.is_some() {
            return plain("DISARM SYSTEM", "or alarm occurs");
        }

        // Exit delay
        if let Some(ex) = &self.exit {
            let left = (ex.started + ex.dur).saturating_duration_since(now).as_secs();
            let head = match ex.how {
                ArmHow::StayKey if self.vista_night => "ARMED NIGHT-STAY",
                ArmHow::StayKey => "ARMED ***STAY***",
                ArmHow::NoEntry => "ARMED *INSTANT*",
                _ => "ARMED ***AWAY***",
            };
            // *197 (0 from the factory) adds the seconds left.
            let show_cd = self.ee.entry_or(Model::Vista20p, "197", 0, 0) != 0;
            let l2 = if show_cd {
                format!("May Exit Now {:02}", left.min(99))
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
            let byp = self.zones.iter().any(|z| z.bypassed);
            // After [*] restarts the exit delay, the door can be used again.
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

        // Walk test
        if self.walk_test {
            return plain("TEST IN PROGRESS", "Code+1 to exit");
        }

        // Latched Alarm Memory after 1st [Code] + 1 disarm
        if let Some((z, k)) = self.vista_alarm_mem_zone {
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

        // Disarmed: check troubles or open zones
        let line1 = if self.chime {
            "DISARMED CHIME"
        } else if self.zones.iter().any(|z| z.bypassed) {
            "DISARMED BYPASS"
        } else {
            "****DISARMED****"
        };
        // A blocked or offline camera is a zone trouble: "CHECK" plus the zone,
        // as for any supervised sensor.
        if (self.troubles & (tb::ZONE_FAULT | tb::ZONE_TAMPER)) != 0 {
            if let Some(z) = (1..=self.n()).find(|&z| self.camera_zone.get(z - 1).copied().unwrap_or(false) && self.violated(z)) {
                let lbl = self.label(z).to_ascii_uppercase();
                let (l1, l2) = split_zone_lines("CHECK", z as u8, &lbl);
                return plain(&l1, &l2);
            }
        }
        let open = self.open_zones();
        if !open.is_empty() {
            return plain(line1, "Hit * for faults");
        }
        let ready = "  Ready to Arm";
        if (self.troubles & tb::AC) != 0 {
            return plain("AC LOSS", ready);
        }
        if (self.troubles & tb::LOW_BATT) != 0 {
            return plain("SYSTEM LO BAT", ready);
        }
        if (self.troubles & tb::BELL) != 0 {
            return plain("BELL FAILURE", ready);
        }
        if (self.troubles & tb::FTC) != 0 {
            return plain("COMM. FAILURE", ready);
        }
        plain(line1, ready)
    }

    /// [Code] + OFF: disarms and silences; a second one clears the memory of
    /// alarm display. With nothing to disarm it removes any bypasses.
    fn vista_off(&mut self, slot: &str, user: u8, now: Instant) {
        self.vista_code_disarm = None;
        self.vista_sounder_mute_until = None;
        // The installer code can only disarm if it was used to arm.
        if slot == "inst" && (self.armed.is_some() || self.exit.is_some()) && !self.vista_armed_by_inst {
            self.error_tone();
            return;
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
        let has_live_alarm = self.alarms.iter().any(|a| !a.silenced);
        let is_armed = self.armed.is_some() || self.exit.is_some();
        if is_armed || has_live_alarm {
            let alarmed_info = self.alarms.iter().find(|a| a.kind != AlarmKind::Duress).map(|a| (a.zone, a.kind));
            if is_armed {
                self.disarm(user, now);
            } else {
                self.silence(user);
            }
            // "The keypad will emit a single tone to confirm that the system is disarmed."
            self.vista_off_beep();
            self.vista_armed_by_inst = false;
            self.vista_night = false;
            if let Some((z, k)) = alarmed_info {
                self.vista_alarm_mem_zone = Some((z, k));
                self.vista_pending_status = None;
                self.vista_speak(vec!["disarmed".into()]);
            } else {
                self.vista_pending_status = Some(status_at);
            }
        } else if self.vista_alarm_mem_zone.is_some() || self.memory_flag {
            // "Enter the security code plus OFF sequence again to clear the
            // keypad's Memory of Alarm display."
            self.vista_alarm_mem_zone = None;
            self.memory_flag = false;
            for z in self.zones.iter_mut() {
                z.alarm = false;
            }
            // *86 (on from the factory): "Alarm Canceled" for a moment, as filmed
            // on a real 6160, then the beep and Ready to Arm.
            if self.ee.entry_or(Model::Vista20p, "86", 0, 1) != 0 {
                self.temp_msg = Some(("Alarm Canceled".into(), String::new(), now + Duration::from_millis(1500)));
            }
            self.vista_off_beep();
            self.log(EvKind::Disarm, "Alarm memory cleared");
            self.vista_pending_status = Some(status_at);
        } else {
            // "All bypasses are removed when an OFF sequence is performed."
            let had = self.zones.iter().any(|z| z.bypassed);
            for z in self.zones.iter_mut() {
                z.bypassed = false;
                z.auto_bypass = false;
            }
            if had {
                self.log(EvKind::Info, "Bypasses removed");
            }
            self.vista_off_beep();
            self.vista_pending_status = Some(status_at);
        }
    }

    /// NIGHT-STAY arms the perimeter plus the zones in zone list 05 (field
    /// *81 here, the *81 Zone List menu on the real panel).
    pub(crate) fn vista_set_night(&mut self) {
        self.vista_night = true;
        self.vista_unbypass_night_zones();
        self.log(EvKind::Arm, "Armed NIGHT-STAY");
        self.out.push(Out::Save);
        self.vista_speak(vec!["armed".into(), "stay".into()]);
    }

    fn vista_night_zone(&self, z: usize) -> bool {
        self.ee.entry_or(Model::Vista20p, "81", z.saturating_sub(1), 0) != 0
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
        let _ = self.kv_arm_action(digit, 0, false, true, now);
    }

    /// Step arming's third press: from NIGHT-STAY up to AWAY.
    pub(crate) fn vista_step_to_away(&mut self) {
        if let Some(a) = self.armed.as_mut() {
            a.mode = ArmMode::Away;
        }
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
        self.vista_speak(vec!["armed".into(), "away".into()]);
    }

    /// Function code 08: communication test (Contact ID 601).
    pub(crate) fn vista_comm_test(&mut self) {
        self.vista_confirm_beep();
        self.log(EvKind::Info, "Communication test sent (Contact ID 601)");
    }

    /// Function code 00 on key D, or 01: single button paging, "999-9999".
    pub(crate) fn vista_page(&mut self) {
        self.vista_confirm_beep();
        self.log(EvKind::Info, "Pager message 999-9999 sent");
        self.temp("PAGING", "999-9999", 3000);
    }

    /// 6160V status LEDs: [READY (green), ARMED (red), MESSAGE (amber), AC/MIC].
    pub fn leds_vista(&self) -> [Led; 4] {
        let ready = if self.armed.is_none() && self.exit.is_none() && self.ready() {
            Led::On
        } else {
            Led::Off
        };
        let armed = if self.armed.is_some() || self.exit.is_some() {
            Led::On
        } else if self.vista_alarm_mem_zone.is_some() {
            Led::Flash
        } else {
            Led::Off
        };
        let msg = if self.troubles != 0 || self.memory_flag {
            Led::Flash
        } else {
            Led::Off
        };
        let ac = if self.inputs.ac_ok { Led::On } else { Led::Off };
        [ready, armed, msg, ac]
    }
}

/// The data field after `f`, the way the real panel steps on to the next
/// field once an entry is complete.
fn next_field(f: &str) -> Option<String> {
    let mut keys: Vec<&'static str> = eeprom::VISTA20P
        .iter()
        .filter(|s| !matches!(s.kind, Kind::Action) && s.key != "02" && s.key.chars().all(|c| c.is_ascii_digit()))
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
