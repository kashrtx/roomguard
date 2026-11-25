//! The PowerSeries PC1832 with its PK5500 full message LCD keypad: every key
//! in every menu, the 2 x 16 display and the four indicator lights. Screens
//! and menus follow the PC1616/PC1832/PC1864 v4.2 manuals and the PK5500
//! installation instructions.

use std::time::Instant;

use super::eeprom::{self, Kind};
use super::panel::*;
use super::types::*;
use crate::util;

pub struct LcdOut {
    pub lines: [String; 2],
    pub cursor: Option<(usize, usize)>,
}

const STAR_MENU: [(u8, &str); 9] = [
    (1, "Zone Bypass"),
    (2, "Trouble Display"),
    (3, "Alarm Memory"),
    (4, "Door Chime"),
    (5, "Access Codes"),
    (6, "User Functions"),
    (7, "Output Control"),
    (9, "No-Entry Arm"),
    (0, "Quick Arm"),
];

const FN_MENU: [(u8, &str); 11] = [
    (1, "Time and Date"),
    (2, "Auto Arm/Disarm"),
    (3, "Auto Arm Time"),
    (4, "System Test"),
    (5, "Enable DLS"),
    (6, "User Call-Up"),
    (8, "Walk Test"),
    (101, "Bright Control"),
    (102, "Contrast Control"),
    (103, "Buzzer Control"),
    (104, "View Event Buf"),
];

pub fn pad(s: &str) -> String {
    let mut t: String = s.chars().take(16).collect();
    while t.chars().count() < 16 {
        t.push(' ');
    }
    t
}

/// A line with the "<>" scroll marker in the last two columns.
pub fn arrows(s: &str) -> String {
    let mut t: String = s.chars().take(13).collect();
    while t.chars().count() < 14 {
        t.push(' ');
    }
    t.push_str("<>");
    t
}

fn plain(a: &str, b: &str) -> LcdOut {
    LcdOut { lines: [pad(a), pad(b)], cursor: None }
}

impl Panel {
    pub(crate) fn press_1832(&mut self, key: Key, now: Instant) {
        let mode = std::mem::replace(&mut self.kp, KpMode::Normal);
        let next = match mode {
            KpMode::Normal => self.k32_normal(key, now),
            KpMode::Star { idx } => self.k32_star(idx, key, now),
            KpMode::NeedCode { purpose, buf } => self.k_need_code(purpose, buf, key, now),
            KpMode::Bypass { buf, idx } => self.k32_bypass(buf, idx, key),
            KpMode::Troubles { idx, sub, .. } => self.k32_troubles(idx, sub, key),
            KpMode::Memory { idx, .. } => self.k32_memory(idx, key),
            KpMode::UserCodes { slot, buf, idx, attrs } => self.k32_usercodes(slot, buf, idx, attrs, key),
            KpMode::UserFn { item, buf, idx } => self.k32_userfn(item, buf, idx, key, now),
            KpMode::Program(p) => self.k_program(p, key, now),
            KpMode::Events { idx, detail } => self.k32_events(idx, detail, key),
            KpMode::Level { which } => self.k32_level(which, key),
            KpMode::CmdOut { idx } => self.k32_cmdout(idx, key, now),
        };
        self.kp = self.pending_kp.take().unwrap_or(next);
        self.dirty = true;
    }

    fn k32_normal(&mut self, key: Key, now: Instant) -> KpMode {
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
                self.lcd_scroll = 0;
                self.temp_msg = None;
                KpMode::Normal
            }
            Key::Left => {
                self.key_beep();
                self.lcd_scroll -= 1;
                KpMode::Normal
            }
            Key::Right => {
                self.key_beep();
                self.lcd_scroll += 1;
                KpMode::Normal
            }
            _ => KpMode::Normal,
        }
    }

    fn star_label(&self, idx: usize) -> &'static str {
        let (d, label) = STAR_MENU[idx % STAR_MENU.len()];
        let armed = self.armed.is_some();
        match d {
            1 if armed => "Interior Arm",
            0 if armed => "Quick Exit",
            _ => label,
        }
    }

    fn k32_star(&mut self, idx: usize, key: Key, now: Instant) -> KpMode {
        let n = STAR_MENU.len();
        match key {
            Key::Left => {
                self.key_beep();
                KpMode::Star { idx: (idx + n - 1) % n }
            }
            Key::Right => {
                self.key_beep();
                KpMode::Star { idx: (idx + 1) % n }
            }
            Key::Star => {
                self.key_beep();
                let d = STAR_MENU[idx % n].0;
                self.star_function(d, now)
            }
            Key::Digit(d) => {
                self.key_beep();
                self.star_function(d, now)
            }
            Key::Hash => {
                self.key_beep();
                KpMode::Normal
            }
            _ => KpMode::Star { idx },
        }
    }

    fn star_function(&mut self, d: u8, now: Instant) -> KpMode {
        let disarmed = self.armed.is_none() && self.exit.is_none();
        match d {
            1 => {
                if self.armed.is_some() {
                    self.reactivate_stay(now);
                    return KpMode::Normal;
                }
                if !disarmed {
                    self.error_tone();
                    return KpMode::Normal;
                }
                if self.ee.toggle(self.model, "015", 5) || self.ee.toggle(self.model, "022", 1) {
                    KpMode::NeedCode { purpose: CodeFor::Bypass, buf: String::new() }
                } else {
                    KpMode::Bypass { buf: String::new(), idx: 0 }
                }
            }
            2 => {
                if self.ee.toggle(self.model, "022", 1) {
                    KpMode::NeedCode { purpose: CodeFor::Troubles, buf: String::new() }
                } else {
                    self.ack_troubles();
                    KpMode::Troubles { idx: 0, sub: 0, mem: false }
                }
            }
            3 => {
                if self.ee.toggle(self.model, "022", 1) {
                    KpMode::NeedCode { purpose: CodeFor::Memory, buf: String::new() }
                } else {
                    KpMode::Memory { level: 0, idx: 0 }
                }
            }
            4 => {
                self.toggle_chime();
                KpMode::Normal
            }
            5 if disarmed => KpMode::NeedCode { purpose: CodeFor::UserCodes, buf: String::new() },
            6 if disarmed => KpMode::NeedCode { purpose: CodeFor::UserFns, buf: String::new() },
            7 => KpMode::CmdOut { idx: 1 },
            8 if disarmed => KpMode::NeedCode { purpose: CodeFor::Installer, buf: String::new() },
            9 if disarmed => KpMode::NeedCode { purpose: CodeFor::NoEntry, buf: String::new() },
            0 => {
                if self.armed.is_some() {
                    self.quick_exit(now);
                } else if disarmed && self.quick_arm_enabled() {
                    self.arm(ArmHow::Quick, 0, now);
                } else {
                    self.error_tone();
                }
                KpMode::Normal
            }
            _ => {
                self.error_tone();
                KpMode::Normal
            }
        }
    }

    fn toggle_bypass(&mut self, z: usize) {
        let c = self.cfg(z);
        if !c.bypassable || c.kind == ZoneKind::Null {
            self.error_tone();
            return;
        }
        let r = &mut self.zones[z - 1];
        r.bypassed = !r.bypassed;
        r.auto_bypass = false;
        let on = r.bypassed;
        self.key_beep();
        self.log(EvKind::Info, format!("Zone {} ({}) {}", z, self.label(z), if on { "bypassed" } else { "unbypassed" }));
        self.dirty = true;
    }

    fn k32_bypass(&mut self, mut buf: String, idx: usize, key: Key) -> KpMode {
        let n = self.n().max(1);
        match key {
            Key::Left => {
                self.key_beep();
                KpMode::Bypass { buf: String::new(), idx: (idx + n - 1) % n }
            }
            Key::Right => {
                self.key_beep();
                KpMode::Bypass { buf: String::new(), idx: (idx + 1) % n }
            }
            Key::Star => {
                self.toggle_bypass(idx % n + 1);
                KpMode::Bypass { buf: String::new(), idx }
            }
            Key::Digit(d) => {
                self.key_beep();
                buf.push(char::from(b'0' + d));
                if buf.len() < 2 {
                    return KpMode::Bypass { buf, idx };
                }
                let code: usize = buf.parse().unwrap_or(0);
                match code {
                    99 => {
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
                            self.log(EvKind::Info, "Last bypass group recalled");
                        } else {
                            self.error_tone();
                        }
                        KpMode::Bypass { buf: String::new(), idx }
                    }
                    95 => {
                        let list: Vec<u8> = (1..=self.n()).filter(|&z| self.zones[z - 1].bypassed).map(|z| z as u8).collect();
                        if !list.is_empty() {
                            self.saved_bypass = list;
                            self.beeps(3);
                            self.temp("Bypass Group", "Programmed", 1800);
                            self.log(EvKind::Info, "Bypass group saved ([95])");
                            self.out.push(Out::Save);
                        } else {
                            self.error_tone();
                        }
                        KpMode::Bypass { buf: String::new(), idx }
                    }
                    91 => {
                        let mut any = false;
                        for z in self.saved_bypass.clone() {
                            let zi = z as usize;
                            if zi >= 1 && zi <= self.n() && self.cfg(zi).bypassable {
                                self.zones[zi - 1].bypassed = true;
                                any = true;
                            }
                        }
                        if any {
                            self.beeps(3);
                            self.temp("Bypass Group", "Recalled", 1800);
                            self.log(EvKind::Info, "Saved bypass group recalled ([91])");
                        } else {
                            self.error_tone();
                        }
                        KpMode::Bypass { buf: String::new(), idx }
                    }
                    0 => {
                        for z in self.zones.iter_mut() {
                            z.bypassed = false;
                            z.auto_bypass = false;
                        }
                        self.beeps(3);
                        self.log(EvKind::Info, "All bypasses removed");
                        KpMode::Bypass { buf: String::new(), idx }
                    }
                    z if z >= 1 && z <= self.n() => {
                        self.toggle_bypass(z);
                        KpMode::Bypass { buf: String::new(), idx: z - 1 }
                    }
                    _ => {
                        self.error_tone();
                        KpMode::Bypass { buf: String::new(), idx }
                    }
                }
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
            _ => KpMode::Bypass { buf, idx },
        }
    }

    /// The numbered trouble list exactly as the PK5500 shows it.
    fn trouble_items(&self) -> Vec<(u8, &'static str)> {
        let t = self.troubles;
        let mut v = Vec::new();
        if t & (tb::LOW_BATT | tb::BELL) != 0 {
            v.push((1, "Service Req'd"));
        }
        if t & tb::AC != 0 {
            v.push((2, "AC Failure"));
        }
        if t & tb::TLM != 0 {
            v.push((3, "Tel Line Fault"));
        }
        if t & tb::FTC != 0 {
            v.push((4, "Fail to Comm"));
        }
        if t & tb::ZONE_FAULT != 0 {
            v.push((5, "Zone Fault"));
        }
        if t & tb::ZONE_TAMPER != 0 {
            v.push((6, "Zone Tamper"));
        }
        if t & tb::CLOCK != 0 {
            v.push((8, "Loss of Time"));
        }
        v
    }

    fn service_items(&self) -> Vec<(u8, &'static str)> {
        let mut v = Vec::new();
        if self.troubles & tb::LOW_BATT != 0 {
            v.push((1, "Low Battery"));
        }
        if self.troubles & tb::BELL != 0 {
            v.push((2, "Bell Circuit"));
        }
        v
    }

    fn trouble_sub_items(&self, sub: u8) -> Vec<(u8, String)> {
        match sub {
            1 => self.service_items().into_iter().map(|(n, s)| (n, s.to_string())).collect(),
            5 => (1..=self.n())
                .filter(|&z| self.zones[z - 1].fault)
                .map(|z| (z as u8, self.label(z)))
                .collect(),
            6 => (1..=self.n())
                .filter(|&z| self.zones[z - 1].tamper)
                .map(|z| (z as u8, self.label(z)))
                .collect(),
            _ => self.trouble_items().into_iter().map(|(n, s)| (n, s.to_string())).collect(),
        }
    }

    fn k32_troubles(&mut self, idx: usize, sub: u8, key: Key) -> KpMode {
        let items = self.trouble_sub_items(sub);
        let n = items.len().max(1);
        let current = items.get(idx % n).map(|i| i.0).unwrap_or(0);
        match key {
            Key::Left => {
                self.key_beep();
                KpMode::Troubles { idx: (idx + n - 1) % n, sub, mem: false }
            }
            Key::Right => {
                self.key_beep();
                KpMode::Troubles { idx: (idx + 1) % n, sub, mem: false }
            }
            Key::Star if sub == 0 && matches!(current, 1 | 5 | 6) => {
                self.key_beep();
                KpMode::Troubles { idx: 0, sub: current, mem: false }
            }
            Key::Digit(1) if sub == 0 && self.troubles & (tb::LOW_BATT | tb::BELL) != 0 => {
                self.key_beep();
                KpMode::Troubles { idx: 0, sub: 1, mem: false }
            }
            Key::Digit(5) if sub == 0 && self.troubles & tb::ZONE_FAULT != 0 => {
                self.key_beep();
                KpMode::Troubles { idx: 0, sub: 5, mem: false }
            }
            Key::Digit(6) if sub == 0 && self.troubles & tb::ZONE_TAMPER != 0 => {
                self.key_beep();
                KpMode::Troubles { idx: 0, sub: 6, mem: false }
            }
            Key::Digit(8) if sub == 0 && self.troubles & tb::CLOCK != 0 => {
                self.key_beep();
                KpMode::NeedCode { purpose: CodeFor::UserFns, buf: String::new() }
            }
            Key::Hash => {
                self.key_beep();
                if sub != 0 {
                    KpMode::Troubles { idx: 0, sub: 0, mem: false }
                } else {
                    self.inputs.ftc = false;
                    self.ack_troubles();
                    KpMode::Normal
                }
            }
            _ => KpMode::Troubles { idx, sub, mem: false },
        }
    }

    fn k32_memory(&mut self, idx: usize, key: Key) -> KpMode {
        let n = self.memory[0].len().max(1);
        match key {
            Key::Left => {
                self.key_beep();
                KpMode::Memory { level: 0, idx: (idx + n - 1) % n }
            }
            Key::Right => {
                self.key_beep();
                KpMode::Memory { level: 0, idx: (idx + 1) % n }
            }
            Key::Hash => {
                self.key_beep();
                KpMode::Normal
            }
            _ => KpMode::Memory { level: 0, idx },
        }
    }

    fn k32_usercodes(&mut self, slot: Option<String>, mut buf: String, idx: usize, attrs: bool, key: Key) -> KpMode {
        let slots = eeprom::user_slots(self.model);
        let n = slots.len();
        let stay = |slot: Option<String>, buf: String, idx: usize, attrs: bool| KpMode::UserCodes { slot, buf, idx, attrs };
        let Some(s) = slot else {
            return match key {
                Key::Left => {
                    self.key_beep();
                    stay(None, String::new(), (idx + n - 1) % n, attrs)
                }
                Key::Right => {
                    self.key_beep();
                    stay(None, String::new(), (idx + 1) % n, attrs)
                }
                Key::Star => {
                    self.key_beep();
                    stay(Some(slots[idx % n].clone()), String::new(), idx, attrs)
                }
                Key::Digit(d) => {
                    self.key_beep();
                    buf.push(char::from(b'0' + d));
                    if buf.len() < 2 {
                        return stay(None, buf, idx, attrs);
                    }
                    if buf == "99" {
                        return stay(None, String::new(), idx, true);
                    }
                    match slots.iter().position(|x| *x == buf) {
                        Some(i) => stay(Some(buf), String::new(), i, attrs),
                        None => {
                            self.error_tone();
                            stay(None, String::new(), idx, attrs)
                        }
                    }
                }
                Key::Hash => {
                    self.key_beep();
                    if attrs {
                        stay(None, String::new(), idx, false)
                    } else {
                        self.out.push(Out::Save);
                        KpMode::Normal
                    }
                }
                _ => stay(None, buf, idx, attrs),
            };
        };
        if attrs {
            return match key {
                Key::Digit(d) if (1..=8).contains(&d) => {
                    if s == "40" {
                        self.error_tone();
                    } else {
                        let a = self.ee.attrs(&s) ^ (1u8 << (d - 1));
                        self.ee.set_attrs(&s, a);
                        self.key_beep();
                    }
                    stay(Some(s), buf, idx, attrs)
                }
                Key::Hash => {
                    self.beeps(3);
                    self.log(EvKind::Program, format!("Code {} attributes changed", s));
                    self.out.push(Out::Save);
                    stay(None, String::new(), idx, attrs)
                }
                _ => stay(Some(s), buf, idx, attrs),
            };
        }
        match key {
            Key::Star if buf.is_empty() => {
                if s == "40" {
                    self.error_tone();
                } else {
                    self.ee.clear_code(&s);
                    self.beeps(3);
                    self.log(EvKind::Program, format!("Access code {} erased", s));
                    self.out.push(Out::Save);
                }
                stay(None, String::new(), idx, attrs)
            }
            Key::Digit(d) => {
                self.key_beep();
                buf.push(char::from(b'0' + d));
                if buf.len() < 4 {
                    return stay(Some(s), buf, idx, attrs);
                }
                if s == "40" && self.ee.toggle(self.model, "015", 6) {
                    self.error_tone();
                    return stay(None, String::new(), idx, attrs);
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
                stay(None, String::new(), idx, attrs)
            }
            Key::Hash => {
                self.key_beep();
                stay(None, String::new(), idx, attrs)
            }
            _ => stay(Some(s), buf, idx, attrs),
        }
    }

    fn k32_userfn(&mut self, item: Option<u8>, mut buf: String, idx: usize, key: Key, now: Instant) -> KpMode {
        let n = FN_MENU.len();
        if let Some(it) = item {
            let need = if it == 1 { 10 } else { 4 };
            return match key {
                Key::Digit(d) => {
                    self.key_beep();
                    buf.push(char::from(b'0' + d));
                    if buf.len() < need {
                        return KpMode::UserFn { item, buf, idx };
                    }
                    let hh: u32 = buf[0..2].parse().unwrap_or(99);
                    let mm: u32 = buf[2..4].parse().unwrap_or(99);
                    if hh > 23 || mm > 59 {
                        self.error_tone();
                    } else if it == 1 {
                        self.clock_offset_ms = util::offset_to_time(util::now_ms(), hh, mm);
                        self.troubles &= !tb::CLOCK;
                        self.beeps(3);
                        self.temp("Time and Date", "Saved", 1500);
                        self.log(EvKind::Info, format!("Panel clock set to {:02}:{:02}", hh, mm));
                        self.out.push(Out::Save);
                    } else {
                        self.auto_arm_hm = Some((hh as u8, mm as u8));
                        self.beeps(3);
                        self.log(EvKind::Info, format!("Auto arm time set to {:02}:{:02}", hh, mm));
                        self.out.push(Out::Save);
                    }
                    KpMode::UserFn { item: None, buf: String::new(), idx }
                }
                Key::Hash => {
                    self.key_beep();
                    KpMode::UserFn { item: None, buf: String::new(), idx }
                }
                _ => KpMode::UserFn { item, buf, idx },
            };
        }
        match key {
            Key::Left => {
                self.key_beep();
                KpMode::UserFn { item: None, buf: String::new(), idx: (idx + n - 1) % n }
            }
            Key::Right => {
                self.key_beep();
                KpMode::UserFn { item: None, buf: String::new(), idx: (idx + 1) % n }
            }
            Key::Star => {
                self.key_beep();
                let code = FN_MENU[idx % n].0;
                self.user_function(code, idx, now)
            }
            Key::Digit(d) => {
                if let Some(pos) = FN_MENU.iter().position(|f| f.0 == d) {
                    self.key_beep();
                    self.user_function(d, pos, now)
                } else {
                    self.error_tone();
                    KpMode::UserFn { item: None, buf: String::new(), idx }
                }
            }
            Key::Hash => {
                self.key_beep();
                self.out.push(Out::Save);
                KpMode::Normal
            }
            _ => KpMode::UserFn { item: None, buf, idx },
        }
    }

    fn user_function(&mut self, code: u8, idx: usize, now: Instant) -> KpMode {
        let back = KpMode::UserFn { item: None, buf: String::new(), idx };
        match code {
            1 | 3 => KpMode::UserFn { item: Some(code), buf: String::new(), idx },
            2 => {
                self.auto_arm = !self.auto_arm;
                if self.auto_arm {
                    self.beeps(3);
                } else {
                    self.long_beep();
                }
                let on = self.auto_arm;
                self.log(EvKind::Info, if on { "Auto arm enabled" } else { "Auto arm disabled" });
                self.out.push(Out::Save);
                back
            }
            4 => {
                self.system_test(now);
                back
            }
            5 => {
                self.beeps(3);
                self.temp("DLS Enabled", "for 6 Hours", 2500);
                self.log(EvKind::Info, "Downloading window opened for six hours");
                back
            }
            6 => {
                self.error_tone();
                self.temp("User Call-Up", "Not Programmed", 2500);
                back
            }
            8 => {
                self.walk_test = !self.walk_test;
                if self.walk_test {
                    self.beeps(3);
                } else {
                    self.long_beep();
                }
                let on = self.walk_test;
                self.log(EvKind::Info, if on { "Walk test started" } else { "Walk test ended" });
                back
            }
            101 => KpMode::Level { which: 1 },
            102 => KpMode::Level { which: 2 },
            103 => KpMode::Level { which: 3 },
            104 => KpMode::Events { idx: 0, detail: false },
            _ => back,
        }
    }

    fn k32_level(&mut self, which: u8, key: Key) -> KpMode {
        let max = if which == 3 { 20 } else { 9 };
        let cur = match which {
            1 => self.brightness,
            2 => self.contrast,
            _ => self.buzzer,
        };
        let next = match key {
            Key::Left => cur.saturating_sub(1),
            Key::Right => (cur + 1).min(max),
            Key::Hash | Key::Star => {
                self.key_beep();
                self.out.push(Out::Save);
                let pos = FN_MENU.iter().position(|f| f.0 == 100 + which).unwrap_or(0);
                return KpMode::UserFn { item: None, buf: String::new(), idx: pos };
            }
            _ => cur,
        };
        match which {
            1 => self.brightness = next,
            2 => self.contrast = next,
            _ => self.buzzer = next,
        }
        self.key_beep();
        KpMode::Level { which }
    }

    fn k32_events(&mut self, idx: usize, detail: bool, key: Key) -> KpMode {
        let n = self.events.len().max(1);
        match key {
            Key::Left => {
                self.key_beep();
                KpMode::Events { idx: (idx + 1).min(n - 1), detail }
            }
            Key::Right => {
                self.key_beep();
                KpMode::Events { idx: idx.saturating_sub(1), detail }
            }
            Key::Star => {
                self.key_beep();
                KpMode::Events { idx, detail: !detail }
            }
            Key::Hash => {
                self.key_beep();
                let pos = FN_MENU.iter().position(|f| f.0 == 104).unwrap_or(0);
                KpMode::UserFn { item: None, buf: String::new(), idx: pos }
            }
            _ => KpMode::Events { idx, detail },
        }
    }

    fn k32_cmdout(&mut self, idx: usize, key: Key, now: Instant) -> KpMode {
        match key {
            Key::Digit(d) if (1..=4).contains(&d) => self.command_output(d, now),
            Key::Left => {
                self.key_beep();
                KpMode::CmdOut { idx: if idx <= 1 { 4 } else { idx - 1 } }
            }
            Key::Right => {
                self.key_beep();
                KpMode::CmdOut { idx: if idx >= 4 { 1 } else { idx + 1 } }
            }
            Key::Star => self.command_output(idx as u8, now),
            Key::Hash => {
                self.key_beep();
                KpMode::Normal
            }
            _ => KpMode::CmdOut { idx },
        }
    }

    fn command_output(&mut self, n: u8, now: Instant) -> KpMode {
        if n == 2 {
            self.sensor_reset(now);
        } else {
            self.beeps(1);
            self.log(EvKind::Info, format!("Command output {} activated", n));
        }
        KpMode::Normal
    }

    /// The four indicators: Ready, Armed, Trouble, AC.
    pub fn leds_5500(&self, now: Instant) -> [Led; 4] {
        if self.all_on(now) || self.walk_test {
            return [Led::Flash, Led::Flash, Led::Flash, Led::On];
        }
        let ready = if self.armed.is_none() && self.ready() { Led::On } else { Led::Off };
        let armed = if self.armed.is_some() || self.exit.is_some() {
            let no_entry = self.armed.map_or(false, |a| a.no_entry) || self.exit.map_or(false, |e| e.how == ArmHow::NoEntry);
            if no_entry {
                Led::Flash
            } else {
                Led::On
            }
        } else {
            Led::Off
        };
        let trouble = if matches!(self.kp, KpMode::Troubles { .. }) && self.troubles != 0 {
            Led::Flash
        } else if self.troubles != 0 {
            Led::On
        } else {
            Led::Off
        };
        let ac = if !self.ee.toggle(Model::Pc1832, "kp077", 5) {
            Led::Off
        } else if self.inputs.ac_ok == self.ee.toggle(Model::Pc1832, "kp077", 6) {
            Led::On
        } else {
            Led::Off
        };
        [ready, armed, trouble, ac]
    }

    pub fn lcd_1832(&self, now: Instant, wall_ms: i64) -> LcdOut {
        if self.all_on(now) {
            let full: String = std::iter::repeat('\u{2588}').take(16).collect();
            return LcdOut { lines: [full.clone(), full], cursor: None };
        }
        if let Some((a, b, until)) = &self.temp_msg {
            if now < *until {
                return plain(a, b);
            }
        }
        if self.lockout_until.map_or(false, |t| now < t) {
            return plain("Keypad Lockout", "Active");
        }
        match &self.kp {
            KpMode::Program(p) => self.lcd_program(p),
            KpMode::Star { idx } => LcdOut { lines: [arrows("Press (*) for"), pad(self.star_label(*idx))], cursor: None },
            KpMode::NeedCode { purpose, buf } => {
                let title = match purpose {
                    CodeFor::Installer | CodeFor::Keypad => "Enter Installer",
                    CodeFor::UserCodes | CodeFor::UserFns => "Enter Master",
                    _ => "Enter Access",
                };
                let stars = "*".repeat(buf.len());
                LcdOut { lines: [pad(title), pad(&format!("Code {}", stars))], cursor: Some((1, 5 + buf.len().min(4))) }
            }
            KpMode::Bypass { buf, idx } => {
                let i = idx % self.n().max(1);
                let r = self.zones.get(i).copied().unwrap_or_default();
                let flag = if r.bypassed {
                    'B'
                } else if r.open || r.tamper || r.fault {
                    'O'
                } else {
                    ' '
                };
                let mut l2: String = self.label(i + 1).chars().take(14).collect();
                while l2.chars().count() < 15 {
                    l2.push(' ');
                }
                l2.push(flag);
                let l1 = if buf.is_empty() { arrows("Zone Search") } else { arrows(&format!("Zone Search {}", buf)) };
                LcdOut { lines: [l1, pad(&l2)], cursor: None }
            }
            KpMode::Troubles { idx, sub, .. } => {
                let items = self.trouble_sub_items(*sub);
                if items.is_empty() {
                    return plain("No Troubles", "Present");
                }
                let (num, text) = &items[*idx % items.len()];
                let l1 = match *sub {
                    1 => arrows("Service Req'd"),
                    5 => arrows("Zone Fault"),
                    6 => arrows("Zone Tamper"),
                    _ => arrows("View Trouble"),
                };
                LcdOut { lines: [l1, pad(&format!("{} {}", num, text))], cursor: None }
            }
            KpMode::Memory { idx, .. } => {
                let list = &self.memory[0];
                if list.is_empty() {
                    return plain("No Alarms in", "Memory");
                }
                let z = list[*idx % list.len()] as usize;
                LcdOut { lines: [arrows("View Memory"), pad(&self.label(z))], cursor: None }
            }
            KpMode::UserCodes { slot, buf, idx, attrs } => self.lcd_usercodes(slot, buf, *idx, *attrs),
            KpMode::UserFn { item, buf, idx } => match item {
                Some(1) => {
                    let mut shown: Vec<char> = "HH:MM MM-DD-YY".chars().collect();
                    let slots = [0usize, 1, 3, 4, 6, 7, 9, 10, 12, 13];
                    for (i, c) in buf.chars().enumerate() {
                        if let Some(&s) = slots.get(i) {
                            shown[s] = c;
                        }
                    }
                    let cur = slots.get(buf.len()).copied().unwrap_or(13);
                    LcdOut { lines: [pad("Time and Date"), pad(&shown.into_iter().collect::<String>())], cursor: Some((1, cur)) }
                }
                Some(_) => {
                    let mut shown: Vec<char> = "HH:MM".chars().collect();
                    let slots = [0usize, 1, 3, 4];
                    for (i, c) in buf.chars().enumerate() {
                        if let Some(&s) = slots.get(i) {
                            shown[s] = c;
                        }
                    }
                    let cur = slots.get(buf.len()).copied().unwrap_or(4);
                    LcdOut { lines: [pad("Auto Arm Time"), pad(&shown.into_iter().collect::<String>())], cursor: Some((1, cur)) }
                }
                None => LcdOut { lines: [arrows("Press (*) for"), pad(FN_MENU[*idx % FN_MENU.len()].1)], cursor: None },
            },
            KpMode::Events { idx, detail } => self.lcd_event(*idx, *detail),
            KpMode::Level { which } => {
                let (title, v, max) = match which {
                    1 => ("Bright Control", self.brightness, 9u8),
                    2 => ("Contrast Control", self.contrast, 9u8),
                    _ => ("Buzzer Control", self.buzzer, 20u8),
                };
                let filled = (v as usize * 12 + max as usize / 2) / max.max(1) as usize;
                let bar: String = (0..12).map(|i| if i < filled { '\u{2588}' } else { '-' }).collect();
                LcdOut { lines: [arrows(title), pad(&format!("{:02} {}", v, bar))], cursor: None }
            }
            KpMode::CmdOut { idx } => {
                let label = if *idx == 2 { "Sensor Reset".to_string() } else { format!("Command O/P {}", idx) };
                LcdOut { lines: [arrows("Select Output"), pad(&label)], cursor: None }
            }
            KpMode::Normal => self.lcd_idle(now, wall_ms),
        }
    }

    fn scroll_screen(&self, items: &[(String, String)], auto_step: i32) -> LcdOut {
        if items.is_empty() {
            return plain("", "");
        }
        let i = (self.lcd_scroll + auto_step).rem_euclid(items.len() as i32) as usize;
        let (a, b) = &items[i];
        if items.len() > 1 && b == "Before Arming" {
            return LcdOut { lines: [pad(a), arrows(b)], cursor: None };
        }
        let l1 = if items.len() > 1 { arrows(a) } else { pad(a) };
        LcdOut { lines: [l1, pad(b)], cursor: None }
    }

    fn clock_lines(&self, wall_ms: i64) -> (String, String) {
        let t = util::local(wall_ms + self.clock_offset_ms);
        let time_s = if self.ee.toggle(Model::Pc1832, "kp076", 3) {
            t.format("%H:%M").to_string()
        } else {
            t.format("%-I:%M %p").to_string()
        };
        (t.format("%a %b %d").to_string(), time_s)
    }

    fn lcd_idle(&self, now: Instant, wall_ms: i64) -> LcdOut {
        // Live asterisk feedback while typing a 4-digit code in Normal or Entry Delay mode.
        if !self.buf.is_empty() {
            let stars = "*".repeat(self.buf.len());
            let title = if self.entry.is_some() {
                "Entry Active"
            } else if self.armed.is_some() {
                "Enter Code to"
            } else {
                "Enter Access"
            };
            return LcdOut {
                lines: [pad(title), pad(&format!("Code {}", stars))],
                cursor: Some((1, 5 + self.buf.len().min(4))),
            };
        }

        let idle_secs = now.saturating_duration_since(self.last_key_at).as_secs();
        let show_clock = self.ee.toggle(Model::Pc1832, "kp076", 2);
        let auto_open = self.ee.toggle(Model::Pc1832, "kp076", 8);

        let live: Vec<&AlarmRt> = self.live_alarms(now).filter(|a| a.kind != AlarmKind::Duress).collect();
        if !live.is_empty() {
            let auto = (idle_secs / 3) as i32;
            let a = live[(self.lcd_scroll + auto).rem_euclid(live.len() as i32) as usize];
            let top = if a.kind.pulsed() {
                "Fire Zone"
            } else if a.kind.is_co() {
                "CO Alarm"
            } else {
                "In Alarm"
            };
            let l1 = if live.len() > 1 { arrows(top) } else { pad(top) };
            return LcdOut { lines: [l1, pad(&self.label(a.zone as usize))], cursor: None };
        }
        if self.entry.is_some() {
            return plain("Entry Active", "Enter Your Code");
        }
        if let Some(ex) = &self.exit {
            let phase = now.saturating_duration_since(ex.started).as_millis() / 2000;
            if self.bypass_any() && phase % 2 == 1 {
                return plain("* WARNING *", "Bypass Active");
            }
            return plain("Exit Delay in", "Progress");
        }
        if self.auto_pre.is_some() {
            return plain("Auto-Arm in", "Progress");
        }
        if let Some(a) = &self.armed {
            let mut items: Vec<(String, String)> = vec![("Enter Code to".to_string(), "Disarm System".to_string())];
            let mode = match (a.mode, a.no_entry) {
                (_, true) => "No Entry Delay",
                (ArmMode::Stay, false) => "In Stay Mode",
                (ArmMode::Away, false) => "In Away Mode",
            };
            items.push(("System Armed".to_string(), mode.to_string()));
            if self.ee.toggle(Model::Pc1832, "kp077", 7) {
                for &z in &self.memory[0] {
                    items.push(("Alarm Memory".to_string(), self.label(z as usize)));
                }
            }
            if show_clock {
                items.push(self.clock_lines(wall_ms));
            }
            let auto = if idle_secs >= 4 { ((idle_secs - 4) / 4 + 1) as i32 } else { 0 };
            return self.scroll_screen(&items, auto);
        }
        let mut items: Vec<(String, String)> = Vec::new();
        let open = self.open_zones();
        if self.ready() {
            items.push(("System is".to_string(), "Ready to Arm".to_string()));
        } else {
            items.push(("Secure System".to_string(), "Before Arming".to_string()));
        }
        for &z in &open {
            items.push((self.label(z as usize), "Open".to_string()));
        }
        if self.memory_flag && !self.memory[0].is_empty() {
            items.push(("Alarm Memory".to_string(), "Press (*3) View".to_string()));
        }
        if self.troubles != 0 {
            items.push(("System Trouble".to_string(), "(*2) to View".to_string()));
        }
        if show_clock {
            items.push(self.clock_lines(wall_ms));
        }
        // Authentic PK5500 auto-scrolling:
        // - Open zones auto-scroll every 2.5 s when [076] Opt 8 is ON.
        // - Idle / Ready / Trouble / Clock alternate every 4 s.
        let auto = if !open.is_empty() && auto_open {
            if idle_secs >= 3 {
                (((now.saturating_duration_since(self.last_key_at).as_millis() - 3000) / 2500) + 1) as i32
            } else {
                0
            }
        } else if items.len() > 1 && idle_secs >= 4 {
            ((idle_secs - 4) / 4 + 1) as i32
        } else {
            0
        };
        self.scroll_screen(&items, auto)
    }

    fn lcd_program(&self, p: &Prog) -> LcdOut {
        let screen = |a: String, b: String, cur: Option<(usize, usize)>| LcdOut { lines: [pad(&a), pad(&b)], cursor: cur };
        if p.reset_stage == 1 {
            return screen("Enter Installer".into(), format!("Code {}", "*".repeat(p.sel.len())), Some((1, 5 + p.sel.len())));
        }
        if p.reset_stage == 2 {
            let act = p.confirm_action.unwrap_or("999");
            let prompt = match act {
                "990" => "Enter 990 Lock",
                "991" => "Enter 991 Unlk",
                _ => "Enter 999 To",
            };
            return screen(prompt.into(), format!("Confirm {}", p.sel), Some((1, 8 + p.sel.len())));
        }
        if let Some(parent) = &p.sub_for {
            return screen(format!("Section [{}]", parent), format!("Sub-Sect {}", p.sel), Some((1, 9 + p.sel.len())));
        }
        let Some(sec) = &p.section else {
            let title = if p.keypad { "Keypad Program" } else { "Enter Section" };
            return screen(title.into(), p.sel.clone(), Some((1, p.sel.len())));
        };
        let Some(sp) = eeprom::spec(self.model, sec) else {
            return screen("Enter Section".into(), String::new(), None);
        };
        let l1 = format!("[{}] {}", sec.trim_start_matches("kp"), sp.name);
        match sp.kind {
            Kind::Toggle(n) => {
                let mut s = String::new();
                for i in 1..=n {
                    let c = char::from(b'0' + i);
                    s.push(if p.work.contains(&c) { c } else { '-' });
                }
                screen(l1, format!("Opt {}", s), None)
            }
            Kind::Data { width, entries } => {
                let w = width.max(1) as usize;
                let e = (p.pos / w).min((entries as usize).saturating_sub(1));
                let digits: String = p.work.iter().skip(e * w).take(w).collect();
                let l2 = format!("{:02}: {}", e + 1, digits);
                let cur = Some((1, 4 + p.pos.saturating_sub(e * w).min(w)));
                screen(l1, if p.hex { format!("{} HEX", l2) } else { l2 }, cur)
            }
            Kind::Phone(_) => {
                let s: String = p.work.iter().collect();
                let count = s.chars().count();
                let tail: String = s.chars().skip(count.saturating_sub(15)).collect();
                let len = tail.chars().count();
                screen(l1, tail, Some((1, len.min(15))))
            }
            Kind::Code(_) => screen(l1, format!("Code {}", "*".repeat(p.work.len())), Some((1, 5 + p.work.len()))),
            _ => screen(l1, String::new(), None),
        }
    }

    fn lcd_usercodes(&self, slot: &Option<String>, buf: &str, idx: usize, attrs: bool) -> LcdOut {
        let slots = eeprom::user_slots(self.model);
        match slot {
            None => {
                let s = &slots[idx % slots.len()];
                let l1 = if attrs { arrows("Select Atrib") } else { arrows("Select Code") };
                let state = if self.ee.has_code(s) { "Programmed" } else { "Not Set" };
                let who = if s == "40" { "Mstr".to_string() } else { s.clone() };
                LcdOut { lines: [l1, pad(&format!("({}) {}", who, state))], cursor: None }
            }
            Some(s) => {
                if attrs {
                    let a = self.ee.attrs(s);
                    let mut opts = String::new();
                    for i in 0..8u8 {
                        opts.push(if a & (1u8 << i) != 0 { char::from(b'1' + i) } else { '-' });
                    }
                    LcdOut { lines: [pad(&format!("({}) Attributes", s)), pad(&format!("Opt {}", opts))], cursor: None }
                } else {
                    LcdOut {
                        lines: [pad(&format!("({}) Enter New", s)), pad(&format!("Code {}", "*".repeat(buf.len())))],
                        cursor: Some((1, 5 + buf.len().min(4))),
                    }
                }
            }
        }
    }

    fn lcd_event(&self, idx: usize, detail: bool) -> LcdOut {
        if self.events.is_empty() {
            return plain("Event Buffer", "Empty");
        }
        let n = self.events.len();
        let i = idx.min(n - 1);
        let ev = &self.events[n - 1 - i];
        if detail {
            let a: String = ev.text.chars().take(16).collect();
            let b: String = ev.text.chars().skip(16).take(16).collect();
            plain(&a, &b)
        } else {
            let t = util::local(ev.ts);
            LcdOut {
                lines: [arrows(&format!("({:03}) Event", n - i)), pad(&t.format("%b %d %H:%M").to_string())],
                cursor: None,
            }
        }
    }
}
