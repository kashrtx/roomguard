//! Wire-level bus protocol frame encoders derived from real DSC Classic,
//! DSC PowerSeries Keybus, and Honeywell/Ademco VISTA ECP captures.
//!
//! These frames allow deterministic, bit-exact verification that RoomGuard's
//! panel state machines, LED masks, beep/tone patterns, cursor positions, and
//! LCD screens stay 1:1 faithful to the original hardware protocols.

#![allow(dead_code)]

use std::time::Instant;

use super::kp_vista::VistaKp;
use super::panel::{ArmHow, CodeFor, KpMode, Panel, tb};
use super::types::{ArmMode, Led, Model};
use crate::audio::Pattern;

// ============================================================================
// 1. DSC Classic (PC1550) 16-bit Keybus Frame
//    panelData[0] = Zone 1-8 LEDs (bits 0..5 = Zones 1..6)
//    panelData[1] = Status LEDs & Beep bit:
//      bit 7: Ready, bit 6: Armed, bit 5: Memory, bit 4: Bypass,
//      bit 3: Trouble, bit 2: Program, bit 1: Fire, bit 0: Beep
// ============================================================================

pub const CLASSIC_READY: u8 = 0x80;
pub const CLASSIC_ARMED: u8 = 0x40;
pub const CLASSIC_MEMORY: u8 = 0x20;
pub const CLASSIC_BYPASS: u8 = 0x10;
pub const CLASSIC_TROUBLE: u8 = 0x08;
pub const CLASSIC_PROGRAM: u8 = 0x04;
pub const CLASSIC_FIRE: u8 = 0x02;
pub const CLASSIC_BEEP: u8 = 0x01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassicFrame {
    pub zones_byte: u8,
    pub status_byte: u8,
}

// ============================================================================
// 2. DSC PowerSeries (PC1832 / PK5500) 0x05 Status Frame & 0x75 Tone Byte
//    lights byte:
//      bit 0: Ready, bit 1: Armed, bit 2: Memory, bit 3: Bypass,
//      bit 4: Trouble, bit 5: Program, bit 6: Fire, bit 7: Backlight
// ============================================================================

pub const PS_LIGHT_READY: u8 = 0x01;
pub const PS_LIGHT_ARMED: u8 = 0x02;
pub const PS_LIGHT_MEMORY: u8 = 0x04;
pub const PS_LIGHT_BYPASS: u8 = 0x08;
pub const PS_LIGHT_TROUBLE: u8 = 0x10;
pub const PS_LIGHT_PROGRAM: u8 = 0x20;
pub const PS_LIGHT_FIRE: u8 = 0x40;
pub const PS_LIGHT_BACKLIGHT: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerSeries05 {
    pub lights: u8,
    pub status: u8,
    pub tone_0x75: u8,
}

// ============================================================================
// 3. Honeywell/Ademco VISTA-20P 0xF7 ECP Display Frame (45 bytes on wire)
// ============================================================================

pub const ECP_B1_BEEP_MASK: u8 = 0x07;
pub const ECP_B1_NIGHT: u8 = 0x10;

pub const ECP_B2_ARMED_STAY: u8 = 0x80;
pub const ECP_B2_LOW_BAT: u8 = 0x40;
pub const ECP_B2_ZONE_FIRE: u8 = 0x20;
pub const ECP_B2_READY: u8 = 0x10;
pub const ECP_B2_SYSTEM_FLAG: u8 = 0x04;
pub const ECP_B2_CHECK_FLAG: u8 = 0x02;
pub const ECP_B2_FIRE: u8 = 0x01;

pub const ECP_B3_INSTANT: u8 = 0x80;
pub const ECP_B3_PROGRAM: u8 = 0x40;
pub const ECP_B3_CHIME: u8 = 0x20;
pub const ECP_B3_BYPASS: u8 = 0x10;
pub const ECP_B3_AC_POWER: u8 = 0x08;
pub const ECP_B3_ARMED_AWAY: u8 = 0x04;
pub const ECP_B3_ZONE_ALARM: u8 = 0x02;
pub const ECP_B3_IN_ALARM: u8 = 0x01;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VistaF7 {
    pub zone_bcd: u8,
    pub byte6_beeps_night: u8,
    pub byte7_status1: u8,
    pub byte8_status2: u8,
    pub prompt_pos: u8,
    pub backlight: bool,
    pub prompt1: [u8; 16],
    pub prompt2: [u8; 16],
}

impl VistaF7 {
    /// Serializes this display state into the exact 45-byte `0xF7` ECP wire packet,
    /// including the two's-complement checksum byte at index 44 (`sum % 256 == 0`).
    pub fn to_wire_bytes(&self, keypad_mask: [u8; 4]) -> [u8; 45] {
        let mut buf = [0u8; 45];
        buf[0] = 0xF7;
        buf[1..5].copy_from_slice(&keypad_mask);
        buf[5] = self.zone_bcd;
        buf[6] = self.byte6_beeps_night;
        buf[7] = self.byte7_status1;
        buf[8] = self.byte8_status2;
        buf[9] = if (self.byte8_status2 & ECP_B3_PROGRAM) != 0 { 0x01 } else { 0x00 };
        buf[10] = self.prompt_pos;
        buf[11] = 0x00;
        buf[12..28].copy_from_slice(&self.prompt1);
        if self.backlight {
            buf[12] |= 0x80;
        }
        buf[28..44].copy_from_slice(&self.prompt2);
        let sum: u8 = buf[0..44].iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        buf[44] = 0u8.wrapping_sub(sum);
        buf
    }

    pub fn beeps(&self) -> u8 {
        self.byte6_beeps_night & ECP_B1_BEEP_MASK
    }
}

fn to_bcd(n: u8) -> u8 {
    let clamped = n.min(99);
    ((clamped / 10) << 4) | (clamped % 10)
}

fn str_to_16(s: &str) -> [u8; 16] {
    let mut out = [b' '; 16];
    for (i, ch) in s.chars().take(16).enumerate() {
        let b = if ch.is_ascii() { ch as u8 } else { b' ' };
        out[i] = b & 0x7F;
    }
    out
}

impl Panel {
    /// Produces the 16-bit DSC Classic Keybus frame for the PC1550 (`panelData[0..=1]`).
    pub fn classic_bus_frame(&self, now: Instant) -> ClassicFrame {
        let leds = self.leds_1550(now);
        let on = |l: Led| l != Led::Off;
        let mut zones = 0u8;
        for z in 0..6 {
            if on(leds[5 + z]) {
                zones |= 1 << z;
            }
        }
        let mut status = 0u8;
        if on(leds[0]) {
            status |= CLASSIC_READY;
        }
        if on(leds[1]) {
            status |= CLASSIC_ARMED;
        }
        if on(leds[2]) {
            status |= CLASSIC_MEMORY;
        }
        if on(leds[3]) {
            status |= CLASSIC_BYPASS;
        }
        if on(leds[4]) {
            status |= CLASSIC_TROUBLE;
        }
        if matches!(self.kp, KpMode::Program(_) | KpMode::UserCodes { .. } | KpMode::UserFn { .. }) {
            status |= CLASSIC_PROGRAM;
        }
        if self.live_alarms(now).any(|a| a.kind.pulsed()) || (self.troubles & tb::FIRE_LOOP) != 0 {
            status |= CLASSIC_FIRE;
        }
        if self.sounder_sent != Pattern::Off {
            status |= CLASSIC_BEEP;
        }
        ClassicFrame { zones_byte: zones, status_byte: status }
    }

    /// Returns the authentic DSC PowerSeries `0x05` / `0x27` wire status byte for the PC1832.
    pub fn powerseries_status_code(&self, now: Instant) -> u8 {
        if self.all_on(now) {
            return 0x2F; // Keypad LCD test
        }
        if self.lockout_until.map_or(false, |t| now < t) {
            return 0x10; // Keypad lockout
        }
        if self.is_blanked_1832(now) {
            return if self.troubles != 0 { 0x17 } else { 0x40 }; // Keypad blanking (with/without trouble)
        }
        if self.pc1832_blank_wake {
            return 0x9F; // Enter access code to wake blanked keypad
        }
        match &self.kp {
            KpMode::Program(p) => {
                if p.keypad {
                    return 0xF8; // Keypad programming
                }
                return 0xE4; // *8 Installer programming
            }
            KpMode::Star { .. } => return if self.armed.is_some() { 0xB8 } else { 0x9E },
            KpMode::NeedCode { purpose, .. } => {
                return match purpose {
                    CodeFor::Installer | CodeFor::Keypad => 0xB7,
                    CodeFor::UserCodes | CodeFor::UserFns => 0xA5,
                    _ => 0x9F,
                };
            }
            KpMode::Bypass { .. } => return 0xA0,
            KpMode::Troubles { sub, .. } => {
                return match *sub {
                    1 => 0xC8,
                    5 => 0xC6,
                    6 => 0xB9,
                    _ => 0xA1,
                };
            }
            KpMode::Memory { .. } => return 0xA2,
            KpMode::UserCodes { slot, .. } => {
                if slot.is_some() {
                    return if self.code_len() == 6 { 0xBC } else { 0xA7 };
                }
                return 0xA6;
            }
            KpMode::UserFn { item, .. } => {
                return match item {
                    Some(1) => 0xAA,
                    Some(3) => 0xAB,
                    _ => 0xA9,
                };
            }
            KpMode::CmdOut { .. } => return 0xB2,
            KpMode::Events { .. } | KpMode::Level { .. } => return 0xA9,
            KpMode::Normal => {}
        }

        if self.live_alarms(now).any(|a| a.kind != super::types::AlarmKind::Duress) {
            return 0x11; // Partition in alarm
        }
        if self.entry.is_some() {
            return if self.memory_flag { 0x0D } else { 0x0C }; // Entry delay after alarm / normal
        }
        if let Some(ex) = &self.exit {
            if self.bypass_any() {
                return 0x15; // Arming with bypassed zones
            }
            if ex.how == ArmHow::NoEntry {
                return 0x09; // Arming: No entry delay
            }
            return 0x08; // Exit delay in progress
        }
        if self.auto_pre.is_some() {
            return 0x14; // Auto-arm in progress
        }
        if self.cmd_out_active.map_or(false, |(_, t)| now < t) {
            return 0x33; // Command output in progress
        }
        if let Some(a) = &self.armed {
            if a.quick_exit.map_or(false, |t| now < t) {
                return 0x0B; // Quick exit in progress
            }
            if a.reactivate_at.map_or(false, |t| now < t) {
                return 0x08; // Stay->Away reactivation exit delay
            }
            return match (a.mode, a.no_entry) {
                (ArmMode::Stay, false) => 0x04,
                (ArmMode::Away, false) => 0x05,
                (ArmMode::Stay, true) => 0x06,
                (ArmMode::Away, true) => 0x16,
            };
        }
        if self.ready() {
            if !self.open_zones().is_empty() {
                0x02 // Force-armable / Stay zones open, partition ready
            } else {
                0x01 // Partition ready
            }
        } else {
            0x03 // Zones open
        }
    }

    /// Produces the DSC PowerSeries `0x05` wire status frame and `0x75` tone byte.
    pub fn powerseries_05_frame(&self, now: Instant) -> PowerSeries05 {
        let status = self.powerseries_status_code(now);
        let leds = self.leds_5500(now);
        let blanked = self.is_blanked_1832(now);
        let mut lights = 0u8;
        if leds[0] != Led::Off {
            lights |= PS_LIGHT_READY;
        }
        if leds[1] != Led::Off {
            lights |= PS_LIGHT_ARMED;
        }
        if !blanked && (self.memory_flag || matches!(self.kp, KpMode::Memory { .. })) {
            lights |= PS_LIGHT_MEMORY;
        }
        let stay_exit = self.exit.map_or(false, |e| matches!(e.how, ArmHow::StayKey | ArmHow::NoEntry));
        if !blanked && (self.bypass_any() || stay_exit || matches!(self.kp, KpMode::Bypass { .. })) {
            lights |= PS_LIGHT_BYPASS;
        }
        if leds[2] != Led::Off {
            lights |= PS_LIGHT_TROUBLE;
        }
        if !blanked && matches!(self.kp, KpMode::Program(_) | KpMode::UserCodes { .. } | KpMode::UserFn { .. }) {
            lights |= PS_LIGHT_PROGRAM;
        }
        if self.live_alarms(now).any(|a| a.kind.pulsed()) {
            lights |= PS_LIGHT_FIRE;
        }
        if !blanked && self.ee.toggle(Model::Pc1832, "016", 5) {
            lights |= PS_LIGHT_BACKLIGHT;
        }

        // Authentic 0x75 wire tone byte: bit 7 = steady, bits 6..4 = count, bits 3..0 = interval (s)
        let tone_0x75 = match self.sounder_sent {
            Pattern::Off => 0x00,
            Pattern::Steady => 0x80,
            Pattern::ShortShortLong { .. } => 0xA1, // constant + 2-beep urgency pulse
            Pattern::Beeps { count, period_ms, .. } => {
                let c = count.min(7) << 4;
                let iv = ((period_ms + 500) / 1000).clamp(1, 15) as u8;
                c | iv
            }
            Pattern::HiLo { .. } => 0x80,
        };

        PowerSeries05 { lights, status, tone_0x75 }
    }

    /// Produces the exact Honeywell/Ademco VISTA-20P `0xF7` ECP display frame.
    pub fn vista_f7_frame(&self, now: Instant, wall_ms: i64) -> VistaF7 {
        let out = self.lcd_vista(now, wall_ms);
        let mut b6 = 0u8;
        let mut b7 = 0u8;
        let mut b8 = 0u8;

        // Authentic 0xF7 beep codes (cbuf[6] & 0x07):
        // 0 = none, 4 = rapid beeping (trouble / final 10s / after-alarm entry),
        // 5 = slow beeping (exit / entry), 6 = continuous/alarm sounder
        let beep_code: u8 = match self.sounder_sent {
            Pattern::Off => 0,
            Pattern::Steady | Pattern::HiLo { .. } => 6,
            Pattern::Beeps { period_ms, .. } if period_ms <= 300 => 4,
            Pattern::Beeps { count: 3, period_ms: 4000, .. } => 6,
            Pattern::Beeps { .. } => 5,
            Pattern::ShortShortLong { .. } => 4,
        };
        b6 |= beep_code & ECP_B1_BEEP_MASK;
        if self.vista_night && (self.armed.is_some() || self.exit.is_some()) {
            b6 |= ECP_B1_NIGHT;
        }

        let is_stay = self.armed.map_or(false, |a| a.mode == ArmMode::Stay)
            || self.exit.map_or(false, |e| matches!(e.how, ArmHow::StayKey | ArmHow::NoEntry));
        let is_away = self.armed.map_or(false, |a| a.mode == ArmMode::Away)
            || self.exit.map_or(false, |e| matches!(e.how, ArmHow::AwayKey | ArmHow::MaxKey | ArmHow::Code | ArmHow::Quick | ArmHow::Auto));
        let is_instant = self.armed.map_or(false, |a| a.no_entry)
            || self.exit.map_or(false, |e| matches!(e.how, ArmHow::NoEntry | ArmHow::MaxKey));
        let live_alarm = self.live_alarms(now).next().copied();
        let live_fire = self.live_alarms(now).any(|a| a.kind.pulsed());

        if is_stay {
            b7 |= ECP_B2_ARMED_STAY;
        }
        if (self.troubles & tb::LOW_BATT) != 0 {
            b7 |= ECP_B2_LOW_BAT;
        }
        if live_fire || self.vista_alarm_mem_zone.map_or(false, |(_, k)| k.pulsed()) {
            b7 |= ECP_B2_ZONE_FIRE;
        }
        if self.armed.is_none() && self.exit.is_none() && self.ready() {
            b7 |= ECP_B2_READY;
        }
        let is_zone_line = out.lines[0].starts_with("FAULT ")
            || out.lines[0].starts_with("BYPAS ")
            || out.lines[0].starts_with("CHECK ")
            || out.lines[0].starts_with("ALARM ")
            || out.lines[0].starts_with("FIRE ");
        if !is_zone_line {
            b7 |= ECP_B2_SYSTEM_FLAG;
        }
        if (self.troubles & (tb::ZONE_FAULT | tb::ZONE_TAMPER | tb::BELL | tb::FTC)) != 0 {
            b7 |= ECP_B2_CHECK_FLAG;
        }
        if live_fire {
            b7 |= ECP_B2_FIRE;
        }

        if is_instant {
            b8 |= ECP_B3_INSTANT;
        }
        if matches!(self.vista_mode, VistaKp::Program { .. } | VistaKp::ZoneList(_)) {
            b8 |= ECP_B3_PROGRAM;
        }
        if self.chime {
            b8 |= ECP_B3_CHIME;
        }
        if self.manual_bypass_any() {
            b8 |= ECP_B3_BYPASS;
        }
        if self.inputs.ac_ok {
            b8 |= ECP_B3_AC_POWER;
        }
        if is_away {
            b8 |= ECP_B3_ARMED_AWAY;
        }
        if live_alarm.is_some() || self.vista_alarm_mem_zone.is_some() {
            b8 |= ECP_B3_ZONE_ALARM;
        }
        if live_alarm.is_some() {
            b8 |= ECP_B3_IN_ALARM;
        }

        // Exact BCD zone number byte (cbuf[5]): 0xEA for exit error, or active/fault/alarm zone
        let zone_bcd = if self.vista_exit_alarm {
            0xEA
        } else if let Some(a) = live_alarm {
            to_bcd(a.zone)
        } else if let Some((z, _)) = self.vista_alarm_mem_zone {
            to_bcd(z)
        } else if let Some(&z) = self.open_zones().first() {
            to_bcd(z)
        } else {
            0x00
        };

        let prompt_pos = match out.cursor {
            Some((row, col)) => ((row.min(1) * 16) + col.min(15)) as u8,
            None => 0,
        };

        VistaF7 {
            zone_bcd,
            byte6_beeps_night: b6,
            byte7_status1: b7,
            byte8_status2: b8,
            prompt_pos,
            backlight: self.vista_backlight(now),
            prompt1: str_to_16(&out.lines[0]),
            prompt2: str_to_16(&out.lines[1]),
        }
    }
}
