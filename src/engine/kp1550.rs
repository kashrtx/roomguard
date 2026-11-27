//! The PC1550 with its PC1500RK LED keypad: what every key does in every
//! mode, and what the eleven lights show. Behaviour follows the PC1550 v4.0
//! installation and instruction manuals.

use std::time::{Duration, Instant};

use super::eeprom::{self, Kind};
use super::panel::*;
use super::types::*;
use crate::util;

/// Which zone light shows each trouble in [*][2].
fn trouble_light(bit: u32) -> usize {
    match bit {
        tb::LOW_BATT => 1,
        tb::AC => 2,
        tb::BELL => 3,
        tb::FTC => 4,
        tb::FIRE_LOOP => 5,
        tb::CLOCK => 6,
        _ => 0,
    }
}

impl Panel {
    pub(crate) fn press_1550(&mut self, key: Key, now: Instant) {
        if self.auto_pre.is_some() {
            // Any key during the auto arm warning minute cancels it.
            self.auto_pre = None;
            self.key_beep();
            self.log(EvKind::Info, "Auto arm aborted at the keypad");
            return;
        }
        let mode = std::mem::replace(&mut self.kp, KpMode::Normal);
        let next = match mode {
            KpMode::Normal => self.k50_normal(key, now),
            KpMode::Star { .. } => self.k50_star(key, now),
            KpMode::NeedCode { purpose, buf } => self.k_need_code(purpose, buf, key, now),
            KpMode::Bypass { .. } => self.k50_bypass(key),
            KpMode::Troubles { mem, .. } => self.k50_troubles(mem, key),
            KpMode::Memory { level, .. } => self.k50_memory(level, key),
            KpMode::UserCodes { slot, buf, .. } => self.k50_usercodes(slot, buf, key),
            KpMode::UserFn { item, buf, .. } => self.k50_userfn(item, buf, key, now),
            KpMode::Program(p) => self.k_program(p, key, now),
            _ => KpMode::Normal,
        };
        self.kp = self.pending_kp.take().unwrap_or(next);
        self.dirty = true;
    }

    fn k50_normal(&mut self, key: Key, now: Instant) -> KpMode {
        match key {
            Key::Digit(d) => {
                self.key_beep();
                self.buf.push(char::from(b'0' + d));
                if self.buf.len() >= 4 {
                    let code = std::mem::take(&mut self.buf);
                    self.normal_code(&code, now);
                }
                KpMode::Normal
            }
            Key::Star => {
                self.key_beep();
                self.buf.clear();
                KpMode::Star { idx: 0 }
            }
            Key::Hash => {
                self.key_beep();
                self.buf.clear();
                self.ack_troubles();
                self.mem_flash_until = None;
                // A fire alarm acknowledged with [#] in its first 30 seconds is silenced.
                let fresh_fire = self
                    .alarms
                    .iter()
                    .any(|a| a.kind.pulsed() && !a.silenced && now.saturating_duration_since(a.started).as_secs() < 30);
                if fresh_fire && self.armed.is_none() {
                    for a in self.alarms.iter_mut() {
                        if a.kind.pulsed() {
                            a.silenced = true;
                        }
                    }
                    self.log(EvKind::Info, "Fire alarm silenced with [#]");
                    self.out.push(Out::Disarmed { after_alarm: true });
                }
                KpMode::Normal
            }
            _ => KpMode::Normal,
        }
    }

    fn k50_star(&mut self, key: Key, now: Instant) -> KpMode {
        self.key_beep();
        let disarmed = self.armed.is_none() && self.exit.is_none();
        match key {
            Key::Digit(1) => {
                if self.armed.is_some() {
                    self.reactivate_stay(now);
                    KpMode::Normal
                } else if !disarmed {
                    self.error_tone();
                    KpMode::Normal
                } else if self.ee.toggle(self.model, "14", 1) {
                    KpMode::NeedCode { purpose: CodeFor::Bypass, buf: String::new() }
                } else {
                    KpMode::Bypass { buf: String::new(), idx: 0 }
                }
            }
            Key::Digit(2) => {
                self.ack_troubles();
                KpMode::Troubles { idx: 0, sub: 0, mem: false }
            }
            Key::Digit(3) => KpMode::Memory { level: 0, idx: 0 },
            Key::Digit(4) => {
                if self.ee.toggle(self.model, "14", 2) {
                    if self.ee.toggle(self.model, "14", 4) {
                        return KpMode::NeedCode { purpose: CodeFor::Download, buf: String::new() };
                    }
                    self.beeps(3);
                    self.log(EvKind::Info, "Downloading call requested, but no computer is set up to answer");
                } else {
                    self.error_tone();
                }
                KpMode::Normal
            }
            Key::Digit(5) if disarmed => KpMode::NeedCode { purpose: CodeFor::UserCodes, buf: String::new() },
            Key::Digit(6) if disarmed => KpMode::NeedCode { purpose: CodeFor::UserFns, buf: String::new() },
            Key::Digit(7) => match self.ee.raw(self.model, "24").as_str() {
                "03" => KpMode::NeedCode { purpose: CodeFor::CmdOut, buf: String::new() },
                "02" | "04" => {
                    self.classic_utility_output(now);
                    KpMode::Normal
                }
                _ => {
                    self.error_tone();
                    KpMode::Normal
                }
            },
            Key::Digit(8) if disarmed => KpMode::NeedCode { purpose: CodeFor::Installer, buf: String::new() },
            Key::Digit(9) if disarmed => KpMode::NeedCode { purpose: CodeFor::NoEntry, buf: String::new() },
            Key::Digit(0) => {
                if self.armed.is_some() {
                    self.quick_exit(now);
                } else if disarmed && self.quick_arm_enabled() {
                    self.arm(ArmHow::Quick, 0, now);
                } else {
                    self.error_tone();
                }
                KpMode::Normal
            }
            Key::Hash => KpMode::Normal,
            Key::Star => KpMode::Star { idx: 0 },
            _ => {
                self.error_tone();
                KpMode::Normal
            }
        }
    }

    /// Only the normally-low reset programming cuts smoke detector power.
    pub(crate) fn classic_utility_output(&mut self, now: Instant) {
        let until = now + Duration::from_secs(5);
        self.util_until = Some(until);
        if self.ee.raw(self.model, "24") == "04" {
            self.sensor_reset(now);
        } else {
            self.log(EvKind::Info, "Utility output active for five seconds");
        }
    }

    fn k50_bypass(&mut self, key: Key) -> KpMode {
        let stay = KpMode::Bypass { buf: String::new(), idx: 0 };
        match key {
            Key::Digit(9) => {
                let mut any = false;
                for z in self.last_bypass.clone() {
                    let zi = z as usize;
                    if zi >= 1 && zi <= self.n() && self.cfg(zi).bypassable {
                        self.zones[zi - 1].bypassed = true;
                        any = true;
                    }
                }
                if any {
                    self.beeps(3);
                    self.log(EvKind::Info, "Bypass group recalled");
                } else {
                    self.error_tone();
                }
                stay
            }
            Key::Digit(d) if d >= 1 && (d as usize) <= self.n() => {
                let z = d as usize;
                let c = self.cfg(z);
                // PC1550 manual: Fire zones cannot be bypassed; 24-hr zones cannot be bypassed while violated.
                if !c.bypassable || c.kind == ZoneKind::Null || (c.kind.is_24h() && self.violated(z)) {
                    self.error_tone();
                } else {
                    let r = &mut self.zones[z - 1];
                    r.bypassed = !r.bypassed;
                    r.auto_bypass = false;
                    let on = r.bypassed;
                    self.key_beep();
                    self.log(EvKind::Info, format!("Zone {} {}", z, if on { "bypassed" } else { "unbypassed" }));
                }
                stay
            }
            Key::Hash => {
                self.key_beep();
                let list: Vec<u8> = (1..=self.n()).filter(|&z| self.zones[z - 1].bypassed).map(|z| z as u8).collect();
                if !list.is_empty() {
                    self.last_bypass = list;
                }
                self.out.push(Out::Save);
                KpMode::Normal
            }
            _ => {
                self.error_tone();
                stay
            }
        }
    }

    fn k50_troubles(&mut self, mem: bool, key: Key) -> KpMode {
        match key {
            Key::Digit(9) => {
                self.key_beep();
                KpMode::Troubles { idx: 0, sub: 0, mem: !mem }
            }
            Key::Hash => {
                self.key_beep();
                // Leaving the trouble view clears a communication failure.
                self.inputs.ftc = false;
                self.ack_troubles();
                KpMode::Normal
            }
            _ => KpMode::Troubles { idx: 0, sub: 0, mem },
        }
    }

    fn k50_memory(&mut self, level: usize, key: Key) -> KpMode {
        match key {
            Key::Digit(_) => {
                // Any digit steps through the three history levels; the
                // keypad beeps once, twice or three times to say which.
                let next = (level + 1) % 3;
                self.beeps(next as u8 + 1);
                KpMode::Memory { level: next, idx: 0 }
            }
            Key::Hash => {
                self.key_beep();
                KpMode::Normal
            }
            _ => KpMode::Memory { level, idx: 0 },
        }
    }

    fn k50_usercodes(&mut self, slot: Option<String>, mut buf: String, key: Key) -> KpMode {
        let idle = |slot: Option<String>, buf: String| KpMode::UserCodes { slot, buf, idx: 0, attrs: false };
        match (slot, key) {
            (None, Key::Digit(d)) if (1..=6).contains(&d) => {
                if d == 1 && self.ee.toggle(self.model, "13", 2) {
                    self.error_tone();
                    return idle(None, String::new());
                }
                self.key_beep();
                idle(Some(d.to_string()), String::new())
            }
            (None, Key::Hash) => {
                self.key_beep();
                self.out.push(Out::Save);
                KpMode::Normal
            }
            (None, _) => {
                self.error_tone();
                idle(None, String::new())
            }
            (Some(s), Key::Digit(d)) => {
                if buf.contains('*') {
                    buf.clear();
                }
                self.key_beep();
                buf.push(char::from(b'0' + d));
                if buf.len() < 4 {
                    return idle(Some(s), buf);
                }
                let taken = self.ee.find_code(&buf).map_or(false, |o| o != s);
                if taken {
                    self.error_tone();
                } else {
                    self.ee.set_code(&s, &buf);
                    self.beeps(3);
                    self.log(EvKind::Program, format!("Access code {} programmed", s));
                    self.out.push(Out::Save);
                }
                idle(None, String::new())
            }
            (Some(s), Key::Star) => {
                if buf.chars().any(|c| c != '*') {
                    buf.clear();
                }
                self.key_beep();
                buf.push('*');
                if buf.len() < 4 {
                    return idle(Some(s), buf);
                }
                if s == "1" {
                    self.error_tone();
                } else {
                    self.ee.clear_code(&s);
                    self.beeps(3);
                    self.log(EvKind::Program, format!("Access code {} erased", s));
                    self.out.push(Out::Save);
                }
                idle(None, String::new())
            }
            (Some(_), Key::Hash) => {
                self.key_beep();
                idle(None, String::new())
            }
            (s, _) => idle(s, buf),
        }
    }

    fn toggle_feedback(&mut self, on: bool, what: &str) {
        if on {
            self.beeps(3);
        } else {
            self.long_beep();
        }
        self.log(EvKind::Info, format!("{} {}", what, if on { "on" } else { "off" }));
        self.out.push(Out::Save);
    }

    fn k50_userfn(&mut self, item: Option<u8>, mut buf: String, key: Key, now: Instant) -> KpMode {
        let menu = KpMode::UserFn { item: None, buf: String::new(), idx: 0 };
        if let Some(it) = item {
            return match key {
                Key::Digit(d) => {
                    self.key_beep();
                    buf.push(char::from(b'0' + d));
                    if buf.len() < 4 {
                        return KpMode::UserFn { item, buf, idx: 0 };
                    }
                    let hh: u32 = buf[0..2].parse().unwrap_or(99);
                    let mm: u32 = buf[2..4].parse().unwrap_or(99);
                    if hh > 23 || mm > 59 {
                        self.error_tone();
                    } else if it == 1 {
                        self.clock_offset_ms = util::offset_to_time(util::now_ms(), hh, mm);
                        self.troubles &= !tb::CLOCK;
                        self.beeps(3);
                        self.log(EvKind::Info, format!("Panel clock set to {:02}:{:02}", hh, mm));
                        self.out.push(Out::Save);
                    } else {
                        let mut raw: Vec<char> = self.ee.raw(self.model, "19").chars().collect();
                        while raw.len() < 8 {
                            raw.push('9');
                        }
                        for (i, c) in buf.chars().enumerate() {
                            raw[i] = c;
                        }
                        self.ee.sections.insert("19".to_string(), raw.into_iter().collect());
                        self.beeps(3);
                        self.log(EvKind::Info, format!("Auto arm time set to {:02}:{:02}", hh, mm));
                        self.out.push(Out::Save);
                    }
                    menu
                }
                Key::Hash => {
                    self.key_beep();
                    menu
                }
                _ => KpMode::UserFn { item, buf, idx: 0 },
            };
        }
        match key {
            Key::Digit(1) => {
                self.key_beep();
                KpMode::UserFn { item: Some(1), buf: String::new(), idx: 0 }
            }
            Key::Digit(2) => {
                self.key_beep();
                KpMode::UserFn { item: Some(2), buf: String::new(), idx: 0 }
            }
            Key::Digit(4) => {
                self.quick_arm = !self.quick_arm;
                let on = self.quick_arm;
                self.toggle_feedback(on, "Quick arm");
                menu
            }
            Key::Digit(5) => {
                self.auto_arm = !self.auto_arm;
                let on = self.auto_arm;
                self.toggle_feedback(on, "Auto arm");
                menu
            }
            Key::Digit(6) => {
                self.chime = !self.chime;
                let on = self.chime;
                self.toggle_feedback(on, "Door chime");
                menu
            }
            Key::Digit(8) => {
                self.system_test(now);
                menu
            }
            Key::Digit(0) => {
                self.walk_test = !self.walk_test;
                let on = self.walk_test;
                self.toggle_feedback(on, "Installer's walk test");
                menu
            }
            Key::Hash => {
                self.key_beep();
                self.out.push(Out::Save);
                KpMode::Normal
            }
            _ => {
                self.error_tone();
                menu
            }
        }
    }

    /// The eleven lights: Ready, Armed, Memory, Bypass, Trouble, then zones 1 to 6.
    pub fn leds_1550(&self, now: Instant) -> [Led; 11] {
        let mut l = [Led::Off; 11];
        if self.all_on(now) {
            return [Led::On; 11];
        }
        let zone = |l: &mut [Led; 11], z: usize, v: Led| {
            if (1..=6).contains(&z) {
                l[4 + z] = v;
            }
        };
        let ready = if self.ready() { Led::On } else { Led::Off };
        match &self.kp {
            KpMode::Program(p) => {
                l[2] = Led::Flash;
                l[3] = Led::Flash;
                l[4] = Led::Flash;
                match &p.section {
                    None => l[1] = Led::On,
                    Some(sec) => {
                        l[0] = if p.hex { Led::Flash } else { Led::On };
                        match eeprom::spec(self.model, sec).map(|s| s.kind) {
                            Some(Kind::Toggle(_)) => {
                                for c in &p.work {
                                    if let Some(d) = c.to_digit(10) {
                                        zone(&mut l, d as usize, Led::On);
                                    }
                                }
                            }
                            Some(Kind::Data { .. }) | Some(Kind::Phone(_)) => {
                                let digit = p.work.get(p.pos).and_then(|c| c.to_digit(16)).unwrap_or(0);
                                for b in 0..4usize {
                                    if digit & (1u32 << b) != 0 {
                                        zone(&mut l, b + 1, Led::On);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                return l;
            }
            KpMode::UserCodes { slot, .. } => {
                l[0] = ready;
                l[2] = Led::Flash;
                l[3] = Led::Flash;
                l[4] = Led::Flash;
                for z in 1..=6usize {
                    let key = z.to_string();
                    let v = if slot.as_deref() == Some(key.as_str()) {
                        Led::Flash
                    } else if self.ee.has_code(&key) {
                        Led::On
                    } else {
                        Led::Off
                    };
                    zone(&mut l, z, v);
                }
                return l;
            }
            KpMode::UserFn { .. } => {
                l[0] = ready;
                l[2] = Led::Flash;
                l[3] = Led::Flash;
                l[4] = Led::Flash;
                return l;
            }
            KpMode::NeedCode { purpose, .. } => {
                l[0] = ready;
                match purpose {
                    CodeFor::UserCodes | CodeFor::UserFns | CodeFor::Installer | CodeFor::Keypad => {
                        // Real PC1550 bus flashes Memory + Bypass + Trouble (Program state)
                        // as soon as *5 / *6 / *8 requests the access code.
                        l[2] = Led::Flash;
                        l[3] = Led::Flash;
                        l[4] = Led::Flash;
                    }
                    CodeFor::Bypass => l[3] = Led::Flash,
                    CodeFor::NoEntry => l[1] = Led::Flash,
                    _ => {}
                }
                return l;
            }
            KpMode::Bypass { .. } => {
                l[0] = ready;
                l[3] = Led::Flash;
                for z in 1..=self.n().min(6) {
                    if self.zones[z - 1].bypassed {
                        zone(&mut l, z, Led::On);
                    }
                }
                return l;
            }
            KpMode::Troubles { mem, .. } => {
                l[4] = Led::Flash;
                let bits = if *mem { self.trouble_mem } else { self.troubles };
                for (bit, _) in trouble_names(self.model) {
                    if bits & bit != 0 {
                        zone(&mut l, trouble_light(bit), Led::On);
                    }
                }
                // A camera problem flashes the light of the zone that camera
                // feeds, without hiding a real trouble on the same light.
                if bits & (tb::ZONE_FAULT | tb::ZONE_TAMPER) != 0 {
                    for z in 1..=self.n().min(6) {
                        if self.camera_zone[z - 1] && l[4 + z] == Led::Off {
                            zone(&mut l, z, Led::Flash);
                        }
                    }
                }
                return l;
            }
            KpMode::Memory { level, .. } => {
                l[2] = Led::Flash;
                if let Some(list) = self.memory.get(*level) {
                    for &z in list {
                        zone(&mut l, z as usize, Led::Flash);
                    }
                }
                return l;
            }
            _ => {}
        }
        let armed = self.armed.is_some();
        let exiting = self.is_on() && !armed;
        if armed || exiting {
            let no_entry = self.armed.map_or(false, |a| a.no_entry) || self.exit.map_or(false, |e| e.how == ArmHow::NoEntry);
            l[1] = if no_entry { Led::Flash } else { Led::On };
        }
        if !armed {
            l[0] = ready;
            if self.mem_flash_until.is_some() {
                l[2] = Led::Flash;
            } else if self.memory_flag {
                l[2] = Led::On;
            }
            if self.bypass_any() {
                l[3] = Led::On;
            }
            if self.troubles != 0 {
                l[4] = Led::On;
            }
            for z in 1..=self.n().min(6) {
                let r = self.zones[z - 1];
                if self.mem_flash_until.is_some() && r.alarm {
                    zone(&mut l, z, Led::Flash);
                } else if r.open || r.tamper || r.fault {
                    zone(&mut l, z, Led::On);
                }
            }
        } else {
            // At the end of the exit delay every light goes out except Armed,
            // unless an alarm occurred in memory, a fire trouble is active, or
            // the installer turned on bypass or alarm display.
            if self.memory_flag {
                l[2] = Led::On;
            }
            if self.ee.toggle(self.model, "31", 4) && self.bypass_any() {
                l[3] = Led::On;
            }
            if (self.troubles & tb::FIRE_LOOP) != 0 {
                l[4] = Led::On;
            }
            if self.ee.toggle(self.model, "12", 3) {
                for z in 1..=self.n().min(6) {
                    if self.zones[z - 1].alarm {
                        zone(&mut l, z, Led::Flash);
                    }
                }
            }
        }
        l
    }
}
