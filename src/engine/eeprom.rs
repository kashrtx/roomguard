//! The panel's EEPROM: supported programming sections with their numbers,
//! layout and default, for both panels. Data is stored as the same hex digits
//! you'd key in at the keypad. Access codes are stored salted and hashed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::types::{Model, ZoneKind, H24};
use crate::util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `entries` groups of `width` digits.
    Data { entries: u8, width: u8 },
    /// Option toggles 1 to n, shown on the zone lights or the LCD.
    Toggle(u8),
    /// Digits until [#], like a phone number.
    Phone(u8),
    /// A 4 digit access code, stored in the code slot named here.
    Code(&'static str),
    /// Does something when entered, like a factory reset.
    Action,
    /// Has subsections, like PowerSeries [005].
    Sub,
}

pub struct Spec {
    pub key: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub default: &'static str,
}

const fn d(entries: u8, width: u8) -> Kind {
    Kind::Data { entries, width }
}

pub const PC1550: &[Spec] = &[
    Spec { key: "01", name: "1st Phone Number", kind: Kind::Phone(16), default: "5551234" },
    Spec { key: "02", name: "1st Account Code", kind: d(1, 4), default: "1234" },
    Spec { key: "03", name: "2nd Phone Number", kind: Kind::Phone(16), default: "" },
    Spec { key: "04", name: "2nd Account Code", kind: d(1, 4), default: "0000" },
    Spec { key: "05", name: "Zone Alarm Reporting Codes", kind: d(6, 2), default: "313233343536" },
    Spec { key: "06", name: "Zone Restoral Reporting Codes", kind: d(6, 2), default: "000000000000" },
    Spec { key: "07", name: "Closing Reporting Codes", kind: d(7, 2), default: "00000000000000" },
    Spec { key: "08", name: "Opening Reporting Codes", kind: d(7, 2), default: "00000000000000" },
    Spec { key: "09", name: "Priority Alarms and Restorals", kind: d(8, 2), default: "0000000000000000" },
    Spec { key: "10", name: "Maintenance Alarms and Restorals", kind: d(7, 2), default: "00000000000000" },
    Spec { key: "11", name: "Zone Definitions", kind: d(6, 2), default: "000301000308" },
    Spec { key: "12", name: "1st System Option Code", kind: Kind::Toggle(6), default: "346" },
    Spec { key: "13", name: "2nd System Option Code", kind: Kind::Toggle(6), default: "1" },
    Spec { key: "14", name: "3rd System Option Code", kind: Kind::Toggle(6), default: "5" },
    Spec { key: "15", name: "Communication Variables", kind: d(2, 2), default: "0300" },
    Spec { key: "16", name: "Zone Bypass Mask", kind: Kind::Toggle(6), default: "123456" },
    Spec { key: "17", name: "System Times", kind: d(6, 3), default: "030120004030050030" },
    Spec { key: "18", name: "Auxiliary Delay Loop Times", kind: d(2, 3), default: "045120" },
    Spec { key: "19", name: "System Clock Times", kind: d(2, 4), default: "99999999" },
    Spec { key: "20", name: "New Installer's Code", kind: Kind::Code("inst"), default: "1500" },
    Spec { key: "21", name: "New Master Code", kind: Kind::Code("1"), default: "1234" },
    Spec { key: "22", name: "2nd Master Code", kind: Kind::Code("m2"), default: "" },
    Spec { key: "23", name: "Communication Formats", kind: d(2, 1), default: "11" },
    Spec { key: "24", name: "PGM Output Options", kind: d(1, 2), default: "04" },
    Spec { key: "25", name: "Communicator Call Directions", kind: d(4, 1), default: "1111" },
    Spec { key: "26", name: "Downloading Telephone Number", kind: Kind::Phone(16), default: "" },
    Spec { key: "27", name: "Downloading Access Code", kind: d(1, 4), default: "1515" },
    Spec { key: "28", name: "Panel Identification Code", kind: d(1, 4), default: "1501" },
    Spec { key: "29", name: "Rings Before Answering", kind: d(1, 2), default: "12" },
    Spec { key: "30", name: "Reset to Factory Default", kind: Kind::Action, default: "" },
    Spec { key: "31", name: "4th System Option Code", kind: Kind::Toggle(6), default: "" },
    Spec { key: "32", name: "5th System Option Code", kind: Kind::Toggle(6), default: "36" },
    Spec { key: "33", name: "Answering Machine Double Call Timer", kind: d(1, 3), default: "060" },
    Spec { key: "34", name: "6th System Option Code", kind: Kind::Toggle(6), default: "4" },
    Spec { key: "35", name: "LINKS1000 Test Reporting Code", kind: d(1, 2), default: "FF" },
    Spec { key: "36", name: "Keypad Lockout Control", kind: d(2, 2), default: "0000" },
    Spec { key: "90", name: "Installer's Lockout Enable", kind: Kind::Action, default: "" },
    Spec { key: "91", name: "Installer's Lockout Disable", kind: Kind::Action, default: "" },
];

pub const PC1832: &[Spec] = &[
    Spec { key: "000", name: "Kp Enrollment", kind: Kind::Sub, default: "" },
    Spec { key: "000.0", name: "Part/Slot Asgn", kind: d(1, 2), default: "11" },
    Spec { key: "000.1", name: "Func Key 1", kind: d(1, 2), default: "03" },
    Spec { key: "000.2", name: "Func Key 2", kind: d(1, 2), default: "04" },
    Spec { key: "000.3", name: "Func Key 3", kind: d(1, 2), default: "06" },
    Spec { key: "000.4", name: "Func Key 4", kind: d(1, 2), default: "14" },
    Spec { key: "000.5", name: "Func Key 5", kind: d(1, 2), default: "16" },
    Spec { key: "001", name: "Zn Defs 01-16", kind: d(16, 2), default: "01050301050802040000000000000000" },
    Spec { key: "002", name: "Zn Defs 17-32", kind: d(16, 2), default: "00000000000000000000000000000000" },
    Spec { key: "005", name: "Sys Times", kind: Kind::Sub, default: "" },
    Spec { key: "005.01", name: "Part 1 Times", kind: d(3, 3), default: "030045120" },
    Spec { key: "005.02", name: "Part 2 Times", kind: d(3, 3), default: "030045120" },
    Spec { key: "005.09", name: "Bell Cutoff", kind: d(1, 3), default: "004" },
    Spec { key: "006", name: "Installer", kind: Kind::Code("inst"), default: "5555" },
    Spec { key: "007", name: "Master Code", kind: Kind::Code("40"), default: "1234" },
    Spec { key: "008", name: "Maint Code", kind: Kind::Code("maint"), default: "" },
    Spec { key: "009", name: "PGM 1-2 Opts", kind: d(2, 2), default: "1910" },
    Spec { key: "010", name: "PGM 3-10 Opt", kind: d(8, 2), default: "0101010101010101" },
    Spec { key: "011", name: "PGM 11-14", kind: d(4, 2), default: "01010101" },
    Spec { key: "012", name: "Kp Lockout", kind: d(2, 3), default: "000000" },
    Spec { key: "013", name: "1st Sys Opt", kind: Kind::Toggle(8), default: "3567" },
    Spec { key: "014", name: "2nd Sys Opt", kind: Kind::Toggle(8), default: "6" },
    Spec { key: "015", name: "3rd Sys Opt", kind: Kind::Toggle(8), default: "1247" },
    Spec { key: "016", name: "4th Sys Opt", kind: Kind::Toggle(8), default: "15" },
    Spec { key: "017", name: "5th Sys Opt", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "018", name: "6th Sys Opt", kind: Kind::Toggle(8), default: "" },
    Spec { key: "019", name: "7th Sys Opt", kind: Kind::Toggle(8), default: "" },
    Spec { key: "020", name: "Kp Zone Asgn", kind: d(8, 2), default: "0000000000000000" },
    Spec { key: "021", name: "8th Sys Opt", kind: Kind::Toggle(8), default: "" },
    Spec { key: "022", name: "9th Sys Opt", kind: Kind::Toggle(8), default: "" },
    Spec { key: "023", name: "10th Sys Opt", kind: Kind::Toggle(8), default: "" },
    Spec { key: "030", name: "Fast Loop", kind: Kind::Toggle(8), default: "" },
    Spec { key: "101", name: "Zone 1 Attr", kind: Kind::Toggle(8), default: "12346" },
    Spec { key: "102", name: "Zone 2 Attr", kind: Kind::Toggle(8), default: "12456" },
    Spec { key: "103", name: "Zone 3 Attr", kind: Kind::Toggle(8), default: "12346" },
    Spec { key: "104", name: "Zone 4 Attr", kind: Kind::Toggle(8), default: "12346" },
    Spec { key: "105", name: "Zone 5 Attr", kind: Kind::Toggle(8), default: "12456" },
    Spec { key: "106", name: "Zone 6 Attr", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "107", name: "Zone 7 Attr", kind: Kind::Toggle(8), default: "12346" },
    Spec { key: "108", name: "Zone 8 Attr", kind: Kind::Toggle(8), default: "1246" },
    Spec { key: "109", name: "Zone 9 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "110", name: "Zone 10 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "111", name: "Zone 11 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "112", name: "Zone 12 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "113", name: "Zone 13 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "114", name: "Zone 14 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "115", name: "Zone 15 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "116", name: "Zone 16 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "117", name: "Zone 17 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "118", name: "Zone 18 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "119", name: "Zone 19 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "120", name: "Zone 20 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "121", name: "Zone 21 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "122", name: "Zone 22 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "123", name: "Zone 23 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "124", name: "Zone 24 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "125", name: "Zone 25 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "126", name: "Zone 26 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "127", name: "Zone 27 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "128", name: "Zone 28 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "129", name: "Zone 29 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "130", name: "Zone 30 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "131", name: "Zone 31 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "132", name: "Zone 32 Attr", kind: Kind::Toggle(8), default: "" },
    Spec { key: "170", name: "PGM Timer", kind: d(1, 3), default: "005" },
    Spec { key: "176", name: "Cross Zn Tmr", kind: d(1, 3), default: "060" },
    Spec { key: "199", name: "AutoArm PrAl", kind: d(1, 3), default: "004" },
    Spec { key: "201", name: "Part Mask", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "202", name: "Part 1 Zones", kind: Kind::Toggle(8), default: "12345678" },
    Spec { key: "301", name: "1st Phone No", kind: Kind::Phone(32), default: "5551234" },
    Spec { key: "302", name: "2nd Phone No", kind: Kind::Phone(32), default: "" },
    Spec { key: "303", name: "3rd Phone No", kind: Kind::Phone(32), default: "" },
    Spec { key: "310", name: "Acct Code", kind: d(1, 4), default: "1234" },
    Spec { key: "311", name: "Part 1 Acct", kind: d(1, 4), default: "1234" },
    Spec { key: "350", name: "Comm Formats", kind: d(2, 2), default: "0303" },
    Spec { key: "377", name: "Comm Vars", kind: d(9, 3), default: "003003003000030010030030007" },
    Spec { key: "380", name: "1st Comm Opt", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "381", name: "2nd Comm Opt", kind: Kind::Toggle(8), default: "56" },
    Spec { key: "382", name: "3rd Comm Opt", kind: Kind::Toggle(8), default: "" },
    Spec { key: "701", name: "1st Intl Opt", kind: Kind::Toggle(8), default: "" },
    Spec { key: "702", name: "2nd Intl Opt", kind: Kind::Toggle(8), default: "2" },
    Spec { key: "900", name: "Panel Ver", kind: Kind::Action, default: "" },
    Spec { key: "901", name: "Walk Test", kind: Kind::Action, default: "" },
    Spec { key: "902", name: "Supv Reset", kind: Kind::Action, default: "" },
    Spec { key: "903", name: "View Supv", kind: Kind::Action, default: "" },
    Spec { key: "989", name: "Dflt Master", kind: Kind::Action, default: "" },
    Spec { key: "990", name: "Inst Lock On", kind: Kind::Action, default: "" },
    Spec { key: "991", name: "Inst Lock Of", kind: Kind::Action, default: "" },
    Spec { key: "999", name: "Factory Dflt", kind: Kind::Action, default: "" },
    Spec { key: "kp074", name: "Key Options", kind: Kind::Toggle(8), default: "123" },
    Spec { key: "kp076", name: "Kp Options 1", kind: Kind::Toggle(8), default: "1248" },
    Spec { key: "kp077", name: "Kp Options 2", kind: Kind::Toggle(8), default: "124567" },
    Spec { key: "kp201", name: "Zn 1 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp202", name: "Zn 2 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp203", name: "Zn 3 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp204", name: "Zn 4 Chime", kind: Kind::Toggle(8), default: "3" },
    Spec { key: "kp205", name: "Zn 5 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp206", name: "Zn 6 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp207", name: "Zn 7 Chime", kind: Kind::Toggle(8), default: "2" },
    Spec { key: "kp208", name: "Zn 8 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp209", name: "Zn 9 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp210", name: "Zn 10 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp211", name: "Zn 11 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp212", name: "Zn 12 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp213", name: "Zn 13 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp214", name: "Zn 14 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp215", name: "Zn 15 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp216", name: "Zn 16 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp217", name: "Zn 17 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp218", name: "Zn 18 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp219", name: "Zn 19 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp220", name: "Zn 20 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp221", name: "Zn 21 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp222", name: "Zn 22 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp223", name: "Zn 23 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp224", name: "Zn 24 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp225", name: "Zn 25 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp226", name: "Zn 26 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp227", name: "Zn 27 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp228", name: "Zn 28 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp229", name: "Zn 29 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp230", name: "Zn 30 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp231", name: "Zn 31 Chime", kind: Kind::Toggle(8), default: "1" },
    Spec { key: "kp232", name: "Zn 32 Chime", kind: Kind::Toggle(8), default: "1" },
];

/// VistaGuard 20P (VISTA-20P compatible) programming fields.
pub const VISTA20P: &[Spec] = &[
    Spec { key: "20", name: "Installer Code", kind: Kind::Code("inst"), default: "4112" },
    Spec { key: "02", name: "Master Code", kind: Kind::Code("02"), default: "1234" },
    Spec { key: "21", name: "Quick Arm Enable", kind: d(1, 1), default: "0" },
    Spec { key: "23", name: "Quick/Forced Bypass", kind: d(1, 1), default: "0" },
    Spec { key: "26", name: "Chime By Zone", kind: d(1, 1), default: "0" },
    Spec { key: "32", name: "Fire Sounder Timeout", kind: d(1, 1), default: "0" },
    Spec { key: "33", name: "Bell Timeout (x4 min)", kind: d(1, 1), default: "1" },
    Spec { key: "34", name: "Exit Delay (sec)", kind: d(1, 2), default: "60" },
    Spec { key: "35", name: "Entry Delay 1 (sec)", kind: d(1, 2), default: "30" },
    Spec { key: "36", name: "Entry Delay 2 (sec)", kind: d(1, 2), default: "30" },
    Spec { key: "37", name: "Audible Exit Warning", kind: d(1, 1), default: "1" },
    Spec { key: "38", name: "Confirmation of Arming Ding", kind: d(1, 1), default: "0" },
    Spec { key: "39", name: "Power Up In Previous State", kind: d(1, 1), default: "1" },
    Spec { key: "41", name: "Primary Phone Number", kind: Kind::Phone(20), default: "5551234" },
    Spec { key: "42", name: "Secondary Phone Number", kind: Kind::Phone(20), default: "" },
    Spec { key: "43", name: "Partition 1 Account Code", kind: d(1, 4), default: "1234" },
    Spec { key: "50", name: "Burglary Dialer Delay", kind: d(1, 1), default: "2" },
    Spec { key: "56", name: "Zone Types (01-48)", kind: d(48, 2), default: concat!("0104030104090210", "00000000000000000000000000000000000000000000000000000000000000000000000000000000") },
    Spec { key: "57", name: "Function Keys A-D", kind: d(4, 2), default: "00000003" },
    Spec { key: "59", name: "Exit Error Report Code", kind: d(1, 1), default: "0" },
    Spec { key: "81", name: "Night-Stay Zones (List 05)", kind: d(8, 1), default: "00000000" },
    Spec { key: "84", name: "Auto-Stay Arming", kind: d(1, 1), default: "3" },
    Spec { key: "86", name: "Cancel Verify Display", kind: d(1, 1), default: "1" },
    Spec { key: "90", name: "Event Log Enables", kind: d(1, 2), default: "15" },
    Spec { key: "91", name: "Option Selection", kind: d(1, 1), default: "8" },
    Spec { key: "93", name: "Reports In Armed Period", kind: d(1, 1), default: "1" },
    Spec { key: "197", name: "Exit Time Display Interval", kind: d(1, 1), default: "0" },
    Spec { key: "198", name: "Display Partition Number", kind: d(1, 1), default: "0" },
    Spec { key: "chime_list", name: "Chime By Zone (List 03)", kind: d(8, 1), default: "10110010" },
    Spec { key: "resp", name: "Zone Response Times (0-3)", kind: d(48, 1), default: "111111111111111111111111111111111111111111111111" },
    Spec { key: "97", name: "Reset Factory Defaults", kind: Kind::Action, default: "" },
    Spec { key: "98", name: "Exit Programming (Lockout)", kind: Kind::Action, default: "" },
    Spec { key: "99", name: "Exit Programming", kind: Kind::Action, default: "" },
];

pub fn table(model: Model) -> &'static [Spec] {
    match model {
        Model::Pc1550 => PC1550,
        Model::Pc1832 => PC1832,
        Model::Vista20p => VISTA20P,
    }
}

pub fn pc1832_zone_section(zone: usize) -> (&'static str, usize) {
    if zone <= 16 { ("001", zone.saturating_sub(1)) } else { ("002", zone - 17) }
}

pub fn spec(model: Model, key: &str) -> Option<&'static Spec> {
    table(model).iter().find(|s| s.key == key)
}

pub fn data_len(kind: Kind) -> usize {
    match kind {
        Kind::Data { entries, width } => entries as usize * width as usize,
        Kind::Phone(max) => max as usize,
        Kind::Code(_) => 4,
        _ => 0,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct CodeSlot {
    pub hash: String,
    pub salt: String,
    /// PowerSeries attributes: bit 0 supervisor, 1 duress, 2 bypass, 6 squawk, 7 one time.
    pub attrs: u8,
}

/// Bumped whenever a factory default is corrected, so saved panels can be
/// brought up to date once (see `Eeprom::migrate`).
pub const EEPROM_REV: u32 = 5;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Eeprom {
    pub sections: BTreeMap<String, String>,
    pub codes: BTreeMap<String, CodeSlot>,
    pub installer_lockout: bool,
    /// Which set of default corrections this save already has.
    pub rev: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct ZoneCfg {
    pub kind: ZoneKind,
    pub audible: bool,
    pub steady: bool,
    pub chime: bool,
    pub bypassable: bool,
    pub force_arm: bool,
}

impl Eeprom {
    pub fn factory(model: Model) -> Eeprom {
        let mut e = Eeprom { rev: EEPROM_REV, ..Eeprom::default() };
        for s in table(model) {
            match s.kind {
                Kind::Code(slot) => {
                    if !s.default.is_empty() {
                        e.set_code(slot, s.default);
                    }
                }
                Kind::Action | Kind::Sub => {}
                _ => {
                    e.sections.insert(s.key.to_string(), s.default.to_string());
                }
            }
        }
        e
    }

    /// Brings a panel saved by an earlier RoomGuard up to the real factory
    /// defaults from the manuals. A section only changes if it still holds
    /// exactly what the earlier version shipped with, so anything you
    /// programmed yourself is left alone. Returns the sections it changed.
    pub fn migrate(&mut self, model: Model) -> Vec<&'static str> {
        let mut changed: Vec<&'static str> = Vec::new();
        if self.rev < 1 {
            // (section, what RoomGuard 2.0.0 shipped, the manual's factory value)
            let fixes: &[(&'static str, &'static str, &'static str)] = match model {
                Model::Pc1550 => &[("29", "08", "12"), ("36", "0505", "0000")],
                Model::Pc1832 => &[
                    ("012", "005005", "000000"),
                    ("013", "8", "3567"),
                    ("014", "67", "6"),
                    ("015", "1247", "147"),
                    ("017", "", "1"),
                    ("018", "7", ""),
                    ("102", "1246", "12456"),
                    ("105", "1246", "12456"),
                    ("106", "16", "1"),
                    ("199", "001", "004"),
                    ("377", "003003003030030003030001000", "003003003000030010030030007"),
                    ("380", "156", "1"),
                    ("381", "", "56"),
                    ("702", "7", "2"),
                ],
                Model::Vista20p => &[],
            };
            for (key, old, new) in fixes.iter() {
                if self.sections.get(*key).map_or(false, |v| v.as_str() == *old) {
                    self.sections.insert(key.to_string(), new.to_string());
                    changed.push(*key);
                }
            }
            self.rev = 1;
        }
        if self.rev < 2 {
            // RoomGuard 2.1's VISTA-20P defaults, against the Programming Guide.
            let fixes: &[(&'static str, &'static str, &'static str)] = match model {
                Model::Vista20p => &[
                    ("21", "1", "0"),
                    ("23", "1", "0"),
                    ("36", "45", "30"),
                    ("59", "1", "0"),
                    ("84", "1", "3"),
                    ("197", "1", "0"),
                ],
                _ => &[],
            };
            for (key, old, new) in fixes.iter() {
                if self.sections.get(*key).map_or(false, |v| v.as_str() == *old) {
                    self.sections.insert(key.to_string(), new.to_string());
                    changed.push(*key);
                }
            }
            self.rev = 2;
        }
        if self.rev < 3 {
            // Key D is printed AWAY on the 6160, so RoomGuard sets it to arm
            // Away (*57 code 03), undoing the paging default 2.1's first build gave it.
            if model == Model::Vista20p && self.sections.get("57").map_or(false, |v| v.as_str() == "00000000") {
                self.sections.insert("57".to_string(), "00000003".to_string());
                changed.push("57");
            }
            self.rev = 3;
        }
        if self.rev < 4 {
            // Enable PC1550 [34] opt 4 (Entry Delay urgency pulsing in last 10s)
            // on untouched saves so the last 10 seconds pulse like the PC1832.
            if model == Model::Pc1550 && self.sections.get("34").map_or(false, |v| v.is_empty()) {
                self.sections.insert("34".to_string(), "4".to_string());
                changed.push("34");
            }
            self.rev = 4;
        }
        if self.rev < 5 {
            // Enable PC1832 [015] opt 2 (audible Panic key) on untouched saves
            // so the Panic button sounds the selected siren out of the box.
            if model == Model::Pc1832 && self.sections.get("015").map_or(false, |v| v.as_str() == "147") {
                self.sections.insert("015".to_string(), "1247".to_string());
                changed.push("015");
            }
            self.rev = 5;
        }
        changed
    }

    pub fn raw(&self, model: Model, key: &str) -> String {
        if let Some(v) = self.sections.get(key) {
            if model == Model::Vista20p && matches!(key, "56" | "resp") {
                let default = spec(model, key).map_or("", |spec| spec.default);
                if let Some(padding) = default.get(v.len()..) {
                    return format!("{}{}", v, padding);
                }
            }
            return v.clone();
        }
        spec(model, key).map(|s| s.default.to_string()).unwrap_or_default()
    }

    /// One entry of a data section as a decimal number.
    pub fn entry(&self, model: Model, key: &str, idx: usize) -> Option<u32> {
        let sp = spec(model, key)?;
        let (entries, width) = match sp.kind {
            Kind::Data { entries, width } => (entries as usize, width as usize),
            _ => return None,
        };
        if idx >= entries {
            return None;
        }
        let raw = self.raw(model, key);
        let s = raw.get(idx * width..idx * width + width)?;
        s.parse::<u32>().ok()
    }

    pub fn entry_or(&self, model: Model, key: &str, idx: usize, fallback: u32) -> u32 {
        self.entry(model, key, idx).unwrap_or(fallback)
    }

    pub fn entry_digits(&self, model: Model, key: &str, idx: usize) -> String {
        let width = match spec(model, key).map(|s| s.kind) {
            Some(Kind::Data { width, .. }) => width as usize,
            _ => return String::new(),
        };
        self.raw(model, key).chars().skip(idx * width).take(width).collect()
    }

    pub fn toggle(&self, model: Model, key: &str, opt: u8) -> bool {
        match std::char::from_digit(opt as u32, 10) {
            Some(c) => self.raw(model, key).contains(c),
            None => false,
        }
    }

    pub fn toggles(&self, model: Model, key: &str) -> [bool; 8] {
        let raw = self.raw(model, key);
        let mut out = [false; 8];
        for (i, slot) in out.iter_mut().enumerate() {
            if let Some(c) = std::char::from_digit(i as u32 + 1, 10) {
                *slot = raw.contains(c);
            }
        }
        out
    }

    pub fn set_toggles(&mut self, key: &str, opts: &[bool]) {
        let mut s = String::new();
        for (i, on) in opts.iter().enumerate() {
            if *on {
                if let Some(c) = std::char::from_digit(i as u32 + 1, 10) {
                    s.push(c);
                }
            }
        }
        self.sections.insert(key.to_string(), s);
    }

    pub fn set_code(&mut self, slot: &str, code: &str) {
        let salt = util::random_token(12);
        let hash = util::hash_secret(&salt, code);
        let attrs = self.codes.get(slot).map(|c| c.attrs).unwrap_or(0);
        self.codes.insert(slot.to_string(), CodeSlot { hash, salt, attrs });
    }

    pub fn clear_code(&mut self, slot: &str) {
        self.codes.remove(slot);
    }

    pub fn has_code(&self, slot: &str) -> bool {
        self.codes.get(slot).map_or(false, |c| !c.hash.is_empty())
    }

    pub fn code_matches(&self, slot: &str, code: &str) -> bool {
        self.codes
            .get(slot)
            .map_or(false, |c| !c.hash.is_empty() && util::hash_secret(&c.salt, code) == c.hash)
    }

    /// Which slot this code belongs to, if any.
    pub fn find_code(&self, code: &str) -> Option<String> {
        self.codes
            .iter()
            .find(|(_, c)| !c.hash.is_empty() && util::hash_secret(&c.salt, code) == c.hash)
            .map(|(k, _)| k.clone())
    }

    pub fn attrs(&self, slot: &str) -> u8 {
        self.codes.get(slot).map_or(0, |c| c.attrs)
    }

    pub fn set_attrs(&mut self, slot: &str, attrs: u8) {
        if let Some(c) = self.codes.get_mut(slot) {
            c.attrs = attrs;
        }
    }

    pub fn zone_cfg(&self, model: Model, zone: usize) -> ZoneCfg {
        match model {
            Model::Pc1550 => {
                let raw: Vec<char> = self.raw(model, "11").chars().collect();
                let i = zone.saturating_sub(1) * 2;
                let d1 = raw.get(i).and_then(|c| c.to_digit(16)).unwrap_or(0);
                let d2 = raw.get(i + 1).and_then(|c| c.to_digit(16)).unwrap_or(0);
                let kind = match d2 {
                    0 => ZoneKind::Delay1,
                    1 => ZoneKind::Instant,
                    2 => ZoneKind::Interior,
                    3 => ZoneKind::StayAway,
                    4 => ZoneKind::H24(H24::Burglary),
                    5 => ZoneKind::H24(H24::BellBuzzer),
                    6 => ZoneKind::H24(H24::Buzzer),
                    7 => ZoneKind::Delay2,
                    8 => ZoneKind::Fire,
                    _ => ZoneKind::Null,
                };
                ZoneCfg {
                    kind,
                    audible: d1 == 0 || d1 == 2,
                    steady: !kind.is_fire(),
                    chime: matches!(kind, ZoneKind::Delay1 | ZoneKind::Delay2 | ZoneKind::Instant),
                    bypassable: !kind.is_fire() && self.toggle(model, "16", zone as u8),
                    force_arm: kind == ZoneKind::Delay2 && self.toggle(model, "32", 5),
                }
            }
            Model::Pc1832 => {
                let (section, index) = pc1832_zone_section(zone);
                let raw = self.raw(model, section);
                let i = index * 2;
                let code: u32 = raw.get(i..i + 2).and_then(|s| s.parse().ok()).unwrap_or(0);
                let kind = match code {
                    1 => ZoneKind::Delay1,
                    2 => ZoneKind::Delay2,
                    3 => ZoneKind::Instant,
                    4 => ZoneKind::Interior,
                    5 => ZoneKind::StayAway,
                    6 => ZoneKind::DelayStayAway,
                    7 => ZoneKind::FireDelayed,
                    8 => ZoneKind::Fire,
                    9 => ZoneKind::H24(H24::Supervisory),
                    10 => ZoneKind::H24(H24::SupervisoryBuzzer),
                    11 => ZoneKind::H24(H24::Burglary),
                    12 => ZoneKind::H24(H24::Holdup),
                    13 => ZoneKind::H24(H24::Gas),
                    14 => ZoneKind::H24(H24::Heat),
                    15 => ZoneKind::H24(H24::Medical),
                    16 => ZoneKind::H24(H24::Panic),
                    17 => ZoneKind::H24(H24::Emergency),
                    18 => ZoneKind::H24(H24::Sprinkler),
                    19 => ZoneKind::H24(H24::Water),
                    20 => ZoneKind::H24(H24::Freeze),
                    21 => ZoneKind::H24(H24::Tamper),
                    25 => ZoneKind::InteriorDelay,
                    26 => ZoneKind::NonAlarm,
                    30 => ZoneKind::H24(H24::FireSupervisory),
                    31 => ZoneKind::DayZone,
                    32 => ZoneKind::InstantStayAway,
                    35 => ZoneKind::H24(H24::BellBuzzer),
                    36 => ZoneKind::H24(H24::NonLatchTamper),
                    37 => ZoneKind::NightZone,
                    41 | 81 => ZoneKind::H24(H24::CarbonMonoxide),
                    _ => ZoneKind::Null,
                };
                let t = self.toggles(model, &format!("{:03}", 100 + zone));
                ZoneCfg {
                    kind,
                    audible: t[0],
                    steady: t[1],
                    chime: t[2],
                    bypassable: t[3] && !kind.is_fire(),
                    force_arm: t[4],
                }
            }
            Model::Vista20p => {
                let raw = self.raw(model, "56");
                let i = zone.saturating_sub(1) * 2;
                let code: u32 = raw.get(i..i + 2).and_then(|s| s.parse().ok()).unwrap_or(0);
                let (kind, audible, steady, chime) = match code {
                    1 => (ZoneKind::Delay1, true, true, true),
                    2 => (ZoneKind::Delay2, true, true, true),
                    3 => (ZoneKind::Instant, true, true, true),
                    4 => (ZoneKind::StayAway, true, true, false),
                    5 => (ZoneKind::DayZone, true, true, false),
                    6 => (ZoneKind::H24(H24::Holdup), false, true, false),
                    7 => (ZoneKind::H24(H24::Panic), true, true, false),
                    8 => (ZoneKind::H24(H24::Medical), false, true, false),
                    9 => (ZoneKind::Fire, true, false, false),
                    10 => (ZoneKind::DelayStayAway, true, true, false),
                    14 => (ZoneKind::H24(H24::CarbonMonoxide), true, false, false),
                    16 => (ZoneKind::FireDelayed, true, false, false),
                    23 => (ZoneKind::NonAlarm, false, true, false),
                    24 => (ZoneKind::Instant, false, true, false),
                    _ => (ZoneKind::Null, false, true, false),
                };
                ZoneCfg {
                    kind,
                    audible,
                    steady,
                    chime,
                    bypassable: !kind.is_fire() && !kind.is_24h(),
                    force_arm: false,
                }
            }
        }
    }

    pub fn default_changed_pc1832_zone_attrs(&mut self, section: &str, definitions: &str) {
        let first = match section {
            "001" => 1,
            "002" => 17,
            _ => return,
        };
        let previous = self.raw(Model::Pc1832, section);
        for index in 0..16 {
            let start = index * 2;
            let old = previous.get(start..start + 2).unwrap_or("00");
            let new = definitions.get(start..start + 2).unwrap_or("00");
            if old != new {
                let code = new.parse().unwrap_or(0);
                let attributes = if code == 0 { "" } else { Self::default_zone_attrs(code) };
                self.sections.insert(format!("{:03}", 100 + first + index), attributes.to_string());
            }
        }
    }

    /// Default PC1832 zone attributes (Sections [101]–[132]) for a given zone
    /// definition code, applied automatically when Section [001] changes.
    /// Straight from the v4.6 manual's attribute table: 1 audible, 2 steady,
    /// 3 chime, 4 bypass, 5 force arm, 6 swinger shutdown, 7 transmission
    /// delay, 8 wireless.
    pub fn default_zone_attrs(def_code: u32) -> &'static str {
        match def_code {
            0 => "",
            1 | 2 | 3 => "12346",
            4 | 25 | 35 => "1246",
            5 | 6 | 37 => "12456",
            7 | 8 | 13 | 14 | 29 | 41 => "1",
            9 => "25",
            10 => "24",
            11 | 32 => "124",
            12 => "2",
            15 | 16 | 17 | 18 | 19 | 20 | 21 => "12",
            22 | 23 | 26 => "5",
            30 => "",
            31 => "124567",
            36 => "26",
            81 | 87 | 88 => "18",
            _ => "12346",
        }
    }

    /// The PK5500 door chime sound for a zone: 1 six beeps, 2 bing bing,
    /// 3 ding dong, 4 alarm tone.
    pub fn chime_sound(&self, zone: usize) -> u8 {
        let t = self.toggles(Model::Pc1832, &format!("kp{:03}", 200 + zone));
        (0..4).find(|&i| t[i]).map(|i| i as u8 + 1).unwrap_or(1)
    }
}

/// Slot names of the user codes for a model, in keypad order.
pub fn user_slots(model: Model) -> Vec<String> {
    match model {
        Model::Pc1550 => (1..=6).map(|n| n.to_string()).collect(),
        Model::Pc1832 => {
            let mut v: Vec<String> = (1..=39).map(|n| format!("{:02}", n)).collect();
            v.push("40".to_string());
            v.extend((41..=71).map(|n| format!("{:02}", n)));
            v
        }
        Model::Vista20p => (2..=49).map(|n| format!("{:02}", n)).collect(),
    }
}

pub fn slot_user(slot: &str) -> u8 {
    match slot {
        "m2" => 7,
        _ => slot.parse::<u8>().unwrap_or(0),
    }
}

pub fn is_master(model: Model, ee: &Eeprom, slot: &str) -> bool {
    match model {
        Model::Pc1550 => slot == "1" || slot == "m2",
        Model::Pc1832 => slot == "40" || (ee.attrs(slot) & 1) != 0,
        Model::Vista20p => slot == "02" || (ee.attrs(slot) & 0x07) == 4,
    }
}
