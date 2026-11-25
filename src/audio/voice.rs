//! Concatenative voice synthesizer for the VistaGuard VG6160V keypad.
//!
//! Real 6160V/6150V keypads do not run neural TTS: they play individual
//! low-bitrate words stored in an onboard voice ROM, stitched together with a
//! signature ~105 ms gap through a 1-inch bandpass-limited plastic speaker.
//! Any word not present in the hardware ROM (e.g. "PANEL" or "TAMPER") is
//! silently skipped, matching the real unit's exact behavior.

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

static ROM_BYTES: &[u8] = include_bytes!("../../assets/voice/vista6160v.bin");
const ROM_RATE: f32 = 8000.0;

/// Official 6160V/6150V zone descriptor words supported by the voice ROM.
const ZONE_VOCAB: &[&str] = &[
    "alarm", "attic", "baby", "back", "basement", "bathroom", "bedroom", "closed", "den",
    "detector", "dining", "door", "emergency", "exit", "fire", "floor", "front", "garage",
    "hall", "inside", "kitchen", "laundry", "library", "living", "main", "master", "medical",
    "motion", "office", "open", "panic", "patio", "police", "pool", "room", "shop", "side",
    "sliding", "smoke", "storage", "supervised", "upstairs", "utility", "window", "zone",
    "1st", "2nd", "3rd",
];

fn bank() -> &'static HashMap<String, Vec<u8>> {
    static BANK: OnceLock<HashMap<String, Vec<u8>>> = OnceLock::new();
    BANK.get_or_init(|| parse_rom(ROM_BYTES))
}

fn parse_rom(b: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut map = HashMap::new();
    if b.len() < 12 || !b.starts_with(b"RGV61601") {
        return map;
    }
    let count = u32::from_le_bytes([b[8], b[9], b[10], b[11]]) as usize;
    let mut o = 12usize;
    for _ in 0..count {
        if o >= b.len() {
            break;
        }
        let klen = b[o] as usize;
        o += 1;
        if o + klen + 4 > b.len() {
            break;
        }
        let key = String::from_utf8_lossy(&b[o..o + klen]).to_string();
        o += klen;
        let dlen = u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
        o += 4;
        if o + dlen > b.len() {
            break;
        }
        map.insert(key, b[o..o + dlen].to_vec());
        o += dlen;
    }
    map
}

/// Standard G.711 u-law byte -> f32 (-1.0..1.0).
#[inline]
fn ulaw_to_f32(u: u8) -> f32 {
    let u = !u;
    let sign = u & 0x80;
    let exp = (u >> 4) & 0x07;
    let mant = u & 0x0F;
    let mut t = ((mant as i32) << 3) + 0x84;
    t <<= exp;
    let s = if sign != 0 { 0x84 - t } else { t - 0x84 };
    (s as f32) / 32768.0
}

/// Filters a zone label against the authentic 6160V voice ROM whitelist.
/// Words not in the ROM (such as "PANEL" or "TAMPER") are skipped.
pub fn filter_zone_words(label: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in label.split(|c: char| !c.is_ascii_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let low = raw.to_ascii_lowercase();
        let word = match low.as_str() {
            "doorway" => "door",
            "first" => "1st",
            "second" => "2nd",
            "third" => "3rd",
            other => other,
        };
        if ZONE_VOCAB.contains(&word) {
            out.push(word.to_string());
            if out.len() >= 3 {
                break;
            }
        }
    }
    out
}

/// Spoken digit tokens for a zone number (e.g. 95 -> ["9", "5"]).
pub fn number_tokens(n: u8) -> Vec<String> {
    if n < 10 {
        vec![n.to_string()]
    } else {
        vec![(n / 10).to_string(), (n % 10).to_string()]
    }
}

pub struct VoicePlayer {
    sr: f32,
    queue: VecDeque<String>,
    cur: Option<&'static [u8]>,
    pos: f32,
    gap_left: u32,
    gap_samples: u32,
    /// 0 = muted, 1..=7 = volume steps (default 5).
    level: u8,
}

impl VoicePlayer {
    pub fn new(sr: f32) -> VoicePlayer {
        // Touch the ROM once so the map is ready before real-time audio.
        let _ = bank();
        VoicePlayer {
            sr: sr.max(8000.0),
            queue: VecDeque::new(),
            cur: None,
            pos: 0.0,
            gap_left: 0,
            gap_samples: (sr.max(8000.0) * 0.130) as u32,
            level: 5,
        }
    }

    pub fn set_level(&mut self, level: u8) {
        self.level = level.min(7);
    }

    pub fn speak(&mut self, tokens: Vec<String>) {
        self.queue.clear();
        self.cur = None;
        self.pos = 0.0;
        self.gap_left = 0;
        if self.level == 0 {
            return;
        }
        let b = bank();
        for t in tokens {
            let k = t.to_ascii_lowercase();
            if b.contains_key(&k) {
                self.queue.push_back(k);
            }
        }
    }

    pub fn stop(&mut self) {
        self.queue.clear();
        self.cur = None;
        self.pos = 0.0;
        self.gap_left = 0;
    }

    pub fn is_speaking(&self) -> bool {
        self.cur.is_some() || !self.queue.is_empty() || self.gap_left > 0
    }

    pub fn next(&mut self) -> f32 {
        if self.level == 0 {
            self.stop();
            return 0.0;
        }
        if self.gap_left > 0 {
            self.gap_left -= 1;
            return 0.0;
        }
        if self.cur.is_none() {
            if let Some(tok) = self.queue.pop_front() {
                if let Some(data) = bank().get(&tok) {
                    if !data.is_empty() {
                        self.cur = Some(data.as_slice());
                        self.pos = 0.0;
                    }
                }
            }
        }
        let Some(data) = self.cur else {
            return 0.0;
        };
        let i0 = self.pos as usize;
        if i0 >= data.len() {
            self.cur = None;
            if !self.queue.is_empty() {
                self.gap_left = self.gap_samples;
            }
            return 0.0;
        }
        let i1 = (i0 + 1).min(data.len() - 1);
        let frac = self.pos - i0 as f32;
        let s0 = ulaw_to_f32(data[i0]);
        let s1 = ulaw_to_f32(data[i1]);
        self.pos += ROM_RATE / self.sr;
        let gain = 0.28 + 0.72 * (self.level as f32 / 7.0);
        (s0 + (s1 - s0) * frac) * gain * 0.92
    }
}
