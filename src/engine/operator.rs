//! The monitoring center. Signals from the panel arrive here after the
//! communicator dials out, and an operator works each alarm the way a real
//! central station does: call the premises, ask for the passcode, then
//! dispatch or cancel. Anything important is also pushed to your phone.
//!
//! Dispatching is simulated. Nobody is ever actually sent anywhere.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::types::*;
use super::OpInput;
use crate::audio::{AudioCmd, ToneSeg};
use crate::config::OperatorCfg;
use crate::util;

const MAX_CHAT: usize = 200;

pub enum OpOut {
    Audio(AudioCmd),
}

/// What the communicator needs to dial and talk to the receiver.
pub struct DialInfo {
    pub phone: String,
    pub account: String,
    pub contact_id: bool,
    pub code42: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Passcode,
    Dispatch,
}

struct Call {
    state: CallState,
    since: Instant,
    since_ms: i64,
    stage: Stage,
    tries: u8,
    kind: AlarmKind,
    where_: String,
}

struct Pending {
    report: Report,
    due: Instant,
    dialing_until: Option<Instant>,
    photo: Option<PathBuf>,
    line: Vec<ToneSeg>,
    silent: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct ChatSave {
    chat: Vec<ChatMsg>,
}

pub struct Operator {
    pub chat: Vec<ChatMsg>,
    pending: Vec<Pending>,
    call: Option<Call>,
    pub seq: u64,
    last_photo: Option<(Instant, PathBuf)>,
    incident: Option<(AlarmKind, u16)>,
    pub owner: String,
    dirty: bool,
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn zone_phrase(z: u16, label: &str) -> String {
    if z == 0 {
        "your keypad".to_string()
    } else if label.is_empty() {
        format!("zone {}", z)
    } else {
        format!("zone {}, {}", z, label)
    }
}

pub fn cid_name(cid: u16) -> &'static str {
    match cid {
        100 => "Medical",
        101 => "Personal Emergency",
        110 => "Fire",
        114 => "Heat",
        120 => "Panic",
        121 => "Duress",
        122 => "Silent Holdup",
        130 => "Burglary",
        144 => "Sensor Tamper",
        151 => "Gas",
        154 => "Water Leak",
        159 => "Low Temp",
        200 => "Fire Supervisory",
        301 => "AC Loss",
        302 => "Low Battery",
        305 => "System Reset",
        308 => "System Shutdown",
        321 => "Bell Trouble",
        351 => "Telco Fault",
        354 => "Fail to Communicate",
        380 => "Sensor Trouble",
        383 => "Sensor Tamper",
        401 => "Open/Close",
        406 => "Cancel",
        461 => "Wrong Code Entry",
        _ => "Event",
    }
}

fn dtmf(c: char) -> Option<(f32, f32)> {
    let (r, k) = match c {
        '1' => (0, 0),
        '2' => (0, 1),
        '3' => (0, 2),
        'A' => (0, 3),
        '4' => (1, 0),
        '5' => (1, 1),
        '6' => (1, 2),
        'B' => (1, 3),
        '7' => (2, 0),
        '8' => (2, 1),
        '9' => (2, 2),
        'C' => (2, 3),
        '*' | 'E' => (3, 0),
        '0' => (3, 1),
        '#' | 'F' => (3, 2),
        'D' => (3, 3),
        _ => return None,
    };
    let rows = [697.0, 770.0, 852.0, 941.0];
    let cols = [1209.0, 1336.0, 1477.0, 1633.0];
    Some((rows[r], cols[k]))
}

fn tone(f1: f32, f2: f32, ms: u32, level: f32) -> ToneSeg {
    ToneSeg { f1, f2, ms, level }
}

fn gap(ms: u32) -> ToneSeg {
    ToneSeg { f1: 0.0, f2: 0.0, ms, level: 0.0 }
}

/// A Contact ID message: account, 18, qualifier, event, partition, zone or
/// user, and the checksum that makes the digit sum a multiple of 15 (with 0
/// counted as 10).
pub fn contact_id_message(r: &Report, account: &str) -> String {
    let acct: String = account.chars().filter(|c| c.is_ascii_hexdigit()).take(4).collect();
    let q = if r.qualifier == 'R' { '3' } else { '1' };
    let body = format!("{:0>4}18{}{:03}01{:03}", acct.to_uppercase(), q, r.cid % 1000, r.zone_or_user % 1000);
    let sum: u32 = body
        .chars()
        .map(|c| {
            let v = c.to_digit(16).unwrap_or(0);
            if v == 0 {
                10
            } else {
                v
            }
        })
        .sum();
    let check = 15 - (sum % 15);
    let cc = match check {
        10 => '0',
        n => char::from_digit(n, 16).unwrap_or('F').to_ascii_uppercase(),
    };
    format!("{}{}", body, cc)
}

fn dial_sequence(r: &Report, d: &DialInfo) -> Vec<ToneSeg> {
    let mut v = vec![tone(350.0, 440.0, 700, 0.5)];
    for c in d.phone.chars().filter(|c| c.is_ascii_digit()) {
        if let Some((a, b)) = dtmf(c) {
            v.push(tone(a, b, 70, 0.6));
            v.push(gap(70));
        }
    }
    v.push(gap(400));
    v.push(tone(440.0, 480.0, 1200, 0.35));
    v.push(gap(700));
    if d.contact_id {
        v.push(tone(1400.0, 0.0, 100, 0.5));
        v.push(gap(100));
        v.push(tone(2300.0, 0.0, 100, 0.5));
        v.push(gap(250));
        for c in contact_id_message(r, &d.account).chars() {
            if let Some((a, b)) = dtmf(c) {
                v.push(tone(a, b, 50, 0.6));
                v.push(gap(50));
            }
        }
        v.push(gap(250));
    } else {
        // 4/2 pulse format: every digit is sent as that many 1900 Hz bursts,
        // the whole message twice so the receiver can check it.
        v.push(tone(2300.0, 0.0, 400, 0.5));
        v.push(gap(300));
        let acct: String = d.account.chars().filter(|c| c.is_ascii_hexdigit()).take(4).collect();
        let msg = format!("{:0>4}{:0>2}", acct, d.code42.chars().take(2).collect::<String>());
        for _round in 0..2 {
            for c in msg.chars() {
                let n = match c.to_digit(16).unwrap_or(10) {
                    0 => 10,
                    n => n,
                };
                for _ in 0..n {
                    v.push(tone(1900.0, 0.0, 25, 0.5));
                    v.push(gap(25));
                }
                v.push(gap(350));
            }
            v.push(gap(500));
        }
    }
    v.push(tone(1400.0, 0.0, 800, 0.5));
    v
}

impl Operator {
    pub fn path() -> PathBuf {
        util::data_dir().join("operator.json")
    }

    pub fn load(owner: String) -> Operator {
        let save = util::load_json::<ChatSave>(&Self::path()).unwrap_or_default();
        Operator {
            chat: save.chat,
            pending: Vec::new(),
            call: None,
            seq: 1,
            last_photo: None,
            incident: None,
            owner,
            dirty: false,
        }
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let start = self.chat.len().saturating_sub(MAX_CHAT);
        let save = ChatSave { chat: self.chat[start..].to_vec() };
        util::save_json(&Self::path(), &save);
        self.dirty = false;
    }

    fn say(&mut self, from: ChatFrom, text: impl Into<String>, photo: Option<PathBuf>) {
        self.chat.push(ChatMsg {
            from,
            text: text.into(),
            ts: util::now_ms(),
            photo: photo.map(|p| p.to_string_lossy().to_string()),
        });
        if self.chat.len() > MAX_CHAT * 2 {
            self.chat.drain(0..MAX_CHAT);
        }
        self.seq += 1;
        self.dirty = true;
    }

    pub fn clear(&mut self) {
        self.chat.clear();
        self.seq += 1;
        self.dirty = true;
    }

    pub fn call_view(&self) -> Option<CallView> {
        self.call.as_ref().map(|c| CallView {
            state: c.state,
            since_ms: c.since_ms,
            caller: "RoomGuard Central".to_string(),
            ask_passcode: c.state == CallState::Talking && c.stage == Stage::Passcode,
            ask_dispatch: c.state == CallState::Talking && c.stage == Stage::Dispatch,
        })
    }

    fn fresh_photo(&self, now: Instant) -> Option<PathBuf> {
        self.last_photo
            .as_ref()
            .filter(|(t, _)| now.saturating_duration_since(*t).as_secs() < 15)
            .map(|(_, p)| p.clone())
    }

    /// A new snapshot: attach it to the newest waiting report that wants one.
    pub fn photo(&mut self, path: PathBuf, now: Instant) {
        for p in self.pending.iter_mut().rev() {
            let wants = matches!(p.report.kind, ReportKind::Alarm(_) | ReportKind::Lockout | ReportKind::Stranger | ReportKind::EntryStarted);
            if wants && p.photo.is_none() {
                p.photo = Some(path.clone());
                break;
            }
        }
        self.last_photo = Some((now, path));
    }

    pub fn note_shutdown(&mut self, name: &str) {
        let text = format!(
            "Hi {}, your RoomGuard app was closed at {} while the system was armed.",
            name,
            util::fmt_clock(util::now_ms())
        );
        self.say(ChatFrom::Operator, text, None);
        self.save();
    }

    /// A report from the panel's communicator.
    pub fn report(&mut self, r: Report, now: Instant, dial: DialInfo, _cfg: &OperatorCfg) -> Vec<OpOut> {
        let silent = matches!(r.kind, ReportKind::EntryStarted | ReportKind::Stranger);
        let (due, line) = if silent {
            // Phone only extras. Wait a moment so the camera snapshot can come along.
            (now + Duration::from_millis(1800), Vec::new())
        } else {
            (now + Duration::from_secs(r.delay_s as u64), dial_sequence(&r, &dial))
        };
        let photo = if matches!(r.kind, ReportKind::Alarm(_) | ReportKind::Lockout) { self.fresh_photo(now) } else { None };
        self.pending.push(Pending { report: r, due, dialing_until: None, photo, line, silent });
        Vec::new()
    }

    pub fn tick(&mut self, now: Instant, cfg: &OperatorCfg, line_audio: bool) -> Vec<OpOut> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            let p = &mut self.pending[i];
            if p.dialing_until.is_none() && now >= p.due {
                let secs: f32 = p.line.iter().map(|s| s.ms as f32 / 1000.0).sum();
                let secs = if p.silent { 0.0 } else { secs.max(2.5) };
                p.dialing_until = Some(now + Duration::from_secs_f32(secs));
                if line_audio && !p.silent {
                    out.push(OpOut::Audio(AudioCmd::Line(p.line.clone())));
                }
            }
            let ready = p.dialing_until.map_or(false, |t| now >= t);
            if ready {
                let done = self.pending.remove(i);
                self.deliver(done, now, cfg, &mut out);
                continue;
            }
            i += 1;
        }
        let missed = matches!(&self.call, Some(c) if c.state == CallState::Ringing && now.saturating_duration_since(c.since).as_secs() >= 30);
        if missed {
            self.call = None;
            self.incident = None;
            out.push(OpOut::Audio(AudioCmd::Ring(false)));
            self.say(
                ChatFrom::Operator,
                "I couldn't reach you, so I'm treating this as a real alarm and dispatching police. (Simulated, nobody is actually sent.)",
                None,
            );
        }
        out
    }

    fn deliver(&mut self, p: Pending, now: Instant, cfg: &OperatorCfg, out: &mut Vec<OpOut>) {
        let r = p.report;
        let name = self.owner.clone();
        let op = cfg.operator_name.clone();
        let who = if matches!(r.kind, ReportKind::Opening | ReportKind::Closing) { "user" } else { "zone" };
        let target = if r.zone_or_user > 0 { format!(", {} {:03}", who, r.zone_or_user) } else { String::new() };
        let sig = format!("Signal received: {}{:03} {}{}", r.qualifier, r.cid, cid_name(r.cid), target);
        match r.kind {
            ReportKind::Alarm(kind) => {
                if kind == AlarmKind::Duress {
                    // Never shown on screen: whoever forced you to disarm could be watching.
                    return;
                }
                self.say(ChatFrom::System, sig, None);
                let where_ = zone_phrase(r.zone_or_user, &r.zone_label);
                if cfg.enabled {
                    let line = match kind {
                        AlarmKind::Fire => format!("Hi {}, this is {} at RoomGuard Central. We have a fire alarm from {}. I'm calling you right now.", name, op, where_),
                        AlarmKind::Medical => format!("Hi {}, {} here at RoomGuard Central. Your keypad sent a medical alarm. I'm calling to check on you.", name, op),
                        AlarmKind::Panic => format!("{}, this is {} at RoomGuard Central. We received a panic alarm from your keypad. I'm calling you now.", name, op),
                        AlarmKind::Tamper => format!("Hi {}, it's {} at RoomGuard Central. {} reported a tamper. Calling you to check.", name, op, cap(&where_)),
                        _ => format!(
                            "Hi {}, this is {} at RoomGuard Central. We just got a {} alarm from {}. I'm calling you now.",
                            name,
                            op,
                            kind.label().to_lowercase(),
                            where_
                        ),
                    };
                    self.say(ChatFrom::Operator, line, p.photo.clone());
                }
                self.incident = Some((kind, r.zone_or_user));
                if cfg.enabled && self.call.is_none() {
                    self.call = Some(Call {
                        state: CallState::Ringing,
                        since: now,
                        since_ms: util::now_ms(),
                        stage: Stage::Passcode,
                        tries: 0,
                        kind,
                        where_,
                    });
                    out.push(OpOut::Audio(AudioCmd::Ring(true)));
                }
            }
            ReportKind::Trouble => {
                self.say(ChatFrom::System, sig, None);
                if cfg.enabled && cfg.trouble_reports {
                    self.say(
                        ChatFrom::Operator,
                        format!("Hi {}, your panel just reported a trouble: {}. Nothing is in alarm, but you'll want to take a look.", name, r.text.to_lowercase()),
                        None,
                    );
                }
            }
            ReportKind::Restore => {
                self.say(ChatFrom::System, sig, None);
            }
            ReportKind::Opening | ReportKind::Closing => {
                if cfg.open_close_reports {
                    self.say(ChatFrom::System, format!("{}. {}", sig, r.text), None);
                }
            }
            ReportKind::PowerUp => {
                self.say(ChatFrom::System, sig, None);
                if cfg.enabled {
                    self.say(ChatFrom::Operator, format!("Hi {}, your system just came back online and it's still armed. Just letting you know.", name), None);
                }
            }
            ReportKind::Lockout => {
                self.say(ChatFrom::System, sig, None);
                if cfg.enabled {
                    self.say(
                        ChatFrom::Operator,
                        format!("{}, someone keyed in several wrong codes at your keypad and it locked itself. Here's what the camera saw.", name),
                        p.photo.clone(),
                    );
                }
            }
            ReportKind::EntryStarted | ReportKind::PhysicalEntry | ReportKind::PhysicalAlarm | ReportKind::Stranger => {}
            ReportKind::Shutdown | ReportKind::Cancel => {
                self.say(ChatFrom::System, sig, None);
            }
        }
    }

    /// The system was disarmed (or an alarm silenced) with a valid code.
    pub fn disarmed(&mut self, after_alarm: bool, _now: Instant, cfg: &OperatorCfg) -> Vec<OpOut> {
        let mut out = Vec::new();
        // Burglary alarms still inside the transmission delay are aborted,
        // exactly like the real panels.
        let before = self.pending.len();
        self.pending
            .retain(|p| !(p.dialing_until.is_none() && matches!(p.report.kind, ReportKind::Alarm(AlarmKind::Burglary) | ReportKind::EntryStarted)));
        if self.pending.len() < before && after_alarm {
            self.say(ChatFrom::System, "Alarm cancelled at the keypad before it was sent", None);
        }
        if !after_alarm || self.incident.is_none() {
            return out;
        }
        self.say(ChatFrom::System, "Signal received: R406 Cancel, alarm cancelled by a user code", None);
        let talking = self.call.as_ref().map_or(false, |c| c.state == CallState::Talking);
        if !talking {
            if self.call.take().is_some() {
                out.push(OpOut::Audio(AudioCmd::Ring(false)));
            }
            if cfg.enabled {
                let owner = self.owner.clone();
                self.say(
                    ChatFrom::Operator,
                    format!("I see the system was just disarmed with a valid code, {}. I'll close this one out. Message me if anything's wrong.", owner),
                    None,
                );
            }
            self.incident = None;
        }
        out
    }

    pub fn input(&mut self, inp: OpInput, now: Instant, cfg: &OperatorCfg) -> Vec<OpOut> {
        let mut out = Vec::new();
        match inp {
            OpInput::Answer => {
                let Some(c) = self.call.as_mut() else {
                    return out;
                };
                if c.state != CallState::Ringing {
                    return out;
                }
                c.state = CallState::Talking;
                c.since = now;
                c.since_ms = util::now_ms();
                let has_pass = cfg.has_passcode();
                if !has_pass {
                    c.stage = Stage::Dispatch;
                }
                let opening = format!(
                    "Hi {}, it's {} from RoomGuard Central. Your {} alarm went off at {}.",
                    self.owner,
                    cfg.operator_name,
                    c.kind.label().to_lowercase(),
                    c.where_
                );
                out.push(OpOut::Audio(AudioCmd::Ring(false)));
                if has_pass {
                    self.say(ChatFrom::Operator, format!("{} Can you give me your passcode, please?", opening), None);
                } else {
                    self.say(
                        ChatFrom::Operator,
                        format!("{} I don't have a passcode on file for you, so I'll take your word for it. Is everything okay, or should I send help?", opening),
                        None,
                    );
                }
            }
            OpInput::Decline => {
                if self.call.take().is_some() {
                    out.push(OpOut::Audio(AudioCmd::Ring(false)));
                    self.say(ChatFrom::Operator, "You declined the call, so I'm dispatching police. (Simulated, nobody is actually sent.)", None);
                    self.incident = None;
                }
            }
            OpInput::Passcode(word) => {
                let waiting = self.call.as_ref().map_or(false, |c| c.state == CallState::Talking && c.stage == Stage::Passcode);
                if !waiting {
                    return out;
                }
                self.say(ChatFrom::Me, "\u{2022}".repeat(word.chars().count().clamp(4, 12)), None);
                if cfg.passcode_ok(&word) {
                    if let Some(c) = self.call.as_mut() {
                        c.stage = Stage::Dispatch;
                    }
                    let owner = self.owner.clone();
                    self.say(ChatFrom::Operator, format!("Thanks {}, that matches. Do you need help, or was this a false alarm?", owner), None);
                } else {
                    let tries = self.call.as_mut().map(|c| {
                        c.tries += 1;
                        c.tries
                    });
                    if tries.unwrap_or(9) < 2 {
                        self.say(ChatFrom::Operator, "That doesn't match what I have on file. Do you want to try once more?", None);
                    } else {
                        self.call = None;
                        self.incident = None;
                        self.say(ChatFrom::Operator, "I'm sorry, that still doesn't match, so I have to dispatch police. (Simulated.) If this is a mistake, disarm at your keypad.", None);
                    }
                }
            }
            OpInput::Dispatch(yes) => {
                if self.call.is_none() {
                    return out;
                }
                self.say(ChatFrom::Me, if yes { "Please send help." } else { "It's a false alarm, I'm okay." }, None);
                if yes {
                    self.say(
                        ChatFrom::Operator,
                        "Okay, I'm sending police now. (Simulated, nobody is actually coming.) Stay somewhere safe and don't go looking for the intruder.",
                        None,
                    );
                } else {
                    self.say(ChatFrom::Operator, "No problem, I'll cancel it. Remember to disarm at the keypad if it's still sounding.", None);
                }
                self.call = None;
                self.incident = None;
            }
            OpInput::Clear => self.clear(),
        }
        out
    }
}
