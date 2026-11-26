//! The synthesizer. Each device is modelled from how it physically makes
//! sound:
//!
//! Keypad sounder: a ceramic piezoelectric disc is a high-Q mechanical
//! resonator driven by a microcontroller square wave. The classic PC1500RK
//! disc is lower, slightly broader and raspier; the GuardSeries PK5500 disc
//! is higher and cleaner, with 21-step pitch/level control (`00` off,
//! `01`..`20` sweeping ~2.05 to 3.65 kHz).
//!
//! Ademco WAVE-2F: a 106 dB self-contained 12 V electronic siren driving a
//! re-entrant exponential plastic horn speaker. Terminal 1 (Steady) emits a
//! continuous piercing horn tone (~2.36 kHz with rich horn harmonics and
//! slight power-rail ripple); Terminal 2 (Warble) emits the iconic rapid
//! asymmetric burglar yelp sweeping ~680 to 1,920 Hz at ~3.2 Hz.
//!
//! Klaxon Master Blaster: a 120 dB motor-driven cast-metal siren whose
//! slotted impeller chops air at ~1,640 Hz at full speed. Spins up under
//! motor torque with brushed-motor whine and 60/120 Hz mains growl, then
//! coasts down over ~4.8 seconds with aerodynamic drag and turbulent rush.
//!
//! ADT RX-7C: a piercing self-contained indoor ceramic piezo sounder that
//! rapidly sweeps and chirps through its ~3.28 kHz piezo resonance inside a
//! vented plastic housing.
//!
//! 10-inch Motor Bell: an eccentric cam drives a steel striker against a
//! 10-inch steel gong ~16.5 times per second. Each blow injects a sharp
//! steel-on-steel impact transient that excites 10 inharmonic dome modes
//! (including natural bolt-asymmetry beating pairs) which shimmer and decay.

use std::collections::VecDeque;

use super::voice::VoicePlayer;
use super::{AudioCmd, BellPattern, ChimeKind, KeypadVoice, Pattern, ToneSeg};
use crate::engine::types::SirenModel;

const TAU: f32 = std::f32::consts::TAU;
const PI: f32 = std::f32::consts::PI;

/// Flushes inaudibly small values to zero. A resonator ringing down after a
/// beep shrinks toward zero forever, and once its numbers get tiny enough to
/// be "denormal" floats, the CPU does that maths dozens of times slower. In an
/// idle synth that's every filter at once, which can starve the audio callback
/// and turn into crackles or beeps that arrive late.
#[inline]
fn ftz(x: f32) -> f32 {
    if x.abs() < 1.0e-20 {
        0.0
    } else {
        x
    }
}

fn ms(sr: f32, ms: u32) -> u32 {
    (sr * ms as f32 / 1000.0) as u32
}

/// Two pole resonator.
#[derive(Clone, Copy)]
struct Res {
    a1: f32,
    a2: f32,
    b: f32,
    sw: f32,
    y1: f32,
    y2: f32,
}

impl Res {
    fn new(sr: f32, f: f32, q: f32) -> Res {
        let mut r = Res { a1: 0.0, a2: 0.0, b: 0.0, sw: 0.0, y1: 0.0, y2: 0.0 };
        r.tune(sr, f, q);
        r
    }
    fn tune(&mut self, sr: f32, f: f32, q: f32) {
        let f = f.clamp(20.0, sr * 0.45);
        let w = TAU * f / sr;
        let r = (-PI * f / (q.max(0.3) * sr)).exp();
        self.a1 = 2.0 * r * w.cos();
        self.a2 = -r * r;
        self.sw = w.sin().max(1e-4);
        self.b = (1.0 - r) * 2.0 * self.sw;
    }
    fn tick(&mut self, x: f32) -> f32 {
        let y = ftz(self.a1 * self.y1 + self.a2 * self.y2 + self.b * x);
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
    /// Kicks the resonator so it rings with roughly this amplitude.
    fn strike(&mut self, amp: f32) {
        self.y1 += amp * self.sw;
    }
    fn energy(&self) -> f32 {
        self.y1.abs() + self.y2.abs()
    }
}

#[derive(Clone, Copy, Default)]
struct OnePole {
    a: f32,
    z: f32,
}

impl OnePole {
    fn new(sr: f32, f: f32) -> OnePole {
        OnePole { a: 1.0 - (-TAU * f.clamp(10.0, sr * 0.45) / sr).exp(), z: 0.0 }
    }
    fn lowpass(&mut self, x: f32) -> f32 {
        self.z = ftz(self.z + self.a * (x - self.z));
        self.z
    }
    fn highpass(&mut self, x: f32) -> f32 {
        x - self.lowpass(x)
    }
}

/// Tiny indoor early-reflection network so sirens sound mounted on a wall in a
/// real room rather than dry synthetic oscillators.
struct Room {
    buf: Vec<f32>,
    idx: usize,
    t1: usize,
    t2: usize,
    t3: usize,
    damp: OnePole,
}

impl Room {
    fn new(sr: f32) -> Room {
        let len = (sr * 0.028).max(32.0) as usize;
        let t1 = ((sr * 0.0067) as usize).clamp(1, len - 1);
        let t2 = ((sr * 0.0139) as usize).clamp(1, len - 1);
        let t3 = ((sr * 0.0224) as usize).clamp(1, len - 1);
        Room { buf: vec![0.0; len], idx: 0, t1, t2, t3, damp: OnePole::new(sr, 4200.0) }
    }
    fn tick(&mut self, x: f32) -> f32 {
        let len = self.buf.len();
        let r1 = self.buf[(self.idx + len - self.t1) % len];
        let r2 = self.buf[(self.idx + len - self.t2) % len];
        let r3 = self.buf[(self.idx + len - self.t3) % len];
        let wet = self.damp.lowpass(r1 * 0.22 + r2 * 0.14 - r3 * 0.09);
        self.buf[self.idx] = ftz(x + wet * 0.18);
        self.idx = (self.idx + 1) % len;
        x * 0.84 + wet * 0.45
    }
    fn energy(&self) -> f32 {
        let len = self.buf.len();
        self.buf[(self.idx + len - self.t1) % len].abs()
            + self.buf[(self.idx + len - self.t3) % len].abs()
    }
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// Band-limited pulse wave with arbitrary duty cycle (0.5 = square).
fn pulse(phase: f32, dt: f32, duty: f32) -> f32 {
    let mut v = if phase < duty { 1.0 } else { -1.0 };
    v += blep(phase, dt);
    let mut p2 = phase - duty;
    if p2 < 0.0 {
        p2 += 1.0;
    }
    v - blep(p2, dt)
}

struct Gate {
    pat: Pattern,
    pos: u32,
}

impl Gate {
    fn set(&mut self, p: Pattern) {
        if p != self.pat {
            self.pat = p;
            self.pos = 0;
        }
    }
    fn next(&mut self, sr: f32) -> (bool, Option<f32>) {
        match self.pat {
            Pattern::Off => (false, None),
            Pattern::Steady => (true, None),
            Pattern::Beeps { on_ms, off_ms, count, period_ms } => {
                let on = ms(sr, on_ms as u32).max(1);
                let unit = on + ms(sr, off_ms as u32);
                let period = ms(sr, period_ms).max(unit * count.max(1) as u32).max(1);
                let p = self.pos;
                self.pos = (self.pos + 1) % period;
                ((p / unit) < count as u32 && (p % unit) < on, None)
            }
            Pattern::ShortShortLong { short_ms, gap_ms, long_ms } => {
                let s = ms(sr, short_ms as u32).max(1);
                let g = ms(sr, gap_ms as u32);
                let l = ms(sr, long_ms as u32).max(1);
                let period = (2 * (s + g) + l + g).max(1);
                let p = self.pos;
                self.pos = (self.pos + 1) % period;
                let on = p < s || (p >= s + g && p < 2 * s + g) || (p >= 2 * (s + g) && p < 2 * (s + g) + l);
                (on, None)
            }
            Pattern::HiLo { hi_hz, lo_hz, step_ms } => {
                let step = ms(sr, step_ms as u32).max(1);
                let period = step * 2;
                let p = self.pos;
                self.pos = (self.pos + 1) % period;
                let f = if p < step { hi_hz as f32 } else { lo_hz as f32 };
                (true, Some(f))
            }
        }
    }
}

/// The keypad piezo sounder.
struct Piezo {
    voice: KeypadVoice,
    res: Res,
    res2: Res,
    body: Res,
    phase: f32,
    f0: f32,
    buzz: f32,
    sat: f32,
    second: f32,
    level: u8,
    sr: f32,
    /// The pitch of a plain beep. On the DSC keypads it's the disc's own
    /// resonance; the Vista 6160 beeps at 1360 Hz.
    tone: f32,
    /// The 6160 note envelope, and harmonic gains cached for one pitch.
    env: f32,
    gains: (f32, [f32; 4]),
}

impl Piezo {
    fn new(sr: f32, voice: KeypadVoice, level: u8) -> Piezo {
        let mut p = Piezo {
            voice,
            res: Res::new(sr, 3000.0, 12.0),
            res2: Res::new(sr, 3060.0, 9.0),
            body: Res::new(sr, 1420.0, 2.2),
            tone: 0.0,
            env: 0.0,
            gains: (0.0, [0.0; 4]),
            phase: 0.0,
            f0: 3000.0,
            buzz: 0.15,
            sat: 1.6,
            second: 0.0,
            level: level.min(20),
            sr,
        };
        p.configure();
        p
    }

    fn set_level(&mut self, level: u8) {
        let clamped = level.min(20);
        if clamped != self.level {
            self.level = clamped;
            if self.voice == KeypadVoice::Pk5500 {
                self.configure();
            }
        }
    }

    fn configure(&mut self) {
        let (f0, q, buzz, sat, second) = match self.voice {
            KeypadVoice::Classic => (2780.0, 8.5, 0.30, 2.2, 0.36),
            KeypadVoice::Pk5500 => {
                // Buzzer Control "adjusts the keypad buzzer tone": 00 is off and
                // 01 to 20 step the pitch. A recording of a real PowerSeries
                // keypad at the factory level (12) sounds at 2295 Hz, rising and
                // falling in about 7 ms (a disc with a Q near 27).
                let f = 2295.0 * 2f32.powf((self.level.max(1) as f32 - 12.0) * 0.0438);
                (f, 27.0, 0.11, 1.45, 0.06)
            }
            KeypadVoice::Vista6160 => {
                // Unused by the 6160's own voice (see next_6160), which was
                // measured from a recording of a real keypad instead.
                (1360.0, 2.0, 0.0, 1.1, 0.0)
            }
        };
        self.f0 = f0;
        self.tone = f0;
        self.buzz = buzz;
        self.sat = sat;
        self.second = second;
        self.res.tune(self.sr, f0, q);
        self.res2.tune(self.sr, f0 * 1.023, q * 0.78);
    }

    fn next(&mut self, on: bool, freq: f32, amp: f32) -> f32 {
        if self.voice == KeypadVoice::Vista6160 {
            return self.next_6160(on, freq, amp);
        }
        let muted = self.voice == KeypadVoice::Pk5500 && self.level == 0;
        let drive = if on && !muted && amp > 0.0 {
            let dt = freq / self.sr;
            self.phase += dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            // Slight gain scaling with PK5500 buzzer level (still audible at 01, crisp at 20).
            let lv = if self.voice == KeypadVoice::Pk5500 {
                0.42 + 0.58 * (self.level as f32 / 20.0)
            } else {
                1.0
            };
            pulse(self.phase, dt, 0.48) * amp * lv
        } else {
            0.0
        };
        let y = self.res.tick(drive) + self.second * self.res2.tick(drive) + 0.14 * self.body.tick(drive);
        // Tones away from the disc's resonance (e.g. Ding-Dong notes) carry cleanly.
        let off = ((freq - self.f0).abs() / self.f0.max(100.0) * 2.2).min(0.85);
        let v = y * (1.0 - self.buzz) + drive * (self.buzz + off) * 0.36;
        (v * self.sat).tanh() / self.sat.tanh()
    }

    /// The 6160's sounder, rebuilt from a recording of a real keypad: a square
    /// wave heard through its small speaker. At 1360 Hz the 3rd harmonic sits
    /// 31 dB down and the 5th 44 dB down; at 1019 Hz the speaker all but
    /// swallows the 3rd (-50 dB) and lets the 5th through (-41 dB). Notes rise
    /// and fall in about 10 ms.
    fn next_6160(&mut self, on: bool, freq: f32, amp: f32) -> f32 {
        let target = if on && amp > 0.0 { amp } else { 0.0 };
        let k = 1.0 - (-1.0 / (0.0045 * self.sr)).exp();
        self.env += (target - self.env) * k;
        if target == 0.0 && self.env < 1.0e-5 {
            self.env = 0.0;
            return 0.0;
        }
        if (self.gains.0 - freq).abs() > 0.5 {
            self.gains = (freq, speaker_6160(freq));
        }
        self.phase += freq / self.sr;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let g = self.gains.1;
        let p = TAU * self.phase;
        let v = g[0] * p.sin() + g[1] * (2.0 * p).sin() + g[2] * (3.0 * p).sin() + g[3] * (5.0 * p).sin();
        v * self.env * 0.9
    }

    fn quiet(&self) -> bool {
        if self.voice == KeypadVoice::Vista6160 {
            return self.env < 1e-5;
        }
        self.res.energy() < 1e-5 && self.res2.energy() < 1e-5 && self.body.energy() < 1e-5
    }
}

/// The 6160 speaker's response in dB, read off the recording: about flat
/// across the two notes, a deep dip near 3 kHz and a lift around 4 to 5 kHz.
fn speaker_6160_db(f: f32) -> f32 {
    const PTS: [(f32, f32); 8] = [
        (400.0, -6.0),
        (1019.0, -1.5),
        (1360.0, 0.0),
        (2200.0, -20.0),
        (3057.0, -40.5),
        (4080.0, -21.5),
        (5095.0, -27.0),
        (6800.0, -30.0),
    ];
    if f <= PTS[0].0 {
        return PTS[0].1;
    }
    for w in PTS.windows(2) {
        let (a, b) = (w[0], w[1]);
        if f <= b.0 {
            let t = (f.ln() - a.0.ln()) / (b.0.ln() - a.0.ln());
            return a.1 + t * (b.1 - a.1);
        }
    }
    -30.0 - (f / 6800.0).log2() * 12.0
}

/// Gains of the fundamental, 2nd, 3rd and 5th harmonics for a 6160 note.
fn speaker_6160(f: f32) -> [f32; 4] {
    let lin = |x: f32| 10f32.powf(speaker_6160_db(x) / 20.0);
    let g1 = lin(f);
    [g1, g1 * 0.004, lin(3.0 * f) / 3.0, lin(5.0 * f) / 5.0]
}

/// One WAVE2 warble sweep, measured from a recording: 119 ms.
const WAVE_SWEEP_S: f32 = 0.119;

/// A tone built from its measured harmonics: gains of the 1st to 6th.
fn additive(phase: f32, g: &[f32; 6]) -> f32 {
    let p = TAU * phase;
    let mut v = 0.0;
    for (i, &gi) in g.iter().enumerate() {
        if gi != 0.0 {
            v += gi * ((i as f32 + 1.0) * p).sin();
        }
    }
    v
}

/// All four bell output devices.
struct Siren {
    model: SirenModel,
    warble: bool,
    sr: f32,
    phase: f32,
    sweep: f32,
    amp: f32,
    hp: OnePole,
    lp: OnePole,
    // Master Blaster motor & turbine
    w: f32,
    theta: f32,
    shaft: f32,
    air: Res,
    hum: f32,
    count: u32,
    // RX-7C ceramic piezo
    rx_t: f32,
    // 10-inch steel gong motor bell (10 inharmonic modes + striker impact)
    modes: [Res; 10],
    weights: [f32; 10],
    strike: f32,
    strike_jitter: f32,
    clack_left: u32,
    clack_len: u32,
    clack_res: Res,
    bell_motor: f32,
    room: Room,
    rng: Rng,
}

impl Siren {
    fn new(sr: f32, model: SirenModel, warble: bool) -> Siren {
        let mode = |f: f32, decay_s: f32| Res::new(sr, f, PI * f * decay_s);
        let (hp_f, lp_f) = match model {
            SirenModel::MasterBlaster => (95.0, 7600.0),
            SirenModel::MotorBell => (220.0, 9500.0),
            SirenModel::Rx7c => (500.0, 9800.0),
            SirenModel::Wave2F => (320.0, 8200.0),
        };
        Siren {
            model,
            warble,
            sr,
            phase: 0.0,
            sweep: 0.0,
            amp: 0.0,
            hp: OnePole::new(sr, hp_f),
            lp: OnePole::new(sr, lp_f),
            // Master Blaster
            w: 0.0,
            theta: 0.0,
            shaft: 0.0,
            air: Res::new(sr, 1640.0, 3.6),
            hum: 0.0,
            count: 0,
            // RX-7C
            rx_t: 0.0,
            // 10-inch steel gong modes (with natural split-mode beating pairs)
            modes: [
                mode(538.0, 2.20),
                mode(544.0, 2.10),
                mode(1185.0, 1.35),
                mode(1199.0, 1.25),
                mode(2040.0, 0.85),
                mode(2068.0, 0.80),
                mode(3090.0, 0.55),
                mode(3940.0, 0.38),
                mode(4810.0, 0.26),
                mode(5690.0, 0.18),
            ],
            weights: [0.95, 0.82, 0.72, 0.60, 0.56, 0.45, 0.40, 0.30, 0.22, 0.16],
            strike: 0.0,
            strike_jitter: 1.0,
            clack_left: 0,
            clack_len: ms(sr, 2).max(4),
            clack_res: Res::new(sr, 3450.0, 1.6),
            bell_motor: 0.0,
            room: Room::new(sr),
            rng: Rng(0x2545_F491),
        }
    }

    fn quiet(&self) -> bool {
        let room_quiet = self.room.energy() < 1e-4;
        match self.model {
            SirenModel::MasterBlaster => self.w < 0.001 && room_quiet,
            SirenModel::MotorBell => self.w < 0.01 && self.modes.iter().all(|m| m.energy() < 1e-4) && room_quiet,
            SirenModel::Rx7c => self.amp < 1e-4 && room_quiet,
            SirenModel::Wave2F => self.amp < 1e-4 && room_quiet,
        }
    }

    fn next(&mut self, power: bool) -> f32 {
        let raw = match self.model {
            SirenModel::Wave2F => self.wave(power),
            SirenModel::MasterBlaster => self.motor(power),
            SirenModel::Rx7c => self.rx7c(power),
            SirenModel::MotorBell => self.bell(power),
        };
        self.room.tick(raw)
    }

    /// Honeywell / Ademco WAVE2 horn siren, rebuilt from a recording of a real
    /// one. Steady: a pure 2082 Hz. Warble: 8.4 sweeps a second, each rising
    /// like a charging RC circuit from about 1570 to 2230 Hz in 104 ms, then
    /// dropping back in 15 ms.
    fn wave(&mut self, power: bool) -> f32 {
        let dt = 1.0 / self.sr;
        let target = if power { 1.0 } else { 0.0 };
        let rate = if power { 1.0 - (-dt * 220.0).exp() } else { 1.0 - (-dt * 45.0).exp() };
        self.amp += (target - self.amp) * rate;
        if !power && self.amp < 1e-4 {
            self.amp = 0.0;
            self.sweep = 0.0;
            return 0.0;
        }
        let (f, g) = if self.warble {
            self.sweep += dt / WAVE_SWEEP_S;
            if self.sweep >= 1.0 {
                self.sweep -= 1.0;
            }
            let t = self.sweep * WAVE_SWEEP_S;
            let rise = |t: f32| 2444.0 - 873.0 * (-t / 0.070).exp();
            let f = if t < 0.104 {
                rise(t)
            } else {
                let u = ((t - 0.104) / 0.015).min(1.0);
                rise(0.104) + (1571.0 - rise(0.104)) * u
            };
            // Harmonics 25, 22, 43 and 52 dB down.
            (f, [1.0, 0.056, 0.079, 0.0071, 0.0025, 0.0])
        } else {
            // Harmonics 30, 47, 45, 52 and 48 dB down.
            (2082.0, [1.0, 0.0316, 0.0045, 0.0056, 0.0025, 0.004])
        };
        self.phase = (self.phase + f * dt).fract();
        additive(self.phase, &g) * self.amp * 0.85
    }

    /// Klaxon Master Blaster, a motor-driven siren, rebuilt from a recording
    /// of a real one. Its rotor sings at 1833 Hz, getting to 85% of that in
    /// about a quarter of a second and the last few percent more slowly, and
    /// it coasts down over a few seconds when the power goes. Its 2nd and 3rd
    /// harmonics are nearly as loud as the note itself.
    fn motor(&mut self, power: bool) -> f32 {
        let dt = 1.0 / self.sr;
        if power {
            let k = if self.w < 0.8 { 7.0 } else { 7.0 - 22.5 * (self.w - 0.8) };
            self.w = (self.w + k * (1.0 - self.w) * dt).min(1.0);
        } else {
            self.w = (self.w - (0.8 * self.w + 0.012) * dt).max(0.0);
        }
        if self.w < 0.001 {
            return 0.0;
        }
        let f = 1833.0 * self.w;
        self.theta = (self.theta + f * dt).fract();
        // The recording holds within about 1%, so only a slight shaft wobble.
        self.shaft = (self.shaft + (f / 12.0) * dt).fract();
        let wobble = 1.0 + 0.01 * (TAU * self.shaft).sin();
        let chop = additive(self.theta, &[1.0, 0.89, 0.89, 0.22, 0.32, 0.1]);
        // Air rushing through the rotor slots.
        self.count = self.count.wrapping_add(1);
        if self.count % 48 == 0 {
            self.air.tune(self.sr, (f * 1.04).max(90.0), 3.4);
        }
        let white = self.rng.next();
        let air_rush = self.air.tick(white) * 2.2 + white * 0.14;
        // A little mains hum from the motor while it's powered.
        self.hum = (self.hum + 120.0 * dt).fract();
        let hum = if power { (TAU * self.hum).sin() * 0.03 * self.w } else { 0.0 };
        let level = self.w.powf(1.65);
        ((chop * wobble * 0.27 + air_rush * 0.03) * level + hum).clamp(-1.0, 1.0)
    }

    /// ADT RX-7C piezo sounder, rebuilt from a recording of a real one. It
    /// warbles around 3555 Hz, 230 Hz either way, 15.4 times a second, and
    /// every start swoops up from about 2600 Hz as its oscillator wakes.
    fn rx7c(&mut self, power: bool) -> f32 {
        let dt = 1.0 / self.sr;
        let target = if power { 1.0 } else { 0.0 };
        let rate = if power { 1.0 - (-dt * 260.0).exp() } else { 1.0 - (-dt * 70.0).exp() };
        if power && self.amp < 1e-3 {
            self.rx_t = 0.0;
        }
        self.amp += (target - self.amp) * rate;
        if !power && self.amp < 1e-4 {
            self.amp = 0.0;
            return 0.0;
        }
        if self.rx_t < 1.0 {
            self.rx_t += dt;
        }
        self.sweep = (self.sweep + 15.4 * dt).fract();
        let tri = 1.0 - 4.0 * (self.sweep - 0.5).abs();
        let f = 3555.0 + 230.0 * tri - 955.0 * (-self.rx_t / 0.015).exp();
        self.phase = (self.phase + f * dt).fract();
        // Harmonics 10, 19 and 15 dB down: a bright piezo.
        additive(self.phase, &[1.0, 0.316, 0.112, 0.178, 0.0, 0.0]) * self.amp * 0.6
    }

    /// 10-inch steel gong motor bell (Amseco MBL-10 / Potter MB-10 style).
    fn bell(&mut self, power: bool) -> f32 {
        let dt = 1.0 / self.sr;
        if power {
            self.w += (1.0 - self.w) * 10.5 * dt;
        } else {
            self.w = (self.w - self.w * 6.5 * dt).max(0.0);
        }
        if self.w > 0.09 {
            self.strike += 16.5 * self.w * self.strike_jitter * dt;
            if self.strike >= 1.0 {
                self.strike -= 1.0;
                // Organic ±4% cam timing jitter + ±10% strike velocity variance.
                self.strike_jitter = 1.0 + 0.04 * self.rng.next();
                let vel = (0.58 + 0.42 * self.w) * (0.90 + 0.10 * self.rng.next());
                for (i, (m, wgt)) in self.modes.iter_mut().zip(self.weights.iter()).enumerate() {
                    // Slight per-mode phase/strike variation avoids robotic comb ringing.
                    let var = 1.0 + 0.08 * self.rng.next() * ((i % 3) as f32 * 0.5 + 0.5);
                    m.strike(vel * wgt * var);
                }
                self.clack_left = self.clack_len;
            }
        }
        // Steel-on-steel hammer impact transient ("clack") on every blow.
        let impact = if self.clack_left > 0 {
            self.clack_left -= 1;
            let env = self.clack_left as f32 / self.clack_len as f32;
            let burst = self.rng.next() * env * env * 1.4;
            self.clack_res.tick(burst) * 1.15
        } else {
            self.clack_res.tick(0.0) * 0.6
        };
        let mut gong = 0.0;
        for m in self.modes.iter_mut() {
            gong += m.tick(0.0);
        }
        // Subtle DC motor & cam gearbox whirr while powered.
        self.bell_motor = (self.bell_motor + 185.0 * dt).fract();
        let mech = if power {
            ((TAU * self.bell_motor).sin() * 0.025 + self.rng.next() * 0.012) * self.w
        } else {
            0.0
        };
        let y = self.lp.lowpass(self.hp.highpass(gong * 0.62 + impact * 0.48 + mech));
        (y * 1.45).tanh() * 0.90
    }
}

#[derive(Clone, Copy)]
struct Seg {
    on: u32,
    off: u32,
    freq: f32,
    decay: f32,
    amp: f32,
    /// Part of a keypress beep, which later sequences queue behind.
    key: bool,
}

pub struct Synth {
    sr: f32,
    voice: KeypadVoice,
    buzzer_level: u8,
    piezo: Piezo,
    voice_player: VoicePlayer,
    sounder: Gate,
    segs: VecDeque<Seg>,
    seg: Option<(Seg, u32, f32)>,
    bell: BellPattern,
    bell_pos: u32,
    squawks: VecDeque<(u32, u32)>,
    siren: Siren,
    preview: Option<(Siren, u32)>,
    vol_master: f32,
    vol_siren: f32,
    vol_keypad: f32,
    vol_line: f32,
    line: VecDeque<ToneSeg>,
    line_seg: ToneSeg,
    line_left: u32,
    lp1: f32,
    lp2: f32,
    line_hp: OnePole,
    line_lp: OnePole,
    ring_on: bool,
    ring_pos: u32,
    ring_phase: f32,
    click_left: u32,
    click_len: u32,
    click_res: Res,
    rng: Rng,
    keepalive: f32,
    ka_phase: f32,
    wake_left: u32,
}

impl Synth {
    pub fn new(sr: f32) -> Synth {
        let sr = sr.max(8000.0);
        Synth {
            sr,
            voice: KeypadVoice::Pk5500,
            buzzer_level: 14,
            piezo: Piezo::new(sr, KeypadVoice::Pk5500, 14),
            voice_player: VoicePlayer::new(sr),
            sounder: Gate { pat: Pattern::Off, pos: 0 },
            segs: VecDeque::new(),
            seg: None,
            bell: BellPattern::Off,
            bell_pos: 0,
            squawks: VecDeque::new(),
            siren: Siren::new(sr, SirenModel::Wave2F, true),
            preview: None,
            vol_master: 0.8,
            vol_siren: 0.45,
            vol_keypad: 0.7,
            vol_line: 0.0,
            line: VecDeque::new(),
            line_seg: ToneSeg { f1: 0.0, f2: 0.0, ms: 0, level: 0.0 },
            line_left: 0,
            lp1: 0.0,
            lp2: 0.0,
            line_hp: OnePole::new(sr, 300.0),
            line_lp: OnePole::new(sr, 3400.0),
            ring_on: false,
            ring_pos: 0,
            ring_phase: 0.0,
            click_left: 0,
            click_len: ms(sr, 6).max(1),
            click_res: Res::new(sr, 1700.0, 2.0),
            rng: Rng(0x9E37_79B9),
            keepalive: 1.0 / 32_000.0,
            ka_phase: 0.0,
            wake_left: 0,
        }
    }

    fn decay_for(&self, tau_s: f32) -> f32 {
        (-1.0 / (tau_s * self.sr)).exp()
    }

    fn push_chime(&mut self, k: ChimeKind) {
        let f0 = self.piezo.tone;
        let sr = self.sr;
        match k {
            ChimeKind::Beeps(n) => {
                for _ in 0..n.max(1) {
                    self.segs.push_back(Seg { on: ms(sr, 68), off: ms(sr, 68), freq: f0, decay: 1.0, amp: 1.0, key: false });
                }
            }
            ChimeKind::BingBing => {
                let d = self.decay_for(0.20);
                for _ in 0..2 {
                    self.segs.push_back(Seg { on: ms(sr, 250), off: ms(sr, 105), freq: 2793.0, decay: d, amp: 1.0, key: false });
                }
            }
            ChimeKind::DingDong => {
                let d1 = self.decay_for(0.28);
                let d2 = self.decay_for(0.44);
                self.segs.push_back(Seg { on: ms(sr, 360), off: ms(sr, 35), freq: 2637.0, decay: d1, amp: 1.0, key: false });
                self.segs.push_back(Seg { on: ms(sr, 640), off: 0, freq: 2093.0, decay: d2, amp: 1.0, key: false });
            }
            ChimeKind::AlarmTone => {
                for i in 0..12 {
                    let f = if i % 2 == 0 { 3200.0 } else { 2450.0 };
                    self.segs.push_back(Seg { on: ms(sr, 52), off: 0, freq: f, decay: 1.0, amp: 1.0, key: false });
                }
            }
        }
    }

    pub fn apply(&mut self, cmd: AudioCmd) {
        match cmd {
            AudioCmd::Voice(v) => {
                if v != self.voice {
                    self.voice = v;
                    self.piezo = Piezo::new(self.sr, v, self.buzzer_level);
                }
            }
            AudioCmd::BuzzerLevel(level) => {
                self.buzzer_level = level.min(20);
                self.piezo.set_level(self.buzzer_level);
            }
            AudioCmd::VoiceLevel(level) => {
                self.voice_player.set_level(level);
            }
            AudioCmd::Speak(tokens) => {
                self.voice_player.speak(tokens);
            }
            AudioCmd::Sounder(p) => self.sounder.set(p),
            AudioCmd::KeyBeep => {
                // A keypress is ~80 ms on PC1550, ~65 ms on PC1832, or a crisp ~46 ms
                // chirp on Vista 6160V. During any continuous sounder tone (steady
                // entry, last-10s pulsing entry, or HiLo alarm), dip around the beep
                // so every digit cuts cleanly through. Pressing a key also fades out
                // any in-progress 6160V voice phrase cleanly instead of hard-pausing
                // mid-syllable and resuming with a click.
                self.segs.clear();
                self.seg = None;
                if self.voice_player.is_speaking() {
                    self.voice_player.stop();
                }
                let f = self.piezo.tone;
                // 46 ms on the 6160, measured from a recording.
                let dur = match self.voice {
                    KeypadVoice::Vista6160 => 46,
                    // Measured from a real PowerSeries keypad.
                    KeypadVoice::Pk5500 => 65,
                    KeypadVoice::Classic => 80,
                };
                if matches!(
                    self.sounder.pat,
                    Pattern::Steady | Pattern::ShortShortLong { .. } | Pattern::HiLo { .. }
                ) {
                    self.segs.push_back(Seg { on: 0, off: ms(self.sr, 30).max(1), freq: f, decay: 1.0, amp: 0.0, key: true });
                    self.segs.push_back(Seg { on: ms(self.sr, dur).max(1), off: ms(self.sr, 40).max(1), freq: f, decay: 1.0, amp: 1.0, key: true });
                } else {
                    self.segs.push_back(Seg { on: ms(self.sr, dur).max(1), off: ms(self.sr, 22), freq: f, decay: 1.0, amp: 1.0, key: true });
                }
            }
            AudioCmd::Tone { hz, on_ms, off_ms, delay_ms } => {
                self.drop_all_but_keys();
                if delay_ms > 0 {
                    // The panel's answer comes a moment after the key.
                    self.segs.push_back(Seg { on: 0, off: ms(self.sr, delay_ms as u32), freq: hz as f32, decay: 1.0, amp: 0.0, key: false });
                }
                self.segs.push_back(Seg {
                    on: ms(self.sr, on_ms as u32).max(1),
                    off: ms(self.sr, off_ms as u32),
                    freq: hz as f32,
                    decay: 1.0,
                    amp: 1.0,
                    key: false,
                });
            }
            AudioCmd::Beeps { count, on_ms, off_ms } => {
                // One sounder per keypad: a new sequence replaces the last one,
                // but a keypress beep that's still sounding finishes first. That
                // way the last digit of a code is heard, then "beep beep beep".
                self.drop_all_but_keys();
                let f = self.piezo.tone;
                let n = count.max(1);
                for i in 0..n {
                    // Add a crisp trailing gap on the final beep if the sounder is running.
                    let off = if off_ms > 0 {
                        ms(self.sr, off_ms as u32)
                    } else if i + 1 < n {
                        ms(self.sr, 60)
                    } else {
                        ms(self.sr, 40)
                    };
                    self.segs.push_back(Seg {
                        on: ms(self.sr, on_ms as u32).max(1),
                        off,
                        freq: f,
                        decay: 1.0,
                        amp: 1.0,
                        key: false,
                    });
                }
            }
            AudioCmd::Chime(k) => {
                self.drop_all_but_keys();
                self.push_chime(k);
            }
            AudioCmd::Bell(b) => {
                if b != self.bell {
                    self.bell = b;
                    self.bell_pos = 0;
                }
            }
            AudioCmd::Squawk(n) => {
                self.squawks.clear();
                for _ in 0..n {
                    self.squawks.push_back((ms(self.sr, 65), ms(self.sr, 235)));
                }
            }
            AudioCmd::Siren { model, warble } => {
                if model != self.siren.model || warble != self.siren.warble {
                    self.siren = Siren::new(self.sr, model, warble);
                }
            }
            AudioCmd::Volumes { master, siren, keypad, line } => {
                self.vol_master = master.clamp(0.0, 1.0);
                self.vol_siren = siren.clamp(0.0, 1.0);
                self.vol_keypad = keypad.clamp(0.0, 1.0);
                self.vol_line = line.clamp(0.0, 1.0);
            }
            AudioCmd::Line(segs) => {
                self.line = segs.into_iter().collect();
                self.line_left = 0;
            }
            AudioCmd::Ring(on) => {
                self.ring_on = on;
                self.ring_pos = 0;
            }
            AudioCmd::Click => self.click_left = self.click_len,
            AudioCmd::Preview { model, warble, ms: dur } => {
                self.preview = Some((Siren::new(self.sr, model, warble), ms(self.sr, dur)));
            }
            AudioCmd::Wake => self.wake_left = ms(self.sr, 2500),
            AudioCmd::Silence => {
                self.sounder.set(Pattern::Off);
                self.segs.clear();
                self.seg = None;
                self.voice_player.stop();
                self.bell = BellPattern::Off;
                self.squawks.clear();
                self.line.clear();
                self.line_left = 0;
                self.ring_on = false;
                self.preview = None;
            }
        }
    }

    /// Clears queued sounds except a keypress beep, which plays out first.
    fn drop_all_but_keys(&mut self) {
        self.segs.retain(|s| s.key);
        if !self.seg.map_or(false, |(s, _, _)| s.key) {
            self.seg = None;
        }
    }

    fn seg_next(&mut self) -> Option<(bool, f32, f32)> {
        if self.seg.is_none() {
            self.seg = self.segs.pop_front().map(|s| (s, 0u32, 1.0f32));
        }
        let (s, pos, env) = self.seg.as_mut()?;
        let p = *pos;
        *pos += 1;
        let e = *env;
        *env *= s.decay;
        let result = (p < s.on, s.freq, s.amp * e);
        let done = p + 1 >= s.on + s.off;
        if done {
            self.seg = None;
        }
        Some(result)
    }

    fn bell_gate(&mut self) -> bool {
        let sec = self.sr as u32;
        let pos = self.bell_pos;
        self.bell_pos = self.bell_pos.wrapping_add(1);
        match self.bell {
            BellPattern::Off => false,
            BellPattern::Steady => true,
            BellPattern::Pulsed => (pos % (2 * sec)) < sec,
            BellPattern::Temporal3 => {
                // NFPA 72 Fire: 0.5s ON, 0.5s OFF x3, then 1.5s OFF (4.0s total).
                let t = pos % (4 * sec);
                t < 3 * sec && (t % sec) < sec / 2
            }
            BellPattern::Temporal4 => {
                // NFPA 720 Carbon Monoxide: 4x (100ms ON, 100ms OFF), then 5.0s OFF (5.8s total).
                let unit = ms(self.sr, 200).max(2);
                let on = ms(self.sr, 100).max(1);
                let burst = unit * 4;
                let cycle = burst + ms(self.sr, 5000);
                let t = pos % cycle.max(1);
                t < burst && (t % unit) < on
            }
        }
    }

    fn squawk_next(&mut self) -> bool {
        let Some(front) = self.squawks.front_mut() else {
            return false;
        };
        if front.0 > 0 {
            front.0 -= 1;
            return true;
        }
        if front.1 > 0 {
            front.1 -= 1;
            return false;
        }
        self.squawks.pop_front();
        false
    }

    fn line_next(&mut self) -> f32 {
        if self.line_left == 0 {
            match self.line.pop_front() {
                Some(s) => {
                    self.line_seg = s;
                    self.line_left = ms(self.sr, s.ms).max(1);
                }
                None => return 0.0,
            }
        }
        self.line_left -= 1;
        let s = self.line_seg;
        let mut v = 0.0;
        if s.f1 > 0.0 {
            self.lp1 = (self.lp1 + s.f1 / self.sr).fract();
            v += (TAU * self.lp1).sin();
        }
        if s.f2 > 0.0 {
            self.lp2 = (self.lp2 + s.f2 / self.sr).fract();
            v += (TAU * self.lp2).sin();
        }
        v = v * s.level * 0.5 + self.rng.next() * 0.004;
        let x = self.line_hp.highpass(v);
        self.line_lp.lowpass(x)
    }

    fn ring_next(&mut self) -> f32 {
        if !self.ring_on {
            return 0.0;
        }
        let cycle = (6.0 * self.sr) as u32;
        self.ring_pos = (self.ring_pos + 1) % cycle.max(1);
        let t = self.ring_pos as f32 / self.sr;
        if t >= 2.0 {
            return 0.0;
        }
        let f = if ((t * 20.0) as u32) % 2 == 0 { 1046.5 } else { 1318.5 };
        self.ring_phase = (self.ring_phase + f / self.sr).fract();
        let env = (t * 40.0).min(1.0) * ((2.0 - t) * 40.0).min(1.0);
        (TAU * self.ring_phase).sin() * 0.18 * env
    }

    fn click_next(&mut self) -> f32 {
        if self.click_left == 0 {
            return self.click_res.tick(0.0);
        }
        self.click_left -= 1;
        let n = self.rng.next() * (self.click_left as f32 / self.click_len as f32);
        self.click_res.tick(n) * 2.0
    }

    pub fn next(&mut self) -> f32 {
        let (bg_on, bg_freq) = self.sounder.next(self.sr);
        let seg_out = self.seg_next();
        let has_seg = seg_out.is_some();
        let (on, freq, amp) = match seg_out {
            // Active keypress / confirmation / chime sequence takes exclusive
            // control of the single keypad piezo (including its silent gaps),
            // so rapid beeps and keypresses cut cleanly through steady tones.
            Some((gate, f, a)) => (gate, f, a),
            None => (bg_on, bg_freq.unwrap_or(self.piezo.tone), 1.0),
        };
        let power = self.bell_gate() | self.squawk_next();
        let speaking = self.voice_player.is_speaking();
        let voice_active = self.voice_player.is_active();
        // Fast path when every voice is silent: skip 48 kHz resonator and tanh
        // math so idle audio uses near-zero CPU and battery.
        let idle = !on
            && !voice_active
            && self.piezo.quiet()
            && !power
            && self.siren.quiet()
            && self.preview.is_none()
            && self.line_left == 0
            && self.line.is_empty()
            && !self.ring_on
            && self.click_left == 0
            && self.click_res.energy() < 1e-5;
        let mix = if idle {
            0.0
        } else {
            // 6160V voice priority: let short confirmation / chime / keypress beeps
            // finish playing cleanly FIRST, then speak while ducking only continuous
            // background sounder tones (exit/entry/alarm). Any active 4 ms stop-fade
            // always drains smoothly so interrupting speech never clicks.
            let keypad_on = if has_seg { on } else { on && !speaking };
            let keypad = self.piezo.next(keypad_on, freq, amp) * self.vol_keypad;
            let spoken = if has_seg && speaking { 0.0 } else { self.voice_player.next() * self.vol_keypad };
            let siren = self.siren.next(power) * self.vol_siren;
            let mut preview = 0.0;
            if let Some((s, left)) = self.preview.as_mut() {
                let on = *left > 0;
                if on {
                    *left -= 1;
                }
                preview = s.next(on) * self.vol_siren;
            }
            let finished = matches!(&self.preview, Some((s, left)) if *left == 0 && s.quiet());
            if finished {
                self.preview = None;
            }
            let line = self.line_next() * self.vol_line;
            let ring = self.ring_next();
            let click = self.click_next() * self.vol_keypad * 0.12;
            (keypad * 0.8 + spoken + siren + preview + line + ring + click) * self.vol_master
        };
        // Keeps the output awake. Silence lets many laptop speakers and USB,
        // HDMI and Bluetooth outputs power their amplifier down, and waking it
        // swallows the next short keypad beep. A 30 Hz tone at about -66 dBFS
        // passes every DAC's filters (a signal at the Nyquist frequency
        // doesn't) yet sits far below hearing at that pitch on any speaker or
        // headphone. Moving toward the keypad lifts it to about -54 dBFS for
        // a couple of seconds, for outputs that sleep more deeply. The tiny
        // alternating step keeps the stream from ever being exact zeros.
        self.ka_phase += 30.0 / self.sr;
        if self.ka_phase >= 1.0 {
            self.ka_phase -= 1.0;
        }
        let ka_level = if self.wake_left > 0 {
            self.wake_left -= 1;
            2.0e-3
        } else {
            5.0e-4
        };
        self.keepalive = -self.keepalive;
        mix.clamp(-0.9999, 0.9999) + (TAU * self.ka_phase).sin() * ka_level + self.keepalive
    }
}
