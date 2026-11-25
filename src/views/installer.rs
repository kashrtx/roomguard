//! The installer view: every programming section of the panel in use, what
//! it's set to right now, what it was from the factory and what it means, so
//! nobody has to dig through the installation manual to see how the panel is
//! programmed. It only shows things; changes are made at the keypad, the same
//! way as on the real panel, and each section says which keys to press.

use eframe::egui::{self, RichText};

use crate::engine::eeprom::{self, Kind, Spec};
use crate::engine::types::Model;
use crate::engine::ProgView;
use crate::theme;

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
        (Model::Pc1832, k) if k.len() == 3 && k.starts_with("10") => &[
            "Audible (off: silent)",
            "Steady bell (off: pulsed)",
            "Door chime",
            "Can be bypassed",
            "Can be force armed",
            "Swinger shutdown",
            "Transmission delay",
            "Wireless zone",
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
        (Model::Pc1832, k) if k.len() == 3 && k.starts_with("10") => "This zone's attributes. They're set to the right defaults for the zone type whenever [001] changes.",
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
        _ => "Unknown",
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
    if let Some(kp) = key.strip_prefix("kp") {
        return format!("[*][8][{}] then [*] for keypad programming, then [{}]", code, kp);
    }
    let _ = m;
    match key.split_once('.') {
        Some((sec, sub)) => format!("[*][8][{}] then [{}] then [{}]", code, sec, sub),
        None => format!("[*][8][{}] then [{}]", code, key),
    }
}

/// Draws the installer view for the panel in use.
pub fn ui(ui: &mut egui::Ui, prog: &ProgView, filter: &mut String) {
    let m = prog.model;
    ui.add_space(8.0);
    ui.label(theme::heading(match m {
        Model::Pc1550 => "RmG Classic PC1550, version 4.1",
        Model::Pc1832 => "GuardSeries PC1832, version 4.6",
    }));
    ui.label(theme::muted(
        "Everything the panel is programmed with, in plain English. Change things at the keypad the same way you would on the real panel: each section shows the keys.",
    ));
    ui.add_space(6.0);
    let code = installer_code(m);
    let inst = if prog.installer_default { format!("still the factory {}", code) } else { "changed (codes are stored scrambled, so it can't be shown)".to_string() };
    let master = if prog.master_default { "still the factory 1234".to_string() } else { "changed".to_string() };
    let mut lines = vec![format!("Installer's code: {}", inst), format!("Master code: {}", master)];
    let mut users: Vec<String> = prog
        .codes
        .iter()
        .filter(|s| s.as_str() != "inst" && s.as_str() != "m2" && s.as_str() != "maint")
        .cloned()
        .collect();
    users.sort();
    lines.push(format!("Access codes programmed: {}", if users.is_empty() { "none".to_string() } else { users.join(", ") }));
    lines.push(format!("Door chime: {}", if prog.chime { "on" } else { "off" }));
    if m == Model::Pc1550 {
        lines.push(format!("Quick arm ([*][6] then 4): {}", if prog.quick_arm { "on" } else { "off" }));
    }
    let auto = match (prog.auto_arm, prog.auto_arm_hm) {
        (true, Some((h, mi))) => format!("on at {:02}:{:02}", h, mi),
        (true, None) => "on, no time set".to_string(),
        (false, _) => "off".to_string(),
    };
    lines.push(format!("Auto-arm: {}", auto));
    if prog.installer_lockout {
        lines.push("Installer's lockout: on".to_string());
    }
    for l in lines {
        ui.label(RichText::new(l).size(13.0));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Find");
        ui.add(egui::TextEdit::singleline(filter).hint_text("a section number or a word, like exit or chime").desired_width(280.0));
        if !filter.is_empty() && ui.button("Clear").clicked() {
            filter.clear();
        }
    });
    ui.add_space(4.0);
    let needle = filter.trim().to_lowercase();
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
        let label = format!("[{}]  {}{}", spec.key.trim_start_matches("kp"), spec.name, if changed { "   (changed)" } else { "" });
        egui::CollapsingHeader::new(RichText::new(label).size(13.0)).show(ui, |ui| {
            if !text.is_empty() {
                ui.label(theme::muted(text));
            }
            match spec.kind {
                Kind::Code(slot) => {
                    let line = match slot {
                        "inst" => format!("Now: {}", if prog.installer_default { format!("the factory {}", code) } else { "changed".to_string() }),
                        "1" | "40" => format!("Now: {}", if prog.master_default { "the factory 1234" } else { "changed" }),
                        other => format!("Now: {}", if prog.codes.iter().any(|c| c == other) { "programmed" } else { "blank" }),
                    };
                    ui.label(line);
                }
                Kind::Action => {}
                _ => {
                    for l in &lines {
                        ui.label(RichText::new(l).monospace().size(12.0));
                    }
                    if changed {
                        let def = describe(m, spec, spec.default);
                        ui.label(theme::muted(format!("RoomGuard default: {}", if spec.default.is_empty() { "all off".to_string() } else { spec.default.to_string() })));
                        if def.len() <= 1 {
                            if let Some(d) = def.first() {
                                ui.label(theme::muted(format!("  {}", d)));
                            }
                        }
                    }
                }
            }
            if let Some((factory, why)) = installed(m, spec.key) {
                ui.label(theme::muted(format!("From the factory this is {}. RoomGuard sets it up this way: {}.", factory, why)));
            }
            ui.label(theme::muted(format!("To change it: {}", how_to(m, spec.key, code))));
        });
    }
    if shown == 0 {
        ui.label(theme::muted("Nothing matches that. Try a section number like 17, or a word like delay."));
    }
}
