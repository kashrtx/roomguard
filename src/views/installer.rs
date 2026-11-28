//! Panel & Installer programming view in Settings.
//!
//! Lets you configure every real panel setting (delays, bell cut-off, zone
//! definitions and attributes, chime sounds, keypad levels, codes, and every
//! raw EEPROM installer section) directly with plain-English controls, no
//! need to memorize [*][8] keypad sequences unless you want to. Every change
//! writes straight to the same EEPROM sections the keypad reads and writes.

use std::collections::BTreeMap;

use eframe::egui::{self, RichText};

use crate::config::AppConfig;
use crate::engine::eeprom::{self, Kind, Spec};
use crate::engine::types::{Model, ZoneKind};
use crate::engine::{EngineHandle, EngineMsg, PanelEdit, ProgView, View};
use crate::theme;

#[derive(Default)]
pub struct InstallerState {
    pub filter: String,
    pub master_code: String,
    pub installer_code: String,
    pub extra_code: String,
    pub raw_drafts: BTreeMap<String, String>,
    pub confirm_default: bool,
}

const FUTURE: &str = "For future use";

/// What each option does when it's ON, option 1 first.
fn options(m: Model, key: &str) -> &'static [&'static str] {
    match (m, key) {
        (Model::Pc1550, "12") => &[
            "Communicator disabled",
            "Transmission limit counts per 24 hours (off: per armed period)",
            "Alarm display while armed",
            "DTMF dialing (off: pulse dialing)",
            "Normally closed loops (off: end of line resistors)",
            "Keypad [P]anic sounds the bell (off: silent)",
        ],
        (Model::Pc1550, "13") => &[
            "Call the 1st phone number only (off: back up to the 2nd)",
            "Master code can't be changed by the user",
            "Bell squawk, one burst arming and two disarming",
            "PC16OUT module enabled",
            "Code 6 is a one time use maid's code",
            "1400 Hz handshake for Radionics (off: 2300 Hz)",
        ],
        (Model::Pc1550, "14") => &[
            "An access code is needed for [*][1] bypassing",
            "[*][4] downloading call enabled",
            "Periodic downloading (off: periodic test transmission)",
            "[*][4] needs an access code",
            "[P]anic key gives keypad beeps",
            "Partial closings identified",
        ],
        (Model::Pc1550, "16") => &[
            "Zone 1 can be bypassed",
            "Zone 2 can be bypassed",
            "Zone 3 can be bypassed",
            "Zone 4 can be bypassed",
            "Zone 5 can be bypassed",
            "Zone 6 can be bypassed",
        ],
        (Model::Pc1550, "31") => &[
            "Answers a call from the downloading computer",
            "Calls the downloading computer back",
            "Answering machine override",
            "Shows bypass status while armed (off: only while disarmed)",
            "50 Hz AC (off: 60 Hz)",
            "Restore on bell time-out (off: restore when the zone restores)",
        ],
        (Model::Pc1550, "32") => &[
            "Losing AC power doesn't cause a trouble",
            "No bell bursts during the auto-arm warning",
            "Home-Away zones get an entry delay (off: they act as interior)",
            "Quick exit with [*][0]",
            "Auxiliary delay zones can be force armed",
            "Standard pulsed fire bell (off: Temporal Three)",
        ],
        (Model::Pc1550, "34") => &[
            "Audible exit delay with urgency (off: a silent exit delay, which is how the panel ships)",
            "Bell squawk during the exit delay",
            "Audible exit fault",
            "Faster beeping in the last 10 seconds of the entry delay",
            "Bell squawk during the entry delay",
            "[F] key disabled",
        ],
        (Model::Pc1832, "013") => &[
            "Normally closed loops (off: end of line resistors)",
            "Double end of line resistors",
            "Shows all troubles while armed (off: only fire troubles)",
            "Tampers and faults don't show as open",
            "Auto-arm schedule in [*][6] and installer programming",
            "Audible exit fault",
            "Event buffer follows swinger shutdown",
            "Temporal Three fire signal (off: standard pulsed)",
        ],
        (Model::Pc1832, "014") => &[
            "Bell squawk when arming and disarming",
            "Bell squawk during auto-arm",
            "Bell squawk during the exit delay",
            "Bell squawk during the entry delay",
            "Bell squawk on trouble",
            "Audible exit delay with urgency (off: silent exit delay)",
            "Exit delay termination",
            "Residential fire bell is continuous (off: stops at bell cut-off)",
        ],
        (Model::Pc1832, "015") => &[
            "Fire key enabled",
            "Panic key audible, bell and beeps (off: silent)",
            "Quick exit with [*][0]",
            "Quick arming, no code needed for [*][0], Stay and Away",
            "A code is needed for bypassing",
            "Master code can't be changed",
            "TLM, the phone line watch, enabled",
            "TLM sounds the bell while armed (off: trouble beeps)",
        ],
        (Model::Pc1832, "016") => &[
            "AC trouble displayed",
            "Trouble light flashes when AC fails",
            "Keypad blanks when not in use",
            "A code is needed to wake a blanked keypad",
            "Keypad backlighting",
            "Power save mode",
            "Bypass status shown while armed",
            "Keypad tampers enabled",
        ],
        (Model::Pc1832, "017") => &[
            "Wireless keys don't use access codes",
            "RF jam logged after 5 minutes (off: 30 seconds)",
            "Audible RF jam trouble beeps",
            "Double hit",
            "Late to close",
            "Daylight saving time",
            FUTURE,
            "Squawk only on Away key arming and disarming",
        ],
        (Model::Pc1832, "018") => &[
            "Test transmission exception",
            FUTURE,
            FUTURE,
            FUTURE,
            "Keypad buzzer follows the bell",
            "Cross zoning (off: police code)",
            "Exit delay restart",
            "AC trouble beeps",
        ],
        (Model::Pc1832, "019") => &[
            FUTURE,
            FUTURE,
            "First zone in alarm",
            FUTURE,
            FUTURE,
            "Green keypad light shows power (off: ready)",
            "[*][6] works for all users (off: master code only)",
            FUTURE,
        ],
        (Model::Pc1832, "021") => &[
            "Codes are blocked during the entry delay",
            FUTURE,
            FUTURE,
            FUTURE,
            FUTURE,
            "Keyswitch disarms only during the entry delay",
            FUTURE,
            FUTURE,
        ],
        (Model::Pc1832, "022") => &[
            "A code is needed for [*][1], [*][2] and [*][3]",
            FUTURE,
            FUTURE,
            "Only the master code bypasses holdup zones",
            FUTURE,
            "RF delinquency",
            FUTURE,
            "Audible exit delay for Stay arming (off: Stay arming is silent)",
        ],
        (Model::Pc1832, "023") => &[
            "Fire key only beeps (off: beeps and rings the bell)",
            FUTURE,
            "Test transmission only while armed",
            "Test transmission cycle in hours (off: days)",
            "Switching from Away to Stay is not allowed",
            "Two way audio stays connected for a new event",
            "Trouble beeps are silent (off: every 10 seconds)",
            "Keyswitch only arms Away",
        ],
        (Model::Pc1832, "030") => &[
            "Zone 1 fast loop response, 36 ms (off: 400 ms)",
            "Zone 2 fast loop response, 36 ms (off: 400 ms)",
            "Zone 3 fast loop response, 36 ms (off: 400 ms)",
            "Zone 4 fast loop response, 36 ms (off: 400 ms)",
            "Zone 5 fast loop response, 36 ms (off: 400 ms)",
            "Zone 6 fast loop response, 36 ms (off: 400 ms)",
            "Zone 7 fast loop response, 36 ms (off: 400 ms)",
            "Zone 8 fast loop response, 36 ms (off: 400 ms)",
        ],
        (Model::Pc1832, k) if k.len() == 3 && matches!(k.parse::<u16>(), Ok(101..=132)) => &[
            "Audible (off: silent)",
            "Steady bell (off: pulsed)",
            "Door chime",
            "Can be bypassed",
            "Can be force armed",
            "Swinger shutdown",
            "Transmission delay",
            "Wireless zone",
            "Cross zoning (requires [018] opt 6 and [176] timer)",
        ],
        (Model::Pc1832, "201") => &[
            "Partition 1 enabled",
            "Partition 2 enabled",
            "Partition 3 enabled",
            "Partition 4 enabled",
            FUTURE,
            FUTURE,
            FUTURE,
            FUTURE,
        ],
        (Model::Pc1832, "202") => &[
            "Zone 1 is in partition 1",
            "Zone 2 is in partition 1",
            "Zone 3 is in partition 1",
            "Zone 4 is in partition 1",
            "Zone 5 is in partition 1",
            "Zone 6 is in partition 1",
            "Zone 7 is in partition 1",
            "Zone 8 is in partition 1",
        ],
        (Model::Pc1832, "380") => &[
            "Communications enabled",
            "Restorals on bell time-out (off: restorals follow zones)",
            "Pulse dialing (off: DTMF)",
            "Switch to pulse dialing on the 5th attempt",
            "3rd telephone number enabled",
            "Alternate dialing between the 1st and 3rd numbers",
            FUTURE,
            "Delinquency follows zone activity",
        ],
        (Model::Pc1832, "381") => &[
            "Keypad ringback when opening after an alarm",
            "Bell ringback when opening after an alarm",
            "SIA sends programmed reporting codes",
            "Closing confirmation, 8 beeps when the closing is received",
            "Talk and listen on phone lines 1 and 3",
            "Talk and listen on phone line 2",
            "Contact ID uses programmed reporting codes",
            "ULC communications priority",
        ],
        (Model::Pc1832, "382") => &[
            FUTURE,
            "Alarm reports during walk test",
            "Communication cancelled message",
            "Call waiting cancel",
            "T-Link interface",
            "AC failure delay in hours (off: minutes)",
            "Residential dial tries only once",
            FUTURE,
        ],
        (Model::Pc1832, "701") => &[
            "50 Hz AC (off: 60 Hz)",
            "Internal crystal time base",
            "AC and battery trouble block arming",
            "Tampers need an installer reset",
            "6 digit access codes",
            "Busy tone detection",
            "High current battery charging",
            "DLS and audio have no priority",
        ],
        (Model::Pc1832, "702") => &[
            "33/67 pulse dialing make/break ratio",
            "Force dialing",
            "Land line test transmission in minutes",
            "1600 Hz handshake",
            "ID tone",
            "2100 Hz ID tone",
            "One time 1 hour DLS window",
            "Bell rings on failure to communicate while armed",
        ],
        (Model::Pc1832, "kp074") => &["Fire key works", "Auxiliary key works", "Panic key works", FUTURE, FUTURE, FUTURE, FUTURE, FUTURE],
        (Model::Pc1832, "kp077") => &["Door chime on openings", "Door chime on closings", "Option 3", "Option 4", "Option 5", "Option 6", "Option 7", "Option 8"],
        _ => &[],
    }
}

/// A name, with its unit, for each entry of a data section.
fn fields(m: Model, key: &str) -> &'static [&'static str] {
    match (m, key) {
        (Model::Pc1550, "05") | (Model::Pc1550, "06") => &["Zone 1", "Zone 2", "Zone 3", "Zone 4", "Zone 5", "Zone 6"],
        (Model::Pc1550, "07") => &["Code 1", "Code 2", "Code 3", "Code 4", "Code 5", "Code 6", "Partial closing"],
        (Model::Pc1550, "08") => &["Code 1", "Code 2", "Code 3", "Code 4", "Code 5", "Code 6", "After an alarm"],
        (Model::Pc1550, "09") => &[
            "Fire loop trouble",
            "Keypad panic",
            "Keypad fire",
            "Keypad auxiliary",
            "Fire loop trouble restore",
            "Keypad panic restore",
            "Keypad fire restore",
            "Keypad auxiliary restore",
        ],
        (Model::Pc1550, "10") => &[
            "Low battery",
            "AC failure",
            "Fuse failure",
            "Low battery restore",
            "AC failure restore",
            "Fuse failure restore",
            "Automatic test",
        ],
        (Model::Pc1550, "15") => &["Swinger shutdown, alarms per zone (00 never shuts down)", "Burglary transmission delay, seconds"],
        (Model::Pc1550, "17") => &[
            "Entry delay, seconds",
            "Exit delay, seconds",
            "Bell cut-off, minutes",
            "AC failure report delay, minutes",
            "Slow loop response, times 10 ms",
            "Test transmission cycle, days",
        ],
        (Model::Pc1550, "18") => &["Auxiliary entry delay, seconds", "Auxiliary exit delay, seconds"],
        (Model::Pc1550, "19") => &["Auto-arm time, HHMM (9999 is off)", "Test transmission time, HHMM (9999 is off)"],
        (Model::Pc1550, "23") => &["Format for the 1st phone number", "Format for the 2nd phone number"],
        (Model::Pc1550, "25") => &["Zone alarms and restorals", "Openings and closings", "Priority alarms", "Maintenance"],
        (Model::Pc1550, "36") => &["Wrong codes before lockout (00 is off)", "Lockout time, minutes"],
        (Model::Pc1832, "005.01") | (Model::Pc1832, "005.02") => &["Entry delay 1, seconds", "Entry delay 2, seconds", "Exit delay, seconds"],
        (Model::Pc1832, "005.09") => &["Bell cut-off, minutes"],
        (Model::Pc1832, "009") => &["PGM 1", "PGM 2"],
        (Model::Pc1832, "012") => &["Wrong codes before lockout (000 is off)", "Lockout time, minutes"],
        (Model::Pc1832, "170") => &["PGM output timer, seconds"],
        (Model::Pc1832, "176") => &["Cross zone and police code timer, seconds"],
        (Model::Pc1832, "199") => &["Auto-arm pre-alert, minutes"],
        (Model::Pc1832, "350") => &["Format for the 1st phone number", "Format for the 2nd phone number"],
        (Model::Pc1832, "377") => &[
            "Swinger shutdown, alarms",
            "Swinger shutdown, tampers",
            "Swinger shutdown, maintenance",
            "Communication delay, seconds",
            "AC failure report delay, minutes",
            "TLM trouble delay, checks",
            "Test transmission cycle, days",
            FUTURE,
            "Zone low battery report delay, days",
        ],
        _ => &[],
    }
}

fn about(m: Model, key: &str) -> &'static str {
    match (m, key) {
        (Model::Pc1550, "01") | (Model::Pc1832, "301") => "The phone number the communicator dials first. RoomGuard's monitoring center answers it.",
        (Model::Pc1550, "02") | (Model::Pc1832, "310") => "The account code sent with every report.",
        (Model::Pc1550, "03") | (Model::Pc1832, "302") => "A second phone number, used as a backup.",
        (Model::Pc1550, "04") => "The account code for the second phone number.",
        (Model::Pc1550, "05") => "Reporting codes sent for an alarm on each zone. 00 sends nothing.",
        (Model::Pc1550, "06") => "Reporting codes sent when a zone restores. 00 sends nothing.",
        (Model::Pc1550, "07") => "Reporting codes sent when each code arms the system.",
        (Model::Pc1550, "08") => "Reporting codes sent when each code disarms the system.",
        (Model::Pc1550, "09") => "Reporting codes for the fire loop and the F, A and P keys.",
        (Model::Pc1550, "10") => "Reporting codes for the battery, AC power, fuses and the automatic test.",
        (Model::Pc1550, "11") | (Model::Pc1832, "001") => "What each zone is. RoomGuard's camera is zone 1, the doorway (a delay zone), and zone 2, the room (an interior zone that's bypassed when you stay home).",
        (Model::Pc1550, "20") | (Model::Pc1832, "006") => "The installer's code, used with [*][8] to program the panel.",
        (Model::Pc1550, "21") => "The master code, code 1. It arms, disarms and opens [*][5] and [*][6].",
        (Model::Pc1832, "007") => "The master code, user 40. It arms, disarms and opens [*][5] and [*][6].",
        (Model::Pc1550, "22") => "A second master code, the master key for several panels. Blank from the factory.",
        (Model::Pc1832, "008") => "The maintenance code. It can only arm the system.",
        (Model::Pc1550, "24") => "What the PGM output does. 04, the factory setting, is the 5 second smoke detector reset that [*][7] fires.",
        (Model::Pc1550, "26") => "The phone number of the downloading computer.",
        (Model::Pc1550, "27") => "The code the downloading computer has to give.",
        (Model::Pc1550, "28") => "The code that identifies this panel to the downloading computer.",
        (Model::Pc1550, "29") => "How many rings before the panel answers a call.",
        (Model::Pc1550, "30") => "Entering [30] resets every section to the factory defaults.",
        (Model::Pc1550, "33") => "The answering machine double call timer, in seconds.",
        (Model::Pc1550, "35") => "The test reporting code for a LINKS1000 cellular communicator.",
        (Model::Pc1550, "90") | (Model::Pc1832, "990") => "Installer's lockout on: a factory reset no longer resets the installer's code.",
        (Model::Pc1550, "91") | (Model::Pc1832, "991") => "Installer's lockout off.",
        (Model::Pc1832, "000.0") => "Which partition and slot this keypad is.",
        (Model::Pc1832, "000.1") | (Model::Pc1832, "000.2") | (Model::Pc1832, "000.3") | (Model::Pc1832, "000.4") | (Model::Pc1832, "000.5") => {
            "What this function key does when it's held for 2 seconds. Holding the digit with the same number does the same thing."
        }
        (Model::Pc1832, "010") => "Programmable outputs 3 to 10.",
        (Model::Pc1832, "011") => "Programmable outputs 11 to 14.",
        (Model::Pc1832, "020") => "Keypad zone assignments.",
        (Model::Pc1832, "303") => "A third phone number.",
        (Model::Pc1832, "311") => "Partition 1's account code, used with Contact ID.",
        (Model::Pc1832, "900") => "Shows the panel's version.",
        (Model::Pc1832, "901") => "Turns the installer's walk test on or off: every zone trip gives a 2 second bell burst.",
        (Model::Pc1832, "902") => "Resets module supervision.",
        (Model::Pc1832, "903") => "Shows which modules are supervised.",
        (Model::Pc1832, "989") => "Puts the master code back to 1234.",
        (Model::Pc1832, "999") => "Resets every section to the factory defaults.",
        (Model::Pc1832, "kp076") => "PK5500 keypad display options.",
        (Model::Pc1832, k) if k.starts_with("kp2") => "The door chime sound this zone makes on the keypad: 1 six beeps, 2 bing bing, 3 ding dong, 4 alarm tone.",
        (Model::Pc1832, k) if k.len() == 3 && matches!(k.parse::<u16>(), Ok(101..=132)) => "This zone's attributes (1-8 plus 9 Cross Zoning). Reset automatically when zone definitions [001]/[002] change.",
        (Model::Vista20p, "31") => "0 = unlimited alarm soundings per zone, 1 = single bell sounding per zone per armed period.",
        (Model::Vista20p, "32") => "0 = fire alarm stops at *33 bell timeout, 1 = no fire sounder timeout (rings until silenced).",
        (Model::Vista20p, "85") => "Cross-zone timer (00=15s, 01=30s ... 15=15m) for zones assigned to Zone List 04 (*81). Applies to zone types 03, 04, and 05.",
        (Model::Vista20p, "panic") => "Keypad emergency pseudo-zone types for Keys A (Zone 95), C (Zone 96), and B (Zone 99): 00=Disabled, 06=24h Silent, 07=24h Audible, 08=24h Aux/Medical, 09=24h Fire.",
        _ => "",
    }
}

/// Sections RoomGuard sets differently from the factory so the camera and
/// the monitoring center work, with the factory value and why.
fn installed(m: Model, key: &str) -> Option<(&'static str, &'static str)> {
    match (m, key) {
        (Model::Pc1550, "01") | (Model::Pc1832, "301") => Some(("blank", "so alarms reach RoomGuard's monitoring center")),
        (Model::Pc1550, "02") => Some(("blank", "so the monitoring center knows it's you")),
        (Model::Pc1550, "05") => Some(("000000000000", "so each zone's alarm is reported")),
        (Model::Pc1550, "11") => Some(("000101020208", "zone 1 is the doorway and zone 2 the room, a Home-Away zone")),
        (Model::Pc1832, "001") => Some(("01030303040404040000000000000000", "zone 2 is the room, an Interior Stay/Away zone")),
        (Model::Pc1832, "350") => Some(("0404 (SIA FSK)", "the monitoring center listens for Contact ID")),
        _ => None,
    }
}

fn zone_type_1550(d: char) -> &'static str {
    match d {
        '0' => "Standard delay",
        '1' => "Instant",
        '2' => "Interior",
        '3' => "Interior Home-Away",
        '4' => "24 hour bell",
        '5' => "24 hour bell/buzzer",
        '6' => "24 hour buzzer",
        '7' => "Auxiliary delay",
        '8' => "Fire",
        '9' => "Not used",
        _ => "Not used",
    }
}

fn zone_speed_1550(d: char) -> &'static str {
    match d {
        '0' => "slow, audible",
        '1' => "slow, silent",
        '2' => "fast, audible",
        '3' => "fast, silent",
        _ => "unknown",
    }
}

fn zone_type_1832(code: &str) -> &'static str {
    match code {
        "00" => "Null (not used)",
        "01" => "Delay 1",
        "02" => "Delay 2",
        "03" => "Instant",
        "04" => "Interior",
        "05" => "Interior Stay/Away",
        "06" => "Delay Stay/Away",
        "07" => "Delayed 24 hour fire",
        "08" => "Standard 24 hour fire",
        "09" => "24 hour supervisory",
        "10" => "24 hour supervisory buzzer",
        "11" => "24 hour burglary",
        "12" => "24 hour holdup",
        "13" => "24 hour gas",
        "14" => "24 hour heat",
        "15" => "24 hour medical",
        "16" => "24 hour panic",
        "17" => "24 hour emergency",
        "18" => "24 hour sprinkler",
        "19" => "24 hour water",
        "20" => "24 hour freeze",
        "21" => "24 hour latching tamper",
        "22" => "Momentary keyswitch",
        "23" => "Maintained keyswitch",
        "25" => "Interior delay",
        "26" => "24 hour non-alarm",
        "29" => "Auto verified fire",
        "30" => "Fire supervisory",
        "31" => "Day zone",
        "32" => "Instant Stay/Away",
        "35" => "24 hour bell/buzzer",
        "36" => "24 hour non-latching tamper",
        "37" => "Night zone",
        "41" | "81" => "24 hour carbon monoxide",
        _ => "Unknown",
    }
}

fn function_key(code: &str) -> &'static str {
    match code {
        "00" => "Nothing",
        "03" => "Stay arm",
        "04" => "Away arm",
        "05" => "No entry arm",
        "06" => "Door chime on and off",
        "07" => "System test",
        "08" => "Bypass mode",
        "09" => "Trouble display",
        "10" => "Alarm memory",
        "14" => "Sensor reset",
        "16" => "Quick exit",
        "17" => "Reactivate Stay/Away zones",
        _ => "Other",
    }
}

fn format_1832(code: &str) -> &'static str {
    match code {
        "01" => "20 BPS, 1400 Hz handshake",
        "02" => "20 BPS, 2300 Hz handshake",
        "03" => "DTMF Contact ID",
        "04" => "SIA FSK",
        "05" => "Pager",
        "06" => "Residential dial",
        "07" => "10 BPS, 1400 Hz handshake",
        "08" => "10 BPS, 2300 Hz handshake",
        _ => "Unknown",
    }
}

fn pgm_1550(code: &str) -> &'static str {
    match code {
        "01" => "Ground start pulse",
        "02" => "Utility output, no code",
        "03" => "Utility output, any code",
        "04" => "5 second reset pulse (smoke detector reset)",
        "05" => "Courtesy pulse",
        "06" => "Keypad buzzer follow",
        "07" => "System armed status",
        "08" => "Strobe (latched alarm)",
        "09" => "Failure to communicate",
        "0A" => "On during entry delay",
        "0B" => "On during exit delay",
        "0C" => "LINKS1000 interface",
        _ => "Unknown",
    }
}

/// The value of a section in words, one line per entry or option.
fn describe(m: Model, spec: &Spec, raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    match spec.kind {
        Kind::Toggle(n) => {
            let names = options(m, spec.key);
            if spec.key.starts_with("kp2") {
                let sound = match raw.chars().next() {
                    Some('2') => "Bing bing",
                    Some('3') => "Ding dong",
                    Some('4') => "Alarm tone",
                    _ => "Six beeps",
                };
                out.push(format!("Chime sound: {}", sound));
                return out;
            }
            for i in 1..=n {
                let on = std::char::from_digit(i as u32, 10).map_or(false, |c| raw.contains(c));
                let name = names.get(i as usize - 1).copied().unwrap_or("");
                let state = if on { "ON " } else { "off" };
                if name.is_empty() {
                    out.push(format!("{}  {}", i, state));
                } else {
                    out.push(format!("{}  {}  {}", i, state, name));
                }
            }
        }
        Kind::Data { entries, width } => {
            let (entries, width) = (entries as usize, width as usize);
            let chars: Vec<char> = raw.chars().collect();
            let names = fields(m, spec.key);
            for e in 0..entries {
                let digits: String = chars.iter().skip(e * width).take(width).collect();
                let meaning = match (m, spec.key) {
                    (Model::Pc1550, "11") => {
                        let d: Vec<char> = digits.chars().collect();
                        let speed = d.first().map_or("unknown", |c| zone_speed_1550(*c));
                        let kind = d.get(1).map_or("Unknown", |c| zone_type_1550(*c));
                        format!("Zone {}: {} {} ({})", e + 1, digits, kind, speed)
                    }
                    (Model::Pc1832, "001") => format!("Zone {:02}: {} {}", e + 1, digits, zone_type_1832(&digits)),
                    (Model::Pc1832, k) if k.starts_with("000.") && k != "000.0" => format!("{} {}", digits, function_key(&digits)),
                    (Model::Pc1832, "350") => format!("{}: {} {}", names.get(e).copied().unwrap_or("Format"), digits, format_1832(&digits)),
                    (Model::Pc1550, "24") => format!("{} {}", digits, pgm_1550(&digits)),
                    _ => match names.get(e) {
                        Some(name) => format!("{}: {}", name, digits),
                        None if entries > 1 => format!("Entry {}: {}", e + 1, digits),
                        None => digits.clone(),
                    },
                };
                out.push(meaning);
            }
        }
        Kind::Phone(_) => out.push(if raw.is_empty() { "Blank".to_string() } else { raw.to_string() }),
        Kind::Code(_) | Kind::Action | Kind::Sub => {}
    }
    out
}

fn installer_code(m: Model) -> &'static str {
    m.installer_default()
}

/// The keys that open this section at the keypad.
fn how_to(m: Model, key: &str, code: &str) -> String {
    if m == Model::Vista20p {
        return format!("[{}][8][0][0] then [*][{}] (or [#][{}] to view)", code, key, key);
    }
    if let Some(kp) = key.strip_prefix("kp") {
        return format!("[*][8][{}] then [*] for keypad programming, then [{}]", code, kp);
    }
    match key.split_once('.') {
        Some((sec, sub)) => format!("[*][8][{}] then [{}] then [{}]", code, sec, sub),
        None => format!("[*][8][{}] then [{}]", code, key),
    }
}

fn section_hdr(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(theme::heading(title));
    ui.add_space(3.0);
}

fn raw_val(prog: &ProgView, key: &str) -> String {
    prog.sections
        .get(key)
        .cloned()
        .unwrap_or_else(|| eeprom::spec(prog.model, key).map(|s| s.default.to_string()).unwrap_or_default())
}

fn toggle_on(prog: &ProgView, key: &str, opt: u8) -> bool {
    let raw = raw_val(prog, key);
    std::char::from_digit(opt as u32, 10).map_or(false, |c| raw.contains(c))
}

fn read_entry(raw: &str, idx: usize, width: usize, fallback: u32) -> u32 {
    raw.get(idx * width..idx * width + width)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(fallback)
}

fn replace_entry(raw: &str, entries: usize, width: usize, idx: usize, val: u32) -> String {
    let need = entries * width;
    let mut padded = raw.to_string();
    while padded.len() < need {
        padded.push('0');
    }
    let mut out = String::with_capacity(need);
    for i in 0..entries {
        if i == idx {
            let max = 10u32.pow(width as u32) - 1;
            out.push_str(&format!("{:0width$}", val.min(max), width = width));
        } else {
            out.push_str(padded.get(i * width..i * width + width).unwrap_or("0"));
        }
    }
    out
}

fn replace_entry_str(raw: &str, entries: usize, width: usize, idx: usize, val: &str) -> String {
    let need = entries * width;
    let mut padded = raw.to_string();
    while padded.len() < need {
        padded.push('0');
    }
    let mut out = String::with_capacity(need);
    for i in 0..entries {
        if i == idx {
            let mut s: String = val.chars().take(width).collect();
            while s.len() < width {
                s.push('0');
            }
            out.push_str(&s);
        } else {
            out.push_str(padded.get(i * width..i * width + width).unwrap_or("0"));
        }
    }
    out
}

const ZONE_CHOICES_1832: &[(&str, &str)] = &[
    ("00", "Null (not used)"),
    ("01", "Delay 1 (Entry/Exit)"),
    ("02", "Delay 2 (Secondary Entry)"),
    ("03", "Instant (Perimeter)"),
    ("04", "Interior"),
    ("05", "Interior Stay/Away"),
    ("06", "Delay Stay/Away"),
    ("08", "24h Standard Fire"),
    ("11", "24h Burglary"),
    ("16", "24h Panic"),
    ("25", "Interior Delay"),
    ("26", "24h Non-Alarm"),
    ("31", "Day Zone"),
    ("32", "Instant Stay/Away"),
    ("37", "Night Zone"),
    ("41", "24h Carbon Monoxide"),
];

const ZONE_CHOICES_1550: &[(char, &str)] = &[
    ('9', "Not Used"),
    ('0', "Standard Delay"),
    ('1', "Instant"),
    ('2', "Interior"),
    ('3', "Interior Home-Away"),
    ('4', "24h Bell"),
    ('5', "24h Bell/Buzzer"),
    ('6', "24h Buzzer"),
    ('7', "Auxiliary Delay"),
    ('8', "24h Fire"),
];

const ZONE_CHOICES_VISTA: &[(&str, &str)] = &[
    ("00", "Not Used"),
    ("01", "Entry/Exit #1"),
    ("02", "Entry/Exit #2"),
    ("03", "Perimeter (Instant)"),
    ("04", "Interior Follower"),
    ("05", "Trouble Day / Alarm Night"),
    ("06", "24h Silent Alarm"),
    ("07", "24h Audible Alarm"),
    ("08", "24h Auxiliary / Medical"),
    ("09", "Fire"),
    ("10", "Interior w/ Delay"),
    ("14", "Carbon Monoxide"),
];

fn zone_type_vista(code: &str) -> &'static str {
    ZONE_CHOICES_VISTA
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| *name)
        .unwrap_or("Unknown")
}

/// Draws the live-editable Panel & Installer Programming tab.
pub fn ui(ui: &mut egui::Ui, cfg: &mut AppConfig, changed: &mut bool, view: &View, engine: &EngineHandle) {
    let prog = &view.prog;
    let armed = view.armed;
    let m = prog.model;

    section_hdr(
        ui,
        match m {
            Model::Pc1550 => "RmG Classic PC1550  (v4.1)",
            Model::Pc1832 => "GuardSeries PC1832  (v4.6)",
            Model::Vista20p => "VistaGuard 20P  (VG6160V Voice)",
        },
    );
    ui.label(theme::muted(match m {
        Model::Vista20p => "Adjust VISTA delays, zones, and 6160V voice settings.",
        _ => "Adjust panel delays, zones, and keypad settings.",
    }));
    if armed {
        ui.add_space(4.0);
        ui.label(RichText::new("Panel is armed. Disarm first to change programming.").color(theme::AMBER));
    }

    // --- 1. Timings (Entry / Exit / Bell Cut-off) ---
    section_hdr(ui, "Delays & bell cut-off");
    ui.add_enabled_ui(!armed, |ui| {
        egui::Grid::new("panel-timings").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            if m == Model::Pc1832 {
                let raw05 = raw_val(prog, "005.01");
                let mut e1 = read_entry(&raw05, 0, 3, 30);
                let mut e2 = read_entry(&raw05, 1, 3, 45);
                let mut ex = read_entry(&raw05, 2, 3, 120);
                let raw09 = raw_val(prog, "005.09");
                let mut bell = read_entry(&raw09, 0, 3, 4);

                ui.label("Entry delay 1");
                if ui.add(egui::DragValue::new(&mut e1).range(0..=255).suffix(" s")).changed() {
                    let v = replace_entry(&raw05, 3, 3, 0, e1);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "005.01".into(), value: v }));
                }
                ui.label(theme::muted("Section [005][01]: primary doorway"));
                ui.end_row();

                ui.label("Entry delay 2");
                if ui.add(egui::DragValue::new(&mut e2).range(0..=255).suffix(" s")).changed() {
                    let v = replace_entry(&raw05, 3, 3, 1, e2);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "005.01".into(), value: v }));
                }
                ui.label(theme::muted("Section [005][01]: secondary entry"));
                ui.end_row();

                ui.label("Exit delay");
                if ui.add(egui::DragValue::new(&mut ex).range(0..=255).suffix(" s")).changed() {
                    let v = replace_entry(&raw05, 3, 3, 2, ex);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "005.01".into(), value: v }));
                }
                ui.label(theme::muted("Section [005][01]: time to leave"));
                ui.end_row();

                ui.label("Bell cut-off");
                if ui.add(egui::DragValue::new(&mut bell).range(1..=99).suffix(" min")).changed() {
                    let v = format!("{:03}", bell);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "005.09".into(), value: v }));
                }
                ui.label(theme::muted("Section [005][09]: siren auto-silence"));
                ui.end_row();
            } else if m == Model::Vista20p {
                let mut ex = read_entry(&raw_val(prog, "34"), 0, 2, 60);
                let mut e1 = read_entry(&raw_val(prog, "35"), 0, 2, 30);
                let mut e2 = read_entry(&raw_val(prog, "36"), 0, 2, 45);
                let mut bell_step = read_entry(&raw_val(prog, "33"), 0, 1, 1);

                ui.label("Exit delay");
                if ui.add(egui::DragValue::new(&mut ex).range(15..=96).suffix(" s")).changed() {
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "34".into(), value: format!("{:02}", ex) }));
                }
                ui.label(theme::muted("Field *34: time to leave"));
                ui.end_row();

                ui.label("Entry delay #1");
                if ui.add(egui::DragValue::new(&mut e1).range(15..=96).suffix(" s")).changed() {
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "35".into(), value: format!("{:02}", e1) }));
                }
                ui.label(theme::muted("Field *35: primary doorway"));
                ui.end_row();

                ui.label("Entry delay #2");
                if ui.add(egui::DragValue::new(&mut e2).range(15..=96).suffix(" s")).changed() {
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "36".into(), value: format!("{:02}", e2) }));
                }
                ui.label(theme::muted("Field *36: secondary entry"));
                ui.end_row();

                ui.label("Bell timeout");
                if ui.add(egui::DragValue::new(&mut bell_step).range(1..=4).suffix(" x 4 min")).changed() {
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "33".into(), value: format!("{}", bell_step) }));
                }
                ui.label(theme::muted("Field *33: 1=4m, 2=8m, 3=12m, 4=16m"));
                ui.end_row();
            } else {
                let raw17 = raw_val(prog, "17");
                let raw18 = raw_val(prog, "18");
                let mut e1 = read_entry(&raw17, 0, 3, 30);
                let mut ex = read_entry(&raw17, 1, 3, 120);
                let mut bell = read_entry(&raw17, 2, 3, 4);
                let mut aux_e = read_entry(&raw18, 0, 3, 45);

                ui.label("Entry delay");
                if ui.add(egui::DragValue::new(&mut e1).range(0..=255).suffix(" s")).changed() {
                    let v = replace_entry(&raw17, 6, 3, 0, e1);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "17".into(), value: v }));
                }
                ui.label(theme::muted("Section [17]: standard delay"));
                ui.end_row();

                ui.label("Auxiliary entry delay");
                if ui.add(egui::DragValue::new(&mut aux_e).range(0..=255).suffix(" s")).changed() {
                    let v = replace_entry(&raw18, 2, 3, 0, aux_e);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "18".into(), value: v }));
                }
                ui.label(theme::muted("Section [18]: auxiliary delay zones"));
                ui.end_row();

                ui.label("Exit delay");
                if ui.add(egui::DragValue::new(&mut ex).range(0..=255).suffix(" s")).changed() {
                    let v = replace_entry(&raw17, 6, 3, 1, ex);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "17".into(), value: v }));
                }
                ui.label(theme::muted("Section [17]: time to leave"));
                ui.end_row();

                ui.label("Bell cut-off");
                if ui.add(egui::DragValue::new(&mut bell).range(1..=99).suffix(" min")).changed() {
                    let v = replace_entry(&raw17, 6, 3, 2, bell);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "17".into(), value: v }));
                }
                ui.label(theme::muted("Section [17]: siren auto-silence"));
                ui.end_row();
            }
        });
    });

    // --- 2. Common Behavior & Options ---
    section_hdr(ui, "Keypad & alarm behavior");
    let mut chime = prog.chime;
    let chime_lbl = match m {
        Model::Vista20p => "Door chime enabled  ([Code] + 9)",
        Model::Pc1832 => "Door chime enabled  ([*][4])",
        Model::Pc1550 => "Door chime enabled  ([*][6][Master][6])",
    };
    if ui.checkbox(&mut chime, chime_lbl).changed() {
        engine.send(EngineMsg::EditPanel(PanelEdit::SetChime(chime)));
    }
    ui.add_enabled_ui(!armed, |ui| {
        if m == Model::Pc1832 {
            let mut audible_exit = toggle_on(prog, "014", 6);
            if ui.checkbox(&mut audible_exit, "Audible exit delay with urgency beeps  ([014] opt 6)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "014".into(), opt: 6, on: audible_exit }));
            }
            let mut stay_audible = toggle_on(prog, "022", 8);
            if ui.checkbox(&mut stay_audible, "Audible exit delay when arming Stay  ([022] opt 8)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "022".into(), opt: 8, on: stay_audible }));
            }
            let mut quick_arm = toggle_on(prog, "015", 4);
            if ui.checkbox(&mut quick_arm, "Quick arming: arm with Stay, Away or [*][0] without a code  ([015] opt 4)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetQuickArm(quick_arm)));
            }
            let mut quick_exit = toggle_on(prog, "015", 3);
            if ui.checkbox(&mut quick_exit, "Quick exit with [*][0] while armed  ([015] opt 3)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "015".into(), opt: 3, on: quick_exit }));
            }
            let mut panic_aud = toggle_on(prog, "015", 2);
            if ui.checkbox(&mut panic_aud, "Panic key sounds the siren and keypad  ([015] opt 2)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "015".into(), opt: 2, on: panic_aud }));
            }
            let mut squawk = toggle_on(prog, "014", 1);
            if ui.checkbox(&mut squawk, "Bell squawk on arming and disarming  ([014] opt 1)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "014".into(), opt: 1, on: squawk }));
            }
            let mut t3 = toggle_on(prog, "013", 8);
            if ui.checkbox(&mut t3, "Temporal Three fire bell pattern (off: standard pulsed)  ([013] opt 8)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "013".into(), opt: 8, on: t3 }));
            }
            let mut scroll_clock = toggle_on(prog, "kp076", 2);
            if ui.checkbox(&mut scroll_clock, "Display clock on LCD while idle  ([kp076] opt 2)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "kp076".into(), opt: 2, on: scroll_clock }));
            }
            let mut clock_24h = toggle_on(prog, "kp076", 3);
            if ui.checkbox(&mut clock_24h, "24-hour clock format on LCD  ([kp076] opt 3)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "kp076".into(), opt: 3, on: clock_24h }));
            }
            let mut scroll_open = toggle_on(prog, "kp076", 8);
            if ui.checkbox(&mut scroll_open, "Auto-scroll open zones on LCD  ([kp076] opt 8)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "kp076".into(), opt: 8, on: scroll_open }));
            }
        } else if m == Model::Vista20p {
            let mut qa = read_entry(&raw_val(prog, "21"), 0, 1, 1) != 0;
            if ui.checkbox(&mut qa, "Quick arm with # + Away/Stay/Max/Instant  (*21)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "21".into(), value: if qa { "1" } else { "0" }.into() }));
            }
            let mut qb = read_entry(&raw_val(prog, "23"), 0, 1, 1) != 0;
            if ui.checkbox(&mut qb, "Quick / forced bypass with [Code] + 6 + #  (*23)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "23".into(), value: if qb { "1" } else { "0" }.into() }));
            }
            let mut aew = read_entry(&raw_val(prog, "37"), 0, 1, 1) != 0;
            if ui.checkbox(&mut aew, "Audible exit warning beeps with urgency  (*37)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "37".into(), value: if aew { "1" } else { "0" }.into() }));
            }
            let mut ding = read_entry(&raw_val(prog, "38"), 0, 1, 0) != 0;
            if ui.checkbox(&mut ding, "Confirmation of arming ding on external bell  (*38)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "38".into(), value: if ding { "1" } else { "0" }.into() }));
            }
            let mut astay = read_entry(&raw_val(prog, "84"), 0, 1, 3) != 0;
            if ui.checkbox(&mut astay, "Auto-Stay: convert Away to Stay if no door opens during exit  (*84)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "84".into(), value: if astay { "3" } else { "0" }.into() }));
            }
            let mut cver = read_entry(&raw_val(prog, "86"), 0, 1, 1) != 0;
            if ui.checkbox(&mut cver, "Flash 'Alarm Canceled' on LCD when clearing alarm memory  (*86)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "86".into(), value: if cver { "1" } else { "0" }.into() }));
            }
            let mut eerr = read_entry(&raw_val(prog, "59"), 0, 1, 0) != 0;
            if ui.checkbox(&mut eerr, "Send exit error report code to monitoring  (*59)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "59".into(), value: if eerr { "1" } else { "0" }.into() }));
            }
            let mut cd = read_entry(&raw_val(prog, "197"), 0, 1, 0);
            ui.horizontal(|ui| {
                ui.label("Exit countdown display interval  (*197)");
                if ui.add(egui::DragValue::new(&mut cd).range(0..=5).suffix(" s")).changed() {
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "197".into(), value: format!("{}", cd) }));
                }
                ui.label(theme::muted(if cd == 0 { "0 = 'You may exit now'" } else { "1-5 s refresh step" }));
            });
            let mut part_disp = read_entry(&raw_val(prog, "198"), 0, 1, 0) != 0;
            if ui.checkbox(&mut part_disp, "Show partition number prefix on idle LCD  (*198)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "198".into(), value: if part_disp { "1" } else { "0" }.into() }));
            }
            let mut cbz = read_entry(&raw_val(prog, "26"), 0, 1, 0) != 0;
            if ui.checkbox(&mut cbz, "Chime by zone using Zone List 03  (*26)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "26".into(), value: if cbz { "1" } else { "0" }.into() }));
            }
            // Function Key D (*57 entry 3) quick selector
            let raw57 = raw_val(prog, "57");
            let cur_d = raw57.get(6..8).unwrap_or("03");
            const VISTA_FN_CHOICES: &[(&str, &str)] = &[
                ("00", "00 Default (Paging)"),
                ("01", "01 Paging (999-9999)"),
                ("02", "02 Display Time/Date"),
                ("03", "03 Arm AWAY"),
                ("04", "04 Arm STAY"),
                ("05", "05 Arm NIGHT-STAY"),
                ("06", "06 Step Arming"),
                ("07", "07 Output Device Toggle"),
                ("08", "08 Comm Test (CID 601)"),
                ("09", "09 Macro 1"),
                ("10", "10 Macro 2"),
                ("11", "11 Macro 3"),
                ("12", "12 Macro 4"),
            ];
            let cur_d_lbl = VISTA_FN_CHOICES.iter().find(|(c, _)| *c == cur_d).map(|(_, l)| *l).unwrap_or(cur_d);
            ui.horizontal(|ui| {
                ui.label("Function key D hold action  (*57)");
                egui::ComboBox::from_id_salt("vista-fn-d")
                    .selected_text(cur_d_lbl)
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for &(code_str, label) in VISTA_FN_CHOICES {
                            if ui.selectable_label(cur_d == code_str, label).clicked() {
                                let next = replace_entry_str(&raw57, 4, 2, 3, code_str);
                                engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "57".into(), value: next }));
                            }
                        }
                    });
            });
        } else {
            let mut audible_exit = toggle_on(prog, "34", 1);
            if ui.checkbox(&mut audible_exit, "Audible exit delay with urgency beeps  ([34] opt 1)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "34".into(), opt: 1, on: audible_exit }));
            }
            let mut entry_urg = toggle_on(prog, "34", 4);
            if ui.checkbox(&mut entry_urg, "Entry delay urgency pulsing in last 10 seconds  ([34] opt 4)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "34".into(), opt: 4, on: entry_urg }));
            }
            let mut quick_arm = prog.quick_arm;
            if ui.checkbox(&mut quick_arm, "Quick arming with [*][0]  ([*][6] opt 4)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetQuickArm(quick_arm)));
            }
            let mut quick_exit = toggle_on(prog, "32", 4);
            if ui.checkbox(&mut quick_exit, "Quick exit with [*][0] while armed  ([32] opt 4)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "32".into(), opt: 4, on: quick_exit }));
            }
            let mut panic_aud = toggle_on(prog, "12", 6);
            if ui.checkbox(&mut panic_aud, "Keypad [P]anic key sounds the bell  ([12] opt 6)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "12".into(), opt: 6, on: panic_aud }));
            }
            let mut squawk = toggle_on(prog, "13", 3);
            if ui.checkbox(&mut squawk, "Bell squawk on arming and disarming  ([13] opt 3)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "13".into(), opt: 3, on: squawk }));
            }
            let mut t3 = !toggle_on(prog, "32", 6);
            if ui.checkbox(&mut t3, "Temporal Three fire bell pattern (off: standard pulsed)  ([32] opt 6)").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "32".into(), opt: 6, on: !t3 }));
            }
        }

        // Keypad lockout (PC1550 [36] & PC1832 [012]; Vista *36 is Entry Delay #2)
        if m != Model::Vista20p {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let (lock_key, width) = if m == Model::Pc1832 { ("012", 3usize) } else { ("36", 2usize) };
                let raw_lock = raw_val(prog, lock_key);
                let mut bad = read_entry(&raw_lock, 0, width, 0);
                let mut mins = read_entry(&raw_lock, 1, width, 0);
                ui.label("Keypad lockout after");
                if ui.add(egui::DragValue::new(&mut bad).range(0..=99)).changed() {
                    let v = replace_entry(&raw_lock, 2, width, 0, bad);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: lock_key.into(), value: v }));
                }
                ui.label("wrong codes (0 = off) for");
                if ui.add(egui::DragValue::new(&mut mins).range(0..=99).suffix(" min")).changed() {
                    let v = replace_entry(&raw_lock, 2, width, 1, mins);
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: lock_key.into(), value: v }));
                }
            });
        }

        // Auto-arm schedule
        ui.horizontal_wrapped(|ui| {
            let mut auto_on = prog.auto_arm;
            let (mut hh, mut mm) = prog.auto_arm_hm.unwrap_or((23, 0));
            if ui.checkbox(&mut auto_on, "Auto-arm daily at").changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetAutoArm {
                    enabled: auto_on,
                    hm: Some((hh, mm)),
                }));
            }
            ui.add_enabled_ui(auto_on, |ui| {
                let h_ch = ui.add(egui::DragValue::new(&mut hh).range(0..=23).fixed_decimals(0)).changed();
                ui.label(":");
                let m_ch = ui.add(egui::DragValue::new(&mut mm).range(0..=59).fixed_decimals(0)).changed();
                if h_ch || m_ch {
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetAutoArm {
                        enabled: auto_on,
                        hm: Some((hh, mm)),
                    }));
                }
            });
        });

    });

    // --- 3. PK5500 LCD & Buzzer Levels (PC1832) ---
    if m == Model::Pc1832 {
        section_hdr(ui, "PK5500 LCD & buzzer levels");
        let mut br = prog.brightness;
        let mut co = prog.contrast;
        let mut bz = prog.buzzer;
        let mut lvl_changed = false;
        egui::Grid::new("pk5500-levels").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Backlight brightness");
            lvl_changed |= ui.add(egui::Slider::new(&mut br, 0..=9)).changed();
            ui.label(theme::muted("[*][6] Brightness (0 to 9)"));
            ui.end_row();

            ui.label("LCD contrast");
            lvl_changed |= ui.add(egui::Slider::new(&mut co, 0..=9)).changed();
            ui.label(theme::muted("[*][6] Contrast (0 to 9)"));
            ui.end_row();

            ui.label("Buzzer tone / level");
            lvl_changed |= ui.add(egui::Slider::new(&mut bz, 0..=20)).changed();
            ui.label(theme::muted(if bz == 0 { "0 = muted" } else { "[*][6] Buzzer (01 to 20 pitch)" }));
            ui.end_row();
        });
        if lvl_changed {
            engine.send(EngineMsg::EditPanel(PanelEdit::SetKeypadLevels { brightness: br, contrast: co, buzzer: bz }));
        }
    }
    if m == Model::Vista20p {
        section_hdr(ui, "6160V Voice annunciator");
        let mut vl = prog.voice_level;
        ui.horizontal(|ui| {
            ui.label("Voice volume");
            if ui.add(egui::Slider::new(&mut vl, 0..=7)).changed() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetVoiceLevel(vl)));
            }
            ui.label(theme::muted(if vl == 0 { "Muted (#+0+2 on keypad)" } else { "0 = mute, 1 to 7 (#+0+2)" }));
        });
    }

    // --- 4. Zones (Names, Definitions, Chime Sounds & Quick Add) ---
    section_hdr(ui, "Zone programming");
    ui.add(
        egui::Label::new(theme::muted(
            "Customize every zone's name and type here. Matching a real wall panel? Copy the zone numbers, names, and types written by your installer inside your keypad door.",
        ))
        .wrap(),
    );
    let page_id = egui::Id::new(("zone-program-page", m.file_key()));
    let next_unused = view.zones.iter().find(|z| z.kind == ZoneKind::Null).map(|z| z.num);
    let active_count = view.zones.iter().filter(|z| z.kind != ZoneKind::Null).count();
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::muted(format!("{active_count} of {} zones active", m.zone_count())));
        if let Some(znum) = next_unused {
            if ui
                .add_enabled(!armed, egui::Button::new(format!("+ Add Zone {:02}", znum)))
                .on_hover_text("Enable the next unused zone slot as Instant (Perimeter)")
                .clicked()
            {
                let default_code = match m {
                    Model::Pc1550 => "01",
                    Model::Pc1832 | Model::Vista20p => "03",
                };
                engine.send(EngineMsg::EditPanel(PanelEdit::SetZoneType {
                    zone: znum,
                    code: default_code.into(),
                }));
                let target_page = (usize::from(znum) - 1) / 8;
                ui.data_mut(|data| data.insert_temp(page_id, target_page));
            }
        }
    });
    let zones = super::zone_page(ui, page_id, m.zone_count());
    let ha_bindings: Vec<u8> = cfg
        .home_assistant
        .bindings
        .iter()
        .filter(|b| b.model == m)
        .map(|b| b.zone)
        .collect();
    let labels = cfg.labels_mut(m);
    ui.add_enabled_ui(!armed, |ui| {
        if m == Model::Pc1832 {
            egui::Grid::new("panel-zones-1832").num_columns(5).spacing([6.0, 6.0]).show(ui, |ui| {
                ui.label(theme::muted("Zone"));
                ui.label(theme::muted("Name"));
                ui.label(theme::muted("Definition ([001]/[002])"));
                ui.label(theme::muted("Chime"));
                ui.label(theme::muted("Options"));
                ui.end_row();

                for z in zones {
                    let is_ha = ha_bindings.contains(&(z as u8));
                    ui.label(RichText::new(if is_ha { format!("Z{:02}*", z) } else { format!("Z{:02}", z) }).font(theme::semi(12.5)))
                        .on_hover_text(if is_ha { "Supplied by Home Assistant" } else { "Panel zone" });
                    if let Some(lbl) = labels.get_mut(z - 1) {
                        if ui.add_sized([102.0, 22.0], egui::TextEdit::singleline(lbl).char_limit(24)).changed() {
                            *changed = true;
                        }
                    }
                    let (section, index) = eeprom::pc1832_zone_section(z);
                    let definitions = raw_val(prog, section);
                    let cur_code = definitions.get(index * 2..index * 2 + 2).unwrap_or("00");
                    let cur_name = zone_type_1832(cur_code);
                    egui::ComboBox::from_id_salt(("zdef1832", z))
                        .selected_text(format!("{} {}", cur_code, cur_name))
                        .width(134.0)
                        .show_ui(ui, |ui| {
                            for &(code_str, label) in ZONE_CHOICES_1832 {
                                if ui.selectable_label(cur_code == code_str, format!("{} {}", code_str, label)).clicked() {
                                    let next = replace_entry_str(&definitions, 16, 2, index, code_str);
                                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: section.into(), value: next }));
                                }
                            }
                        });

                    let kp_key = format!("kp{:03}", 200 + z);
                    let cur_chime = raw_val(prog, &kp_key);
                    let cur_c = cur_chime.chars().next().unwrap_or('1');
                    let chime_label = match cur_c {
                        '2' => "Bing bing",
                        '3' => "Ding dong",
                        '4' => "Alarm tone",
                        _ => "Six beeps",
                    };
                    egui::ComboBox::from_id_salt(("zchime", z))
                        .selected_text(chime_label)
                        .width(78.0)
                        .show_ui(ui, |ui| {
                            for (c, lbl) in [('1', "Six beeps"), ('2', "Bing bing"), ('3', "Ding dong"), ('4', "Alarm tone")] {
                                if ui.selectable_label(cur_c == c, lbl).clicked() {
                                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: kp_key.clone(), value: c.to_string() }));
                                }
                            }
                        });

                    let attr_key = format!("{:03}", 100 + z);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (opt, short, tip) in [
                            (1u8, "Aud", "Audible alarm"),
                            (3, "Chm", "Door chime"),
                            (4, "Byp", "Bypassable"),
                            (5, "Frc", "Force arm"),
                        ] {
                            let mut on = toggle_on(prog, &attr_key, opt);
                            if ui.checkbox(&mut on, short).on_hover_text(tip).changed() {
                                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: attr_key.clone(), opt, on }));
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        } else if m == Model::Vista20p {
            let raw56 = raw_val(prog, "56");
            let raw_resp = raw_val(prog, "resp");
            egui::Grid::new("panel-zones-vista").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(theme::muted("Zone"));
                ui.label(theme::muted("Name"));
                ui.label(theme::muted("Zone Type (*56)"));
                ui.label(theme::muted("Loop Response"));
                ui.end_row();

                for z in zones {
                    let is_ha = ha_bindings.contains(&(z as u8));
                    ui.label(RichText::new(if is_ha { format!("Z{:02}*", z) } else { format!("Z{:02}", z) }).font(theme::semi(12.5)))
                        .on_hover_text(if is_ha { "Supplied by Home Assistant" } else { "Panel zone" });
                    if let Some(lbl) = labels.get_mut(z - 1) {
                        let resp = ui.add_sized([135.0, 22.0], egui::TextEdit::singleline(lbl).char_limit(24));
                        if resp.changed() {
                            *changed = true;
                        }
                        let (spoken, matched) = crate::audio::voice::spoken_zone_preview(lbl, z as u8);
                        resp.on_hover_text(if matched {
                            format!("6160V voice will speak: \"{}\"", spoken)
                        } else {
                            format!("Words not in 6160V voice ROM. Voice falls back to: \"{}\"", spoken)
                        });
                    }
                    let cur_code = raw56.get((z - 1) * 2..(z - 1) * 2 + 2).unwrap_or("00");
                    egui::ComboBox::from_id_salt(("zdefvista", z))
                        .selected_text(format!("{} {}", cur_code, zone_type_vista(cur_code)))
                        .width(175.0)
                        .show_ui(ui, |ui| {
                            for &(code_str, label) in ZONE_CHOICES_VISTA {
                                if ui.selectable_label(cur_code == code_str, format!("{} {}", code_str, label)).clicked() {
                                    let next = replace_entry_str(&raw56, m.zone_count(), 2, z - 1, code_str);
                                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "56".into(), value: next }));
                                }
                            }
                        });
                    let cur_r = raw_resp.chars().nth(z - 1).unwrap_or('1');
                    let r_lbl = match cur_r {
                        '0' => "10 ms (Fast)",
                        '2' => "700 ms",
                        '3' => "1.2 s",
                        _ => "350 ms",
                    };
                    egui::ComboBox::from_id_salt(("zrespvista", z))
                        .selected_text(r_lbl)
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            for (c, lbl) in [('0', "10 ms (Fast)"), ('1', "350 ms (Normal)"), ('2', "700 ms"), ('3', "1.2 s")] {
                                if ui.selectable_label(cur_r == c, lbl).clicked() {
                                    let next = replace_entry_str(&raw_resp, m.zone_count(), 1, z - 1, &c.to_string());
                                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "resp".into(), value: next }));
                                }
                            }
                        });
                    ui.end_row();
                }
            });
        } else {
            let raw11 = raw_val(prog, "11");
            egui::Grid::new("panel-zones-1550").num_columns(5).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(theme::muted("Zone"));
                ui.label(theme::muted("Name"));
                ui.label(theme::muted("Definition ([11])"));
                ui.label(theme::muted("Audible"));
                ui.label(theme::muted("Bypass ([16])"));
                ui.end_row();

                for z in 1..=6usize {
                    let is_ha = ha_bindings.contains(&(z as u8));
                    ui.label(RichText::new(if is_ha { format!("Z{}*", z) } else { format!("Z{}", z) }).font(theme::semi(12.5)))
                        .on_hover_text(if is_ha { "Supplied by Home Assistant" } else { "Panel zone" });
                    if let Some(lbl) = labels.get_mut(z - 1) {
                        if ui.add_sized([125.0, 22.0], egui::TextEdit::singleline(lbl).char_limit(24)).changed() {
                            *changed = true;
                        }
                    }
                    let chars: Vec<char> = raw11.chars().collect();
                    let d1 = chars.get((z - 1) * 2).copied().unwrap_or('0');
                    let d2 = chars.get((z - 1) * 2 + 1).copied().unwrap_or('0');
                    egui::ComboBox::from_id_salt(("zdef1550", z))
                        .selected_text(format!("{} {}", d2, zone_type_1550(d2)))
                        .width(155.0)
                        .show_ui(ui, |ui| {
                            for &(ch, label) in ZONE_CHOICES_1550 {
                                if ui.selectable_label(d2 == ch, format!("{} {}", ch, label)).clicked() {
                                    let pair = format!("{}{}", d1, ch);
                                    let next = replace_entry_str(&raw11, 6, 2, z - 1, &pair);
                                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "11".into(), value: next }));
                                }
                            }
                        });
                    let mut aud = d1 == '0' || d1 == '2';
                    if ui.checkbox(&mut aud, "").changed() {
                        let new_d1 = match (d1, aud) {
                            ('2' | '3', true) => '2',
                            ('2' | '3', false) => '3',
                            (_, true) => '0',
                            (_, false) => '1',
                        };
                        let pair = format!("{}{}", new_d1, d2);
                        let next = replace_entry_str(&raw11, 6, 2, z - 1, &pair);
                        engine.send(EngineMsg::EditPanel(PanelEdit::SetSection { key: "11".into(), value: next }));
                    }
                    let mut byp = toggle_on(prog, "16", z as u8);
                    if ui.checkbox(&mut byp, "").changed() {
                        engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle { key: "16".into(), opt: z as u8, on: byp }));
                    }
                    ui.end_row();
                }
            });
        }
    });
    if m == Model::Vista20p {
        ui.add_space(6.0);
        egui::CollapsingHeader::new("6160V speakable zone words (48 words)")
            .id_salt("vista-vocab-panel")
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(theme::muted(
                        "The 6160V voice chip speaks up to 3 words per zone from this built-in ROM list (plus Doorway/First/Second/Third). Names outside this list still show on the LCD, and voice speaks 'Zone XX':",
                    ))
                    .wrap(),
                );
                ui.add_space(3.0);
                ui.add(
                    egui::Label::new(RichText::new(crate::audio::voice::vocab_display_list()).size(12.0).color(theme::TEXT))
                        .wrap(),
                );
            });
    }
}

pub fn advanced_ui(ui: &mut egui::Ui, prog: &ProgView, armed: bool, engine: &EngineHandle, st: &mut InstallerState) {
    let m = prog.model;
    let code = installer_code(m);
    let mut lock = prog.installer_lockout;
    if ui.add_enabled(!armed, egui::Checkbox::new(&mut lock, "Installer lockout (factory reset keeps your installer code)")).changed() {
        engine.send(EngineMsg::EditPanel(PanelEdit::SetInstallerLockout(lock)));
    }

    // Access codes
    section_hdr(ui, "Access codes");
    ui.add_enabled_ui(!armed, |ui| {
        let master_slot = match m {
            Model::Pc1832 => "40",
            Model::Vista20p => "02",
            Model::Pc1550 => "1",
        };
        let extra_slot = match m {
            Model::Pc1832 => "maint",
            Model::Vista20p => "03",
            Model::Pc1550 => "m2",
        };
        let extra_label = match m {
            Model::Pc1832 => "Maintenance code ([008])",
            Model::Vista20p => "User 03 code ([Master]+8+03)",
            Model::Pc1550 => "2nd Master code ([22])",
        };
        egui::Grid::new("panel-codes").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Master code");
            ui.add(
                egui::TextEdit::singleline(&mut st.master_code)
                    .password(true)
                    .char_limit(4)
                    .hint_text(if prog.master_default { "factory 1234" } else { "4 digits" })
                    .desired_width(110.0),
            );
            let ok_m = st.master_code.len() == 4 && st.master_code.chars().all(|c| c.is_ascii_digit());
            if ui.add_enabled(ok_m, egui::Button::new("Save")).clicked() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetCode {
                    slot: master_slot.into(),
                    code: std::mem::take(&mut st.master_code),
                }));
            }
            ui.end_row();

            ui.label("Installer code");
            ui.add(
                egui::TextEdit::singleline(&mut st.installer_code)
                    .password(true)
                    .char_limit(4)
                    .hint_text(if prog.installer_default { format!("factory {}", code) } else { "4 digits".into() })
                    .desired_width(110.0),
            );
            let ok_i = st.installer_code.len() == 4 && st.installer_code.chars().all(|c| c.is_ascii_digit());
            if ui.add_enabled(ok_i, egui::Button::new("Save")).clicked() {
                engine.send(EngineMsg::EditPanel(PanelEdit::SetCode {
                    slot: "inst".into(),
                    code: std::mem::take(&mut st.installer_code),
                }));
            }
            ui.end_row();

            ui.label(extra_label);
            let has_extra = prog.codes.iter().any(|c| c == extra_slot);
            ui.add(
                egui::TextEdit::singleline(&mut st.extra_code)
                    .password(true)
                    .char_limit(4)
                    .hint_text(if has_extra { "set (4 digits)" } else { "blank" })
                    .desired_width(110.0),
            );
            ui.horizontal(|ui| {
                let ok_e = st.extra_code.len() == 4 && st.extra_code.chars().all(|c| c.is_ascii_digit());
                if ui.add_enabled(ok_e, egui::Button::new("Save")).clicked() {
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetCode {
                        slot: extra_slot.into(),
                        code: std::mem::take(&mut st.extra_code),
                    }));
                }
                if has_extra && ui.button("Clear").clicked() {
                    engine.send(EngineMsg::EditPanel(PanelEdit::SetCode {
                        slot: extra_slot.into(),
                        code: String::new(),
                    }));
                }
            });
            ui.end_row();
        });
        let mut users: Vec<String> = prog
            .codes
            .iter()
            .filter(|s| s.as_str() != "inst" && s.as_str() != "m2" && s.as_str() != "maint")
            .cloned()
            .collect();
        users.sort();
        ui.label(theme::muted(format!(
            "Programmed user slots: {}. (You can also add/edit user codes via [*][5] on the keypad.)",
            if users.is_empty() { "none".to_string() } else { users.join(", ") }
        )));
    });

    // --- 6. Factory Reset Panel Programming ---
    ui.add_space(6.0);
    ui.add_enabled_ui(!armed, |ui| {
        if !st.confirm_default {
            if ui.button("Reset panel programming to defaults").clicked() {
                st.confirm_default = true;
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Reset all EEPROM sections for this panel?").color(theme::AMBER));
                if super::accent_button(ui, "Confirm reset", theme::RED).clicked() {
                    st.confirm_default = false;
                    st.raw_drafts.clear();
                    engine.send(EngineMsg::EditPanel(PanelEdit::FactoryDefault));
                }
                if ui.button("Cancel").clicked() {
                    st.confirm_default = false;
                }
            });
        }
    });

    // --- 7. All Installer Sections (Searchable & Directly Editable) ---
    section_hdr(ui, "All installer sections (advanced)");
    ui.horizontal(|ui| {
        ui.label("Find");
        ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("section number or word, e.g. 015, exit, chime").desired_width(280.0));
        if !st.filter.is_empty() && ui.button("Clear").clicked() {
            st.filter.clear();
        }
    });
    ui.add_space(4.0);
    let needle = st.filter.trim().to_lowercase();
    let mut shown = 0;
    for spec in eeprom::table(m) {
        if matches!(spec.kind, Kind::Sub) {
            continue;
        }
        let raw = prog.sections.get(spec.key).cloned().unwrap_or_else(|| spec.default.to_string());
        let lines = describe(m, spec, &raw);
        let text = about(m, spec.key);
        if !needle.is_empty() {
            let hay = format!("{} {} {} {}", spec.key, spec.name, text, lines.join(" ")).to_lowercase();
            if !hay.contains(&needle) {
                continue;
            }
        }
        shown += 1;
        let changed = !matches!(spec.kind, Kind::Code(_) | Kind::Action) && raw != spec.default;
        let label = format!("[{}]  {}{}", spec.key.trim_start_matches("kp"), spec.name, if changed { "   (modified)" } else { "" });
        egui::CollapsingHeader::new(RichText::new(label).size(13.0)).show(ui, |ui| {
            if !text.is_empty() {
                ui.label(theme::muted(text));
            }
            match spec.kind {
                Kind::Toggle(n) => {
                    let names = options(m, spec.key);
                    ui.add_enabled_ui(!armed, |ui| {
                        for i in 1..=n {
                            let mut on = toggle_on(prog, spec.key, i);
                            let name = names.get(i as usize - 1).copied().unwrap_or("");
                            let lbl = if name.is_empty() {
                                format!("Option {}", i)
                            } else {
                                format!("{}: {}", i, name)
                            };
                            if ui.checkbox(&mut on, lbl).changed() {
                                engine.send(EngineMsg::EditPanel(PanelEdit::SetToggle {
                                    key: spec.key.to_string(),
                                    opt: i,
                                    on,
                                }));
                            }
                        }
                    });
                }
                Kind::Data { .. } | Kind::Phone(_) => {
                    for l in &lines {
                        ui.label(RichText::new(l).monospace().size(12.0));
                    }
                    ui.add_enabled_ui(!armed, |ui| {
                        let mut clear_draft = false;
                        {
                            let draft = st.raw_drafts.entry(spec.key.to_string()).or_insert_with(|| raw.clone());
                            ui.horizontal(|ui| {
                                ui.label(theme::muted("Raw value:"));
                                ui.add(egui::TextEdit::singleline(draft).font(egui::TextStyle::Monospace).desired_width(220.0));
                                if ui.button("Apply").clicked() {
                                    engine.send(EngineMsg::EditPanel(PanelEdit::SetSection {
                                        key: spec.key.to_string(),
                                        value: draft.clone(),
                                    }));
                                    clear_draft = true;
                                }
                                if *draft != raw && ui.small_button("Revert").clicked() {
                                    *draft = raw.clone();
                                }
                            });
                        }
                        if clear_draft {
                            st.raw_drafts.remove(spec.key);
                        }
                    });
                    if changed {
                        ui.label(theme::muted(format!(
                            "RoomGuard default: {}",
                            if spec.default.is_empty() { "blank" } else { spec.default }
                        )));
                    }
                }
                Kind::Code(slot) => {
                    let line = match slot {
                        "inst" => format!("Now: {}", if prog.installer_default { format!("factory {}", code) } else { "changed".to_string() }),
                        "1" | "40" => format!("Now: {}", if prog.master_default { "factory 1234" } else { "changed" }),
                        other => format!("Now: {}", if prog.codes.iter().any(|c| c == other) { "programmed" } else { "blank" }),
                    };
                    ui.label(line);
                }
                Kind::Action => {}
                Kind::Sub => {}
            }
            if let Some((factory, why)) = installed(m, spec.key) {
                ui.label(theme::muted(format!("Factory: {}. RoomGuard sets this so: {}.", factory, why)));
            }
            ui.label(theme::muted(format!("Keypad sequence: {}", how_to(m, spec.key, code))));
        });
    }
    if shown == 0 {
        ui.label(theme::muted("Nothing matches that. Try a section number like 17 or 005, or a word like delay."));
    }
}
