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

struct WordClip {
    data: Vec<u8>,
    /// Per-word perceptual loudness equalization gain computed once at load
    /// so all 76 words (8 real + 68 cloned) speak at the exact same volume.
    gain: f32,
}

fn bank() -> &'static HashMap<String, WordClip> {
    static BANK: OnceLock<HashMap<String, WordClip>> = OnceLock::new();
    BANK.get_or_init(|| parse_rom(ROM_BYTES))
}

/// Computes perceptual speech-band loudness (A-weighted approximation at 8 kHz)
/// combining active-frame RMS and 85th-percentile stressed-syllable loudness.
fn perceived_loudness_ulaw(ulaw: &[u8]) -> f32 {
    if ulaw.is_empty() {
        return 0.0;
    }
    // 2nd-order Butterworth high-pass at 520 Hz (fs = 8000 Hz) + gentle 2.2 kHz
    // presence shelf to approximate IEC 61672 A-weighting over 200-3600 Hz.
    let mut hp_x1 = 0.0f32;
    let mut hp_x2 = 0.0f32;
    let mut hp_y1 = 0.0f32;
    let mut hp_y2 = 0.0f32;
    // Exact RBJ 2nd-order Butterworth HPF (fc=520 Hz, fs=8000 Hz, Q=0.7071)
    let (hb0, hb1, hb2, ha1, ha2) = (0.74864f32, -1.49728f32, 0.74864f32, -1.43307f32, 0.56149f32);
    // Exact RBJ Peaking EQ (fc=2300 Hz, fs=8000 Hz, Q=0.75, +4.0 dB ear-sensitivity peak)
    let (pb0, pb1, pb2, pa1, pa2) = (1.19880f32, 0.30819f32, 0.12140f32, 0.30819f32, 0.32020f32);
    let mut pk_x1 = 0.0f32;
    let mut pk_x2 = 0.0f32;
    let mut pk_y1 = 0.0f32;
    let mut pk_y2 = 0.0f32;

    let mut w = Vec::with_capacity(ulaw.len());
    for &u in ulaw {
        let x = ulaw_to_f32(u);
        let h = hb0 * x + hb1 * hp_x1 + hb2 * hp_x2 - ha1 * hp_y1 - ha2 * hp_y2;
        hp_x2 = hp_x1;
        hp_x1 = x;
        hp_y2 = hp_y1;
        hp_y1 = h;
        let y = pb0 * h + pb1 * pk_x1 + pb2 * pk_x2 - pa1 * pk_y1 - pa2 * pk_y2;
        pk_x2 = pk_x1;
        pk_x1 = h;
        pk_y2 = pk_y1;
        pk_y1 = y;
        w.push(y);
    }

    let win = 160usize; // 20 ms at 8 kHz
    let hop = 40usize;  // 5 ms
    let mut frames = Vec::new();
    if w.len() <= win {
        let rms = (w.iter().map(|v| v * v).sum::<f32>() / w.len().max(1) as f32).sqrt();
        frames.push(rms);
    } else {
        let mut i = 0;
        while i + win <= w.len() {
            let s: f32 = w[i..i + win].iter().map(|v| v * v).sum();
            frames.push((s / win as f32).sqrt());
            i += hop;
        }
    }
    let peak_f = frames.iter().copied().fold(0.0f32, f32::max);
    if peak_f < 1e-5 {
        return 0.0;
    }
    // High stressed-syllable gate (35% of peak, ~-9 dB) so quiet trailing
    // consonants (like the tail of "instant") never dilute stressed-syllable loudness.
    let mut active: Vec<f32> = frames.into_iter().filter(|&v| v > peak_f * 0.35).collect();
    if active.is_empty() {
        return peak_f;
    }
    let active_rms = (active.iter().map(|v| v * v).sum::<f32>() / active.len() as f32).sqrt();
    active.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p90 = active[(active.len() * 90 / 100).min(active.len() - 1)];
    0.30 * active_rms + 0.70 * p90
}

/// Transparent soft-peak protector: 100% linear below 0.85, smoothly saturates
/// above 0.85 so equalized words never hard-clip or distort.
#[inline]
fn soft_limit(v: f32) -> f32 {
    let a = v.abs();
    if a <= 0.85 {
        v
    } else {
        v.signum() * (0.85 + 0.13 * ((a - 0.85) / 0.13).tanh())
    }
}

fn parse_rom(b: &[u8]) -> HashMap<String, WordClip> {
    let mut raw_map = Vec::new();
    if b.len() < 12 || !b.starts_with(b"RGV61601") {
        return HashMap::new();
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
        raw_map.push((key, b[o..o + dlen].to_vec()));
        o += dlen;
    }
    // Target perceptual loudness: median across the entire 76-word bank lifted
    // slightly to match the clearest announcer words ("stay", "exit_now").
    let mut louds: Vec<f32> = raw_map
        .iter()
        .map(|(_, d)| perceived_loudness_ulaw(d))
        .filter(|&v| v > 1e-4)
        .collect();
    louds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let target = if louds.is_empty() {
        0.35
    } else {
        // Use 55th percentile so quieter words ("armed", "disarmed") lift smoothly
        // and sharp words ("instant", "system") tame down to match "stay"/"away".
        louds[(louds.len() * 55 / 100).min(louds.len() - 1)]
    };
    let mut map = HashMap::with_capacity(raw_map.len());
    for (key, data) in raw_map {
        let l = perceived_loudness_ulaw(&data);
        let gain = if l > 1e-4 { (target / l).clamp(0.55, 1.65) } else { 1.0 };
        map.insert(key, WordClip { data, gain });
    }
    map
}

#[cfg(test)]
pub fn audit_equalized_loudness() -> Vec<(String, f32, f32)> {
    let mut out = Vec::new();
    for (k, clip) in bank() {
        let raw = perceived_loudness_ulaw(&clip.data);
        out.push((k.clone(), clip.gain, raw * clip.gain));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
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
    cur_gain: f32,
    pos: f32,
    gap_left: u32,
    gap_samples: u32,
    /// 0 = muted, 1..=7 = volume steps (default 5).
    level: u8,
    /// Short 4 ms ramp to zero when speech is interrupted so there is never a click/pop.
    last_out: f32,
    fade_left: u32,
    fade_len: u32,
    fade_start: f32,
}

impl VoicePlayer {
    pub fn new(sr: f32) -> VoicePlayer {
        // Touch the ROM once so the map is ready before real-time audio.
        let _ = bank();
        let sr = sr.max(8000.0);
        VoicePlayer {
            sr,
            queue: VecDeque::new(),
            cur: None,
            cur_gain: 1.0,
            pos: 0.0,
            gap_left: 0,
            gap_samples: (sr * 0.075) as u32,
            level: 5,
            last_out: 0.0,
            fade_left: 0,
            fade_len: (sr * 0.004) as u32 + 1,
            fade_start: 0.0,
        }
    }

    pub fn set_level(&mut self, level: u8) {
        self.level = level.min(7);
    }

    fn begin_fade(&mut self) {
        if self.last_out.abs() > 1e-4 {
            self.fade_start = self.last_out;
            self.fade_left = self.fade_len;
        }
    }

    pub fn speak(&mut self, tokens: Vec<String>) {
        self.begin_fade();
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
        self.begin_fade();
        self.queue.clear();
        self.cur = None;
        self.pos = 0.0;
        self.gap_left = 0;
    }

    pub fn is_speaking(&self) -> bool {
        self.cur.is_some() || !self.queue.is_empty() || self.gap_left > 0
    }

    pub fn is_fading(&self) -> bool {
        self.fade_left > 0
    }

    pub fn is_active(&self) -> bool {
        self.is_speaking() || self.fade_left > 0
    }

    pub fn next(&mut self) -> f32 {
        if self.fade_left > 0 {
            self.fade_left -= 1;
            let t = self.fade_left as f32 / self.fade_len as f32;
            let y = self.fade_start * t;
            self.last_out = y;
            return y;
        }
        if self.level == 0 {
            self.stop();
            self.last_out = 0.0;
            return 0.0;
        }
        if self.gap_left > 0 {
            self.gap_left -= 1;
            self.last_out = 0.0;
            return 0.0;
        }
        if self.cur.is_none() {
            if let Some(tok) = self.queue.pop_front() {
                if let Some(clip) = bank().get(&tok) {
                    if !clip.data.is_empty() {
                        self.cur = Some(clip.data.as_slice());
                        self.cur_gain = clip.gain;
                        self.pos = 0.0;
                    }
                }
            }
        }
        let Some(data) = self.cur else {
            self.last_out = 0.0;
            return 0.0;
        };
        let i0 = self.pos as usize;
        if i0 >= data.len() {
            self.cur = None;
            if !self.queue.is_empty() {
                self.gap_left = self.gap_samples;
            }
            self.last_out = 0.0;
            return 0.0;
        }
        let i1 = (i0 + 1).min(data.len() - 1);
        let frac = self.pos - i0 as f32;
        let s0 = ulaw_to_f32(data[i0]);
        let s1 = ulaw_to_f32(data[i1]);
        self.pos += ROM_RATE / self.sr;
        let gain = 0.28 + 0.72 * (self.level as f32 / 7.0);
        let eq = soft_limit((s0 + (s1 - s0) * frac) * self.cur_gain);
        let y = eq * gain * 0.92;
        self.last_out = y;
        y
    }
}
