//! Types shared by the panel engine and the interface.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Model {
    Pc1550,
    #[default]
    Pc1832,
    Vista20p,
}

impl Model {
    pub const fn zone_count(self) -> usize {
        match self {
            Model::Pc1550 => 6,
            Model::Pc1832 => 32,
            Model::Vista20p => 48,
        }
    }
    pub fn panel_name(self) -> &'static str {
        match self {
            Model::Pc1550 => "PC1550",
            Model::Pc1832 => "PC1832",
            Model::Vista20p => "Vista 20P",
        }
    }
    pub fn keypad_name(self) -> &'static str {
        match self {
            Model::Pc1550 => "PC1500RK",
            Model::Pc1832 => "PK5500",
            Model::Vista20p => "VG6160V",
        }
    }
    pub fn era(self) -> &'static str {
        match self {
            Model::Pc1550 => "RmG Classic LED keypad, 1990s",
            Model::Pc1832 => "GuardSeries LCD keypad, 2000s",
            Model::Vista20p => "VistaGuard Voice LCD keypad, 2000s",
        }
    }
    pub fn file_key(self) -> &'static str {
        match self {
            Model::Pc1550 => "pc1550",
            Model::Pc1832 => "pc1832",
            Model::Vista20p => "vista20p",
        }
    }
    /// How long the emergency and function keys must be held, in ms.
    pub fn hold_ms(self) -> u128 {
        match self {
            Model::Pc1550 => 1000,
            Model::Pc1832 | Model::Vista20p => 2000,
        }
    }
    pub fn installer_default(self) -> &'static str {
        match self {
            Model::Pc1550 => "1500",
            Model::Pc1832 => "5555",
            Model::Vista20p => "4112",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SirenModel {
    #[default]
    Wave2F,
    MasterBlaster,
    Rx7c,
    MotorBell,
}

impl SirenModel {
    pub const ALL: [SirenModel; 4] = [SirenModel::Wave2F, SirenModel::MasterBlaster, SirenModel::Rx7c, SirenModel::MotorBell];
    pub fn label(self) -> &'static str {
        match self {
            SirenModel::Wave2F => "WAVE-2F Horn Siren",
            SirenModel::MasterBlaster => "Master Blaster Motor Siren",
            SirenModel::Rx7c => "RX-7C Indoor Piezo",
            SirenModel::MotorBell => "10 inch motor bell",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            SirenModel::Wave2F => "Flush mount re-entrant horn siren, 106 dB (steady or warble/yelp)",
            SirenModel::MasterBlaster => "120 dB cast-metal motor siren, 1640 Hz, spins up and coasts down",
            SirenModel::Rx7c => "Piercing indoor ceramic piezo warble sounder, 90-106 dB",
            SirenModel::MotorBell => "10-inch steel gong motor bell with metallic striker clatter",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Digit(u8),
    Star,
    Hash,
    Fire,
    Aux,
    Panic,
    Left,
    Right,
    Stay,
    Away,
    Chime,
    Reset,
    Exit,
}

impl Key {
    pub fn is_emergency(self) -> bool {
        matches!(self, Key::Fire | Key::Aux | Key::Panic)
    }
    pub fn is_function(self) -> bool {
        matches!(self, Key::Stay | Key::Away | Key::Chime | Key::Reset | Key::Exit)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArmMode {
    Away,
    Stay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Led {
    Off,
    On,
    Flash,
    Fast,
}

impl Led {
    pub fn lit(self, t: f64) -> bool {
        match self {
            Led::Off => false,
            Led::On => true,
            Led::Flash => t.rem_euclid(1.0) < 0.5,
            Led::Fast => t.rem_euclid(0.4) < 0.2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H24 {
    Burglary,
    BellBuzzer,
    Buzzer,
    Panic,
    Medical,
    Holdup,
    Emergency,
    Gas,
    Heat,
    Water,
    Freeze,
    Sprinkler,
    Supervisory,
    SupervisoryBuzzer,
    FireSupervisory,
    Tamper,
    NonLatchTamper,
    CarbonMonoxide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneKind {
    Null,
    Delay1,
    Delay2,
    Instant,
    Interior,
    StayAway,
    DelayStayAway,
    InteriorDelay,
    InstantStayAway,
    NightZone,
    DayZone,
    NonAlarm,
    Fire,
    FireDelayed,
    H24(H24),
}

impl ZoneKind {
    pub fn is_delay(self) -> bool {
        matches!(self, ZoneKind::Delay1 | ZoneKind::Delay2 | ZoneKind::DelayStayAway)
    }
    pub fn is_fire(self) -> bool {
        matches!(self, ZoneKind::Fire | ZoneKind::FireDelayed)
    }
    pub fn is_24h(self) -> bool {
        matches!(self, ZoneKind::Fire | ZoneKind::FireDelayed | ZoneKind::H24(_))
    }
    pub fn is_stay_away(self) -> bool {
        matches!(self, ZoneKind::StayAway | ZoneKind::DelayStayAway | ZoneKind::InstantStayAway | ZoneKind::NightZone)
    }
    pub fn short(self) -> &'static str {
        match self {
            ZoneKind::Null => "Not used",
            ZoneKind::Delay1 => "Delay",
            ZoneKind::Delay2 => "Delay 2",
            ZoneKind::Instant => "Instant",
            ZoneKind::Interior => "Interior",
            ZoneKind::StayAway => "Stay/Away",
            ZoneKind::DelayStayAway => "Delay Stay/Away",
            ZoneKind::InteriorDelay => "Interior/Delay",
            ZoneKind::InstantStayAway => "Instant Stay/Away",
            ZoneKind::NightZone => "Night zone",
            ZoneKind::DayZone => "Day zone",
            ZoneKind::NonAlarm => "24 hr non-alarm",
            ZoneKind::Fire => "Fire",
            ZoneKind::FireDelayed => "Delayed fire",
            ZoneKind::H24(h) => match h {
                H24::Burglary => "24 hr burglary",
                H24::BellBuzzer => "24 hr bell/buzzer",
                H24::Buzzer => "24 hr buzzer",
                H24::Panic => "24 hr panic",
                H24::Medical => "24 hr medical",
                H24::Holdup => "24 hr holdup",
                H24::Emergency => "24 hr emergency",
                H24::Gas => "24 hr gas",
                H24::Heat => "24 hr heat",
                H24::Water => "24 hr water",
                H24::Freeze => "24 hr freeze",
                H24::Sprinkler => "24 hr sprinkler",
                H24::Supervisory => "24 hr supervisory",
                H24::SupervisoryBuzzer => "24 hr supv buzzer",
                H24::FireSupervisory => "24 hr fire supv",
                H24::Tamper => "24 hr tamper",
                H24::NonLatchTamper => "24 hr tamper (nl)",
                H24::CarbonMonoxide => "24 hr CO alarm",
            },
        }
    }
    pub fn alarm_kind(self) -> AlarmKind {
        match self {
            ZoneKind::Fire | ZoneKind::FireDelayed => AlarmKind::Fire,
            ZoneKind::H24(h) => match h {
                H24::Panic => AlarmKind::Panic,
                H24::Medical => AlarmKind::Medical,
                H24::Holdup => AlarmKind::Holdup,
                H24::Emergency => AlarmKind::Emergency,
                H24::Gas => AlarmKind::Gas,
                H24::Heat => AlarmKind::Heat,
                H24::Water | H24::Sprinkler => AlarmKind::Water,
                H24::Freeze => AlarmKind::Freeze,
                H24::Supervisory | H24::SupervisoryBuzzer | H24::FireSupervisory => AlarmKind::Supervisory,
                H24::Tamper | H24::NonLatchTamper => AlarmKind::Tamper,
                H24::CarbonMonoxide => AlarmKind::CarbonMonoxide,
                H24::Burglary | H24::BellBuzzer | H24::Buzzer => AlarmKind::Burglary,
            },
            _ => AlarmKind::Burglary,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmKind {
    Burglary,
    Fire,
    CarbonMonoxide,
    Panic,
    Medical,
    Duress,
    Tamper,
    Holdup,
    Emergency,
    Gas,
    Heat,
    Water,
    Freeze,
    Supervisory,
}

impl AlarmKind {
    pub fn label(self) -> &'static str {
        match self {
            AlarmKind::Burglary => "Burglary",
            AlarmKind::Fire => "Fire",
            AlarmKind::CarbonMonoxide => "Carbon Monoxide",
            AlarmKind::Panic => "Panic",
            AlarmKind::Medical => "Medical",
            AlarmKind::Duress => "Duress",
            AlarmKind::Tamper => "Tamper",
            AlarmKind::Holdup => "Holdup",
            AlarmKind::Emergency => "Emergency",
            AlarmKind::Gas => "Gas",
            AlarmKind::Heat => "Heat",
            AlarmKind::Water => "Water",
            AlarmKind::Freeze => "Freeze",
            AlarmKind::Supervisory => "Supervisory",
        }
    }
    /// Contact ID event code.
    pub fn cid(self) -> u16 {
        match self {
            AlarmKind::Burglary => 130,
            AlarmKind::Fire => 110,
            AlarmKind::CarbonMonoxide => 162,
            AlarmKind::Panic => 120,
            AlarmKind::Medical => 100,
            AlarmKind::Duress => 121,
            AlarmKind::Tamper => 144,
            AlarmKind::Holdup => 122,
            AlarmKind::Emergency => 101,
            AlarmKind::Gas => 151,
            AlarmKind::Heat => 114,
            AlarmKind::Water => 154,
            AlarmKind::Freeze => 159,
            AlarmKind::Supervisory => 200,
        }
    }
    /// Fire style alarms pulse the bell; CO uses Temporal 4; everything else sounds steady.
    pub fn pulsed(self) -> bool {
        matches!(self, AlarmKind::Fire | AlarmKind::Heat)
    }
    pub fn is_co(self) -> bool {
        matches!(self, AlarmKind::CarbonMonoxide)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Ready,
    NotReady,
    Exit,
    Armed,
    Entry,
    Alarm,
    Program,
    Lockout,
}

#[derive(Clone, Debug)]
pub struct ZoneView {
    pub num: u8,
    pub label: String,
    pub kind: ZoneKind,
    pub open: bool,
    pub bypassed: bool,
    /// True when the panel automatically deactivated this interior zone for Stay mode
    /// (not manually bypassed by the user).
    pub auto_bypass: bool,
    pub alarm: bool,
    pub tamper: bool,
    pub fault: bool,
    pub camera: bool,
    /// Why a camera zone is open, when it's the camera itself: blocked or offline.
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvKind {
    Alarm,
    Arm,
    Disarm,
    Trouble,
    Restore,
    Program,
    Info,
    Camera,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventRow {
    pub ts: i64,
    pub kind: EvKind,
    pub text: String,
    /// Contact ID event code (e.g. 401, 130, 625) for the VISTA-20P #60 event log.
    #[serde(default)]
    pub cid: u16,
    /// Contact ID qualifier ('E' = new event / opening, 'R' = restore / closing).
    #[serde(default)]
    pub qual: char,
    /// Zone or user number associated with the event (0 for system events).
    #[serde(default)]
    pub zu: u16,
    /// True if `zu` is a user number ('U'), false if a zone ('Z').
    #[serde(default)]
    pub is_user: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatFrom {
    Operator,
    System,
    Me,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMsg {
    pub from: ChatFrom,
    pub text: String,
    pub ts: i64,
    #[serde(default)]
    pub photo: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallState {
    Ringing,
    Talking,
}

#[derive(Clone, Debug)]
pub struct CallView {
    pub state: CallState,
    pub since_ms: i64,
    pub caller: String,
    pub ask_passcode: bool,
    pub ask_dispatch: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Alarm(AlarmKind),
    Cancel,
    Opening,
    Closing,
    Trouble,
    Restore,
    Shutdown,
    PowerUp,
    Lockout,
    EntryStarted,
    Stranger,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub id: u64,
    pub kind: ReportKind,
    pub cid: u16,
    pub qualifier: char,
    pub zone_or_user: u16,
    pub zone_label: String,
    pub text: String,
    pub delay_s: u32,
    pub ts: i64,
}
